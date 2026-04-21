use crate::{BitReader, BitstreamError, BitstreamResult};

pub fn read_ue(br: &mut BitReader<'_>) -> BitstreamResult<u32> {
    let mut zeros = 0u32;
    while br.read_bit()? == false {
        zeros += 1;
        if zeros > 31 {
            return Err(BitstreamError::Invalid("ue(v): too many leading zeros"));
        }
    }

    if zeros == 0 {
        return Ok(0);
    }

    let suffix = br.read_bits_u32(zeros)?;
    let code_num = (1u32 << zeros) - 1 + suffix;
    Ok(code_num)
}

pub fn read_se(br: &mut BitReader<'_>) -> BitstreamResult<i32> {
    let code_num = read_ue(br)? as i32;
    let m = (code_num + 1) >> 1;
    if (code_num & 1) == 0 { Ok(-m) } else { Ok(m) }
}
