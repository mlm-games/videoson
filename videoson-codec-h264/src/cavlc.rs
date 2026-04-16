// videoson-codec-h264/src/cavlc.rs
//
// Minimal CAVLC residual decoding for 4x4 / 2x2 blocks.
// No external deps. Enough to decode baseline-style CAVLC Intra slices.
//
// This implements residual_block_cavlc() for maxNumCoeff in {16,15,4}.
// Note: Some implementations reuse the same total_zeros VLC tables for maxNumCoeff=15 and 16.
// We do the same, but we validate the decoded total_zeros does not exceed (maxNumCoeff - TotalCoeff).

use videoson_common::{BitReader, BitstreamError, BitstreamResult};

#[inline]
fn show_bits(br: &mut BitReader<'_>, n: u32) -> BitstreamResult<u32> {
    let pos = br.bit_pos();
    let v = br.read_bits_u32(n)?;
    br.set_bit_pos(pos)?;
    Ok(v)
}

#[inline]
fn get_bits(br: &mut BitReader<'_>, n: u32) -> BitstreamResult<u32> {
    br.read_bits_u32(n)
}

fn read_level_prefix(br: &mut BitReader<'_>) -> BitstreamResult<u32> {
    // level_prefix is unary: leading zeros then a 1
    let mut zeros = 0u32;
    while !br.read_bit()? {
        zeros += 1;
        if zeros > 31 {
            return Err(BitstreamError::Invalid("level_prefix too long"));
        }
    }
    Ok(zeros)
}

// -------- coeff_token tables (compact) --------
// Index 0 => 0<=nC<2, 1 => 2<=nC<4, 2 => 4<=nC<8
const COEFF_TOKEN_CODE: [[[u8; 16]; 4]; 3] = [
    [
        [5, 7, 7, 7, 7, 15, 11, 8, 15, 11, 15, 11, 15, 11, 7, 4],
        [1, 4, 6, 6, 6, 6, 14, 10, 14, 10, 14, 10, 1, 14, 10, 6],
        [0, 1, 5, 5, 5, 5, 5, 13, 9, 13, 9, 13, 9, 13, 9, 5],
        [0, 0, 3, 3, 4, 4, 4, 4, 4, 12, 12, 8, 12, 8, 12, 8],
    ],
    [
        [11, 7, 7, 7, 4, 7, 15, 11, 15, 11, 8, 15, 11, 7, 9, 7],
        [2, 7, 10, 6, 6, 6, 6, 14, 10, 14, 10, 14, 10, 11, 8, 6],
        [0, 3, 9, 5, 5, 5, 5, 13, 9, 13, 9, 13, 9, 6, 10, 5],
        [0, 0, 5, 4, 6, 8, 4, 4, 4, 12, 8, 12, 12, 8, 1, 4],
    ],
    [
        [15, 11, 8, 15, 11, 9, 8, 15, 11, 15, 11, 8, 13, 9, 5, 1],
        [14, 15, 12, 10, 8, 14, 10, 14, 14, 10, 14, 10, 7, 12, 8, 4],
        [0, 13, 14, 11, 9, 13, 9, 13, 10, 13, 9, 13, 9, 11, 7, 3],
        [0, 0, 12, 11, 10, 9, 8, 13, 12, 12, 12, 8, 12, 10, 6, 2],
    ],
];

const COEFF_TOKEN_SIZE: [[[u8; 16]; 4]; 3] = [
    [
        [6, 8, 9, 10, 11, 13, 13, 13, 14, 14, 15, 15, 16, 16, 16, 16],
        [2, 6, 8, 9, 10, 11, 13, 13, 14, 14, 15, 15, 15, 16, 16, 16],
        [0, 3, 7, 8, 9, 10, 11, 13, 13, 14, 14, 15, 15, 16, 16, 16],
        [0, 0, 5, 6, 7, 8, 9, 10, 11, 13, 14, 14, 15, 15, 16, 16],
    ],
    [
        [6, 6, 7, 8, 8, 9, 11, 11, 12, 12, 12, 13, 13, 13, 14, 14],
        [2, 5, 6, 6, 7, 8, 9, 11, 11, 12, 12, 13, 13, 14, 14, 14],
        [0, 3, 6, 6, 7, 8, 9, 11, 11, 12, 12, 13, 13, 13, 14, 14],
        [0, 0, 4, 4, 5, 6, 6, 7, 9, 11, 11, 12, 13, 13, 13, 14],
    ],
    [
        [6, 6, 6, 7, 7, 7, 7, 8, 8, 9, 9, 9, 10, 10, 10, 10],
        [4, 5, 5, 5, 5, 6, 6, 7, 8, 8, 9, 9, 9, 10, 10, 10],
        [0, 4, 5, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 10],
        [0, 0, 4, 4, 4, 4, 4, 5, 6, 7, 8, 8, 9, 10, 10, 10],
    ],
];

// chroma DC coeff_token (nC == -1, TotalCoeff 0..3)
const COEFF_TOKEN_CODE_CHROMA: [[u8; 4]; 4] =
    [[7, 4, 3, 2], [1, 6, 3, 3], [0, 1, 2, 2], [0, 0, 5, 0]];
const COEFF_TOKEN_SIZE_CHROMA: [[u8; 4]; 4] =
    [[6, 6, 6, 6], [1, 6, 7, 8], [0, 3, 7, 8], [0, 0, 6, 7]];

// nC >= 8 fixed-length 6-bit mapping (index = next 6 bits)
// entry = (total_coeff<<2) | trailing_ones, 0xFF invalid
const COEFF_TOKEN_FLC_NC8: [u8; 64] = [
    4, 5, 0xFF, 0, 8, 9, 10, 0xFF, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51,
    52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67,
];

fn decode_coeff_token(
    br: &mut BitReader<'_>,
    n_c: i32,
    max_total_coeff: u32,
) -> BitstreamResult<(u32, u32)> {
    if n_c == -1 {
        // chroma DC
        let prefix = show_bits(br, 16)?;
        for total in 0..=3 {
            for t1 in 0..=3 {
                if t1 > total {
                    continue;
                }
                let sz = COEFF_TOKEN_SIZE_CHROMA[t1 as usize][total as usize];
                if sz == 0 {
                    continue;
                }
                let got = (prefix >> (16 - sz as u32)) as u8;
                let code = COEFF_TOKEN_CODE_CHROMA[t1 as usize][total as usize];
                if got == code {
                    let _ = get_bits(br, sz as u32)?;
                    return Ok((total as u32, t1 as u32));
                }
            }
        }
        return Err(BitstreamError::Invalid("coeff_token chromaDC: no match"));
    }

    if n_c >= 8 {
        let idx = show_bits(br, 6)? as usize;
        let v = COEFF_TOKEN_FLC_NC8[idx];
        if v == 0xFF {
            return Err(BitstreamError::Invalid("coeff_token FLC: invalid"));
        }
        let _ = get_bits(br, 6)?;
        let total = (v as u32) >> 2;
        let t1 = (v as u32) & 3;
        if total > max_total_coeff {
            return Err(BitstreamError::Invalid(
                "coeff_token FLC: total_coeff > max",
            ));
        }
        return Ok((total, t1));
    }

    let table = if n_c < 2 {
        0
    } else if n_c < 4 {
        1
    } else {
        2
    };

    let prefix = show_bits(br, 16)?;
    for total in 0..=16u32 {
        for t1 in 0..=3u32 {
            if t1 > total {
                continue;
            }
            if total > max_total_coeff {
                continue;
            }
            let sz = COEFF_TOKEN_SIZE[table][t1 as usize][total as usize];
            if sz == 0 {
                continue;
            }
            let got = (prefix >> (16 - sz as u32)) as u8;
            let code = COEFF_TOKEN_CODE[table][t1 as usize][total as usize];
            if got == code {
                let _ = get_bits(br, sz as u32)?;
                return Ok((total, t1));
            }
        }
    }

    Err(BitstreamError::Invalid("coeff_token: no match"))
}

// total_zeros (4x4) compact tables
const ZERO_SIZE: [u8; 135] = [
    1, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 9, 3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 6, 6, 6, 6, 4,
    3, 3, 3, 4, 4, 3, 3, 4, 5, 5, 6, 5, 6, 5, 3, 4, 4, 3, 3, 3, 4, 3, 4, 5, 5, 5, 4, 4, 4, 3, 3, 3,
    3, 3, 4, 5, 4, 5, 6, 5, 3, 3, 3, 3, 3, 3, 4, 3, 6, 6, 5, 3, 3, 3, 2, 3, 4, 3, 6, 6, 4, 5, 3, 2,
    2, 3, 3, 6, 6, 6, 4, 2, 2, 3, 2, 5, 5, 5, 3, 2, 2, 2, 4, 4, 4, 3, 3, 1, 3, 4, 4, 2, 1, 3, 3, 3,
    1, 2, 2, 2, 1, 1, 1,
];
const ZERO_CODE: [u8; 135] = [
    1, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 3, 2, 1, 7, 6, 5, 4, 3, 5, 4, 3, 2, 3, 2, 3, 2, 1, 0, 5,
    7, 6, 5, 4, 3, 4, 3, 2, 3, 2, 1, 1, 0, 3, 7, 5, 4, 6, 5, 4, 3, 3, 2, 2, 1, 0, 5, 4, 3, 7, 6, 5,
    4, 3, 2, 1, 1, 0, 1, 1, 7, 6, 5, 4, 3, 2, 1, 1, 0, 1, 1, 5, 4, 3, 3, 2, 1, 1, 0, 1, 1, 1, 3, 3,
    2, 2, 1, 0, 1, 0, 1, 3, 2, 1, 1, 1, 1, 0, 1, 1, 2, 1, 3, 0, 1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 0,
    1, 1, 0, 1, 0, 1, 0,
];
const ZERO_INDEX: [u8; 15] = [
    0, 16, 31, 45, 58, 70, 81, 91, 100, 108, 115, 121, 126, 130, 133,
];

// total_zeros chroma DC (2x2): flattened for totalCoeff=1..3
const ZERO_SIZE_CHROMA: [u8; 9] = [1, 2, 3, 3, 1, 2, 2, 1, 1];
const ZERO_CODE_CHROMA: [u8; 9] = [1, 1, 1, 0, 1, 1, 0, 1, 0];

fn decode_total_zeros(
    br: &mut BitReader<'_>,
    total_coeff: u32,
    max_coeff: u32,
) -> BitstreamResult<u32> {
    if total_coeff == max_coeff {
        return Ok(0);
    }

    if max_coeff == 4 {
        // chroma DC 2x2
        if !(1..=3).contains(&total_coeff) {
            return Err(BitstreamError::Invalid("chromaDC total_coeff out of range"));
        }
        let start = match total_coeff {
            1 => 0,
            2 => 4,
            3 => 7,
            _ => 0,
        };
        let end = match total_coeff {
            1 => 4,
            2 => 7,
            3 => 9,
            _ => 0,
        };
        let prefix = show_bits(br, 8)?;
        for (zeros, idx) in (start..end).enumerate() {
            let sz = ZERO_SIZE_CHROMA[idx] as u32;
            let code = ZERO_CODE_CHROMA[idx] as u32;
            let got = prefix >> (8 - sz);
            if got == code {
                let _ = get_bits(br, sz)?;
                return Ok(zeros as u32);
            }
        }
        return Err(BitstreamError::Invalid("chromaDC total_zeros: no match"));
    }

    // For max_coeff 15 and 16 we reuse the same table, but validate result <= max_coeff - total_coeff.
    if total_coeff == 0 || total_coeff > 15 {
        return Err(BitstreamError::Invalid(
            "total_coeff out of range for total_zeros",
        ));
    }

    let tc = total_coeff as usize;
    let start = ZERO_INDEX[tc - 1] as usize;
    let end = if tc == 15 {
        ZERO_CODE.len()
    } else {
        ZERO_INDEX[tc] as usize
    };

    let prefix = show_bits(br, 9)?;
    for zeros in 0..(end - start) {
        let sz = ZERO_SIZE[start + zeros] as u32;
        let code = ZERO_CODE[start + zeros] as u32;
        let got = prefix >> (9 - sz);
        if got == code {
            let _ = get_bits(br, sz)?;
            let z = zeros as u32;
            if z > (max_coeff - total_coeff) {
                return Err(BitstreamError::Invalid(
                    "total_zeros > (max_coeff - total_coeff)",
                ));
            }
            return Ok(z);
        }
    }

    Err(BitstreamError::Invalid("total_zeros: no match"))
}

// run_before (zerosLeft-dependent)
const RUN_SIZE: [u8; 42] = [
    1, 1, //
    1, 2, 2, //
    2, 2, 2, 2, //
    2, 2, 2, 3, 3, //
    2, 2, 3, 3, 3, 3, //
    2, 3, 3, 3, 3, 3, 3, //
    3, 3, 3, 3, 3, 3, 3, 4, 5, 6, 7, 8, 9, 10, 11,
];
const RUN_CODE: [u8; 42] = [
    1, 0, //
    1, 1, 0, //
    3, 2, 1, 0, //
    3, 2, 1, 1, 0, //
    3, 2, 3, 2, 1, 0, //
    3, 0, 1, 3, 2, 5, 4, //
    7, 6, 5, 4, 3, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];
const RUN_INDEX: [u8; 7] = [0, 2, 5, 9, 14, 20, 27];

fn decode_run_before(br: &mut BitReader<'_>, zeros_left: u32) -> BitstreamResult<u32> {
    if zeros_left == 0 {
        return Ok(0);
    }
    let zl = zeros_left.min(7) as usize;
    let start = RUN_INDEX[zl - 1] as usize;
    let end = if zl == 7 {
        RUN_CODE.len()
    } else {
        RUN_INDEX[zl] as usize
    };

    let prefix = show_bits(br, 11)?;
    for run in 0..(end - start) {
        let sz = RUN_SIZE[start + run] as u32;
        let code = RUN_CODE[start + run] as u32;
        let got = prefix >> (11 - sz);
        if got == code {
            let _ = get_bits(br, sz)?;
            return Ok(run as u32);
        }
    }
    Err(BitstreamError::Invalid("run_before: no match"))
}

pub struct CoeffBlock {
    pub levels_scan: [i32; 16], // scan positions [0..max_coeff)
    pub max_coeff: u32,
    pub total_coeff: u32,
}

pub fn residual_block_cavlc(
    br: &mut BitReader<'_>,
    n_c: i32,
    max_coeff: u32,
) -> BitstreamResult<CoeffBlock> {
    if !(max_coeff == 4 || max_coeff == 15 || max_coeff == 16) {
        return Err(BitstreamError::Invalid("unsupported max_coeff"));
    }

    let (total_coeff, trailing_ones) = decode_coeff_token(br, n_c, max_coeff)?;

    let mut levels_tmp = [0i32; 16];
    let mut runs = [0u32; 16];

    if total_coeff == 0 {
        return Ok(CoeffBlock {
            levels_scan: [0i32; 16],
            max_coeff,
            total_coeff: 0,
        });
    }

    // 1) trailing ones signs
    for i in 0..trailing_ones {
        let sign = br.read_bit()? as i32;
        levels_tmp[i as usize] = if sign != 0 { -1 } else { 1 };
    }

    // 2) remaining levels
    let mut suffix_len: u32 = if total_coeff > 10 && trailing_ones < 3 {
        1
    } else {
        0
    };

    for i in trailing_ones..total_coeff {
        let level_prefix = read_level_prefix(br)?;

        let mut level_code: i32;
        let suffix_size: u32;

        if level_prefix < 14 {
            suffix_size = suffix_len;
            level_code = (level_prefix as i32) << suffix_len;
        } else if level_prefix == 14 {
            suffix_size = if suffix_len == 0 { 4 } else { suffix_len };
            level_code = (level_prefix as i32) << suffix_len;
        } else {
            // >= 15
            suffix_size = 12;
            if suffix_len == 0 {
                suffix_len = 1; // matches common reference behavior
            }
            level_code = (level_prefix as i32) << suffix_len;
        }

        if suffix_size != 0 {
            let level_suffix = get_bits(br, suffix_size)? as i32;
            level_code += level_suffix;
        }

        if level_prefix >= 15 && suffix_len == 0 {
            level_code += 15;
        }
        if level_prefix >= 16 {
            level_code += ((1i32 << (level_prefix as i32 - 3)) - 4096);
        }

        if i == trailing_ones && trailing_ones < 3 {
            level_code += 2;
        }

        let mut level_val = if (level_code & 1) == 0 {
            (level_code + 2) >> 1
        } else {
            (-level_code - 1) >> 1
        };

        if suffix_len == 0 {
            suffix_len = 1;
        }
        if level_val.abs() > (3 << (suffix_len - 1)) && suffix_len < 6 {
            suffix_len += 1;
        }

        levels_tmp[i as usize] = level_val;
    }

    // 3) total_zeros
    let zeros_left = if total_coeff == max_coeff {
        0
    } else {
        decode_total_zeros(br, total_coeff, max_coeff)?
    };

    // 4) run_before
    let mut z = zeros_left;
    for i in 0..(total_coeff - 1) {
        let run = if z > 0 { decode_run_before(br, z)? } else { 0 };
        runs[i as usize] = run;
        z = z.saturating_sub(run);
    }
    runs[(total_coeff - 1) as usize] = z;

    // 5) combine into scan positions
    let mut out_scan = [0i32; 16];
    let mut coeff_num: i32 = -1;
    for i in (0..total_coeff).rev() {
        coeff_num += runs[i as usize] as i32 + 1;
        let idx = coeff_num as usize;
        if idx >= max_coeff as usize {
            return Err(BitstreamError::Invalid("coeff_num out of range"));
        }
        out_scan[idx] = levels_tmp[i as usize];
    }

    Ok(CoeffBlock {
        levels_scan: out_scan,
        max_coeff,
        total_coeff,
    })
}
