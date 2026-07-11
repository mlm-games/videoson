#![no_std]

extern crate alloc;

mod vp8_decoder;

pub use vp8_decoder::Vp8Decoder;

pub type VP8Decoder = Vp8Decoder;

use videoson_core::VideoDecoder as _;

impl videoson_core::RegisterableVideoDecoder for Vp8Decoder {
    fn try_registry_new(
        params: &videoson_core::VideoCodecParams,
        opts: &videoson_core::VideoDecoderOptions,
    ) -> videoson_core::Result<alloc::boxed::Box<dyn videoson_core::VideoDecoder>> {
        Ok(alloc::boxed::Box::new(Self::try_new(params, opts)?))
    }

    fn supported_codecs() -> &'static [videoson_core::SupportedVideoCodec] {
        &[videoson_core::SupportedVideoCodec {
            codec_type: videoson_core::CodecType::VP8,
            short_name: "vp8",
            long_name: "VP8",
        }]
    }
}
