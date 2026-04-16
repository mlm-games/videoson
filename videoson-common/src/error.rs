extern crate alloc;

use alloc::string::String;
use core::fmt;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BitstreamError {
    Eof,
    Invalid(&'static str),
    Message(String),
}

impl fmt::Display for BitstreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BitstreamError::Eof => write!(f, "end of bitstream"),
            BitstreamError::Invalid(s) => write!(f, "invalid bitstream: {s}"),
            BitstreamError::Message(s) => write!(f, "{s}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for BitstreamError {}

pub type BitstreamResult<T> = core::result::Result<T, BitstreamError>;
