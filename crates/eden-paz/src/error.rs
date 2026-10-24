//! Error types shared by the eden-rs core crates.

use std::fmt;
use std::io;

/// Result alias used throughout `eden-paz`.
pub type Result<T> = std::result::Result<T, PazError>;

/// Errors produced while opening or decoding a Musica archive.
#[derive(Debug)]
pub enum PazError {
    Io(io::Error),
    InvalidHeader(String),
    InvalidIndex(String),
    Crypto(String),
    Image(String),
    Ani(String),
    EntryNotFound(String),
    Exe(String),
    Cache(String),
}

impl fmt::Display for PazError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PazError::Io(e) => write!(f, "I/O error: {e}"),
            PazError::InvalidHeader(s) => write!(f, "invalid PAZ header: {s}"),
            PazError::InvalidIndex(s) => write!(f, "invalid PAZ index: {s}"),
            PazError::Crypto(s) => write!(f, "crypto error: {s}"),
            PazError::Image(s) => write!(f, "image decode error: {s}"),
            PazError::Ani(s) => write!(f, ".ani decode error: {s}"),
            PazError::EntryNotFound(s) => write!(f, "entry not found: {s}"),
            PazError::Exe(s) => write!(f, "executable parse error: {s}"),
            PazError::Cache(s) => write!(f, "cache error: {s}"),
        }
    }
}

impl std::error::Error for PazError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PazError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for PazError {
    fn from(e: io::Error) -> Self {
        PazError::Io(e)
    }
}
