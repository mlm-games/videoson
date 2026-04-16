// videoson-codec-h264/src/lib.rs
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod cabac;
mod cabac_models;
mod cabac_residual;
mod cavlc;
mod decoder;
mod intra_pred;
mod pps;
mod slice;
mod sps;
mod transform;

pub use decoder::H264Decoder;