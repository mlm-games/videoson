extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use crate::{PixelFormat, VideoFramePlanes};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColorInfo {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}

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
    pub pts: Option<i64>,
    pub plane_data: Vec<VideoPlane>,
    pub color_info: ColorInfo,
}

impl VideoFrame {
    pub fn with_pts(mut self, pts: Option<i64>) -> Self {
        self.pts = pts;
        self
    }

    pub fn with_color_info(mut self, color_info: ColorInfo) -> Self {
        self.color_info = color_info;
        self
    }

    pub fn new_mono_u8(width: u32, height: u32, stride: usize, y: Vec<u8>) -> Self {
        Self {
            width,
            height,
            planes: VideoFramePlanes::Mono,
            pixfmt: PixelFormat::Gray,
            bit_depth: 8,
            pts: None,
            plane_data: vec![VideoPlane {
                stride,
                data: PlaneData::U8(y),
            }],
            color_info: ColorInfo::default(),
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
            pts: None,
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
            color_info: ColorInfo::default(),
        }
    }

    pub fn new_nv12_u8(
        width: u32,
        height: u32,
        y_stride: usize,
        uv_stride: usize,
        y: Vec<u8>,
        uv: Vec<u8>,
    ) -> Self {
        Self {
            width,
            height,
            planes: VideoFramePlanes::Nv12,
            pixfmt: PixelFormat::Nv12,
            bit_depth: 8,
            pts: None,
            plane_data: vec![
                VideoPlane {
                    stride: y_stride,
                    data: PlaneData::U8(y),
                },
                VideoPlane {
                    stride: uv_stride,
                    data: PlaneData::U8(uv),
                },
            ],
            color_info: ColorInfo::default(),
        }
    }
}

pub fn interleave_uv_nv12(
    u: &[u8],
    u_stride: usize,
    v: &[u8],
    v_stride: usize,
    uv_w: usize,
    uv_h: usize,
) -> Vec<u8> {
    let mut uv = Vec::with_capacity(uv_w * uv_h * 2);
    for row in 0..uv_h {
        let u_base = row * u_stride;
        let v_base = row * v_stride;
        for col in 0..uv_w {
            uv.push(*u.get(u_base + col).unwrap_or(&128));
            uv.push(*v.get(v_base + col).unwrap_or(&128));
        }
    }
    uv
}
