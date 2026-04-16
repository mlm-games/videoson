use crate::{BitstreamError, BitstreamResult};

#[derive(Debug, Clone, Copy)]
pub struct NalHeader {
    pub nal_ref_idc: u8,
    pub nal_unit_type: u8,
}

impl NalHeader {
    #[inline]
    pub fn parse(b0: u8) -> BitstreamResult<Self> {
        let forbidden_zero = (b0 >> 7) & 1;
        if forbidden_zero != 0 {
            return Err(BitstreamError::Invalid("NAL header: forbidden_zero_bit=1"));
        }
        let nal_ref_idc = (b0 >> 5) & 0b11;
        let nal_unit_type = b0 & 0b1_1111;
        Ok(Self {
            nal_ref_idc,
            nal_unit_type,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NalUnitRef<'a> {
    pub header: NalHeader,
    pub payload_ebsp: &'a [u8],
}

pub fn annexb_nals(data: &[u8]) -> impl Iterator<Item = BitstreamResult<NalUnitRef<'_>>> + '_ {
    AnnexBIter { data, i: 0 }
}

struct AnnexBIter<'a> {
    data: &'a [u8],
    i: usize,
}

impl<'a> AnnexBIter<'a> {
    fn find_start_code(&self, from: usize) -> Option<(usize, usize)> {
        let d = self.data;
        let mut i = from;
        while i + 3 <= d.len() {
            if i + 4 <= d.len() && d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 0 && d[i + 3] == 1 {
                return Some((i, 4));
            }
            if d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1 {
                return Some((i, 3));
            }
            i += 1;
        }
        None
    }
}

impl<'a> Iterator for AnnexBIter<'a> {
    type Item = BitstreamResult<NalUnitRef<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let (sc0, sc0_len) = match self.find_start_code(self.i) {
            Some(v) => v,
            None => return None,
        };

        let nal_start = sc0 + sc0_len;
        let (sc1, _sc1_len) = match self.find_start_code(nal_start) {
            Some(v) => v,
            None => (self.data.len(), 0),
        };

        self.i = sc1;

        if nal_start >= sc1 {
            return Some(Err(BitstreamError::Invalid("empty NAL unit")));
        }

        let nal = &self.data[nal_start..sc1];
        if nal.is_empty() {
            return Some(Err(BitstreamError::Invalid("empty NAL unit")));
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
