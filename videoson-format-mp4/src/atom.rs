extern crate alloc;

use videoson_core::VideosonError;

#[derive(Debug, Clone, Copy)]
pub struct AtomHeader {
    pub size: u64,
    pub typ: [u8; 4],
    pub header_len: usize,
}

#[inline]
fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

#[inline]
fn be_u64(b: &[u8]) -> u64 {
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

pub fn read_atom_header(data: &[u8], pos: usize) -> Result<AtomHeader, VideosonError> {
    if pos + 8 > data.len() {
        return Err(VideosonError::NeedMoreData);
    }
    let size32 = be_u32(&data[pos..pos + 4]) as u64;
    let typ: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();

    if size32 == 0 {
        return Ok(AtomHeader {
            size: (data.len() - pos) as u64,
            typ,
            header_len: 8,
        });
    }

    if size32 == 1 {
        if pos + 16 > data.len() {
            return Err(VideosonError::NeedMoreData);
        }
        let size64 = be_u64(&data[pos + 8..pos + 16]);
        if size64 < 16 {
            return Err(VideosonError::InvalidData("mp4: invalid extended size"));
        }
        return Ok(AtomHeader {
            size: size64,
            typ,
            header_len: 16,
        });
    }

    if size32 < 8 {
        return Err(VideosonError::InvalidData("mp4: invalid atom size"));
    }

    Ok(AtomHeader {
        size: size32,
        typ,
        header_len: 8,
    })
}

pub fn iter_children<'a>(
    data: &'a [u8],
    start: usize,
    end: usize,
) -> impl Iterator<Item = Result<(AtomHeader, usize, usize), VideosonError>> + 'a {
    let mut pos = start;
    core::iter::from_fn(move || {
        if pos >= end {
            return None;
        }
        match read_atom_header(data, pos) {
            Ok(h) => {
                let atom_end = pos + (h.size as usize);
                if atom_end > end {
                    return Some(Err(VideosonError::InvalidData(
                        "mp4: child atom overruns parent",
                    )));
                }
                let payload_start = pos + h.header_len;
                let payload_end = atom_end;
                let out = Ok((h, payload_start, payload_end));
                pos = atom_end;
                Some(out)
            }
            Err(e) => Some(Err(e)),
        }
    })
}

pub fn find_child(
    data: &[u8],
    start: usize,
    end: usize,
    typ: &[u8; 4],
) -> Result<Option<(usize, usize)>, VideosonError> {
    for child in iter_children(data, start, end) {
        let (h, ps, pe) = child?;
        if &h.typ == typ {
            return Ok(Some((ps, pe)));
        }
    }
    Ok(None)
}
