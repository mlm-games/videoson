extern crate alloc;

use alloc::collections::VecDeque;

use oxideav_vp8::state::Vp8DecoderState;

use videoson_core::{
    CodecType, Packet, Result, VideoCodecParams, VideoDecoder, VideoDecoderOptions, VideoFrame,
    VideoOutputFormat, VideosonError, interleave_uv_nv12,
};

pub struct Vp8Decoder {
    params: VideoCodecParams,
    opts: VideoDecoderOptions,
    dec: Vp8DecoderState,
    queued: VecDeque<VideoFrame>,
}

impl Vp8Decoder {
    fn wants_nv12(&self) -> bool {
        matches!(self.opts.output_format, VideoOutputFormat::Nv12)
    }

    fn push_frame(&mut self, f: oxideav_vp8::decoder::Vp8DecodedFrame, pts: Option<i64>) -> Result<()> {
        let w = f.width as usize;
        let h = f.height as usize;
        let cw = (w + 1) / 2;
        let ch = (h + 1) / 2;

        // Reject non-4:2:0 chroma
        if (!f.u.is_empty() && (f.u.len() / cw.max(1)) > ch)
            || (!f.v.is_empty() && (f.v.len() / cw.max(1)) > ch)
        {
            return Err(VideosonError::Unsupported(
                "VP8: only 4:2:0 chroma is supported",
            ));
        }

        if self.wants_nv12() {
            let uv = interleave_uv_nv12(&f.u, cw, &f.v, cw, cw, ch)?;
            self.queued.push_back(
                VideoFrame::new_nv12_u8(f.width, f.height, w, cw * 2, f.y, uv).with_pts(pts),
            );
        } else {
            self.queued.push_back(
                VideoFrame::new_yuv420_u8(f.width, f.height, w, cw, cw, f.y, f.u, f.v)
                    .with_pts(pts),
            );
        }
        Ok(())
    }
}

impl VideoDecoder for Vp8Decoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::VP8 {
            return Err(VideosonError::InvalidData("params.codec is not VP8"));
        }

        if matches!(opts.output_format, VideoOutputFormat::P010) {
            return Err(VideosonError::Unsupported(
                "P010 output is not supported for VP8",
            ));
        }

        Ok(Self {
            params: params.clone(),
            opts: *opts,
            dec: Vp8DecoderState::new(),
            queued: VecDeque::new(),
        })
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let frame = self
            .dec
            .decode_frame(&packet.data)
            .map_err(|e| VideosonError::Message(alloc::format!("VP8: {e}").into()))?;

        // Invisible alt-ref frames update decoder state but are not displayed.
        if self.dec.last_frame_shown() == Some(false) {
            return Ok(());
        }

        self.push_frame(frame, packet.pts)
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Ok(self.queued.pop_front())
    }

    fn send_eos(&mut self) -> Result<()> {
        Ok(())
    }

    fn reset(&mut self) {
        self.dec = Vp8DecoderState::new();
        self.queued.clear();
    }

    fn output_format(&self) -> VideoOutputFormat {
        match self.opts.output_format {
            VideoOutputFormat::Nv12 => VideoOutputFormat::Nv12,
            VideoOutputFormat::Native | VideoOutputFormat::Yuv420 => VideoOutputFormat::Yuv420,
            VideoOutputFormat::P010 => VideoOutputFormat::Yuv420,
        }
    }
}
