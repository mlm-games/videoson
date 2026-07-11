// Output: videoson_core::VideoFrame
// - default / Yuv420: planar YUV420, tightly packed
// - Nv12: semi-planar Y + UV interleaved, tightly packed

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::string::ToString;
use alloc::vec::Vec;

use rust_h264::decoder::OrderedDecoder as Inner;
use rust_h264::nal::{parse_annex_b, parse_avcc, parse_avcc_config, NalUnit};

use videoson_core::{
    interleave_uv_nv12, CodecType, NalFormat, Packet, Result, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFrame, VideoOutputFormat, VideosonError,
};

pub struct RustH264Decoder {
    params: VideoCodecParams,
    opts: VideoDecoderOptions,
    nal_format: NalFormat,
    avcc_length_size: Option<usize>,
    dec: Inner,
    out: VecDeque<VideoFrame>,
    pts_queue: VecDeque<i64>,
    last_packet_pts: Option<i64>,
}

impl RustH264Decoder {
    fn map_err<E: core::fmt::Display>(e: E) -> VideosonError {
        VideosonError::Message(e.to_string().into())
    }

    fn wants_nv12(&self) -> bool {
        matches!(self.opts.output_format, VideoOutputFormat::Nv12)
    }

    fn push_frame(&mut self, f: rust_h264::decoder::Frame) {
        let w = f.width as usize;
        let h = f.height as usize;

        if f.y.len() < w * h {
            return;
        }
        let y_visible = &f.y[..w * h];
        let frame_pts = self.pts_queue.pop_front().or(self.last_packet_pts);

        // rust_h264 may leave empty/zero chroma for mono.
        let cw_m = (w + 1) / 2;
        let ch_m = (h + 1) / 2;
        let is_mono = f.u.is_empty()
            || (f.u.len() <= cw_m * ch_m
                && f.u.iter().all(|&b| b == 0)
                && f.v.iter().all(|&b| b == 0));

        if is_mono {
            self.out.push_back(
                VideoFrame::new_mono_u8(f.width, f.height, w, y_visible.to_vec())
                    .with_pts(frame_pts),
            );
            return;
        }

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
            let uv = interleave_uv_nv12(u_visible, cw, v_visible, cw, cw, ch);
            self.out.push_back(
                VideoFrame::new_nv12_u8(f.width, f.height, w, cw * 2, y_visible.to_vec(), uv)
                    .with_pts(frame_pts),
            );
        } else {
            self.out.push_back(
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
                .with_pts(frame_pts),
            );
        }
    }

    fn feed_nal(&mut self, nal: &NalUnit<'_>) -> Result<()> {
        match self.dec.decode_nal(nal) {
            Ok(frames) => {
                for frame in frames {
                    self.push_frame(frame);
                }
            }
            Err(e) => return Err(Self::map_err(e)),
        }
        Ok(())
    }

    fn prime_with_avcc_extradata(&mut self) -> Result<()> {
        if self.params.extradata.is_empty() {
            return Ok(());
        }

        // MP4 demuxers often include the 8-byte "avcC" box header.
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

        let nal_format = params.nal_format.unwrap_or(NalFormat::AnnexB);

        let mut me = Self {
            params: params.clone(),
            opts: *opts,
            nal_format,
            avcc_length_size: None,
            dec: Inner::new(),
            out: VecDeque::new(),
            pts_queue: VecDeque::new(),
            last_packet_pts: None,
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
        if let Some(pts) = packet.pts {
            self.pts_queue.push_back(pts);
            self.last_packet_pts = Some(pts);
        }

        let nals = self.parse_packet_nals(&packet.data);
        for nal in &nals {
            self.feed_nal(nal)?;
        }
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.out.pop_front())
    }

    fn send_eos(&mut self) -> Result<()> {
        for frame in self.dec.flush() {
            self.push_frame(frame);
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.dec = Inner::new();
        self.out.clear();
        self.pts_queue.clear();
        self.last_packet_pts = None;
        self.avcc_length_size = None;

        if matches!(self.nal_format, NalFormat::Avcc { .. }) {
            let _ = self.prime_with_avcc_extradata();
        }
    }

    fn output_format(&self) -> VideoOutputFormat {
        match self.opts.output_format {
            VideoOutputFormat::Nv12 => VideoOutputFormat::Nv12,
            _ => VideoOutputFormat::Yuv420,
        }
    }
}
