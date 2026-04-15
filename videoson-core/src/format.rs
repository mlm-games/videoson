// videoson-core/src/format.rs
extern crate alloc;

use alloc::vec::Vec;

use crate::{Packet, Result, TimeBase, VideoCodecParams};

#[derive(Debug, Clone)]
pub struct Track {
    pub id: u32,
    pub codec_params: VideoCodecParams,
    pub time_base: Option<TimeBase>,
}

pub trait Demuxer: Send {
    fn tracks(&self) -> &[Track];

    fn next_packet(&mut self) -> Result<Option<Packet>>;
}
