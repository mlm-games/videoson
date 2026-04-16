#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod codec;
mod errors;
mod format;
mod packet;
mod units;
mod video;

pub use codec::*;
pub use errors::*;
pub use format::*;
pub use packet::*;
pub use units::*;
pub use video::*;

pub type Result<T> = core::result::Result<T, VideosonError>;
