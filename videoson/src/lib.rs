#![cfg_attr(not(feature = "std"), no_std)]

pub use videoson_core::{
    CodecType, Packet, PixelFormat, PlaneData, Result, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFrame, VideoFramePlanes, VideoPlane, VideosonError,
};

pub mod prelude {
    pub use videoson_core::{
        CodecType, Packet, PixelFormat, Result, VideoCodecParams, VideoDecoder,
        VideoDecoderOptions, VideoFrame, VideosonError,
    };
}
