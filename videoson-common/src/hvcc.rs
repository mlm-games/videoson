use crate::{BitstreamError, BitstreamResult};

#[derive(Debug, Clone, Copy)]
pub struct HvccConfig {
    pub nal_len_size: u8,
    pub num_nal_arrays: u8,
}

pub struct HvccNalIter<'a> {
    data: &'a [u8],
    pos: usize,
    array_idx: u8,
    num_arrays: u8,
    nals_remaining: u16,
    nal_unit_type: u8,
}

fn read_u16(data: &[u8], pos: usize) -> Option<u16> {
    if pos + 2 > data.len() {
        return None;
    }
    Some(((data[pos] as u16) << 8) | data[pos + 1] as u16)
}

pub fn parse_hvcc_extradata(extradata: &[u8]) -> BitstreamResult<HvccConfig> {
    if extradata.len() < 23 {
        return Err(BitstreamError::Invalid("hvcC too short"));
    }
    let length_size_minus_one = extradata[21] & 0b11;
    let nal_len_size = length_size_minus_one + 1;
    if !(1..=4).contains(&nal_len_size) {
        return Err(BitstreamError::Invalid("hvcC invalid nal_len_size"));
    }
    let num_nal_arrays = extradata[22];
    Ok(HvccConfig {
        nal_len_size,
        num_nal_arrays,
    })
}

pub fn hvcc_nal_bytes<'a>(data: &'a [u8]) -> impl Iterator<Item = BitstreamResult<&'a [u8]>> + 'a {
    HvccNalIter {
        data,
        pos: 23,
        array_idx: 0,
        num_arrays: if data.len() >= 23 { data[22] } else { 0 },
        nals_remaining: 0,
        nal_unit_type: 0,
    }
}

impl<'a> Iterator for HvccNalIter<'a> {
    type Item = BitstreamResult<&'a [u8]>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.nals_remaining > 0 {
                self.nals_remaining -= 1;
                let len = match read_u16(self.data, self.pos) {
                    Some(l) => l as usize,
                    None => return Some(Err(BitstreamError::Eof)),
                };
                self.pos += 2;
                if self.pos + len > self.data.len() {
                    return Some(Err(BitstreamError::Eof));
                }
                let nal_bytes = &self.data[self.pos..self.pos + len];
                self.pos += len;
                return Some(Ok(nal_bytes));
            }

            if self.array_idx >= self.num_arrays {
                return None;
            }

            if self.pos + 3 > self.data.len() {
                return Some(Err(BitstreamError::Eof));
            }
            let _array_completeness = (self.data[self.pos] >> 7) & 1;
            self.nal_unit_type = self.data[self.pos] & 0b111111;
            self.pos += 1;

            match read_u16(self.data, self.pos) {
                Some(n) => self.nals_remaining = n,
                None => return Some(Err(BitstreamError::Eof)),
            }
            self.pos += 2;
            self.array_idx += 1;
        }
    }
}
