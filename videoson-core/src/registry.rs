extern crate alloc;

use alloc::vec::Vec;
use alloc::boxed::Box;

use crate::{CodecType, Result, VideoCodecParams, VideoDecoder, VideoDecoderOptions};

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
        codec_type: CodecType,
        params: &VideoCodecParams,
        opts: &VideoDecoderOptions,
    ) -> Option<Result<Box<dyn VideoDecoder>>> {
        for reg in &self.video {
            if reg.supported.iter().any(|s| s.codec_type == codec_type) {
                return Some((reg.factory)(params, opts));
            }
        }
        None
    }
}
