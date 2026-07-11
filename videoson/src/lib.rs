#![no_std]

extern crate alloc;

pub use videoson_core::{
    CodecRegistry, CodecType, ColorInfo, NalFormat, Packet, PixelFormat, PlaneData,
    RegisterableVideoDecoder, Result, SupportedVideoCodec, VideoCodecParams, VideoDecoder,
    VideoDecoderOptions, VideoFrame, VideoFramePlanes, VideoOutputFormat, VideoPlane,
    VideosonError, interleave_uv_nv12, tight_pack_plane,
};

#[cfg(feature = "h264")]
pub use videoson_codec_h264 as codec_h264;

#[cfg(feature = "h264")]
pub use videoson_codec_h264::H264Decoder;

#[cfg(feature = "h265")]
pub use videoson_codec_h265 as codec_h265;

#[cfg(feature = "h265")]
pub use videoson_codec_h265::H265Decoder;

#[cfg(feature = "vp8")]
pub use videoson_codec_vp8 as codec_vp8;

#[cfg(feature = "vp8")]
pub use videoson_codec_vp8::Vp8Decoder;

#[cfg(feature = "rav1d")]
pub use videoson_codec_rav1d as codec_rav1d;

#[cfg(feature = "rav1d")]
pub use videoson_codec_rav1d::Rav1dSafeDecoder;

#[cfg(feature = "ivf")]
pub use videoson_format_ivf as format_ivf;

#[cfg(feature = "ivf")]
pub use videoson_format_ivf::{IvfCodec, IvfDemuxer, IvfFileHeader, IvfFrameHeader};

pub fn default_registry() -> CodecRegistry {
    let mut reg = CodecRegistry::new();

    #[cfg(feature = "h264")]
    reg.register_video_decoder::<H264Decoder>();

    #[cfg(feature = "h265")]
    reg.register_video_decoder::<H265Decoder>();

    #[cfg(feature = "vp8")]
    reg.register_video_decoder::<Vp8Decoder>();

    #[cfg(feature = "rav1d")]
    reg.register_video_decoder::<Rav1dSafeDecoder>();

    reg
}

pub mod prelude {
    pub use videoson_core::{
        CodecRegistry, CodecType, Packet, PixelFormat, Result, VideoCodecParams, VideoDecoder,
        VideoDecoderOptions, VideoFrame, VideoOutputFormat, VideosonError,
    };
}
