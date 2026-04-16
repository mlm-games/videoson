use videoson_core::{
    CodecType, Packet, Result, VideoCodecParams, VideoDecoder, VideoDecoderOptions, VideoFrame,
    VideosonError,
};

pub struct Av1Decoder {
    params: VideoCodecParams,
    _opts: VideoDecoderOptions,
}

impl VideoDecoder for Av1Decoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self> {
        if params.codec != CodecType::AV1 {
            return Err(VideosonError::InvalidData("not AV1"));
        }
        Ok(Self {
            params: params.clone(),
            _opts: *opts,
        })
    }

    fn codec_params(&self) -> &VideoCodecParams {
        &self.params
    }

    fn send_packet(&mut self, _packet: &Packet) -> Result<()> {
        Err(VideosonError::Unsupported(
            "AV1 decoder not yet implemented",
        ))
    }

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>> {
        Err(VideosonError::Unsupported(
            "AV1 decoder not yet implemented",
        ))
    }

    fn send_eos(&mut self) -> Result<()> {
        Ok(())
    }

    fn reset(&mut self) {}
}
