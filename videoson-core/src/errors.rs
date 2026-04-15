// videoson/videoson-core/src/errors.rs
extern crate alloc;

use alloc::string::String;
use core::fmt;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum VideosonError {
    Unsupported(&'static str),
    InvalidData(&'static str),
    NeedMoreData,
    Eof,
    Message(String),
}

impl fmt::Display for VideosonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VideosonError::Unsupported(s) => write!(f, "unsupported: {s}"),
            VideosonError::InvalidData(s) => write!(f, "invalid data: {s}"),
            VideosonError::NeedMoreData => write!(f, "need more data"),
            VideosonError::Eof => write!(f, "end of stream"),
            VideosonError::Message(s) => write!(f, "{s}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for VideosonError {}
