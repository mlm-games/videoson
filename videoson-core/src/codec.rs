// videoson/videoson-core/src/codec.rs
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VideoFramePlanes {
    Mono,
    Yuv420,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NalFormat {
    AnnexB,
    Avcc { nal_len_size: u8 },
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
}

impl Default for VideoDecoderOptions {
    fn default() -> Self {
        Self { verify: false }
    }
}

pub trait VideoDecoder: Send {
    fn try_new(params: &VideoCodecParams, opts: &VideoDecoderOptions) -> Result<Self>
    where
        Self: Sized;

    fn codec_params(&self) -> &VideoCodecParams;

    fn send_packet(&mut self, packet: &Packet) -> Result<()>;

    fn receive_frame(&mut self) -> Result<Option<VideoFrame>>;

    fn send_eos(&mut self) -> Result<()> {
        Ok(())
    }

    fn reset(&mut self);
}
