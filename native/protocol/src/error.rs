//! Why a message could not be read or written.

use std::io;

use crate::MAX_PAYLOAD;

/// Why a message could not be read or written.
#[derive(Debug)]
pub enum ProtocolError {
    /// The pipe failed.
    Io(io::Error),
    /// A payload longer than [`MAX_PAYLOAD`].
    TooLong(usize),
    /// A kind this side does not accept.
    UnknownKind(u8),
    /// A payload of the wrong length for its kind.
    BadLength {
        /// The message kind.
        kind: u8,
        /// The payload's length.
        length: usize,
    },
    /// A field out of bounds or malformed, and which.
    BadValue(&'static str),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "pipe input or output failed: {error}"),
            Self::TooLong(length) => write!(f, "payload of {length} bytes exceeds {MAX_PAYLOAD}"),
            Self::UnknownKind(kind) => write!(f, "message kind {kind} is not accepted here"),
            Self::BadLength { kind, length } => write!(f, "message kind {kind} cannot be {length} bytes"),
            Self::BadValue(what) => write!(f, "invalid value: {what}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
