#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod rust_h264_decoder;

pub use rust_h264_decoder::RustH264Decoder;

pub type H264Decoder = RustH264Decoder;
