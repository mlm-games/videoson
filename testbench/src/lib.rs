use anyhow::Result;

use less_avc::{
    ycbcr_image::{DataPlane, Planes, YCbCrImage},
    BitDepth,
};

pub fn next_multiple(a: u32, b: u32) -> u32 {
    a.div_ceil(b) * b
}

#[inline]
fn pack_to_12be(v0: u16, v1: u16) -> [u8; 3] {
    debug_assert!(v0 < 4096);
    debug_assert!(v1 < 4096);
    [
        ((v0 & 0x0FF0) >> 4) as u8,
        (((v0 & 0x000F) << 4) as u8) | (((v1 & 0x0F00) >> 8) as u8),
        (v1 & 0x00FF) as u8,
    ]
}

#[inline]
fn unpack_12be(bytes: [u8; 3]) -> (u16, u16) {
    let b0 = bytes[0] as u16;
    let b1 = bytes[1] as u16;
    let b2 = bytes[2] as u16;

    let v0 = (b0 << 4) | ((b1 & 0xF0) >> 4);
    let v1 = ((b1 & 0x0F) << 8) | b2;
    (v0, v1)
}

pub enum OwnedImage {
    Mono8 {
        width: u32,
        height: u32,
        y: Vec<u8>,
        y_stride: usize,
    },
    Yuv4208 {
        width: u32,
        height: u32,
        y: Vec<u8>,
        u: Vec<u8>,
        v: Vec<u8>,
        y_stride: usize,
        uv_stride: usize,
    },
    Mono12 {
        width: u32,
        height: u32,
        y_packed: Vec<u8>,
        y_stride_bytes: usize,
        y_pixels: Vec<u16>,
    },
}

impl OwnedImage {
    pub fn width(&self) -> u32 {
        match self {
            OwnedImage::Mono8 { width, .. }
            | OwnedImage::Yuv4208 { width, .. }
            | OwnedImage::Mono12 { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            OwnedImage::Mono8 { height, .. }
            | OwnedImage::Yuv4208 { height, .. }
            | OwnedImage::Mono12 { height, .. } => *height,
        }
    }

    pub fn view(&self) -> YCbCrImage<'_> {
        match self {
            OwnedImage::Mono8 {
                width,
                height,
                y,
                y_stride,
            } => YCbCrImage {
                planes: Planes::Mono(DataPlane {
                    data: y.as_slice(),
                    stride: *y_stride,
                    bit_depth: BitDepth::Depth8,
                }),
                width: *width,
                height: *height,
            },
            OwnedImage::Yuv4208 {
                width,
                height,
                y,
                u,
                v,
                y_stride,
                uv_stride,
            } => YCbCrImage {
                planes: Planes::YCbCr((
                    DataPlane {
                        data: y.as_slice(),
                        stride: *y_stride,
                        bit_depth: BitDepth::Depth8,
                    },
                    DataPlane {
                        data: u.as_slice(),
                        stride: *uv_stride,
                        bit_depth: BitDepth::Depth8,
                    },
                    DataPlane {
                        data: v.as_slice(),
                        stride: *uv_stride,
                        bit_depth: BitDepth::Depth8,
                    },
                )),
                width: *width,
                height: *height,
            },
            OwnedImage::Mono12 {
                width,
                height,
                y_packed,
                y_stride_bytes,
                ..
            } => YCbCrImage {
                planes: Planes::Mono(DataPlane {
                    data: y_packed.as_slice(),
                    stride: *y_stride_bytes,
                    bit_depth: BitDepth::Depth12,
                }),
                width: *width,
                height: *height,
            },
        }
    }

    pub fn y_visible_u8(&self) -> Option<Vec<u8>> {
        match self {
            OwnedImage::Mono8 {
                width,
                height,
                y,
                y_stride,
            } => {
                let w = *width as usize;
                let h = *height as usize;
                let mut out = vec![0u8; w * h];
                for row in 0..h {
                    out[row * w..row * w + w]
                        .copy_from_slice(&y[row * *y_stride..row * *y_stride + w]);
                }
                Some(out)
            }
            OwnedImage::Yuv4208 {
                width,
                height,
                y,
                y_stride,
                ..
            } => {
                let w = *width as usize;
                let h = *height as usize;
                let mut out = vec![0u8; w * h];
                for row in 0..h {
                    out[row * w..row * w + w]
                        .copy_from_slice(&y[row * *y_stride..row * *y_stride + w]);
                }
                Some(out)
            }
            OwnedImage::Mono12 { .. } => None,
        }
    }

    pub fn uv_visible_u8(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        match self {
            OwnedImage::Yuv4208 {
                width,
                height,
                u,
                v,
                uv_stride,
                ..
            } => {
                let cw = ((*width as usize) + 1) / 2;
                let ch = ((*height as usize) + 1) / 2;
                let mut uo = vec![0u8; cw * ch];
                let mut vo = vec![0u8; cw * ch];
                for row in 0..ch {
                    uo[row * cw..row * cw + cw]
                        .copy_from_slice(&u[row * *uv_stride..row * *uv_stride + cw]);
                    vo[row * cw..row * cw + cw]
                        .copy_from_slice(&v[row * *uv_stride..row * *uv_stride + cw]);
                }
                Some((uo, vo))
            }
            _ => None,
        }
    }

    pub fn y_visible_u16(&self) -> Option<&[u16]> {
        match self {
            OwnedImage::Mono12 { y_pixels, .. } => Some(y_pixels.as_slice()),
            _ => None,
        }
    }
}

pub fn gen_mono8(width: u32, height: u32, invert: bool) -> Result<OwnedImage> {
    let y_stride = next_multiple(width, 16) as usize;
    let alloc_rows = next_multiple(height, 16) as usize;
    let mut y = vec![0u8; y_stride * alloc_rows];

    for r in 0..height as usize {
        for c in 0..width as usize {
            let base = if width > 1 {
                ((c as u32) * 255 / (width - 1)) as u8
            } else {
                0
            };
            let v = if invert {
                255u8.wrapping_sub(base)
            } else {
                base
            };
            y[r * y_stride + c] = v;
        }
    }

    Ok(OwnedImage::Mono8 {
        width,
        height,
        y,
        y_stride,
    })
}

pub fn gen_yuv4208(width: u32, height: u32, invert: bool) -> Result<OwnedImage> {
    let y_stride = next_multiple(width, 16) as usize;
    let alloc_rows = next_multiple(height, 16) as usize;
    let mut y = vec![0u8; y_stride * alloc_rows];

    for r in 0..height as usize {
        for c in 0..width as usize {
            let base = if width > 1 {
                ((c as u32) * 255 / (width - 1)) as u8
            } else {
                0
            };
            let v0 = if invert {
                255u8.wrapping_sub(base)
            } else {
                base
            };
            y[r * y_stride + c] = v0;
        }
    }

    let cw = (width + 1) / 2;
    let ch = (height + 1) / 2;
    let uv_stride = next_multiple(cw, 8) as usize;
    let uv_rows = next_multiple(ch, 8) as usize;

    let u = vec![128u8; uv_stride * uv_rows];
    let v = vec![128u8; uv_stride * uv_rows];

    Ok(OwnedImage::Yuv4208 {
        width,
        height,
        y,
        u,
        v,
        y_stride,
        uv_stride,
    })
}

pub fn gen_mono12(width: u32, height: u32, invert: bool) -> Result<OwnedImage> {
    if width % 2 != 0 {
        anyhow::bail!("mono12 generator requires even width (12-bit packing)");
    }

    let padded_w = next_multiple(width, 16) as usize;
    let padded_h = next_multiple(height, 16) as usize;

    let y_stride_bytes = padded_w * 3 / 2;
    let mut y_packed = vec![0u8; y_stride_bytes * padded_h];

    let mut y_pixels = vec![0u16; (width as usize) * (height as usize)];

    for r in 0..height as usize {
        let mut row_vals = vec![0u16; padded_w];
        for c in 0..(width as usize) {
            let base = if width > 1 {
                ((c as u32) * 4095 / (width - 1)) as u16
            } else {
                0
            };
            let v = if invert { 4095u16 - base } else { base };
            row_vals[c] = v;
            y_pixels[r * (width as usize) + c] = v;
        }

        let mut row_bytes = vec![0u8; y_stride_bytes];
        let mut out_i = 0usize;
        for pair in row_vals.chunks_exact(2) {
            let b = pack_to_12be(pair[0], pair[1]);
            row_bytes[out_i..out_i + 3].copy_from_slice(&b);
            out_i += 3;
        }

        let dst = &mut y_packed[r * y_stride_bytes..(r + 1) * y_stride_bytes];
        dst.copy_from_slice(&row_bytes);
    }

    if width >= 2 && height >= 1 {
        let first3 = [y_packed[0], y_packed[1], y_packed[2]];
        let (a, b) = unpack_12be(first3);
        let exp0 = y_pixels[0];
        let exp1 = y_pixels[1];
        debug_assert_eq!(a, exp0);
        debug_assert_eq!(b, exp1);
    }

    Ok(OwnedImage::Mono12 {
        width,
        height,
        y_packed,
        y_stride_bytes,
        y_pixels,
    })
}
