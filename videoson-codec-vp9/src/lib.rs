#![no_std]

extern crate alloc;

mod vp9_decoder;

pub use vp9_decoder::Vp9Decoder;

pub type VP9Decoder = Vp9Decoder;

use videoson_core::VideoDecoder as _;

impl videoson_core::RegisterableVideoDecoder for Vp9Decoder {
    fn try_registry_new(
        params: &videoson_core::VideoCodecParams,
        opts: &videoson_core::VideoDecoderOptions,
    ) -> videoson_core::Result<alloc::boxed::Box<dyn videoson_core::VideoDecoder>> {
        Ok(alloc::boxed::Box::new(Self::try_new(params, opts)?))
    }

    fn supported_codecs() -> &'static [videoson_core::SupportedVideoCodec] {
        &[videoson_core::SupportedVideoCodec {
            codec_type: videoson_core::CodecType::VP9,
            short_name: "vp9",
            long_name: "VP9",
        }]
    }
}
