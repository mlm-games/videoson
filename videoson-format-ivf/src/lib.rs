#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod demuxer;
mod header;

pub use demuxer::IvfDemuxer;
pub use header::{IvfCodec, IvfFileHeader, IvfFrameHeader};
