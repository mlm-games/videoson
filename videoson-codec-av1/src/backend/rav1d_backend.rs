// videoson-codec-av1/src/backend/rav1d_backend.rs
extern crate alloc;

use alloc::collections::VecDeque;

use videoson_core::{
    CodecType, Packet, Result, VideoCodecParams, VideoDecoder, VideoDecoderOptions, VideoFrame,
    VideosonError,
};

pub struct Av1Decoder {
    params: VideoCodecParams,
    _opts: VideoDecoderOptions,
    queue: VecDeque<VideoFrame>,
}

impl VideoDecoder for Av1Decoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::AV1 {
            return Err(VideosonError::InvalidData("params.codec is not AV1"));
        }

        Ok(Self {
            params: params.clone(),
            _opts: *opts,
            queue: VecDeque::new(),
        })
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, _packet: &Packet) -> Result<()> {
        Err(VideosonError::Unsupported(
            "AV1 decoder using rav1d backend - requires Send-capable context (not yet available)",
        ))
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Err(VideosonError::Unsupported(
            "AV1 decoder using rav1d backend - requires Send-capable context (not yet available)",
        ))
    }

    fn send_eos(&mut self) -> Result<()> {
        Ok(())
    }

    fn reset(&mut self) {
        self.queue.clear();
    }
}
