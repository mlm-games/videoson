// videoson-format-ivf/src/lib.rs
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod header;
mod demuxer;

pub use demuxer::IvfDemuxer;
pub use header::{IvfCodec, IvfFileHeader, IvfFrameHeader};