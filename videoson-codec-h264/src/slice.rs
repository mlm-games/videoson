// videoson-codec-h264/src/slice.rs
extern crate alloc;

use videoson_common::{read_se, read_ue, BitReader, BitstreamError, BitstreamResult};
use videoson_core::VideosonError;

use crate::cabac::{decode_mb_type_intra, init_ctx_i_slice_0_10, CabacDecoder};
use crate::decoder::{ParamSets, PendingPic, PendingPlanes};
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

pub fn parse_slice_header_rbsp(
    rbsp: &[u8],
    ps: &ParamSets,
) -> BitstreamResult<(SliceHeader, usize)> {
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

    let header_bits = br.bit_pos();

    Ok((
        SliceHeader {
            first_mb_in_slice,
            slice_type,
            pps_id,
            frame_num,
            idr_pic_id: Some(idr_pic_id),
            pic_order_cnt_lsb,
            slice_qp_delta,
        },
        header_bits,
    ))
}

fn map_bs(e: BitstreamError) -> VideosonError {
    match e {
        BitstreamError::Eof => VideosonError::NeedMoreData,
        BitstreamError::Invalid(s) => VideosonError::InvalidData(s),
        BitstreamError::Message(s) => VideosonError::Message(s),
        _ => VideosonError::InvalidData("unknown bitstream error"),
    }
}

pub fn decode_idr_ipcm_slice_into_pic(
    rbsp: &[u8],
    header_bits: usize,
    ps: &ParamSets,
    sh: &SliceHeader,
    pps: &Pps,
    pic: &mut PendingPic,
) -> Result<(), VideosonError> {
    let sps = ps.get_sps(pps.sps_id)?;

    if sh.first_mb_in_slice as usize >= pic.total_mbs() {
        return Err(VideosonError::InvalidData("first_mb_in_slice out of range"));
    }

    let slice_qpy: i32 = 26 + pps.pic_init_qp_minus26 + sh.slice_qp_delta;

    let mut br = BitReader::new(rbsp);
    br.set_bit_pos(header_bits).map_err(map_bs)?;

    if !pps.entropy_coding_mode_flag {
        // CAVLC path
        let mut mb_addr = sh.first_mb_in_slice as usize;
        while mb_addr < pic.total_mbs() && br.more_rbsp_data().map_err(map_bs)? {
            let mb_x = mb_addr % pic.mbs_w;
            let mb_y = mb_addr / pic.mbs_w;

            let mb_type = read_ue(&mut br).map_err(map_bs)? as u8;
            if mb_type != 25 {
                return Err(VideosonError::Unsupported(
                    "CAVLC: only I_PCM macroblocks supported in M0",
                ));
            }

            br.byte_align_zero().map_err(map_bs)?;

            match (&mut pic.planes, pic.bit_depth, pic.chroma_format_idc) {
                (PendingPlanes::Mono8 { y }, bit_depth, 0) if bit_depth <= 8 => {
                    let w = pic.width as usize;
                    let h = pic.height as usize;
                    let y_stride = pic.y_stride;
                    for r in 0..16 {
                        for c in 0..16 {
                            let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 16 + c;
                            let yy = mb_y * 16 + r;
                            if x < w && yy < h {
                                y[yy * y_stride + x] = b;
                            }
                        }
                    }
                }
                (PendingPlanes::Yuv4208 { y, u, v }, bit_depth, 1) if bit_depth <= 8 => {
                    let w = pic.width as usize;
                    let h = pic.height as usize;
                    let y_stride = pic.y_stride;
                    let cw = pic.chroma_w;
                    let ch = pic.chroma_h;
                    let uv_stride = pic.uv_stride;
                    for r in 0..16 {
                        for c in 0..16 {
                            let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 16 + c;
                            let yy = mb_y * 16 + r;
                            if x < w && yy < h {
                                y[yy * y_stride + x] = b;
                            }
                        }
                    }
                    for r in 0..8 {
                        for c in 0..8 {
                            let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 8 + c;
                            let yy = mb_y * 8 + r;
                            if x < cw && yy < ch {
                                u[yy * uv_stride + x] = b;
                            }
                        }
                    }
                    for r in 0..8 {
                        for c in 0..8 {
                            let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 8 + c;
                            let yy = mb_y * 8 + r;
                            if x < cw && yy < ch {
                                v[yy * uv_stride + x] = b;
                            }
                        }
                    }
                }
                (PendingPlanes::Mono16 { y }, bit_depth, 0) if bit_depth > 8 => {
                    let w = pic.width as usize;
                    let h = pic.height as usize;
                    let y_stride = pic.y_stride;
                    for r in 0..16 {
                        for c in 0..16 {
                            let s = br.read_bits_u16(bit_depth as u32).map_err(map_bs)? as u16;
                            let x = mb_x * 16 + c;
                            let yy = mb_y * 16 + r;
                            if x < w && yy < h {
                                y[yy * y_stride + x] = s;
                            }
                        }
                    }
                }
                (PendingPlanes::Yuv42016 { y, u, v }, bit_depth, 1) if bit_depth > 8 => {
                    let w = pic.width as usize;
                    let h = pic.height as usize;
                    let y_stride = pic.y_stride;
                    let cw = pic.chroma_w;
                    let ch = pic.chroma_h;
                    let uv_stride = pic.uv_stride;
                    for r in 0..16 {
                        for c in 0..16 {
                            let s = br.read_bits_u16(bit_depth as u32).map_err(map_bs)? as u16;
                            let x = mb_x * 16 + c;
                            let yy = mb_y * 16 + r;
                            if x < w && yy < h {
                                y[yy * y_stride + x] = s;
                            }
                        }
                    }
                    for r in 0..8 {
                        for c in 0..8 {
                            let s = br.read_bits_u16(bit_depth as u32).map_err(map_bs)? as u16;
                            let x = mb_x * 8 + c;
                            let yy = mb_y * 8 + r;
                            if x < cw && yy < ch {
                                u[yy * uv_stride + x] = s;
                            }
                        }
                    }
                    for r in 0..8 {
                        for c in 0..8 {
                            let s = br.read_bits_u16(bit_depth as u32).map_err(map_bs)? as u16;
                            let x = mb_x * 8 + c;
                            let yy = mb_y * 8 + r;
                            if x < cw && yy < ch {
                                v[yy * uv_stride + x] = s;
                            }
                        }
                    }
                }
                _ => {
                    return Err(VideosonError::InvalidData(
                        "internal: PendingPic plane layout mismatch",
                    ))
                }
            }

            pic.mark_mb_decoded(mb_addr, mb_type);
            mb_addr += 1;
        }

        Ok(())
    } else {
        // CABAC path
        if pic.bit_depth > 8 {
            return Err(VideosonError::Unsupported(
                "CABAC: bit_depth > 8 not supported in M0",
            ));
        }

        while !br.is_byte_aligned() {
            let b = br.read_bit().map_err(map_bs)?;
            if !b {
                return Err(VideosonError::InvalidData(
                    "CABAC alignment: expected cabac_alignment_one_bit == 1",
                ));
            }
        }

        let slice_bytes = br.remaining_bytes().map_err(map_bs)?;

        let mut cabac = CabacDecoder::new(slice_bytes)?;
        let mut ctx0_10 = init_ctx_i_slice_0_10(slice_qpy);

        let mut mb_addr = sh.first_mb_in_slice as usize;

        loop {
            if mb_addr >= pic.total_mbs() {
                return Err(VideosonError::InvalidData("CABAC: mb_addr out of range"));
            }

            let mb_x = mb_addr % pic.mbs_w;
            let mb_y = mb_addr / pic.mbs_w;

            let left = if mb_x > 0 {
                pic.neighbor_mb_type(mb_addr - 1)
            } else {
                None
            };
            let top = if mb_y > 0 {
                pic.neighbor_mb_type(mb_addr - pic.mbs_w)
            } else {
                None
            };

            let mb_type = decode_mb_type_intra(&mut cabac, &mut ctx0_10, left, top);
            if mb_type != 25 {
                return Err(VideosonError::Unsupported(
                    "CABAC: only I_PCM macroblocks supported in M0",
                ));
            }

            let mut br_pcm = BitReader::new(slice_bytes);
            br_pcm.set_bit_pos(cabac.bit_pos()).map_err(map_bs)?;
            br_pcm.byte_align_zero().map_err(map_bs)?;

            match (&mut pic.planes, pic.chroma_format_idc) {
                (PendingPlanes::Mono8 { y }, 0) => {
                    let w = pic.width as usize;
                    let h = pic.height as usize;
                    let y_stride = pic.y_stride;
                    for r in 0..16 {
                        for c in 0..16 {
                            let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 16 + c;
                            let yy = mb_y * 16 + r;
                            if x < w && yy < h {
                                y[yy * y_stride + x] = b;
                            }
                        }
                    }
                }
                (PendingPlanes::Yuv4208 { y, u, v }, 1) => {
                    let w = pic.width as usize;
                    let h = pic.height as usize;
                    let y_stride = pic.y_stride;
                    let cw = pic.chroma_w;
                    let ch = pic.chroma_h;
                    let uv_stride = pic.uv_stride;
                    for r in 0..16 {
                        for c in 0..16 {
                            let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 16 + c;
                            let yy = mb_y * 16 + r;
                            if x < w && yy < h {
                                y[yy * y_stride + x] = b;
                            }
                        }
                    }
                    for r in 0..8 {
                        for c in 0..8 {
                            let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 8 + c;
                            let yy = mb_y * 8 + r;
                            if x < cw && yy < ch {
                                u[yy * uv_stride + x] = b;
                            }
                        }
                    }
                    for r in 0..8 {
                        for c in 0..8 {
                            let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                            let x = mb_x * 8 + c;
                            let yy = mb_y * 8 + r;
                            if x < cw && yy < ch {
                                v[yy * uv_stride + x] = b;
                            }
                        }
                    }
                }
                _ => {
                    return Err(VideosonError::InvalidData(
                        "internal: CABAC expects 8-bit PendingPlanes",
                    ))
                }
            }

            cabac.set_bit_pos(br_pcm.bit_pos())?;
            cabac.reinit_engine()?;

            pic.mark_mb_decoded(mb_addr, mb_type);

            let end = cabac.decode_end_of_slice_flag();
            if end {
                break;
            }

            mb_addr += 1;
        }

        Ok(())
    }
}
