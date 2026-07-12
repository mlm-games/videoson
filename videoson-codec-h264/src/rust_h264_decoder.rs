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
    in_idr_picture: bool,
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
            return Err(VideosonError::InvalidData(
                "H.264: decoded Y plane smaller than expected dimensions",
            ));
        }
        let y_visible = &f.y[..w * h];
        let poc = f.pic_order_cnt;

        let is_mono = f.u.is_empty() && f.v.is_empty();
        let frame = if is_mono {
            VideoFrame::new_mono_u8(f.width, f.height, w, y_visible.to_vec()).with_pts(pts)
        } else {
            let cw = (w + 1) / 2;
            let ch = (h + 1) / 2;
            let chroma_samples = cw * ch;

            // Non-4:2:0 detection: if U/V plane byte count implies a chroma
            // height greater than ch (4:2:2 or 4:4:4), reject.
            if (!f.u.is_empty() && (f.u.len() / cw.max(1)) > ch)
                || (!f.v.is_empty() && (f.v.len() / cw.max(1)) > ch)
            {
                return Err(VideosonError::Unsupported(
                    "H.264: only 4:2:0 chroma is supported",
                ));
            }

            // If chroma is truncated, zero-pad to expected size rather than
            // rejecting the frame. Some encoders (e.g. less_avc with odd
            // dimensions) don't properly signal monochrome, causing the
            // decoder to return incomplete chroma planes.
            let mut u_buf = alloc::vec![0u8; chroma_samples];
            let u_copy = core::cmp::min(f.u.len(), chroma_samples);
            u_buf[..u_copy].copy_from_slice(&f.u[..u_copy]);

            let mut v_buf = alloc::vec![0u8; chroma_samples];
            let v_copy = core::cmp::min(f.v.len(), chroma_samples);
            v_buf[..v_copy].copy_from_slice(&f.v[..v_copy]);

            if self.wants_nv12() {
                let uv = interleave_uv_nv12(&u_buf, cw, &v_buf, cw, cw, ch)?;
                VideoFrame::new_nv12_u8(f.width, f.height, w, cw * 2, y_visible.to_vec(), uv)
                    .with_pts(pts)
            } else {
                VideoFrame::new_yuv420_u8(
                    f.width, f.height, w, cw, cw,
                    y_visible.to_vec(), u_buf, v_buf,
                )
                .with_pts(pts)
            }
        };

        let gop = self.idr_count;
        self.pending.push(OrderedFrame { gop, poc, frame });
        Ok(())
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
                // decode_nal returns the PREVIOUS completed picture.
                // Its PTS was stored when its first VCL slice was fed.
                self.push_frame(frame, self.pending_pts)?;
            }
            Ok(None) => {}
            Err(e) => return Err(Self::map_err(e)),
        }

        // IDR handling fires once per IDR access unit, not once per slice NAL.
        // A multi-slice IDR stream may have several SliceIdr NALs; only the
        // first one should flush the GOP and increment the counter.
        if is_idr && !self.in_idr_picture {
            if self.pending.iter().any(|f| f.gop == self.idr_count) {
                self.flush_pending_for_gop(self.idr_count);
            }
            self.idr_count += 1;
            self.in_idr_picture = true;
        } else if is_vcl_nal(nal) && !is_idr {
            // First VCL NAL of a non-IDR picture ends the IDR access unit.
            self.in_idr_picture = false;
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
            in_idr_picture: false,
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
        if let Some(frame) = self.dec.flush() {
            self.push_frame(frame, self.pending_pts)?;
        }
        self.drain_all_pending();
        Ok(())
    }

    fn reset(&mut self) {
        self.dec = Inner::new();
        self.queued.clear();
        self.pending.clear();
        self.idr_count = 0;
        self.in_idr_picture = false;
        self.pending_pts = None;
        self.avcc_length_size = None;

        if matches!(self.nal_format, NalFormat::Avcc { .. }) {
            let _ = self.prime_with_avcc_extradata();
        }
    }

    fn output_format(&self) -> VideoOutputFormat {
        // NOTE: if the stream is monochrome the returned frame will be
        // PixelFormat::Gray, not Yuv420. output_format() returns the
        // *requested* format; check the frame's pixfmt for actual format.
        VideoOutputFormat::Yuv420
    }
}
