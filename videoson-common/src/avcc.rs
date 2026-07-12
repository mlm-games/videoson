use crate::{BitstreamError, BitstreamResult, NalHeader, NalUnitRef};

#[derive(Debug, Clone, Copy)]
pub struct AvccConfig {
    pub nal_len_size: u8,
}

pub fn parse_avcc_extradata(extradata: &[u8]) -> BitstreamResult<AvccConfig> {
    if extradata.len() < 7 {
        return Err(BitstreamError::Invalid("avcC too short"));
    }
    let length_size_minus_one = extradata[4] & 0b11;
    let nal_len_size = length_size_minus_one + 1;
    if !(1..=4).contains(&nal_len_size) {
        return Err(BitstreamError::Invalid("avcC invalid nal_len_size"));
    }
    Ok(AvccConfig { nal_len_size })
}

pub fn avcc_nals<'a>(
    data: &'a [u8],
    nal_len_size: u8,
) -> impl Iterator<Item = BitstreamResult<NalUnitRef<'a>>> + 'a {
    AvccIter {
        data,
        i: 0,
        nal_len_size,
    }
}

struct AvccIter<'a> {
    data: &'a [u8],
    i: usize,
    nal_len_size: u8,
}

impl<'a> Iterator for AvccIter<'a> {
    type Item = BitstreamResult<NalUnitRef<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.i >= self.data.len() {
            return None;
        }

        let n = self.nal_len_size as usize;
        let end = self.i.checked_add(n)?;
        if end > self.data.len() {
            return Some(Err(BitstreamError::Eof));
        }

        let mut len: usize = 0;
        for b in &self.data[self.i..end] {
            len = (len << 8) | (*b as usize);
        }
        self.i = end;

        let nal_end = self.i.checked_add(len)?;
        if nal_end > self.data.len() {
            return Some(Err(BitstreamError::Eof));
        }

        let nal = &self.data[self.i..nal_end];
        self.i = nal_end;

        if nal.is_empty() {
            return Some(Err(BitstreamError::Invalid("empty NAL")));
        }

        let header = match NalHeader::parse(nal[0]) {
            Ok(h) => h,
            Err(e) => return Some(Err(e)),
        };

        Some(Ok(NalUnitRef {
            header,
            payload_ebsp: &nal[1..],
        }))
    }
}
