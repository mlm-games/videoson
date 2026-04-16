// videoson-codec-h264/src/lib.rs
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod cabac;
mod decoder;
mod pps;
mod slice;
mod sps;

pub use decoder::H264Decoder;

#[cfg(feature = "backend-rust-h264")]
mod rust_h264_decoder;
#[cfg(feature = "backend-rust-h264")]
pub use rust_h264_decoder::RustH264Decoder;