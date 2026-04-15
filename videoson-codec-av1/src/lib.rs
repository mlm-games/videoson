// videoson-codec-av1/src/lib.rs
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod decoder;

pub use decoder::Av1Decoder;
