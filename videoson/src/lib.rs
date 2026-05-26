#![no_std]

pub use videoson_core::{
    CodecType, Packet, PixelFormat, PlaneData, Result, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFrame, VideoFramePlanes, VideoPlane, VideosonError,
    CodecRegistry, RegisterableVideoDecoder, SupportedVideoCodec, NalFormat,
};

#[cfg(feature = "h264")]
pub use videoson_codec_h264 as codec_h264;

#[cfg(feature = "h264")]
pub use videoson_codec_h264::H264Decoder;

#[cfg(feature = "rav1d")]
pub use videoson_codec_rav1d as codec_rav1d;

#[cfg(feature = "rav1d")]
pub use videoson_codec_rav1d::Rav1dSafeDecoder;

pub mod prelude {
    pub use videoson_core::{
        CodecType, Packet, PixelFormat, Result, VideoCodecParams, VideoDecoder,
        VideoDecoderOptions, VideoFrame, VideosonError, CodecRegistry,
    };
}
