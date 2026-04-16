// videoson-codec-h264/src/slice.rs
extern crate alloc;

use videoson_common::{read_se, read_ue, BitReader, BitstreamError, BitstreamResult};
use videoson_core::VideosonError;

use crate::cabac::{decode_mb_type_intra, init_ctx_i_slice_0_10, CabacDecoder};
use crate::cabac_models::CabacModels;
use crate::cabac_residual::{residual_4x4block_cabac, CtxBlockCat};
use crate::cavlc::{residual_block_cavlc, CoeffBlock};
use crate::decoder::{ParamSets, PendingPic, PendingPlanes};
use crate::intra_pred::{pred_chroma_8x8, pred_luma_16x16, pred_luma_4x4, Ref4x4};
use crate::pps::Pps;
use crate::transform::{
    clip_u8, dequant_4x4, inv_hadamard_2x2, inv_hadamard_4x4, inv_transform_4x4,
};

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

/// Returns (SliceHeader, header_bits_consumed)
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
        return Err(BitstreamError::Invalid("only I slices supported"));
    }

    let frame_num_bits = sps.frame_num_bits();
    let frame_num = br.read_bits_u32(frame_num_bits)?;

    let idr_pic_id = read_ue(&mut br)?;

    let pic_order_cnt_lsb = if sps.pic_order_cnt_type == 0 {
        let n = sps.pic_order_cnt_lsb_bits();
        Some(br.read_bits_u32(n)?)
    } else {
        return Err(BitstreamError::Invalid(
            "pic_order_cnt_type != 0 not supported",
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

// Zigzag scan for 4x4: scan_idx -> raster_idx
const ZIGZAG_4X4: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

// Intra coded_block_pattern mapping (Table 9-5 style mapping used by many decoders)
const INTRA_GOLOMB_TO_CBP: [u8; 48] = [
    47, 31, 15, 0, 23, 27, 29, 30, 7, 11, 13, 14, 39, 43, 45, 46, 16, 3, 5, 10, 12, 19, 21, 26, 28,
    35, 37, 42, 44, 1, 2, 4, 8, 17, 18, 20, 24, 6, 9, 22, 25, 32, 33, 34, 36, 40, 38, 41,
];

const QP_CHROMA_MAP: [i32; 52] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, //
    10, 11, 12, 13, 14, 15, 16, 17, 18, 19, //
    20, 21, 22, 23, 24, 25, 26, 27, 28, 29, //
    29, 30, 31, 32, 33, 34, 35, 36, 37, 38, //
    39, 40, 41, 42, 43, 44, 45, 46, 47, 48, //
    49, 50,
];

fn clip_qp(qp: i32) -> i32 {
    qp.clamp(0, 51)
}

fn qp_c_from_qp_y(qp_y: i32, chroma_qp_index_offset: i32) -> i32 {
    let idx = clip_qp(qp_y + chroma_qp_index_offset) as usize;
    QP_CHROMA_MAP[idx]
}

fn get_luma_u8(pic: &PendingPic, x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 {
        return 128;
    }
    let x = x as usize;
    let y = y as usize;
    if x >= pic.width as usize || y >= pic.height as usize {
        return 128;
    }
    match &pic.planes {
        PendingPlanes::Mono8 { y: yy } => yy[y * pic.y_stride + x],
        PendingPlanes::Yuv4208 { y: yy, .. } => yy[y * pic.y_stride + x],
        _ => 128,
    }
}

fn get_chroma_u8(plane: &[u8], stride: usize, w: usize, h: usize, x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 {
        return 128;
    }
    let x = x as usize;
    let y = y as usize;
    if x >= w || y >= h {
        return 128;
    }
    plane[y * stride + x]
}

fn refs_4x4_luma(pic: &PendingPic, mb_x: usize, mb_y: usize, bx: usize, by: usize) -> Ref4x4 {
    let (x0, y0) = (mb_x * 16 + bx * 4, mb_y * 16 + by * 4);

    let tl = get_luma_u8(pic, x0 as i32 - 1, y0 as i32 - 1);

    let mut t = [128u8; 8];
    for i in 0..8 {
        t[i] = get_luma_u8(pic, x0 as i32 + i as i32, y0 as i32 - 1);
    }

    let mut l = [128u8; 4];
    for i in 0..4 {
        l[i] = get_luma_u8(pic, x0 as i32 - 1, y0 as i32 + i as i32);
    }

    Ref4x4 { tl, t, l }
}

fn refs_16x16_luma(pic: &PendingPic, mb_x: usize, mb_y: usize) -> ([u8; 16], [u8; 16], u8) {
    let x0 = (mb_x * 16) as i32;
    let y0 = (mb_y * 16) as i32;

    let tl = get_luma_u8(pic, x0 - 1, y0 - 1);

    let mut top = [128u8; 16];
    for i in 0..16 {
        top[i] = get_luma_u8(pic, x0 + i as i32, y0 - 1);
    }
    let mut left = [128u8; 16];
    for i in 0..16 {
        left[i] = get_luma_u8(pic, x0 - 1, y0 + i as i32);
    }
    (top, left, tl)
}

fn refs_8x8_chroma(
    pic: &PendingPic,
    mb_x: usize,
    mb_y: usize,
    plane: &[u8],
    stride: usize,
) -> ([u8; 8], [u8; 8], u8) {
    let x0 = (mb_x * 8) as i32;
    let y0 = (mb_y * 8) as i32;

    let tl = get_chroma_u8(plane, stride, pic.chroma_w, pic.chroma_h, x0 - 1, y0 - 1);

    let mut top = [128u8; 8];
    for i in 0..8 {
        top[i] = get_chroma_u8(
            plane,
            stride,
            pic.chroma_w,
            pic.chroma_h,
            x0 + i as i32,
            y0 - 1,
        );
    }
    let mut left = [128u8; 8];
    for i in 0..8 {
        left[i] = get_chroma_u8(
            plane,
            stride,
            pic.chroma_w,
            pic.chroma_h,
            x0 - 1,
            y0 + i as i32,
        );
    }
    (top, left, tl)
}

fn map_coeffs_scan_to_raster(coeff_scan: &[i32; 16]) -> [i32; 16] {
    let mut out = [0i32; 16];
    for s in 0..16 {
        let r = ZIGZAG_4X4[s];
        out[r] = coeff_scan[s];
    }
    out
}

fn inv_residual_from_coeff_scan(coeff_scan_full: [i32; 16], qp: i32) -> [i32; 16] {
    let coeff_raster = map_coeffs_scan_to_raster(&coeff_scan_full);

    let mut dq = [0i32; 16];
    for i in 0..16 {
        dq[i] = dequant_4x4(qp, coeff_raster[i], i);
    }

    inv_transform_4x4(dq)
}

fn write_4x4(
    dst: &mut [u8],
    stride: usize,
    w: usize,
    h: usize,
    x0: usize,
    y0: usize,
    blk: &[u8; 16],
) {
    for y in 0..4 {
        for x in 0..4 {
            let xx = x0 + x;
            let yy = y0 + y;
            if xx < w && yy < h {
                dst[yy * stride + xx] = blk[y * 4 + x];
            }
        }
    }
}

fn write_4x4_pred_plus_res(
    dst: &mut [u8],
    stride: usize,
    w: usize,
    h: usize,
    x0: usize,
    y0: usize,
    pred: [u8; 16],
    res: [i32; 16],
) {
    for y in 0..4 {
        for x in 0..4 {
            let xx = x0 + x;
            let yy = y0 + y;
            if xx < w && yy < h {
                let p = pred[y * 4 + x] as i32;
                let r = res[y * 4 + x];
                dst[yy * stride + xx] = clip_u8(p + r);
            }
        }
    }
}

fn pred4_from_pred16(pred16: &[u8; 256], bx: usize, by: usize) -> [u8; 16] {
    let mut out = [0u8; 16];
    for y in 0..4 {
        for x in 0..4 {
            out[y * 4 + x] = pred16[(by * 4 + y) * 16 + (bx * 4 + x)];
        }
    }
    out
}

fn pred4_from_pred8(pred8: &[u8; 64], bx: usize, by: usize) -> [u8; 16] {
    let mut out = [0u8; 16];
    for y in 0..4 {
        for x in 0..4 {
            out[y * 4 + x] = pred8[(by * 4 + y) * 8 + (bx * 4 + x)];
        }
    }
    out
}

// -------------------------
// CAVLC helpers (Part 3)
// -------------------------

fn cavlc_nc_luma(pic: &PendingPic, mb_addr: usize, mb_x: usize, mb_y: usize, blk: usize) -> i32 {
    let col = blk % 4;
    let row = blk / 4;

    let na = if col > 0 {
        pic.nz_y[pic.idx_y4(mb_addr, blk - 1)] as i32
    } else if mb_x > 0 {
        let left_mb = mb_addr - 1;
        let left_blk = row * 4 + 3;
        if pic.filled[left_mb] {
            pic.nz_y[pic.idx_y4(left_mb, left_blk)] as i32
        } else {
            -1
        }
    } else {
        -1
    };

    let nb = if row > 0 {
        pic.nz_y[pic.idx_y4(mb_addr, blk - 4)] as i32
    } else if mb_y > 0 {
        let top_mb = mb_addr - pic.mbs_w;
        let top_blk = 12 + col;
        if pic.filled[top_mb] {
            pic.nz_y[pic.idx_y4(top_mb, top_blk)] as i32
        } else {
            -1
        }
    } else {
        -1
    };

    match (na >= 0, nb >= 0) {
        (true, true) => (na + nb + 1) / 2,
        (true, false) => na,
        (false, true) => nb,
        _ => 0,
    }
}

fn cavlc_nc_chroma(
    pic: &PendingPic,
    mb_addr: usize,
    mb_x: usize,
    mb_y: usize,
    blk: usize,
    nz_plane: &[u8],
) -> i32 {
    let col = blk % 2;
    let row = blk / 2;

    let na = if col > 0 {
        nz_plane[pic.idx_c4(mb_addr, blk - 1)] as i32
    } else if mb_x > 0 {
        let left_mb = mb_addr - 1;
        let left_blk = row * 2 + 1;
        if pic.filled[left_mb] {
            nz_plane[pic.idx_c4(left_mb, left_blk)] as i32
        } else {
            -1
        }
    } else {
        -1
    };

    let nb = if row > 0 {
        nz_plane[pic.idx_c4(mb_addr, blk - 2)] as i32
    } else if mb_y > 0 {
        let top_mb = mb_addr - pic.mbs_w;
        let top_blk = 2 + col;
        if pic.filled[top_mb] {
            nz_plane[pic.idx_c4(top_mb, top_blk)] as i32
        } else {
            -1
        }
    } else {
        -1
    };

    match (na >= 0, nb >= 0) {
        (true, true) => (na + nb + 1) / 2,
        (true, false) => na,
        (false, true) => nb,
        _ => 0,
    }
}

fn mpm_intra4x4(pic: &PendingPic, mb_addr: usize, mb_x: usize, mb_y: usize, blk: usize) -> u8 {
    let col = blk % 4;
    let row = blk / 4;

    let a = if col > 0 {
        Some(pic.intra4x4_modes[pic.idx_y4(mb_addr, blk - 1)])
    } else if mb_x > 0 {
        let left_mb = mb_addr - 1;
        let left_blk = row * 4 + 3;
        if pic.filled[left_mb] {
            Some(pic.intra4x4_modes[pic.idx_y4(left_mb, left_blk)])
        } else {
            None
        }
    } else {
        None
    };

    let b = if row > 0 {
        Some(pic.intra4x4_modes[pic.idx_y4(mb_addr, blk - 4)])
    } else if mb_y > 0 {
        let top_mb = mb_addr - pic.mbs_w;
        let top_blk = 12 + col;
        if pic.filled[top_mb] {
            Some(pic.intra4x4_modes[pic.idx_y4(top_mb, top_blk)])
        } else {
            None
        }
    } else {
        None
    };

    match (a, b) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        _ => 2, // DC
    }
}

// -------------------------
// CABAC helpers (Part 4a)
// -------------------------

fn decode_intra_chroma_pred_mode_cabac(
    cabac: &mut CabacDecoder<'_>,
    models: &mut CabacModels,
    ctx_inc: usize, // 0..2
) -> u8 {
    // truncated unary with Cmax=3, ctxIdxOffset=64 (Table 9-17)
    // We use:
    // bin0: ctxIdx = 64 + ctxInc
    // bin1/bin2: ctxIdx = 67
    let b0 = cabac.decode_decision(models.ctx_mut(64 + ctx_inc));
    if b0 == 0 {
        return 0;
    }
    let b1 = cabac.decode_decision(models.ctx_mut(67));
    if b1 == 0 {
        return 1;
    }
    let b2 = cabac.decode_decision(models.ctx_mut(67));
    if b2 == 0 {
        return 2;
    }
    3
}

fn decode_mb_qp_delta_cabac(
    cabac: &mut CabacDecoder<'_>,
    models: &mut CabacModels,
    ctx_inc: usize, // 0..2
) -> i32 {
    // Signed unary-ish scheme used in practice for mb_qp_delta with ctxIdxOffset=60.
    // First bin uses ctx 60+ctxInc, next uses 62, further uses 63, sign in bypass.
    let b0 = cabac.decode_decision(models.ctx_mut(60 + ctx_inc));
    if b0 == 0 {
        return 0;
    }

    let mut abs: u32 = 1;
    let b1 = cabac.decode_decision(models.ctx_mut(62));
    if b1 != 0 {
        abs += 1;
        loop {
            let bx = cabac.decode_decision(models.ctx_mut(63));
            if bx == 0 {
                break;
            }
            abs += 1;
            if abs > 52 {
                // absurd for mb_qp_delta
                break;
            }
        }
    }

    let sign = cabac.decode_bypass();
    if sign != 0 {
        -(abs as i32)
    } else {
        abs as i32
    }
}

fn coded_left_luma_dc(pic: &PendingPic, mb_addr: usize, mb_x: usize) -> bool {
    if mb_x == 0 {
        return false;
    }
    let left = mb_addr - 1;
    pic.filled[left] && pic.nz_y_dc[left] != 0
}

fn coded_top_luma_dc(pic: &PendingPic, mb_addr: usize, mb_y: usize) -> bool {
    if mb_y == 0 {
        return false;
    }
    let top = mb_addr - pic.mbs_w;
    pic.filled[top] && pic.nz_y_dc[top] != 0
}

fn coded_left_luma_ac(pic: &PendingPic, mb_addr: usize, mb_x: usize, blk: usize) -> bool {
    let bx = blk % 4;
    let by = blk / 4;
    if bx > 0 {
        pic.nz_y[pic.idx_y4(mb_addr, blk - 1)] != 0
    } else if mb_x > 0 {
        let left_mb = mb_addr - 1;
        if !pic.filled[left_mb] {
            return false;
        }
        let left_blk = by * 4 + 3;
        pic.nz_y[pic.idx_y4(left_mb, left_blk)] != 0
    } else {
        false
    }
}

fn coded_top_luma_ac(pic: &PendingPic, mb_addr: usize, mb_y: usize, blk: usize) -> bool {
    let bx = blk % 4;
    let by = blk / 4;
    if by > 0 {
        pic.nz_y[pic.idx_y4(mb_addr, blk - 4)] != 0
    } else if mb_y > 0 {
        let top_mb = mb_addr - pic.mbs_w;
        if !pic.filled[top_mb] {
            return false;
        }
        let top_blk = 12 + bx;
        pic.nz_y[pic.idx_y4(top_mb, top_blk)] != 0
    } else {
        false
    }
}

fn coded_left_chroma_dc(pic: &PendingPic, mb_addr: usize, mb_x: usize, is_u: bool) -> bool {
    if mb_x == 0 {
        return false;
    }
    let left = mb_addr - 1;
    if !pic.filled[left] {
        return false;
    }
    if is_u {
        pic.nz_u_dc[left] != 0
    } else {
        pic.nz_v_dc[left] != 0
    }
}

fn coded_top_chroma_dc(pic: &PendingPic, mb_addr: usize, mb_y: usize, is_u: bool) -> bool {
    if mb_y == 0 {
        return false;
    }
    let top = mb_addr - pic.mbs_w;
    if !pic.filled[top] {
        return false;
    }
    if is_u {
        pic.nz_u_dc[top] != 0
    } else {
        pic.nz_v_dc[top] != 0
    }
}

fn coded_left_chroma_ac(
    pic: &PendingPic,
    mb_addr: usize,
    mb_x: usize,
    blk: usize,
    nz: &[u8],
) -> bool {
    let bx = blk % 2;
    let by = blk / 2;
    if bx > 0 {
        nz[pic.idx_c4(mb_addr, blk - 1)] != 0
    } else if mb_x > 0 {
        let left_mb = mb_addr - 1;
        if !pic.filled[left_mb] {
            return false;
        }
        let left_blk = by * 2 + 1;
        nz[pic.idx_c4(left_mb, left_blk)] != 0
    } else {
        false
    }
}

fn coded_top_chroma_ac(
    pic: &PendingPic,
    mb_addr: usize,
    mb_y: usize,
    blk: usize,
    nz: &[u8],
) -> bool {
    let bx = blk % 2;
    let by = blk / 2;
    if by > 0 {
        nz[pic.idx_c4(mb_addr, blk - 2)] != 0
    } else if mb_y > 0 {
        let top_mb = mb_addr - pic.mbs_w;
        if !pic.filled[top_mb] {
            return false;
        }
        let top_blk = 2 + bx;
        nz[pic.idx_c4(top_mb, top_blk)] != 0
    } else {
        false
    }
}

// --------------------------------------
// Main entry: decode slice into PendingPic
// --------------------------------------

pub fn decode_idr_slice_into_pic(
    rbsp: &[u8],
    header_bits: usize,
    ps: &ParamSets,
    sh: &SliceHeader,
    pps: &Pps,
    pic: &mut PendingPic,
) -> Result<(), VideosonError> {
    let _sps = ps.get_sps(pps.sps_id)?;

    if pic.bit_depth != 8 {
        return Err(VideosonError::Unsupported("Part4a: only 8-bit supported"));
    }
    if pic.chroma_format_idc != 0 && pic.chroma_format_idc != 1 {
        return Err(VideosonError::Unsupported(
            "Part4a: only mono and 4:2:0 supported",
        ));
    }

    let mut br = BitReader::new(rbsp);
    br.set_bit_pos(header_bits).map_err(map_bs)?;

    let slice_qpy: i32 = clip_qp(26 + pps.pic_init_qp_minus26 + sh.slice_qp_delta);

    if !pps.entropy_coding_mode_flag {
        // ==========================
        // CAVLC path (Part 3)
        // ==========================
        let mut mb_addr = sh.first_mb_in_slice as usize;
        let mut qp_y = slice_qpy;

        while mb_addr < pic.total_mbs() && br.more_rbsp_data().map_err(map_bs)? {
            let mb_x = mb_addr % pic.mbs_w;
            let mb_y = mb_addr / pic.mbs_w;

            let mb_type = read_ue(&mut br).map_err(map_bs)? as u32;

            // 25 = I_PCM
            if mb_type == 25 {
                br.byte_align_zero().map_err(map_bs)?;

                match &mut pic.planes {
                    PendingPlanes::Mono8 { y } => {
                        for r in 0..16 {
                            for c in 0..16 {
                                let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 16 + c;
                                let yy = mb_y * 16 + r;
                                if x < pic.width as usize && yy < pic.height as usize {
                                    y[yy * pic.y_stride + x] = b;
                                }
                            }
                        }
                    }
                    PendingPlanes::Yuv4208 { y, u, v } => {
                        for r in 0..16 {
                            for c in 0..16 {
                                let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 16 + c;
                                let yy = mb_y * 16 + r;
                                if x < pic.width as usize && yy < pic.height as usize {
                                    y[yy * pic.y_stride + x] = b;
                                }
                            }
                        }
                        for r in 0..8 {
                            for c in 0..8 {
                                let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 8 + c;
                                let yy = mb_y * 8 + r;
                                if x < pic.chroma_w && yy < pic.chroma_h {
                                    u[yy * pic.uv_stride + x] = b;
                                }
                            }
                        }
                        for r in 0..8 {
                            for c in 0..8 {
                                let b = br.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 8 + c;
                                let yy = mb_y * 8 + r;
                                if x < pic.chroma_w && yy < pic.chroma_h {
                                    v[yy * pic.uv_stride + x] = b;
                                }
                            }
                        }
                    }
                    _ => return Err(VideosonError::InvalidData("plane mismatch")),
                }

                pic.nz_y_dc[mb_addr] = 0;
                for i in 0..16 {
                    pic.nz_y[pic.idx_y4(mb_addr, i)] = 0;
                    pic.intra4x4_modes[pic.idx_y4(mb_addr, i)] = 2;
                }
                for i in 0..4 {
                    pic.nz_u[pic.idx_c4(mb_addr, i)] = 0;
                    pic.nz_v[pic.idx_c4(mb_addr, i)] = 0;
                }

                pic.mark_mb_decoded(mb_addr, 25);
                mb_addr += 1;
                continue;
            }

            // mb_type 0 => I_NxN (Intra4x4)
            if mb_type == 0 {
                // parse 16 intra4x4 modes first
                for blk in 0..16 {
                    let mpm = mpm_intra4x4(pic, mb_addr, mb_x, mb_y, blk);
                    let prev_flag = br.read_bit().map_err(map_bs)?;
                    let mode = if prev_flag {
                        mpm
                    } else {
                        let rem = br.read_bits_u32(3).map_err(map_bs)? as u8;
                        if rem < mpm {
                            rem
                        } else {
                            rem + 1
                        }
                    };
                    pic.intra4x4_modes[pic.idx_y4(mb_addr, blk)] = mode;
                }

                let intra_chroma_pred_mode = if pic.chroma_format_idc == 1 {
                    read_ue(&mut br).map_err(map_bs)? as u8
                } else {
                    0
                };

                let cbp_code = read_ue(&mut br).map_err(map_bs)? as usize;
                if cbp_code > 47 {
                    return Err(VideosonError::InvalidData(
                        "coded_block_pattern out of range",
                    ));
                }
                let cbp = INTRA_GOLOMB_TO_CBP[cbp_code] as u32;
                let cbp_luma = cbp & 0x0F;
                let cbp_chroma = (cbp >> 4) & 0x03;

                if cbp_luma != 0 || cbp_chroma != 0 {
                    let mb_qp_delta = read_se(&mut br).map_err(map_bs)?;
                    qp_y = clip_qp(qp_y + mb_qp_delta);
                }

                // luma blocks
                let y = match &mut pic.planes {
                    PendingPlanes::Yuv4208 { y, .. } => y,
                    PendingPlanes::Mono8 { y } => y,
                    _ => return Err(VideosonError::InvalidData("plane mismatch")),
                };

                let y_w = pic.width as usize;
                let y_h = pic.height as usize;

                for blk in 0..16 {
                    let bx = blk % 4;
                    let by = blk / 4;

                    let mode = pic.intra4x4_modes[pic.idx_y4(mb_addr, blk)];
                    let refs = refs_4x4_luma(pic, mb_x, mb_y, bx, by);
                    let pred = pred_luma_4x4(mode, refs);

                    let grp = (bx / 2) + (by / 2) * 2;
                    let has_residual = ((cbp_luma >> grp) & 1) != 0;

                    if has_residual {
                        let n_c = cavlc_nc_luma(pic, mb_addr, mb_x, mb_y, blk);
                        let coeff = residual_block_cavlc(&mut br, n_c, 16).map_err(map_bs)?;
                        pic.nz_y[pic.idx_y4(mb_addr, blk)] = coeff.total_coeff as u8;

                        let res = inv_residual_from_coeff_scan(coeff.levels_scan, qp_y);

                        let x0 = mb_x * 16 + bx * 4;
                        let y0 = mb_y * 16 + by * 4;
                        write_4x4_pred_plus_res(y, pic.y_stride, y_w, y_h, x0, y0, pred, res);
                    } else {
                        pic.nz_y[pic.idx_y4(mb_addr, blk)] = 0;
                        let x0 = mb_x * 16 + bx * 4;
                        let y0 = mb_y * 16 + by * 4;
                        write_4x4(y, pic.y_stride, y_w, y_h, x0, y0, &pred);
                    }
                }
                pic.nz_y_dc[mb_addr] = 0;

                // chroma
                if pic.chroma_format_idc == 1 {
                    let qp_c = qp_c_from_qp_y(qp_y, pps.chroma_qp_index_offset);

                    let PendingPlanes::Yuv4208 { y: _, u, v } = &mut pic.planes else {
                        return Err(VideosonError::InvalidData("expected chroma planes"));
                    };

                    let pred_u8x8 = {
                        let (top, left, tl) = refs_8x8_chroma(pic, mb_x, mb_y, u, pic.uv_stride);
                        pred_chroma_8x8(intra_chroma_pred_mode, top, left, tl)
                    };
                    let pred_v8x8 = {
                        let (top, left, tl) = refs_8x8_chroma(pic, mb_x, mb_y, v, pic.uv_stride);
                        pred_chroma_8x8(intra_chroma_pred_mode, top, left, tl)
                    };

                    if cbp_chroma == 0 {
                        for by in 0..2 {
                            for bx in 0..2 {
                                let blk = by * 2 + bx;
                                let pred4u = pred4_from_pred8(&pred_u8x8, bx, by);
                                let pred4v = pred4_from_pred8(&pred_v8x8, bx, by);
                                let x0 = mb_x * 8 + bx * 4;
                                let y0 = mb_y * 8 + by * 4;

                                write_4x4(
                                    u,
                                    pic.uv_stride,
                                    pic.chroma_w,
                                    pic.chroma_h,
                                    x0,
                                    y0,
                                    &pred4u,
                                );
                                write_4x4(
                                    v,
                                    pic.uv_stride,
                                    pic.chroma_w,
                                    pic.chroma_h,
                                    x0,
                                    y0,
                                    &pred4v,
                                );

                                pic.nz_u[pic.idx_c4(mb_addr, blk)] = 0;
                                pic.nz_v[pic.idx_c4(mb_addr, blk)] = 0;
                            }
                        }
                    } else {
                        let u_dc = residual_block_cavlc(&mut br, -1, 4).map_err(map_bs)?;
                        let v_dc = residual_block_cavlc(&mut br, -1, 4).map_err(map_bs)?;
                        let u_dc_in = inv_hadamard_2x2([
                            u_dc.levels_scan[0],
                            u_dc.levels_scan[1],
                            u_dc.levels_scan[2],
                            u_dc.levels_scan[3],
                        ]);
                        let v_dc_in = inv_hadamard_2x2([
                            v_dc.levels_scan[0],
                            v_dc.levels_scan[1],
                            v_dc.levels_scan[2],
                            v_dc.levels_scan[3],
                        ]);

                        for by in 0..2 {
                            for bx in 0..2 {
                                let blk = by * 2 + bx;
                                let (u_ac, v_ac) = if cbp_chroma == 2 {
                                    let ncu =
                                        cavlc_nc_chroma(pic, mb_addr, mb_x, mb_y, blk, &pic.nz_u);
                                    let ncv =
                                        cavlc_nc_chroma(pic, mb_addr, mb_x, mb_y, blk, &pic.nz_v);
                                    (
                                        residual_block_cavlc(&mut br, ncu, 15).map_err(map_bs)?,
                                        residual_block_cavlc(&mut br, ncv, 15).map_err(map_bs)?,
                                    )
                                } else {
                                    (
                                        CoeffBlock {
                                            levels_scan: [0; 16],
                                            max_coeff: 15,
                                            total_coeff: 0,
                                        },
                                        CoeffBlock {
                                            levels_scan: [0; 16],
                                            max_coeff: 15,
                                            total_coeff: 0,
                                        },
                                    )
                                };

                                pic.nz_u[pic.idx_c4(mb_addr, blk)] = u_ac.total_coeff as u8;
                                pic.nz_v[pic.idx_c4(mb_addr, blk)] = v_ac.total_coeff as u8;

                                let mut scan_u = [0i32; 16];
                                let mut scan_v = [0i32; 16];
                                scan_u[0] = u_dc_in[blk];
                                scan_v[0] = v_dc_in[blk];
                                if cbp_chroma == 2 {
                                    for s in 0..15 {
                                        scan_u[s + 1] = u_ac.levels_scan[s];
                                        scan_v[s + 1] = v_ac.levels_scan[s];
                                    }
                                }

                                let res_u = inv_residual_from_coeff_scan(scan_u, qp_c);
                                let res_v = inv_residual_from_coeff_scan(scan_v, qp_c);

                                let pred4u = pred4_from_pred8(&pred_u8x8, bx, by);
                                let pred4v = pred4_from_pred8(&pred_v8x8, bx, by);

                                let x0 = mb_x * 8 + bx * 4;
                                let y0 = mb_y * 8 + by * 4;

                                write_4x4_pred_plus_res(
                                    u,
                                    pic.uv_stride,
                                    pic.chroma_w,
                                    pic.chroma_h,
                                    x0,
                                    y0,
                                    pred4u,
                                    res_u,
                                );
                                write_4x4_pred_plus_res(
                                    v,
                                    pic.uv_stride,
                                    pic.chroma_w,
                                    pic.chroma_h,
                                    x0,
                                    y0,
                                    pred4v,
                                    res_v,
                                );
                            }
                        }
                    }

                    let _ = qp_c;
                }

                pic.mark_mb_decoded(mb_addr, mb_type as u8);
                mb_addr += 1;
                continue;
            }

            return Err(VideosonError::Unsupported(
                "CAVLC: unsupported mb_type in I-slice",
            ));
        }

        Ok(())
    } else {
        // ==========================
        // CABAC path (Part 4a): I_PCM + Intra16x16 only
        // ==========================
        if !br.is_byte_aligned() {
            while !br.is_byte_aligned() {
                let b = br.read_bit().map_err(map_bs)?;
                if !b {
                    return Err(VideosonError::InvalidData(
                        "CABAC alignment: expected cabac_alignment_one_bit == 1",
                    ));
                }
            }
        }

        let slice_bytes = br.remaining_bytes().map_err(map_bs)?;
        let mut cabac = CabacDecoder::new(slice_bytes)?;
        let mut ctx0_10 = init_ctx_i_slice_0_10(slice_qpy);
        let mut models = CabacModels::new_i_slice(slice_qpy);

        let mut mb_addr = sh.first_mb_in_slice as usize;
        let mut qp_y = slice_qpy;

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
            pic.mark_mb_decoded(mb_addr, mb_type);

            if mb_type == 25 {
                // I_PCM in CABAC
                let mut br_pcm = BitReader::new(slice_bytes);
                br_pcm.set_bit_pos(cabac.bit_pos()).map_err(map_bs)?;
                br_pcm.byte_align_zero().map_err(map_bs)?;

                match &mut pic.planes {
                    PendingPlanes::Mono8 { y } => {
                        for r in 0..16 {
                            for c in 0..16 {
                                let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 16 + c;
                                let yy = mb_y * 16 + r;
                                if x < pic.width as usize && yy < pic.height as usize {
                                    y[yy * pic.y_stride + x] = b;
                                }
                            }
                        }
                    }
                    PendingPlanes::Yuv4208 { y, u, v } => {
                        for r in 0..16 {
                            for c in 0..16 {
                                let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 16 + c;
                                let yy = mb_y * 16 + r;
                                if x < pic.width as usize && yy < pic.height as usize {
                                    y[yy * pic.y_stride + x] = b;
                                }
                            }
                        }
                        for r in 0..8 {
                            for c in 0..8 {
                                let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 8 + c;
                                let yy = mb_y * 8 + r;
                                if x < pic.chroma_w && yy < pic.chroma_h {
                                    u[yy * pic.uv_stride + x] = b;
                                }
                            }
                        }
                        for r in 0..8 {
                            for c in 0..8 {
                                let b = br_pcm.read_bits_u32(8).map_err(map_bs)? as u8;
                                let x = mb_x * 8 + c;
                                let yy = mb_y * 8 + r;
                                if x < pic.chroma_w && yy < pic.chroma_h {
                                    v[yy * pic.uv_stride + x] = b;
                                }
                            }
                        }
                    }
                    _ => return Err(VideosonError::InvalidData("CABAC: plane mismatch")),
                }

                // contexts reset after PCM
                cabac.set_bit_pos(br_pcm.bit_pos())?;
                cabac.reinit_engine()?;

                // clear nz contexts for safety
                pic.nz_y_dc[mb_addr] = 0;
                pic.nz_u_dc[mb_addr] = 0;
                pic.nz_v_dc[mb_addr] = 0;
                for i in 0..16 {
                    pic.nz_y[pic.idx_y4(mb_addr, i)] = 0;
                }
                for i in 0..4 {
                    pic.nz_u[pic.idx_c4(mb_addr, i)] = 0;
                    pic.nz_v[pic.idx_c4(mb_addr, i)] = 0;
                }
                pic.intra_chroma_pred_mode_mb[mb_addr] = 0;
                pic.qp_delta_nonzero_mb[mb_addr] = false;

                let end = cabac.decode_end_of_slice_flag();
                if end {
                    break;
                }
                mb_addr += 1;
                continue;
            }

            if mb_type == 0 {
                return Err(VideosonError::Unsupported(
                    "CABAC: Intra4x4 not yet supported (Part 4b)",
                ));
            }
            if !(1..=24).contains(&mb_type) {
                return Err(VideosonError::Unsupported("CABAC: unsupported mb_type"));
            }

            // Intra16x16 derived from mb_type
            let mbt = (mb_type as u32) - 1;
            let intra16_mode = (mbt % 4) as u8;
            let cbp_chroma = ((mbt / 4) % 3) as u32;
            let cbp_luma = if mb_type <= 12 { 0u32 } else { 15u32 };

            // intra_chroma_pred_mode (CABAC-coded)
            let intra_chroma_pred_mode = if pic.chroma_format_idc == 1 {
                let a = if mb_x > 0 && pic.filled[mb_addr - 1] {
                    (pic.intra_chroma_pred_mode_mb[mb_addr - 1] != 0) as usize
                } else {
                    0
                };
                let b = if mb_y > 0 && pic.filled[mb_addr - pic.mbs_w] {
                    (pic.intra_chroma_pred_mode_mb[mb_addr - pic.mbs_w] != 0) as usize
                } else {
                    0
                };
                decode_intra_chroma_pred_mode_cabac(&mut cabac, &mut models, a + b)
            } else {
                0
            };
            pic.intra_chroma_pred_mode_mb[mb_addr] = intra_chroma_pred_mode;

            // mb_qp_delta (CABAC-coded) only if there are residuals
            let delta_qp = if cbp_luma != 0 || cbp_chroma != 0 {
                let a = if mb_x > 0 && pic.filled[mb_addr - 1] {
                    pic.qp_delta_nonzero_mb[mb_addr - 1] as usize
                } else {
                    0
                };
                let b = if mb_y > 0 && pic.filled[mb_addr - pic.mbs_w] {
                    pic.qp_delta_nonzero_mb[mb_addr - pic.mbs_w] as usize
                } else {
                    0
                };
                decode_mb_qp_delta_cabac(&mut cabac, &mut models, a + b)
            } else {
                0
            };
            pic.qp_delta_nonzero_mb[mb_addr] = delta_qp != 0;
            qp_y = clip_qp(qp_y + delta_qp);

            // prediction
            let pred16 = {
                let (top, left, tl) = refs_16x16_luma(pic, mb_x, mb_y);
                pred_luma_16x16(intra16_mode, top, left, tl)
            };

            // Build MB output into temporaries to avoid borrow conflicts
            let mut luma_mb = pred16;
            let mut u_mb = [128u8; 64];
            let mut v_mb = [128u8; 64];

            // Luma residual
            if cbp_luma == 0 {
                pic.nz_y_dc[mb_addr] = 0;
                for blk in 0..16 {
                    pic.nz_y[pic.idx_y4(mb_addr, blk)] = 0;
                }
            } else {
                // DC
                let dc = residual_4x4block_cabac(
                    &mut cabac,
                    &mut models,
                    CtxBlockCat::Intra16x16DC,
                    16,
                    coded_left_luma_dc(pic, mb_addr, mb_x),
                    coded_top_luma_dc(pic, mb_addr, mb_y),
                )?;
                pic.nz_y_dc[mb_addr] = dc.total_coeff as u8;

                let dc_inv = {
                    let mut dc_raster = [0i32; 16];
                    for s in 0..16 {
                        dc_raster[ZIGZAG_4X4[s]] = dc.levels_scan16[s];
                    }
                    inv_hadamard_4x4(dc_raster)
                };

                for by in 0..4 {
                    for bx in 0..4 {
                        let blk = by * 4 + bx;

                        let ac = residual_4x4block_cabac(
                            &mut cabac,
                            &mut models,
                            CtxBlockCat::Intra16x16AC,
                            15,
                            coded_left_luma_ac(pic, mb_addr, mb_x, blk),
                            coded_top_luma_ac(pic, mb_addr, mb_y, blk),
                        )?;
                        pic.nz_y[pic.idx_y4(mb_addr, blk)] = ac.total_coeff as u8;

                        let mut scan_full = ac.levels_scan16; // already AC placed at scan 1..15
                        scan_full[0] = dc_inv[blk];

                        let res = inv_residual_from_coeff_scan(scan_full, qp_y);

                        // add residual into luma_mb
                        for y in 0..4 {
                            for x in 0..4 {
                                let idx = (by * 4 + y) * 16 + (bx * 4 + x);
                                let p = luma_mb[idx] as i32;
                                let r = res[y * 4 + x];
                                luma_mb[idx] = clip_u8(p + r);
                            }
                        }
                    }
                }
            }

            // Chroma residual
            if pic.chroma_format_idc == 1 {
                let qp_c = qp_c_from_qp_y(qp_y, pps.chroma_qp_index_offset);

                // prediction 8x8 from already-decoded neighbors
                let (pred_u8x8, pred_v8x8) = match &pic.planes {
                    PendingPlanes::Yuv4208 { u, v, .. } => {
                        let (topu, leftu, tlu) = refs_8x8_chroma(pic, mb_x, mb_y, u, pic.uv_stride);
                        let (topv, leftv, tlv) = refs_8x8_chroma(pic, mb_x, mb_y, v, pic.uv_stride);
                        (
                            pred_chroma_8x8(intra_chroma_pred_mode, topu, leftu, tlu),
                            pred_chroma_8x8(intra_chroma_pred_mode, topv, leftv, tlv),
                        )
                    }
                    _ => ([128u8; 64], [128u8; 64]),
                };

                u_mb = pred_u8x8;
                v_mb = pred_v8x8;

                if cbp_chroma == 0 {
                    pic.nz_u_dc[mb_addr] = 0;
                    pic.nz_v_dc[mb_addr] = 0;
                    for blk in 0..4 {
                        pic.nz_u[pic.idx_c4(mb_addr, blk)] = 0;
                        pic.nz_v[pic.idx_c4(mb_addr, blk)] = 0;
                    }
                } else {
                    // Chroma DC 2x2
                    let u_dc = residual_4x4block_cabac(
                        &mut cabac,
                        &mut models,
                        CtxBlockCat::ChromaDC,
                        4,
                        coded_left_chroma_dc(pic, mb_addr, mb_x, true),
                        coded_top_chroma_dc(pic, mb_addr, mb_y, true),
                    )?;
                    let v_dc = residual_4x4block_cabac(
                        &mut cabac,
                        &mut models,
                        CtxBlockCat::ChromaDC,
                        4,
                        coded_left_chroma_dc(pic, mb_addr, mb_x, false),
                        coded_top_chroma_dc(pic, mb_addr, mb_y, false),
                    )?;
                    pic.nz_u_dc[mb_addr] = u_dc.total_coeff as u8;
                    pic.nz_v_dc[mb_addr] = v_dc.total_coeff as u8;

                    let u_dc_in = inv_hadamard_2x2([
                        u_dc.levels_scan16[0],
                        u_dc.levels_scan16[1],
                        u_dc.levels_scan16[2],
                        u_dc.levels_scan16[3],
                    ]);
                    let v_dc_in = inv_hadamard_2x2([
                        v_dc.levels_scan16[0],
                        v_dc.levels_scan16[1],
                        v_dc.levels_scan16[2],
                        v_dc.levels_scan16[3],
                    ]);

                    for by in 0..2 {
                        for bx in 0..2 {
                            let blk = by * 2 + bx;

                            let (u_ac, v_ac) = if cbp_chroma == 2 {
                                (
                                    residual_4x4block_cabac(
                                        &mut cabac,
                                        &mut models,
                                        CtxBlockCat::ChromaAC,
                                        15,
                                        coded_left_chroma_ac(pic, mb_addr, mb_x, blk, &pic.nz_u),
                                        coded_top_chroma_ac(pic, mb_addr, mb_y, blk, &pic.nz_u),
                                    )?,
                                    residual_4x4block_cabac(
                                        &mut cabac,
                                        &mut models,
                                        CtxBlockCat::ChromaAC,
                                        15,
                                        coded_left_chroma_ac(pic, mb_addr, mb_x, blk, &pic.nz_v),
                                        coded_top_chroma_ac(pic, mb_addr, mb_y, blk, &pic.nz_v),
                                    )?,
                                )
                            } else {
                                (
                                    crate::cabac_residual::CabacCoeffBlock {
                                        levels_scan16: [0; 16],
                                        total_coeff: 0,
                                        coded_block_flag: false,
                                    },
                                    crate::cabac_residual::CabacCoeffBlock {
                                        levels_scan16: [0; 16],
                                        total_coeff: 0,
                                        coded_block_flag: false,
                                    },
                                )
                            };

                            pic.nz_u[pic.idx_c4(mb_addr, blk)] = u_ac.total_coeff as u8;
                            pic.nz_v[pic.idx_c4(mb_addr, blk)] = v_ac.total_coeff as u8;

                            let mut scan_u = u_ac.levels_scan16;
                            let mut scan_v = v_ac.levels_scan16;
                            scan_u[0] = u_dc_in[blk];
                            scan_v[0] = v_dc_in[blk];

                            let res_u = inv_residual_from_coeff_scan(scan_u, qp_c);
                            let res_v = inv_residual_from_coeff_scan(scan_v, qp_c);

                            // add into u_mb/v_mb
                            for y in 0..4 {
                                for x in 0..4 {
                                    let idx = (by * 4 + y) * 8 + (bx * 4 + x);
                                    u_mb[idx] = clip_u8(u_mb[idx] as i32 + res_u[y * 4 + x]);
                                    v_mb[idx] = clip_u8(v_mb[idx] as i32 + res_v[y * 4 + x]);
                                }
                            }
                        }
                    }
                }
            }

            // Commit MB samples to frame planes
            match &mut pic.planes {
                PendingPlanes::Mono8 { y } => {
                    // luma only
                    for r in 0..16 {
                        for c in 0..16 {
                            let x = mb_x * 16 + c;
                            let yy = mb_y * 16 + r;
                            if x < pic.width as usize && yy < pic.height as usize {
                                y[yy * pic.y_stride + x] = luma_mb[r * 16 + c];
                            }
                        }
                    }
                }
                PendingPlanes::Yuv4208 { y, u, v } => {
                    for r in 0..16 {
                        for c in 0..16 {
                            let x = mb_x * 16 + c;
                            let yy = mb_y * 16 + r;
                            if x < pic.width as usize && yy < pic.height as usize {
                                y[yy * pic.y_stride + x] = luma_mb[r * 16 + c];
                            }
                        }
                    }
                    for r in 0..8 {
                        for c in 0..8 {
                            let x = mb_x * 8 + c;
                            let yy = mb_y * 8 + r;
                            if x < pic.chroma_w && yy < pic.chroma_h {
                                u[yy * pic.uv_stride + x] = u_mb[r * 8 + c];
                                v[yy * pic.uv_stride + x] = v_mb[r * 8 + c];
                            }
                        }
                    }
                }
                _ => return Err(VideosonError::InvalidData("plane mismatch")),
            }

            // end_of_slice_flag
            let end = cabac.decode_end_of_slice_flag();
            if end {
                break;
            }
            mb_addr += 1;
        }

        Ok(())
    }
}
