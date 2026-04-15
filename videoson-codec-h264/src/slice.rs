// videoson-codec-h264/src/slice.rs
extern crate alloc;

use alloc::vec::Vec;

use videoson_common::{read_se, read_ue, BitReader, BitstreamError, BitstreamResult};
use videoson_core::{PlaneData, VideoFrame, VideoPlane, VideosonError};

use crate::cabac::{decode_mb_type_intra, init_ctx_i_slice_0_10, CabacDecoder};
use crate::decoder::ParamSets;
use crate::pps::Pps;

#[derive(Debug, Clone)]
pub struct SliceHeader {
    pub first_mb_in_slice: u32,
    pub slice_type: u32,
    pub pps_id: u32,

    pub frame_num: u32,

    pub idr_pic_id: Option<u32>,
    pub pic_order_cnt_lsb: Option<u32>,

    pub slice_qp_delta: i32,
}

fn is_i_slice(slice_type: u32) -> bool {
    (slice_type % 5) == 2
}

pub fn parse_slice_header_rbsp(rbsp: &[u8], ps: &ParamSets) -> BitstreamResult<SliceHeader> {
    let mut br = BitReader::new(rbsp);

    let first_mb_in_slice = read_ue(&mut br)?;
    let slice_type = read_ue(&mut br)?;
    let pps_id = read_ue(&mut br)?;

    let pps = ps
        .get_pps(pps_id)
        .map_err(|_| BitstreamError::Invalid("missing PPS for slice"))?;
    let sps = ps
        .get_sps(pps.sps_id)
        .map_err(|_| BitstreamError::Invalid("missing SPS for slice"))?;

    if !is_i_slice(slice_type) {
        return Err(BitstreamError::Invalid("only I slices supported in M0"));
    }

    let frame_num_bits = sps.frame_num_bits();
    let frame_num = br.read_bits_u32(frame_num_bits)?;

    let idr_pic_id = read_ue(&mut br)?;

    let pic_order_cnt_lsb = if sps.pic_order_cnt_type == 0 {
        let n = sps.pic_order_cnt_lsb_bits();
        Some(br.read_bits_u32(n)?)
    } else {
        return Err(BitstreamError::Invalid(
            "pic_order_cnt_type != 0 not supported in M0",
        ));
    };

    let _no_output_of_prior_pics_flag = br.read_bit()?;
    let _long_term_reference_flag = br.read_bit()?;

    let slice_qp_delta = read_se(&mut br)?;

    Ok(SliceHeader {
        first_mb_in_slice,
        slice_type,
        pps_id,
        frame_num,
        idr_pic_id: Some(idr_pic_id),
        pic_order_cnt_lsb,
        slice_qp_delta,
    })
}

fn fill_plane_u8(width: usize, height: usize, stride: usize, value: u8) -> Vec<u8> {
    vec![value; stride * height]
}

fn fill_plane_u16(width: usize, height: usize, stride: usize, value: u16) -> Vec<u16> {
    let _ = width;
    vec![value; stride * height]
}

pub fn decode_idr_ipcm_slice(
    rbsp: &[u8],
    ps: &ParamSets,
    sh: &SliceHeader,
    pps: &Pps,
) -> core::result::Result<VideoFrame, VideosonError> {
    let sps = ps.get_sps(pps.sps_id)?;

    if sh.first_mb_in_slice != 0 {
        return Err(VideosonError::Unsupported(
            "first_mb_in_slice != 0 not supported in M0",
        ));
    }

    let mut br = BitReader::new(rbsp);
    let _first_mb_in_slice = read_ue(&mut br).map_err(map_bs)?;
    let _slice_type = read_ue(&mut br).map_err(map_bs)?;
    let _pps_id = read_ue(&mut br).map_err(map_bs)?;

    let frame_num_bits = sps.frame_num_bits();
    let _frame_num = br.read_bits_u32(frame_num_bits).map_err(map_bs)?;
    let _idr_pic_id = read_ue(&mut br).map_err(map_bs)?;

    if sps.pic_order_cnt_type == 0 {
        let n = sps.pic_order_cnt_lsb_bits();
        let _poc = br.read_bits_u32(n).map_err(map_bs)?;
    }

    let _no_output_of_prior_pics_flag = br.read_bit().map_err(map_bs)?;
    let _long_term_reference_flag = br.read_bit().map_err(map_bs)?;
    let _slice_qp_delta = read_se(&mut br).map_err(map_bs)?;

    let slice_qpy: i32 = 26 + pps.pic_init_qp_minus26 + sh.slice_qp_delta;

    let (width, height) = sps.display_dimensions();
    let width_us = width as usize;
    let height_us = height as usize;

    let bit_depth = sps.bit_depth_luma;
    let chroma_format_idc = sps.chroma_format_idc;

    if chroma_format_idc != 0 && chroma_format_idc != 1 {
        return Err(VideosonError::Unsupported(
            "only chroma_format_idc 0 (mono) and 1 (4:2:0) supported in M0",
        ));
    }

    let mbs_w = sps.mbs_width() as usize;
    let mbs_h = sps.mbs_height() as usize;

    let y_stride = width_us;
    let chroma_w = (width_us + 1) / 2;
    let chroma_h = (height_us + 1) / 2;

    if !pps.entropy_coding_mode_flag {
        // CAVLC path
        if bit_depth <= 8 {
            let mut y = fill_plane_u8(width_us, height_us, y_stride, 0);
            let (mut u, mut v) = if chroma_format_idc == 1 {
                (
                    fill_plane_u8(chroma_w, chroma_h, chroma_w, 128),
                    fill_plane_u8(chroma_w, chroma_h, chroma_w, 128),
                )
            } else {
                (Vec::new(), Vec::new())
            };

            for mb_y in 0..mbs_h {
                for mb_x in 0..mbs_w {
                    let mb_type = read_ue(&mut br).map_err(map_bs)?;
                    if mb_type != 25 {
                        return Err(VideosonError::Unsupported(
                            "only I_PCM macroblocks supported in M0",
                        ));
                    }
                    br.byte_align_zero().map_err(map_bs)?;

                    for r in 0..16 {
                        for c in 0..16 {
                            let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 16 + c;
                            let y_row = mb_y * 16 + r;
                            if x < width_us && y_row < height_us {
                                y[y_row * y_stride + x] = b;
                            }
                        }
                    }
                    if chroma_format_idc == 1 {
                        for r in 0..8 {
                            for c in 0..8 {
                                let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 8 + c;
                                let y_row = mb_y * 8 + r;
                                if x < chroma_w && y_row < chroma_h {
                                    u[y_row * chroma_w + x] = b;
                                }
                            }
                        }
                        for r in 0..8 {
                            for c in 0..8 {
                                let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 8 + c;
                                let y_row = mb_y * 8 + r;
                                if x < chroma_w && y_row < chroma_h {
                                    v[y_row * chroma_w + x] = b;
                                }
                            }
                        }
                    }
                }
            }
            if chroma_format_idc == 0 {
                Ok(VideoFrame {
                    width,
                    height,
                    planes: videoson_core::VideoFramePlanes::Mono,
                    pixfmt: videoson_core::PixelFormat::Gray,
                    bit_depth,
                    plane_data: vec![VideoPlane {
                        stride: y_stride,
                        data: PlaneData::U8(y),
                    }],
                })
            } else {
                Ok(VideoFrame {
                    width,
                    height,
                    planes: videoson_core::VideoFramePlanes::Yuv420,
                    pixfmt: videoson_core::PixelFormat::Yuv420,
                    bit_depth,
                    plane_data: vec![
                        VideoPlane {
                            stride: y_stride,
                            data: PlaneData::U8(y),
                        },
                        VideoPlane {
                            stride: chroma_w,
                            data: PlaneData::U8(u),
                        },
                        VideoPlane {
                            stride: chroma_w,
                            data: PlaneData::U8(v),
                        },
                    ],
                })
            }
        } else {
            let mut y = fill_plane_u16(width_us, height_us, y_stride, 0);
            let (mut u, mut v) = if chroma_format_idc == 1 {
                (
                    fill_plane_u16(chroma_w, chroma_h, chroma_w, 1 << (bit_depth - 1)),
                    fill_plane_u16(chroma_w, chroma_h, chroma_w, 1 << (bit_depth - 1)),
                )
            } else {
                (Vec::new(), Vec::new())
            };

            for mb_y in 0..mbs_h {
                for mb_x in 0..mbs_w {
                    let mb_type = read_ue(&mut br).map_err(map_bs)?;
                    if mb_type != 25 {
                        return Err(VideosonError::Unsupported(
                            "only I_PCM macroblocks supported in M0",
                        ));
                    }
                    br.byte_align_zero().map_err(map_bs)?;

                    for r in 0..16 {
                        for c in 0..16 {
                            let s = br.read_bits_u16(bit_depth as u32).map_err(map_bs)? as u16;
                            let x = mb_x * 16 + c;
                            let y_row = mb_y * 16 + r;
                            if x < width_us && y_row < height_us {
                                y[y_row * y_stride + x] = s;
                            }
                        }
                    }
                    if chroma_format_idc == 1 {
                        for r in 0..8 {
                            for c in 0..8 {
                                let s = br.read_bits_u16(bit_depth as u32).map_err(map_bs)? as u16;
                                let x = mb_x * 8 + c;
                                let y_row = mb_y * 8 + r;
                                if x < chroma_w && y_row < chroma_h {
                                    u[y_row * chroma_w + x] = s;
                                }
                            }
                        }
                        for r in 0..8 {
                            for c in 0..8 {
                                let s = br.read_bits_u16(bit_depth as u32).map_err(map_bs)? as u16;
                                let x = mb_x * 8 + c;
                                let y_row = mb_y * 8 + r;
                                if x < chroma_w && y_row < chroma_h {
                                    v[y_row * chroma_w + x] = s;
                                }
                            }
                        }
                    }
                }
            }
            if chroma_format_idc == 0 {
                Ok(VideoFrame {
                    width,
                    height,
                    planes: videoson_core::VideoFramePlanes::Mono,
                    pixfmt: videoson_core::PixelFormat::Gray,
                    bit_depth,
                    plane_data: vec![VideoPlane {
                        stride: y_stride,
                        data: PlaneData::U16(y),
                    }],
                })
            } else {
                Ok(VideoFrame {
                    width,
                    height,
                    planes: videoson_core::VideoFramePlanes::Yuv420,
                    pixfmt: videoson_core::PixelFormat::Yuv420,
                    bit_depth,
                    plane_data: vec![
                        VideoPlane {
                            stride: y_stride,
                            data: PlaneData::U16(y),
                        },
                        VideoPlane {
                            stride: chroma_w,
                            data: PlaneData::U16(u),
                        },
                        VideoPlane {
                            stride: chroma_w,
                            data: PlaneData::U16(v),
                        },
                    ],
                })
            }
        }
    } else {
        // CABAC path
        if bit_depth > 8 {
            return Err(VideosonError::Unsupported(
                "CABAC: bit_depth > 8 not supported in M0",
            ));
        }

        if !br.is_byte_aligned() {
            let one = br.read_bit().map_err(map_bs)?;
            if !one {
                return Err(VideosonError::InvalidData(
                    "CABAC alignment: expected 1 bit",
                ));
            }
            while !br.is_byte_aligned() {
                let z = br.read_bit().map_err(map_bs)?;
                if z {
                    return Err(VideosonError::InvalidData(
                        "CABAC alignment: expected 0 bits",
                    ));
                }
            }
        }

        let slice_data = br.remaining_bytes().map_err(map_bs)?;
        let mut bitpos: usize = 0;

        let mut ctx0_10 = init_ctx_i_slice_0_10(slice_qpy);

        let total_mbs = mbs_w * mbs_h;
        let mut mb_types: Vec<u8> = vec![0u8; total_mbs];

        let mut y = fill_plane_u8(width_us, height_us, y_stride, 0);
        let mut u = if chroma_format_idc == 1 {
            fill_plane_u8(chroma_w, chroma_h, chroma_w, 128)
        } else {
            Vec::new()
        };
        let mut v = if chroma_format_idc == 1 {
            fill_plane_u8(chroma_w, chroma_h, chroma_w, 128)
        } else {
            Vec::new()
        };

        for mb_y in 0..mbs_h {
            for mb_x in 0..mbs_w {
                let mb_idx = mb_y * mbs_w + mb_x;

                if (bitpos & 7) != 0 {
                    return Err(VideosonError::InvalidData(
                        "CABAC: expected byte-aligned new macroblock",
                    ));
                }

                let byte_pos = bitpos >> 3;
                let mut cabac = CabacDecoder::new(&slice_data[byte_pos..])
                    .map_err(|_| VideosonError::NeedMoreData)?;

                let left = if mb_x > 0 {
                    Some(mb_types[mb_idx - 1])
                } else {
                    None
                };
                let top = if mb_y > 0 {
                    Some(mb_types[mb_idx - mbs_w])
                } else {
                    None
                };

                let mb_type = decode_mb_type_intra(&mut cabac, &mut ctx0_10, left, top);
                mb_types[mb_idx] = mb_type;

                bitpos += cabac.bits_read();

                if mb_type != 25 {
                    return Err(VideosonError::Unsupported(
                        "CABAC: only I_PCM macroblocks supported in M0",
                    ));
                }

                let mut br_pcm = BitReader::new(slice_data);
                br_pcm.set_bit_pos(bitpos).map_err(map_bs)?;
                br_pcm.byte_align_zero().map_err(map_bs)?;

                for r in 0..16 {
                    for c in 0..16 {
                        let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                        let x = mb_x * 16 + c;
                        let y_row = mb_y * 16 + r;
                        if x < width_us && y_row < height_us {
                            y[y_row * y_stride + x] = b;
                        }
                    }
                }
                if chroma_format_idc == 1 {
                    for r in 0..8 {
                        for c in 0..8 {
                            let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 8 + c;
                            let y_row = mb_y * 8 + r;
                            if x < chroma_w && y_row < chroma_h {
                                u[y_row * chroma_w + x] = b;
                            }
                        }
                    }
                    for r in 0..8 {
                        for c in 0..8 {
                            let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 8 + c;
                            let y_row = mb_y * 8 + r;
                            if x < chroma_w && y_row < chroma_h {
                                v[y_row * chroma_w + x] = b;
                            }
                        }
                    }
                }

                bitpos = br_pcm.bit_pos();
            }
        }

        if chroma_format_idc == 0 {
            Ok(VideoFrame {
                width,
                height,
                planes: videoson_core::VideoFramePlanes::Mono,
                pixfmt: videoson_core::PixelFormat::Gray,
                bit_depth,
                plane_data: vec![VideoPlane {
                    stride: y_stride,
                    data: PlaneData::U8(y),
                }],
            })
        } else {
            Ok(VideoFrame {
                width,
                height,
                planes: videoson_core::VideoFramePlanes::Yuv420,
                pixfmt: videoson_core::PixelFormat::Yuv420,
                bit_depth,
                plane_data: vec![
                    VideoPlane {
                        stride: y_stride,
                        data: PlaneData::U8(y),
                    },
                    VideoPlane {
                        stride: chroma_w,
                        data: PlaneData::U8(u),
                    },
                    VideoPlane {
                        stride: chroma_w,
                        data: PlaneData::U8(v),
                    },
                ],
            })
        }
    }
}

fn map_bs(e: BitstreamError) -> VideosonError {
    match e {
        BitstreamError::Eof => VideosonError::NeedMoreData,
        BitstreamError::Invalid(s) => VideosonError::InvalidData(s),
        BitstreamError::Message(s) => VideosonError::Message(s),
        _ => VideosonError::InvalidData("unknown bitstream error"),
    }
}
