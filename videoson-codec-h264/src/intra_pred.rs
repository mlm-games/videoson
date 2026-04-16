// videoson-codec-h264/src/intra_pred.rs
//
// Part 2: intra prediction primitives (8-bit).
// In Part 3, after we decode residual coefficients, we'll do:
// recon = pred + residual, clipped to [0,255].

use crate::transform::clip_u8;

#[inline]
fn avg2(a: u8, b: u8) -> u8 {
    ((a as u16 + b as u16 + 1) >> 1) as u8
}

#[inline]
fn avg3(a: u8, b: u8, c: u8) -> u8 {
    ((a as u16 + 2 * (b as u16) + c as u16 + 2) >> 2) as u8
}

/// Reference samples for luma Intra4x4 prediction.
///
/// Layout commonly described in the spec:
///   top:    A B C D E F G H
///   top-left: X
///   left:   I J K L
///
/// We store:
/// - tl = X
/// - t[0..8) = A..H
/// - l[0..4) = I..L
#[derive(Debug, Clone, Copy)]
pub struct Ref4x4 {
    pub tl: u8,
    pub t: [u8; 8],
    pub l: [u8; 4],
}

pub fn pred_luma_4x4(mode: u8, r: Ref4x4) -> [u8; 16] {
    let mut out = [0u8; 16];

    let a = r.t[0];
    let b = r.t[1];
    let c = r.t[2];
    let d = r.t[3];
    let e = r.t[4];
    let f = r.t[5];
    let g = r.t[6];
    let h = r.t[7];
    let i = r.l[0];
    let j = r.l[1];
    let k = r.l[2];
    let l = r.l[3];
    let x = r.tl;

    match mode {
        0 => {
            // Vertical
            for y in 0..4 {
                for xx in 0..4 {
                    out[y * 4 + xx] = r.t[xx];
                }
            }
        }
        1 => {
            // Horizontal
            for y in 0..4 {
                for xx in 0..4 {
                    out[y * 4 + xx] = r.l[y];
                }
            }
        }
        2 => {
            // DC
            let sum = (a as u16 + b as u16 + c as u16 + d as u16)
                + (i as u16 + j as u16 + k as u16 + l as u16);
            let dc = ((sum + 4) >> 3) as u8;
            out.fill(dc);
        }
        3 => {
            // Diagonal down-left
            let p0 = avg3(a, b, c);
            let p1 = avg3(b, c, d);
            let p2 = avg3(c, d, e);
            let p3 = avg3(d, e, f);
            let p4 = avg3(e, f, g);
            let p5 = avg3(f, g, h);
            let p6 = avg3(g, h, h);

            out[0] = p0;
            out[1] = p1;
            out[2] = p2;
            out[3] = p3;
            out[4] = p1;
            out[5] = p2;
            out[6] = p3;
            out[7] = p4;
            out[8] = p2;
            out[9] = p3;
            out[10] = p4;
            out[11] = p5;
            out[12] = p3;
            out[13] = p4;
            out[14] = p5;
            out[15] = p6;
        }
        4 => {
            // Diagonal down-right
            out[0] = avg3(j, i, x);
            out[1] = avg3(x, a, b);
            out[2] = avg3(a, b, c);
            out[3] = avg3(b, c, d);

            out[4] = avg3(k, j, i);
            out[5] = avg3(j, i, x);
            out[6] = avg3(x, a, b);
            out[7] = avg3(a, b, c);

            out[8] = avg3(l, k, j);
            out[9] = avg3(k, j, i);
            out[10] = avg3(j, i, x);
            out[11] = avg3(x, a, b);

            out[12] = avg3(l, l, k);
            out[13] = avg3(l, k, j);
            out[14] = avg3(k, j, i);
            out[15] = avg3(j, i, x);
        }
        5 => {
            // Vertical-right
            out[0] = avg2(x, a);
            out[1] = avg2(a, b);
            out[2] = avg2(b, c);
            out[3] = avg2(c, d);

            out[4] = avg3(i, x, a);
            out[5] = avg3(x, a, b);
            out[6] = avg3(a, b, c);
            out[7] = avg3(b, c, d);

            out[8] = avg3(j, i, x);
            out[9] = avg3(i, x, a);
            out[10] = avg3(x, a, b);
            out[11] = avg3(a, b, c);

            out[12] = avg3(k, j, i);
            out[13] = avg3(j, i, x);
            out[14] = avg3(i, x, a);
            out[15] = avg3(x, a, b);
        }
        6 => {
            // Horizontal-down
            out[0] = avg2(x, i);
            out[4] = avg2(i, j);
            out[8] = avg2(j, k);
            out[12] = avg2(k, l);

            out[1] = avg3(x, i, j);
            out[5] = avg3(i, j, k);
            out[9] = avg3(j, k, l);
            out[13] = avg3(k, l, l);

            out[2] = avg3(a, x, i);
            out[6] = avg3(x, i, j);
            out[10] = avg3(i, j, k);
            out[14] = avg3(j, k, l);

            out[3] = avg3(b, a, x);
            out[7] = avg3(a, x, i);
            out[11] = avg3(x, i, j);
            out[15] = avg3(i, j, k);
        }
        7 => {
            // Vertical-left
            let p0 = avg2(a, b);
            let p1 = avg2(b, c);
            let p2 = avg2(c, d);
            let p3 = avg2(d, e);
            let p4 = avg2(e, f);
            let p5 = avg2(f, g);
            let p6 = avg2(g, h);

            let q0 = avg3(a, b, c);
            let q1 = avg3(b, c, d);
            let q2 = avg3(c, d, e);
            let q3 = avg3(d, e, f);
            let q4 = avg3(e, f, g);
            let q5 = avg3(f, g, h);
            let q6 = avg3(g, h, h);

            out[0] = p0;
            out[1] = p1;
            out[2] = p2;
            out[3] = p3;
            out[4] = q0;
            out[5] = q1;
            out[6] = q2;
            out[7] = q3;
            out[8] = p1;
            out[9] = p2;
            out[10] = p3;
            out[11] = p4;
            out[12] = q1;
            out[13] = q2;
            out[14] = q3;
            out[15] = q4;

            let _ = (p5, p6, q5, q6);
        }
        8 => {
            // Horizontal-up
            let p0 = avg2(i, j);
            let p1 = avg2(j, k);
            let p2 = avg2(k, l);
            let p3 = avg2(l, l);

            let q0 = avg3(i, j, k);
            let q1 = avg3(j, k, l);
            let q2 = avg3(k, l, l);
            let q3 = avg3(l, l, l);

            out[0] = p0;
            out[1] = q0;
            out[2] = p1;
            out[3] = q1;
            out[4] = p1;
            out[5] = q1;
            out[6] = p2;
            out[7] = q2;
            out[8] = p2;
            out[9] = q2;
            out[10] = p3;
            out[11] = q3;
            out[12] = p3;
            out[13] = q3;
            out[14] = p3;
            out[15] = q3;
        }
        _ => {
            // fallback DC
            let sum = (a as u16 + b as u16 + c as u16 + d as u16)
                + (i as u16 + j as u16 + k as u16 + l as u16);
            let dc = ((sum + 4) >> 3) as u8;
            out.fill(dc);
        }
    }

    out
}

/// Luma Intra16x16 prediction.
///
/// mode:
/// 0 = Vertical
/// 1 = Horizontal
/// 2 = DC
/// 3 = Plane
pub fn pred_luma_16x16(mode: u8, top: [u8; 16], left: [u8; 16], tl: u8) -> [u8; 256] {
    let mut out = [0u8; 256];

    match mode {
        0 => {
            // Vertical
            for y in 0..16 {
                for x in 0..16 {
                    out[y * 16 + x] = top[x];
                }
            }
        }
        1 => {
            // Horizontal
            for y in 0..16 {
                for x in 0..16 {
                    out[y * 16 + x] = left[y];
                }
            }
        }
        2 => {
            // DC
            let mut sum: u32 = 0;
            for v in top {
                sum += v as u32;
            }
            for v in left {
                sum += v as u32;
            }
            let dc = ((sum + 16) >> 5) as u8;
            out.fill(dc);
        }
        3 => {
            // Plane (simplified but standard integer form)
            // Uses top[8..15], left[8..15], and top-left.
            let mut h: i32 = 0;
            let mut v: i32 = 0;
            for i in 1..=8 {
                h += i as i32 * (top[7 + i] as i32 - top[7 - i] as i32);
                v += i as i32 * (left[7 + i] as i32 - left[7 - i] as i32);
            }
            let a = 16 * (top[15] as i32 + left[15] as i32);
            let b = (5 * h + 32) >> 6;
            let c = (5 * v + 32) >> 6;

            for y in 0..16 {
                for x in 0..16 {
                    let p = (a + b * (x as i32 - 7) + c * (y as i32 - 7) + 16) >> 5;
                    out[y * 16 + x] = clip_u8(p);
                }
            }

            let _ = tl;
        }
        _ => {
            // fallback to DC
            let mut sum: u32 = 0;
            for v in top {
                sum += v as u32;
            }
            for v in left {
                sum += v as u32;
            }
            let dc = ((sum + 16) >> 5) as u8;
            out.fill(dc);
        }
    }

    out
}

/// Chroma prediction (8x8) for 4:2:0.
/// mode:
/// 0 = DC
/// 1 = Horizontal
/// 2 = Vertical
/// 3 = Plane
pub fn pred_chroma_8x8(mode: u8, top: [u8; 8], left: [u8; 8], tl: u8) -> [u8; 64] {
    let mut out = [0u8; 64];

    match mode {
        0 => {
            // DC
            let mut sum: u32 = 0;
            for v in top {
                sum += v as u32;
            }
            for v in left {
                sum += v as u32;
            }
            let dc = ((sum + 8) >> 4) as u8;
            out.fill(dc);
        }
        1 => {
            // Horizontal
            for y in 0..8 {
                for x in 0..8 {
                    out[y * 8 + x] = left[y];
                }
            }
        }
        2 => {
            // Vertical
            for y in 0..8 {
                for x in 0..8 {
                    out[y * 8 + x] = top[x];
                }
            }
        }
        3 => {
            // Plane (similar style)
            let mut h: i32 = 0;
            let mut v: i32 = 0;
            for i in 1..=4 {
                h += i as i32 * (top[3 + i] as i32 - top[3 - i] as i32);
                v += i as i32 * (left[3 + i] as i32 - left[3 - i] as i32);
            }
            let a = 16 * (top[7] as i32 + left[7] as i32);
            let b = (17 * h + 16) >> 5;
            let c = (17 * v + 16) >> 5;

            for y in 0..8 {
                for x in 0..8 {
                    let p = (a + b * (x as i32 - 3) + c * (y as i32 - 3) + 16) >> 5;
                    out[y * 8 + x] = clip_u8(p);
                }
            }

            let _ = tl;
        }
        _ => {
            // fallback DC
            let mut sum: u32 = 0;
            for v in top {
                sum += v as u32;
            }
            for v in left {
                sum += v as u32;
            }
            let dc = ((sum + 8) >> 4) as u8;
            out.fill(dc);
        }
    }

    out
}
