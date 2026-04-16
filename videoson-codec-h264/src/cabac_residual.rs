// videoson-codec-h264/src/cabac_residual.rs
//
// CABAC residual decoding for H.264 4x4 / AC-only / chroma DC 2x2 blocks.
// Implements the residual_4x4block_cabac syntax at a practical level.
//
// This module depends on:
// - CabacDecoder decision + bypass decoding
// - CABAC context models initialized for I-slices

use crate::cabac::{CabacDecoder, CtxState};
use crate::cabac_models::CabacModels;
use videoson_core::{Result, VideosonError};

#[derive(Debug, Clone, Copy)]
pub enum CtxBlockCat {
    Intra16x16DC = 0,
    Intra16x16AC = 1,
    Luma4x4 = 2,
    ChromaDC = 3,
    ChromaAC = 4,
}

const SIG_OFF: [usize; 5] = [0, 15, 29, 44, 47];
const LAST_OFF: [usize; 5] = [0, 15, 29, 44, 47];

#[inline]
fn sig_ctx_label(cat: CtxBlockCat, scanning_pos: usize) -> usize {
    95 + SIG_OFF[cat as usize] + scanning_pos
}

#[inline]
fn last_ctx_label(cat: CtxBlockCat, scanning_pos: usize) -> usize {
    156 + LAST_OFF[cat as usize] + scanning_pos
}

#[inline]
fn cbf_ctx_label(cat: CtxBlockCat, ctx_var: usize) -> usize {
    75 + 4 * (cat as usize) + ctx_var
}

#[inline]
fn abs_base(cat: CtxBlockCat) -> usize {
    217 + 10 * (cat as usize)
}

fn decode_eg0_bypass(c: &mut CabacDecoder<'_>) -> u32 {
    let mut k: u32 = 0;
    let mut s: u32 = 0;
    loop {
        let b = c.decode_bypass() as u32;
        if b == 0 {
            break;
        }
        s += 1u32 << k;
        k += 1;
    }
    if k > 0 {
        let suffix = c.decode_bypass_bits(k) as u32;
        s += suffix;
    }
    s
}

fn decode_coeff_abs_level_minus1(
    c: &mut CabacDecoder<'_>,
    m: &mut CabacModels,
    cat: CtxBlockCat,
    num_eq1: &mut u32,
    num_gt1: &mut u32,
) -> Result<u32> {
    let ctx_abs_lev1 = if *num_gt1 != 0 { 4 } else { (*num_eq1).min(3) } as usize;

    let ctx_abs_lev2 = (*num_gt1).min(4) as usize;

    let base = abs_base(cat);

    let mut prefix: u32 = 0;
    for bin_idx in 0..14 {
        let label = if bin_idx == 0 {
            base + ctx_abs_lev1
        } else {
            base + 5 + ctx_abs_lev2
        };

        let b = c.decode_decision(m.ctx_mut(label)) as u32;
        if b == 0 {
            return Ok(prefix);
        }
        prefix += 1;
    }

    let suffix = decode_eg0_bypass(c);
    Ok(14 + suffix)
}

pub struct CabacCoeffBlock {
    pub levels_scan16: [i32; 16],
    pub total_coeff: u32,
    pub coded_block_flag: bool,
}

pub fn residual_4x4block_cabac(
    c: &mut CabacDecoder<'_>,
    m: &mut CabacModels,
    cat: CtxBlockCat,
    max_num_coeff: usize,
    coded_left: bool,
    coded_top: bool,
) -> Result<CabacCoeffBlock> {
    if !(max_num_coeff == 16 || max_num_coeff == 15 || max_num_coeff == 4) {
        return Err(VideosonError::Unsupported(
            "CABAC residual: unsupported MaxNumCoeff",
        ));
    }

    let ctx_var = (coded_left as usize) + 2 * (coded_top as usize);
    let cbf = c.decode_decision(m.ctx_mut(cbf_ctx_label(cat, ctx_var))) != 0;
    if !cbf {
        return Ok(CabacCoeffBlock {
            levels_scan16: [0; 16],
            total_coeff: 0,
            coded_block_flag: false,
        });
    }

    let mut sig_pos: [usize; 16] = [0; 16];
    let mut sig_count: usize = 0;

    let mut last_reached = false;
    for i in 0..(max_num_coeff - 1) {
        let sig = c.decode_decision(m.ctx_mut(sig_ctx_label(cat, i))) != 0;
        if sig {
            sig_pos[sig_count] = i;
            sig_count += 1;

            let last = c.decode_decision(m.ctx_mut(last_ctx_label(cat, i))) != 0;
            if last {
                last_reached = true;
                break;
            }
        }
    }

    if !last_reached {
        sig_pos[sig_count] = max_num_coeff - 1;
        sig_count += 1;
    }

    let mut num_eq1: u32 = 0;
    let mut num_gt1: u32 = 0;

    let mut out = [0i32; 16];

    for j in (0..sig_count).rev() {
        let p = sig_pos[j];

        let abs_m1 = decode_coeff_abs_level_minus1(c, m, cat, &mut num_eq1, &mut num_gt1)?;
        let abs = (abs_m1 + 1) as i32;

        let sign = c.decode_bypass() as i32;
        let level = if sign != 0 { -abs } else { abs };

        if abs == 1 {
            num_eq1 += 1;
        } else {
            num_gt1 += 1;
        }

        match max_num_coeff {
            16 => {
                out[p] = level;
            }
            15 => {
                out[p + 1] = level;
            }
            4 => {
                out[p] = level;
            }
            _ => {}
        }
    }

    Ok(CabacCoeffBlock {
        levels_scan16: out,
        total_coeff: sig_count as u32,
        coded_block_flag: true,
    })
}
