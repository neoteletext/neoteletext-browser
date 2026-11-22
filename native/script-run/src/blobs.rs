//! Named files passed between the runner's two processes. The process
//! that reads and writes the disk is not confined and runs no script; the
//! worker that runs the script, the decoder's engine and the mixer is
//! confined as the script host is and touches no file (section 16: "It reads
//! untrusted scripts like the browser does and so confines itself the
//! same way"). Everything one gives the other is a list of named byte
//! strings in this one form:
//!
//! ```text
//! count: u32, then for each: name length: u8, name, length: u32, bytes
//! ```
//!
//! Numbers are big-endian. A name is 1 to [`MAX_BLOB_NAME_BYTES`] of
//! ASCII letters, digits, `.`, `-` and `_`, and never starts with a dot,
//! so that it is safe as a file name. What the worker writes is read by
//! the unconfined process as untrusted, within every bound here.

use std::io::{Read, Write};

use crate::limits::{MAX_BLOB_BYTES, MAX_BLOB_NAME_BYTES, MAX_BLOBS, MAX_BLOBS_BYTES};

/// One named file.
pub type Blob = (String, Vec<u8>);

/// Why a list of files was not read or written.
#[derive(Debug)]
pub enum BlobError {
    /// The pipe failed or ended early.
    Io(std::io::Error),
    /// A count, a length or a name out of its bounds; which.
    Bad(&'static str),
}

impl core::fmt::Display for BlobError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "the pipe between the runner's processes: {error}"),
            Self::Bad(what) => write!(formatter, "the runner's other process sent a bad {what}"),
        }
    }
}

impl core::error::Error for BlobError {}

/// Whether `name` is a name a file may be passed under.
#[must_use]
pub fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_BLOB_NAME_BYTES
        && !name.starts_with('.')
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// Writes `blobs`.
///
/// # Errors
/// When one is out of its bounds, or the pipe fails.
pub fn write(out: &mut dyn Write, blobs: &[Blob]) -> Result<(), BlobError> {
    let count = u32::try_from(blobs.len()).ok().filter(|_| blobs.len() <= MAX_BLOBS).ok_or(BlobError::Bad("count"))?;
    let total = blobs.iter().fold(0_usize, |sum, (_, bytes)| sum.saturating_add(bytes.len()));
    if total > MAX_BLOBS_BYTES {
        return Err(BlobError::Bad("total length"));
    }
    out.write_all(&count.to_be_bytes()).map_err(BlobError::Io)?;
    for (name, bytes) in blobs {
        let name_length = u8::try_from(name.len()).ok().filter(|_| is_name(name)).ok_or(BlobError::Bad("name"))?;
        let length = u32::try_from(bytes.len()).ok().filter(|_| bytes.len() <= MAX_BLOB_BYTES).ok_or(BlobError::Bad("length"))?;
        out.write_all(&[name_length]).map_err(BlobError::Io)?;
        out.write_all(name.as_bytes()).map_err(BlobError::Io)?;
        out.write_all(&length.to_be_bytes()).map_err(BlobError::Io)?;
        out.write_all(bytes).map_err(BlobError::Io)?;
    }
    out.flush().map_err(BlobError::Io)
}

fn number(input: &mut dyn Read) -> Result<usize, BlobError> {
    let mut bytes = [0_u8; 4];
    input.read_exact(&mut bytes).map_err(BlobError::Io)?;
    usize::try_from(u32::from_be_bytes(bytes)).map_err(|_| BlobError::Bad("length"))
}

/// Reads a list of files: every count and length is held to its bound
/// before anything is allocated for it, and no name is given twice.
///
/// # Errors
/// When the pipe ends early or anything is out of its bounds.
pub fn read(input: &mut dyn Read) -> Result<Vec<Blob>, BlobError> {
    let count = number(input)?;
    if count > MAX_BLOBS {
        return Err(BlobError::Bad("count"));
    }
    let mut blobs: Vec<Blob> = Vec::new();
    let mut total = 0_usize;
    for _ in 0..count {
        let mut name_length = [0_u8; 1];
        input.read_exact(&mut name_length).map_err(BlobError::Io)?;
        let mut name = vec![0_u8; usize::from(u8::from_be_bytes(name_length)).min(MAX_BLOB_NAME_BYTES)];
        if name.len() != usize::from(u8::from_be_bytes(name_length)) {
            return Err(BlobError::Bad("name"));
        }
        input.read_exact(&mut name).map_err(BlobError::Io)?;
        let name = String::from_utf8(name).ok().filter(|name| is_name(name)).ok_or(BlobError::Bad("name"))?;
        if blobs.iter().any(|(held, _)| *held == name) {
            return Err(BlobError::Bad("name given twice"));
        }
        let length = number(input)?;
        total = total.saturating_add(length);
        if length > MAX_BLOB_BYTES || total > MAX_BLOBS_BYTES {
            return Err(BlobError::Bad("length"));
        }
        // Read as it arrives, so that a length that lies allocates only
        // what was really sent.
        let mut bytes = Vec::new();
        let taken = input.take(u64::try_from(length).unwrap_or(u64::MAX)).read_to_end(&mut bytes).map_err(BlobError::Io)?;
        if taken != length {
            return Err(BlobError::Io(std::io::ErrorKind::UnexpectedEof.into()));
        }
        blobs.push((name, bytes));
    }
    Ok(blobs)
}

/// The bytes of the file named `name`.
#[must_use]
pub fn find<'a>(blobs: &'a [Blob], name: &str) -> Option<&'a [u8]> {
    blobs.iter().find(|(held, _)| held == name).map(|(_, bytes)| bytes.as_slice())
}
