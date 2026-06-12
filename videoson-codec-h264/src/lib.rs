#![no_std]

extern crate alloc;

mod rust_h264_decoder;

pub use rust_h264_decoder::RustH264Decoder;

pub type H264Decoder = RustH264Decoder;

use videoson_core::VideoDecoder as _;

impl videoson_core::RegisterableVideoDecoder for RustH264Decoder {
    fn try_registry_new(
        params: &videoson_core::VideoCodecParams,
        opts: &videoson_core::VideoDecoderOptions,
    ) -> videoson_core::Result<alloc::boxed::Box<dyn videoson_core::VideoDecoder>> {
        Ok(alloc::boxed::Box::new(Self::try_new(params, opts)?))
    }

    fn supported_codecs() -> &'static [videoson_core::SupportedVideoCodec] {
        &[videoson_core::SupportedVideoCodec {
            codec_type: videoson_core::CodecType::H264,
            short_name: "h264",
            long_name: "H.264 / AVC",
        }]
    }
}
