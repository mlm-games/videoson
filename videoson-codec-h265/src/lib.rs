#![no_std]

extern crate alloc;

mod rust_h265_decoder;

pub use rust_h265_decoder::RustH265Decoder;

pub type H265Decoder = RustH265Decoder;

use videoson_core::VideoDecoder as _;

impl videoson_core::RegisterableVideoDecoder for RustH265Decoder {
    fn try_registry_new(
        params: &videoson_core::VideoCodecParams,
        opts: &videoson_core::VideoDecoderOptions,
    ) -> videoson_core::Result<alloc::boxed::Box<dyn videoson_core::VideoDecoder>> {
        Ok(alloc::boxed::Box::new(Self::try_new(params, opts)?))
    }

    fn supported_codecs() -> &'static [videoson_core::SupportedVideoCodec] {
        &[videoson_core::SupportedVideoCodec {
            codec_type: videoson_core::CodecType::H265,
            short_name: "h265",
            long_name: "H.265 / HEVC",
        }]
    }
}
