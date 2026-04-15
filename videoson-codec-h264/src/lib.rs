// videoson-codec-h264/src/lib.rs
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod decoder;
mod pps;
mod slice;
mod sps;

pub use decoder::H264Decoder;