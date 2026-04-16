// videoson-codec-h264/src/transform.rs
//
// Part 2: math building blocks for full intra decoding.
// In Part 3 we'll feed CAVLC residual coefficients into these.
// This module is intentionally "bitstream-free".

#[inline]
fn clip_i32(v: i32, lo: i32, hi: i32) -> i32 {
    v.clamp(lo, hi)
}

#[inline]
pub fn clip_u8(v: i32) -> u8 {
    clip_i32(v, 0, 255) as u8
}

// Spec dequant coeff table for 4x4 (no scaling matrices; weightScale assumed default).
// This is the commonly used 6x16 table.
pub const DEQUANT_COEF_4X4: [[i32; 16]; 6] = [
    [
        10, 13, 10, 13, //
        13, 16, 13, 16, //
        10, 13, 10, 13, //
        13, 16, 13, 16,
    ],
    [
        11, 14, 11, 14, //
        14, 18, 14, 18, //
        11, 14, 11, 14, //
        14, 18, 14, 18,
    ],
    [
        13, 16, 13, 16, //
        16, 20, 16, 20, //
        13, 16, 13, 16, //
        16, 20, 16, 20,
    ],
    [
        14, 18, 14, 18, //
        18, 23, 18, 23, //
        14, 18, 14, 18, //
        18, 23, 18, 23,
    ],
    [
        16, 20, 16, 20, //
        20, 25, 20, 25, //
        16, 20, 16, 20, //
        20, 25, 20, 25,
    ],
    [
        18, 23, 18, 23, //
        23, 29, 23, 29, //
        18, 23, 18, 23, //
        23, 29, 23, 29,
    ],
];

#[inline]
pub fn dequant_4x4(qp: i32, coeff: i32, i: usize) -> i32 {
    // qp in 0..51
    let qp = qp.clamp(0, 51);
    let q = (qp % 6) as usize;
    let shift = (qp / 6) as u32;
    coeff * (DEQUANT_COEF_4X4[q][i]) * (1 << shift)
}

/// Inverse integer transform for one 4x4 block.
/// Input: dequantized coefficients in raster order (x + 4*y).
/// Output: residual samples in raster order.
pub fn inv_transform_4x4(mut c: [i32; 16]) -> [i32; 16] {
    // Vertical
    for x in 0..4 {
        let c0 = c[0 * 4 + x];
        let c1 = c[1 * 4 + x];
        let c2 = c[2 * 4 + x];
        let c3 = c[3 * 4 + x];

        let a0 = c0 + c2;
        let a1 = c0 - c2;
        let a2 = (c1 >> 1) - c3;
        let a3 = c1 + (c3 >> 1);

        c[0 * 4 + x] = a0 + a3;
        c[1 * 4 + x] = a1 + a2;
        c[2 * 4 + x] = a1 - a2;
        c[3 * 4 + x] = a0 - a3;
    }

    // Horizontal + final scaling
    let mut r = [0i32; 16];
    for y in 0..4 {
        let c0 = c[y * 4 + 0];
        let c1 = c[y * 4 + 1];
        let c2 = c[y * 4 + 2];
        let c3 = c[y * 4 + 3];

        let a0 = c0 + c2;
        let a1 = c0 - c2;
        let a2 = (c1 >> 1) - c3;
        let a3 = c1 + (c3 >> 1);

        // The spec's inverse transform includes a /64 scaling with rounding.
        r[y * 4 + 0] = (a0 + a3 + 32) >> 6;
        r[y * 4 + 1] = (a1 + a2 + 32) >> 6;
        r[y * 4 + 2] = (a1 - a2 + 32) >> 6;
        r[y * 4 + 3] = (a0 - a3 + 32) >> 6;
    }

    r
}

/// 4x4 Hadamard inverse (used for DC blocks like Intra16x16DC).
/// Input/Output in raster order.
pub fn inv_hadamard_4x4(mut d: [i32; 16]) -> [i32; 16] {
    // Horizontal
    for y in 0..4 {
        let a0 = d[y * 4 + 0] + d[y * 4 + 2];
        let a1 = d[y * 4 + 0] - d[y * 4 + 2];
        let a2 = d[y * 4 + 1] - d[y * 4 + 3];
        let a3 = d[y * 4 + 1] + d[y * 4 + 3];

        d[y * 4 + 0] = a0 + a3;
        d[y * 4 + 1] = a1 + a2;
        d[y * 4 + 2] = a1 - a2;
        d[y * 4 + 3] = a0 - a3;
    }

    // Vertical
    let mut out = [0i32; 16];
    for x in 0..4 {
        let a0 = d[0 * 4 + x] + d[2 * 4 + x];
        let a1 = d[0 * 4 + x] - d[2 * 4 + x];
        let a2 = d[1 * 4 + x] - d[3 * 4 + x];
        let a3 = d[1 * 4 + x] + d[3 * 4 + x];

        out[0 * 4 + x] = (a0 + a3) >> 1;
        out[1 * 4 + x] = (a1 + a2) >> 1;
        out[2 * 4 + x] = (a1 - a2) >> 1;
        out[3 * 4 + x] = (a0 - a3) >> 1;
    }

    out
}

/// 2x2 Hadamard inverse (used for chroma DC in 4:2:0).
pub fn inv_hadamard_2x2(d: [i32; 4]) -> [i32; 4] {
    // [a b; c d]
    let a = d[0];
    let b = d[1];
    let c = d[2];
    let dd = d[3];

    let t0 = a + dd;
    let t1 = a - dd;
    let t2 = b + c;
    let t3 = b - c;

    // output scaled (common integer form)
    [
        (t0 + t2) >> 1,
        (t1 + t3) >> 1,
        (t1 - t3) >> 1,
        (t0 - t2) >> 1,
    ]
}
