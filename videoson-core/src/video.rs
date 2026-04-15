// videoson/videoson-core/src/video.rs
extern crate alloc;

use alloc::vec::Vec;

use crate::{PixelFormat, VideoFramePlanes};

#[derive(Debug, Clone)]
pub enum PlaneData {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

impl PlaneData {
    pub fn len_bytes(&self) -> usize {
        match self {
            PlaneData::U8(v) => v.len(),
            PlaneData::U16(v) => v.len() * 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VideoPlane {
    pub stride: usize,
    pub data: PlaneData,
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub planes: VideoFramePlanes,
    pub pixfmt: PixelFormat,
    pub bit_depth: u8,
    pub plane_data: Vec<VideoPlane>,
}

impl VideoFrame {
    pub fn new_mono_u8(width: u32, height: u32, stride: usize, y: Vec<u8>) -> Self {
        Self {
            width,
            height,
            planes: VideoFramePlanes::Mono,
            pixfmt: PixelFormat::Gray,
            bit_depth: 8,
            plane_data: vec![VideoPlane {
                stride,
                data: PlaneData::U8(y),
            }],
        }
    }

    pub fn new_yuv420_u8(
        width: u32,
        height: u32,
        y_stride: usize,
        u_stride: usize,
        v_stride: usize,
        y: Vec<u8>,
        u: Vec<u8>,
        v: Vec<u8>,
    ) -> Self {
        Self {
            width,
            height,
            planes: VideoFramePlanes::Yuv420,
            pixfmt: PixelFormat::Yuv420,
            bit_depth: 8,
            plane_data: vec![
                VideoPlane {
                    stride: y_stride,
                    data: PlaneData::U8(y),
                },
                VideoPlane {
                    stride: u_stride,
                    data: PlaneData::U8(u),
                },
                VideoPlane {
                    stride: v_stride,
                    data: PlaneData::U8(v),
                },
            ],
        }
    }
}
