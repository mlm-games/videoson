#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "std")]
mod demuxer;

#[cfg(feature = "std")]
pub use demuxer::MkvDemuxer;

#[cfg(not(feature = "std"))]
compile_error!("videoson-format-mkv currently requires the `std` feature.");
