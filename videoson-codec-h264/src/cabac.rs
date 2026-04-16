// videoson-codec-h264/src/cabac.rs
#![allow(clippy::identity_op)]

use videoson_core::{Result, VideosonError};

#[derive(Debug, Clone, Copy)]
pub struct CtxState {
    pub p_state_idx: u8,
    pub val_mps: u8,
}

const RANGE_TAB_LPS: [[u16; 4]; 64] = [
    [128, 176, 208, 240],
    [128, 167, 197, 227],
    [128, 158, 187, 216],
    [123, 150, 178, 205],
    [116, 142, 169, 195],
    [111, 135, 160, 185],
    [105, 128, 152, 175],
    [100, 122, 144, 166],
    [95, 116, 137, 158],
    [90, 110, 130, 150],
    [85, 104, 123, 142],
    [81, 99, 117, 135],
    [77, 94, 111, 128],
    [73, 89, 105, 122],
    [69, 85, 100, 116],
    [66, 80, 95, 110],
    [62, 76, 90, 104],
    [59, 72, 86, 99],
    [56, 69, 81, 94],
    [53, 65, 77, 89],
    [51, 62, 73, 85],
    [48, 59, 69, 80],
    [46, 56, 66, 76],
    [43, 53, 63, 72],
    [41, 50, 59, 69],
    [39, 48, 56, 65],
    [37, 45, 54, 62],
    [35, 43, 51, 59],
    [33, 41, 48, 56],
    [32, 39, 46, 53],
    [30, 37, 43, 50],
    [29, 35, 41, 48],
    [27, 33, 39, 45],
    [26, 31, 37, 43],
    [24, 30, 35, 41],
    [23, 28, 33, 39],
    [22, 27, 32, 37],
    [21, 26, 30, 35],
    [20, 24, 29, 33],
    [19, 23, 27, 31],
    [18, 22, 26, 30],
    [17, 21, 25, 28],
    [16, 20, 23, 27],
    [15, 19, 22, 25],
    [14, 18, 21, 24],
    [14, 17, 20, 23],
    [13, 16, 19, 22],
    [12, 15, 18, 21],
    [12, 14, 17, 20],
    [11, 14, 16, 19],
    [11, 13, 15, 18],
    [10, 12, 15, 17],
    [10, 12, 14, 16],
    [9, 11, 13, 15],
    [9, 11, 12, 14],
    [8, 10, 12, 14],
    [8, 9, 11, 13],
    [7, 9, 11, 12],
    [7, 9, 10, 12],
    [7, 8, 10, 11],
    [6, 8, 9, 11],
    [6, 7, 9, 10],
    [6, 7, 8, 9],
    [2, 2, 2, 2],
];

const TRANS_IDX_LPS: [u8; 64] = [
    0, 0, 1, 2, 2, 4, 4, 5, 6, 7, 8, 9, 9, 11, 11, 12, 13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21,
    21, 22, 22, 23, 24, 24, 25, 26, 26, 27, 27, 28, 29, 29, 30, 30, 30, 31, 32, 32, 33, 33, 33, 34,
    34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 38, 38, 63,
];

const TRANS_IDX_MPS: [u8; 64] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50,
    51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 62, 63,
];

fn clip3(minv: i32, maxv: i32, v: i32) -> i32 {
    v.clamp(minv, maxv)
}

const INIT_I_0_10: [(i8, i8); 11] = [
    (20, -15),
    (2, 54),
    (3, 74),
    (20, -15),
    (2, 54),
    (3, 74),
    (-28, 127),
    (-23, 104),
    (-6, 53),
    (-1, 54),
    (7, 51),
];

pub fn init_ctx_i_slice_0_10(slice_qpy: i32) -> [CtxState; 11] {
    let qp = clip3(0, 51, slice_qpy);
    core::array::from_fn(|i| {
        let (m, n) = INIT_I_0_10[i];
        let pre = clip3(1, 126, (((m as i32) * qp) >> 4) + (n as i32));
        if pre <= 63 {
            CtxState {
                p_state_idx: (63 - pre) as u8,
                val_mps: 0,
            }
        } else {
            CtxState {
                p_state_idx: (pre - 64) as u8,
                val_mps: 1,
            }
        }
    })
}

pub struct CabacDecoder<'a> {
    cod_i_range: u16,
    cod_i_offset: u16,
    data: &'a [u8],
    byte: usize,
    bit: u8,
}

impl<'a> CabacDecoder<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self> {
        if data.len() < 2 {
            return Err(VideosonError::NeedMoreData);
        }
        let mut d = Self {
            cod_i_range: 510,
            cod_i_offset: 0,
            data,
            byte: 0,
            bit: 0,
        };
        d.reinit_engine()?;
        Ok(d)
    }

    #[inline]
    fn read_bit(&mut self) -> u16 {
        if self.byte >= self.data.len() {
            return 0;
        }
        let b = (self.data[self.byte] >> (7 - self.bit)) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.byte += 1;
        }
        b as u16
    }

    #[inline]
    pub fn bits_read(&self) -> usize {
        self.byte * 8 + (self.bit as usize)
    }

    #[inline]
    pub fn bit_pos(&self) -> usize {
        self.bits_read()
    }

    pub fn set_bit_pos(&mut self, bit_pos: usize) -> Result<()> {
        if bit_pos > self.data.len() * 8 {
            return Err(VideosonError::NeedMoreData);
        }
        self.byte = bit_pos >> 3;
        self.bit = (bit_pos & 7) as u8;
        Ok(())
    }

    pub fn reinit_engine(&mut self) -> Result<()> {
        self.cod_i_range = 510;
        self.cod_i_offset = 0;
        let mut off: u16 = 0;
        for _ in 0..9 {
            off = (off << 1) | self.read_bit();
        }
        self.cod_i_offset = off;
        Ok(())
    }

    fn renormalize(&mut self) {
        while self.cod_i_range < 256 {
            self.cod_i_range <<= 1;
            self.cod_i_offset <<= 1;
            self.cod_i_offset |= self.read_bit();
        }
    }

    pub fn decode_decision(&mut self, ctx: &mut CtxState) -> u8 {
        let q = (self.cod_i_range >> 6) & 3;
        let cod_i_range_lps = RANGE_TAB_LPS[ctx.p_state_idx as usize][q as usize];
        self.cod_i_range -= cod_i_range_lps;

        let bin_val: u8;
        if self.cod_i_offset >= self.cod_i_range {
            bin_val = 1 - ctx.val_mps;
            self.cod_i_offset -= self.cod_i_range;
            self.cod_i_range = cod_i_range_lps;
            if ctx.p_state_idx == 0 {
                ctx.val_mps = 1 - ctx.val_mps;
            }
            ctx.p_state_idx = TRANS_IDX_LPS[ctx.p_state_idx as usize];
        } else {
            bin_val = ctx.val_mps;
            ctx.p_state_idx = TRANS_IDX_MPS[ctx.p_state_idx as usize];
        }
        self.renormalize();
        bin_val
    }

    pub fn decode_terminate(&mut self) -> u8 {
        self.cod_i_range -= 2;
        if self.cod_i_offset >= self.cod_i_range {
            return 1;
        }
        self.renormalize();
        0
    }

    #[inline]
    pub fn decode_end_of_slice_flag(&mut self) -> bool {
        self.decode_terminate() != 0
    }
}

pub fn decode_mb_type_intra(
    cabac: &mut CabacDecoder<'_>,
    ctx0_10: &mut [CtxState; 11],
    left_mb_type: Option<u8>,
    top_mb_type: Option<u8>,
) -> u8 {
    const MB_TYPE_I_NXN: u8 = 0;
    const MB_TYPE_I_PCM: u8 = 25;

    let mut ctx_idx_inc = 0;
    if let Some(t) = left_mb_type {
        if t != MB_TYPE_I_NXN {
            ctx_idx_inc += 1;
        }
    }
    if let Some(t) = top_mb_type {
        if t != MB_TYPE_I_NXN {
            ctx_idx_inc += 1;
        }
    }

    let bin0 = cabac.decode_decision(&mut ctx0_10[3 + ctx_idx_inc]);
    if bin0 == 0 {
        return MB_TYPE_I_NXN;
    }

    let bin_t = cabac.decode_terminate();
    if bin_t == 1 {
        return MB_TYPE_I_PCM;
    }

    let bin1 = cabac.decode_decision(&mut ctx0_10[6]);
    let bin2 = cabac.decode_decision(&mut ctx0_10[7]);
    let cbp_chroma = if bin2 == 1 {
        let bin3 = cabac.decode_decision(&mut ctx0_10[8]);
        if bin3 == 1 {
            2
        } else {
            1
        }
    } else {
        0
    };

    let bin4 = cabac.decode_decision(&mut ctx0_10[9]);
    let bin5 = cabac.decode_decision(&mut ctx0_10[10]);
    let pred_mode = (bin4 as u8) * 2 + (bin5 as u8);

    let mut mb_type = 1 + pred_mode + 4 * (cbp_chroma as u8);
    if bin1 == 1 {
        mb_type += 12;
    }
    mb_type
}
