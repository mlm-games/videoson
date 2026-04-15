#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod atom;
mod demuxer;

pub use demuxer::{Mp4Demuxer, Mp4Track};
