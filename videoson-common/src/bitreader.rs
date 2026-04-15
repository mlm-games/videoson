// videoson/videoson-common/src/bitreader.rs
use crate::{BitstreamError, BitstreamResult};

pub struct BitReader<'a> {
    buf: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, bit_pos: 0 }
    }

    #[inline]
    pub fn bits_remaining(&self) -> usize {
        self.buf
            .len()
            .saturating_mul(8)
            .saturating_sub(self.bit_pos)
    }

    #[inline]
    pub fn is_byte_aligned(&self) -> bool {
        (self.bit_pos & 7) == 0
    }

    #[inline]
    pub fn byte_pos(&self) -> usize {
        self.bit_pos >> 3
    }

    #[inline]
    pub fn read_bit(&mut self) -> BitstreamResult<bool> {
        if self.bit_pos >= self.buf.len() * 8 {
            return Err(BitstreamError::Eof);
        }
        let byte = self.buf[self.bit_pos >> 3];
        let shift = 7 - (self.bit_pos & 7);
        self.bit_pos += 1;
        Ok(((byte >> shift) & 1) != 0)
    }

    #[inline]
    pub fn read_bits_u32(&mut self, n: u32) -> BitstreamResult<u32> {
        if n > 32 {
            return Err(BitstreamError::Invalid("read_bits_u32: n > 32"));
        }
        let mut v: u32 = 0;
        for _ in 0..n {
            v = (v << 1) | (self.read_bit()? as u32);
        }
        Ok(v)
    }

    #[inline]
    pub fn read_bits_u16(&mut self, n: u32) -> BitstreamResult<u16> {
        if n > 16 {
            return Err(BitstreamError::Invalid("read_bits_u16: n > 16"));
        }
        Ok(self.read_bits_u32(n)? as u16)
    }

    pub fn byte_align(&mut self) -> BitstreamResult<()> {
        let rem = self.bit_pos & 7;
        if rem == 0 {
            return Ok(());
        }
        let to_skip = 8 - rem;
        for _ in 0..to_skip {
            let _ = self.read_bit()?;
        }
        Ok(())
    }

    pub fn byte_align_zero(&mut self) -> BitstreamResult<()> {
        let rem = self.bit_pos & 7;
        if rem == 0 {
            return Ok(());
        }
        let to_skip = 8 - rem;
        for _ in 0..to_skip {
            if self.read_bit()? {
                return Err(BitstreamError::Invalid(
                    "non-zero alignment bit where zero was required",
                ));
            }
        }
        Ok(())
    }

    pub fn read_bytes(&mut self, out: &mut [u8]) -> BitstreamResult<()> {
        if !self.is_byte_aligned() {
            return Err(BitstreamError::Invalid("read_bytes: not byte-aligned"));
        }
        let pos = self.byte_pos();
        let end = pos + out.len();
        if end > self.buf.len() {
            return Err(BitstreamError::Eof);
        }
        out.copy_from_slice(&self.buf[pos..end]);
        self.bit_pos += out.len() * 8;
        Ok(())
    }

    #[inline]
    pub fn bit_pos(&self) -> usize {
        self.bit_pos
    }

    pub fn set_bit_pos(&mut self, bit_pos: usize) -> BitstreamResult<()> {
        if bit_pos > self.buf.len() * 8 {
            return Err(BitstreamError::Eof);
        }
        self.bit_pos = bit_pos;
        Ok(())
    }

    pub fn remaining_bytes(&self) -> BitstreamResult<&'a [u8]> {
        if !self.is_byte_aligned() {
            return Err(BitstreamError::Invalid("remaining_bytes: not byte-aligned"));
        }
        let pos = self.byte_pos();
        Ok(&self.buf[pos..])
    }
}
