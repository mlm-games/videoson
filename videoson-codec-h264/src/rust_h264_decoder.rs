extern crate alloc;

use alloc::collections::VecDeque;
use alloc::string::ToString;
use alloc::vec::Vec;

use rust_h264::decoder::Decoder as Inner;
use rust_h264::nal::{NalUnit, NalUnitType, parse_annex_b, parse_avcc, parse_avcc_config};

use videoson_core::{
    CodecType, NalFormat, Packet, Result, VideoCodecParams, VideoDecoder, VideoDecoderOptions,
    VideoFrame, VideoOutputFormat, VideosonError, interleave_uv_nv12,
};

struct OrderedFrame {
    gop: u32,
    poc: i32,
    frame: VideoFrame,
}

fn is_vcl_nal(nal: &NalUnit<'_>) -> bool {
    matches!(
        nal.nal_unit_type,
        NalUnitType::Slice | NalUnitType::SliceIdr
    )
}

pub struct RustH264Decoder {
    params: VideoCodecParams,
    opts: VideoDecoderOptions,
    nal_format: NalFormat,
    avcc_length_size: Option<usize>,
    dec: Inner,
    queued: VecDeque<VideoFrame>,
    pending: Vec<OrderedFrame>,
    idr_count: u32,
    last_emitted_poc: Option<i32>,
    pending_pts: Option<i64>,
}

impl RustH264Decoder {
    fn map_err<E: core::fmt::Display>(e: E) -> VideosonError {
        VideosonError::Message(e.to_string().into())
    }

    fn wants_nv12(&self) -> bool {
        matches!(self.opts.output_format, VideoOutputFormat::Nv12)
    }

    fn push_frame(&mut self, f: rust_h264::decoder::Frame, pts: Option<i64>) -> Result<()> {
        let w = f.width as usize;
        let h = f.height as usize;

        if f.y.len() < w * h {
            return Ok(());
        }
        let y_visible = &f.y[..w * h];
        let poc = f.pic_order_cnt;

        let is_mono = f.u.is_empty();
        let frame = if is_mono {
            VideoFrame::new_mono_u8(f.width, f.height, w, y_visible.to_vec()).with_pts(pts)
        } else {
            let cw = (w + 1) / 2;
            let ch = (h + 1) / 2;
            let u_visible = if f.u.len() >= cw * ch {
                &f.u[..cw * ch]
            } else {
                &f.u
            };
            let v_visible = if f.v.len() >= cw * ch {
                &f.v[..cw * ch]
            } else {
                &f.v
            };

            if self.wants_nv12() {
                let uv = interleave_uv_nv12(u_visible, cw, v_visible, cw, cw, ch)?;
                VideoFrame::new_nv12_u8(f.width, f.height, w, cw * 2, y_visible.to_vec(), uv)
                    .with_pts(pts)
            } else {
                VideoFrame::new_yuv420_u8(
                    f.width,
                    f.height,
                    w,
                    cw,
                    cw,
                    y_visible.to_vec(),
                    u_visible.to_vec(),
                    v_visible.to_vec(),
                )
                .with_pts(pts)
            }
        };

        let gop = self.idr_count;
        self.pending.push(OrderedFrame { gop, poc, frame });
        self.try_drain_pending();
        Ok(())
    }

    fn try_drain_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        self.pending.sort_by(|a, b| a.poc.cmp(&b.poc));

        while !self.pending.is_empty() {
            let next_poc = self.pending[0].poc;
            let can_emit = match self.last_emitted_poc {
                None => next_poc == 0,
                Some(last) => next_poc > last,
            };
            if !can_emit {
                break;
            }
            let ordered = self.pending.remove(0);
            self.last_emitted_poc = Some(ordered.poc);
            self.queued.push_back(ordered.frame);
        }
    }

    fn flush_pending_for_gop(&mut self, gop: u32) {
        let mut gop_frames = Vec::new();
        let mut remaining = Vec::new();
        for f in self.pending.drain(..) {
            if f.gop == gop {
                gop_frames.push(f);
            } else {
                remaining.push(f);
            }
        }
        self.pending = remaining;

        gop_frames.sort_by(|a, b| a.poc.cmp(&b.poc));
        for ordered in gop_frames {
            self.queued.push_back(ordered.frame);
        }
    }

    fn drain_all_pending(&mut self) {
        self.pending.sort_by(|a, b| match a.gop.cmp(&b.gop) {
            core::cmp::Ordering::Equal => a.poc.cmp(&b.poc),
            other => other,
        });
        for ordered in self.pending.drain(..) {
            self.queued.push_back(ordered.frame);
        }
    }

    fn feed_nal(&mut self, nal: &NalUnit<'_>, pts: Option<i64>) -> Result<()> {
        let is_idr = matches!(nal.nal_unit_type, NalUnitType::SliceIdr);

        match self.dec.decode_nal(nal) {
            Ok(Some(frame)) => {
                // frame is the PREVIOUS picture that just completed.
                // Its PTS was stored in pending_pts when its first slice was fed.
                self.push_frame(frame, self.pending_pts)?;
            }
            Ok(None) => {}
            Err(e) => return Err(Self::map_err(e)),
        }

        if is_idr {
            if self.idr_count > 0 {
                let prev_gop = self.idr_count - 1;
                self.flush_pending_for_gop(prev_gop);
            }
            self.idr_count += 1;
            self.last_emitted_poc = None;
        }

        // Track PTS for the picture currently being decoded.
        if is_vcl_nal(nal) {
            self.pending_pts = pts;
        }

        Ok(())
    }

    fn prime_with_avcc_extradata(&mut self) -> Result<()> {
        if self.params.extradata.is_empty() {
            return Ok(());
        }

        let payload: &[u8] =
            if self.params.extradata.len() >= 8 && &self.params.extradata[4..8] == b"avcC" {
                &self.params.extradata[8..]
            } else {
                self.params.extradata.as_slice()
            };

        let cfg = parse_avcc_config(payload).map_err(Self::map_err)?;
        self.avcc_length_size = Some(cfg.length_size);

        for nal in cfg.sps_nals.iter().chain(cfg.pps_nals.iter()) {
            let _ = self.dec.decode_nal(nal).map_err(Self::map_err)?;
        }
        Ok(())
    }

    fn parse_packet_nals<'a>(&self, data: &'a [u8]) -> Vec<NalUnit<'a>> {
        match self.nal_format {
            NalFormat::AnnexB => parse_annex_b(data),
            NalFormat::Avcc { nal_len_size } => {
                let n = self.avcc_length_size.unwrap_or(nal_len_size as usize);
                parse_avcc(data, n)
            }
            _ => parse_annex_b(data),
        }
    }
}

impl VideoDecoder for RustH264Decoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::H264 {
            return Err(VideosonError::InvalidData("params.codec is not H264"));
        }

        if matches!(opts.output_format, VideoOutputFormat::P010) {
            return Err(VideosonError::Unsupported(
                "P010 output is not supported for H.264",
            ));
        }

        let nal_format = params.nal_format.unwrap_or(NalFormat::AnnexB);

        let mut me = Self {
            params: params.clone(),
            opts: *opts,
            nal_format,
            avcc_length_size: None,
            dec: Inner::new(),
            queued: VecDeque::new(),
            pending: Vec::new(),
            idr_count: 0,
            last_emitted_poc: None,
            pending_pts: None,
        };

        if matches!(me.nal_format, NalFormat::Avcc { .. }) {
            me.prime_with_avcc_extradata()?;
        }

        Ok(me)
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let nals = self.parse_packet_nals(&packet.data);
        for nal in &nals {
            self.feed_nal(nal, packet.pts)?;
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.queued.pop_front())
    }

    fn send_eos(&mut self) -> Result<()> {
        // Flush the last pending picture from the decoder
        if let Some(frame) = self.dec.flush() {
            self.push_frame(frame, self.pending_pts)?;
        }
        // Drain all remaining pending frames sorted by POC (display order)
        self.drain_all_pending();
        Ok(())
    }

    fn reset(&mut self) {
        self.dec = Inner::new();
        self.queued.clear();
        self.pending.clear();
        self.idr_count = 0;
        self.last_emitted_poc = None;
        self.pending_pts = None;
        self.avcc_length_size = None;

        if matches!(self.nal_format, NalFormat::Avcc { .. }) {
            let _ = self.prime_with_avcc_extradata();
        }
    }

    fn output_format(&self) -> VideoOutputFormat {
        match self.opts.output_format {
            VideoOutputFormat::Nv12 => VideoOutputFormat::Nv12,
            VideoOutputFormat::Native | VideoOutputFormat::Yuv420 => VideoOutputFormat::Yuv420,
            VideoOutputFormat::P010 => VideoOutputFormat::Yuv420,
        }
    }
}
