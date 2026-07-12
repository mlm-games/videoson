extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::{CodecType, Result, VideoCodecParams, VideoDecoder, VideoDecoderOptions, VideosonError};

pub struct SupportedVideoCodec {
    pub codec_type: CodecType,
    pub short_name: &'static str,
    pub long_name: &'static str,
}

pub type VideoDecoderFactoryFn =
    fn(&VideoCodecParams, &VideoDecoderOptions) -> Result<Box<dyn VideoDecoder>>;

pub trait RegisterableVideoDecoder: VideoDecoder {
    fn try_registry_new(
        params: &VideoCodecParams,
        opts: &VideoDecoderOptions,
    ) -> Result<Box<dyn VideoDecoder>>
    where
        Self: Sized;

    fn supported_codecs() -> &'static [SupportedVideoCodec];
}

struct RegisteredVideoDecoder {
    factory: VideoDecoderFactoryFn,
    supported: &'static [SupportedVideoCodec],
}

pub struct CodecRegistry {
    video: Vec<RegisteredVideoDecoder>,
}

impl Default for CodecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CodecRegistry {
    pub fn new() -> Self {
        Self { video: Vec::new() }
    }

    pub fn register_video_decoder<T: RegisterableVideoDecoder>(&mut self) {
        self.video.push(RegisteredVideoDecoder {
            factory: T::try_registry_new,
            supported: T::supported_codecs(),
        });
    }

    pub fn make_video_decoder(
        &self,
        params: &VideoCodecParams,
        opts: &VideoDecoderOptions,
    ) -> Result<Box<dyn VideoDecoder>> {
        for reg in &self.video {
            if reg.supported.iter().any(|s| s.codec_type == params.codec) {
                return (reg.factory)(params, opts);
            }
        }
        Err(VideosonError::Unsupported("no decoder registered for codec"))
    }
}
