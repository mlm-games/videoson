#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod backend;

pub use backend::Av1Decoder;
