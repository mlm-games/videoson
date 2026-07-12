extern crate alloc;

use alloc::vec::Vec;

use crate::{Packet, Result, VideoFrame};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodecType {
    H264,
    AV1,
    VP9,
    VP8,
    H265,
    Theora,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelFormat {
    Gray,
    Yuv420,
    Nv12,
    P010,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VideoFramePlanes {
    Mono,
    Yuv420,
    Nv12,
    P010,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NalFormat {
    AnnexB,
    Avcc { nal_len_size: u8 },
    Hvcc { nal_len_size: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoOutputFormat {
    #[default]
    Native,
    Yuv420,
    Nv12,
    P010,
}

#[derive(Debug, Clone)]
pub struct VideoCodecParams {
    pub codec: CodecType,
    pub coded_width: u32,
    pub coded_height: u32,
    pub extradata: Vec<u8>,
    pub nal_format: Option<NalFormat>,
}

impl VideoCodecParams {
    pub fn new(codec: CodecType) -> Self {
        Self {
            codec,
            coded_width: 0,
            coded_height: 0,
            extradata: Vec::new(),
            nal_format: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VideoDecoderOptions {
    pub verify: bool,
    pub output_format: VideoOutputFormat,
    /// When set, truncated chroma planes are zero-padded to the expected size
    /// instead of raising `InvalidData`.  This is a compatibility escape hatch
    /// for streams that don't properly signal monochrome (e.g. lessAVC).
    /// Default: `false` (strict validation).
    pub tolerate_truncated_chroma: bool,
}

impl Default for VideoDecoderOptions {
    fn default() -> Self {
        Self {
            verify: false,
            output_format: VideoOutputFormat::Native,
            tolerate_truncated_chroma: false,
        }
    }
}

// NOTE - Removed `: Send` to support wasm
pub trait VideoDecoder {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self>
    where
        Self: Sized;

    fn codec_params(&self) -> &VideoCodecParams;

    fn send_packet(&mut self, packet: &Packet) -> Result<()>;

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>>;

    fn send_eos(&mut self) -> Result<()> {
        Ok(())
    }

    /// Reset decoder state as if newly constructed.
    ///
    /// May fail if re-priming codec extradata fails (e.g. corrupted avcC/hvcC).
    fn reset(&mut self) -> Result<()> {
        Ok(())
    }

    /// Set frame duration in microseconds for POC-based PTS re-computation.
    /// Default is no-op; decoders that support POC (e.g. H.265) override
    /// this to correct PTS when the container PTS is mis-muxed.
    fn set_frame_duration_micros(&mut self, _us: u64) {}

    /// Returns the output format the caller requested via `VideoDecoderOptions`.
    ///
    /// This reflects the *requested* format, which the decoder will honour
    /// when possible.  The actual per-frame format may differ:
    ///
    /// | Stream property       | Actual format                |
    /// |-----------------------|------------------------------|
    /// | 8-bit colour          | matches request              |
    /// | 10/12-bit colour      | `Yuv420` (U16 planes)        |
    /// | Monochrome            | `Gray` (single Y plane)      |
    ///
    /// Always inspect the `VideoFrame` fields (`pixfmt`, `bit_depth`,
    /// `plane_data`) to determine the per-frame pixel format.
    fn requested_output_format(&self) -> VideoOutputFormat {
        VideoOutputFormat::Native
    }
}
