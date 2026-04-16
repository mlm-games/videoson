extern crate alloc;

use alloc::vec::Vec;

fn has_emulation_prevention(ebsp: &[u8]) -> bool {
    if ebsp.len() < 3 {
        return false;
    }
    let mut i = 0usize;
    while i + 2 < ebsp.len() {
        if ebsp[i] == 0x00 && ebsp[i + 1] == 0x00 && ebsp[i + 2] == 0x03 {
            return true;
        }
        i += 1;
    }
    false
}

pub fn ebsp_to_rbsp<'a>(ebsp: &'a [u8], scratch: &'a mut Vec<u8>) -> &'a [u8] {
    if !has_emulation_prevention(ebsp) {
        return ebsp;
    }

    scratch.clear();
    scratch.reserve(ebsp.len());

    let mut zeros = 0u8;
    for &b in ebsp {
        if zeros >= 2 && b == 0x03 {
            zeros = 0;
            continue;
        }
        scratch.push(b);
        if b == 0x00 {
            zeros = zeros.saturating_add(1);
        } else {
            zeros = 0;
        }
    }

    scratch.as_slice()
}
