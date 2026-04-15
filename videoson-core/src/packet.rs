// videoson/videoson-core/src/packet.rs
extern crate alloc;

use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub struct Packet {
    pub track_id: u32,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub duration: Option<i64>,
    pub data: Vec<u8>,
}

impl Packet {
    pub fn new(track_id: u32, data: Vec<u8>) -> Self {
        Self {
            track_id,
            pts: None,
            dts: None,
            duration: None,
            data,
        }
    }
}
