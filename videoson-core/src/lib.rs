// videoson/videoson-core/src/lib.rs
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod codec;
mod errors;
mod packet;
mod video;

pub use codec::*;
pub use errors::*;
pub use packet::*;
pub use video::*;

pub type Result<T> = core::result::Result<T, VideosonError>;