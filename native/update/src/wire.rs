//! The trusted core's messages to the updater and the updater's answers
//! (`installation-and-updates.md` section 3): the core says when to look
//! and which version to stage; the updater says what the repository's
//! newest version is, that a version is staged, or why it could not do
//! either. The core parses none of the updater's formats, only these
//! frames, each a kind byte, a big-endian length and a payload, read
//! within its bound: the updater is the process that reads the network,
//! so what it says is untrusted.

use std::io::{self, Read, Write};

use crate::version::{self, Version};

/// The timestamp's life, in days (section 9.3; the owner's answer to
/// section 12, item 8): the release tool signs it so, and the core says
/// "Updates unavailable" after so long without a successful check.
pub const TIMESTAMP_DAYS: i64 = 7;
/// The longest payload of any message.
pub const MAX_MESSAGE: usize = 512;
/// The longest reason the updater gives, in bytes.
pub const MAX_REASON: usize = 300;
const _: () = assert!(MAX_REASON + version::MAX_TEXT + 32 <= MAX_MESSAGE, "every message fits");
/// A frame's kind byte and length.
const HEADER_BYTES: usize = 5;

const KIND_CHECK: u8 = 1;
const KIND_DOWNLOAD: u8 = 2;
const KIND_LATEST: u8 = 11;
const KIND_STAGED: u8 = 12;
const KIND_FAILED: u8 = 13;

/// What the trusted core tells the updater.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToUpdater {
    /// Look for a newer version (at start, then every 24 hours).
    Check,
    /// Download and stage this version, which the user accepted.
    Download(Version),
}

/// What the updater tells the trusted core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromUpdater {
    /// The repository's metadata verified: its newest version for this
    /// platform, if it has one, and when that was learnt (seconds since
    /// 1970, UTC).
    Latest {
        /// The newest version.
        version: Option<Version>,
        /// When.
        checked: i64,
    },
    /// This version is staged, complete, for the launcher's next start.
    Staged(Version),
    /// A check or a download failed: why, and when a check last
    /// succeeded, if ever.
    Failed {
        /// Why, in words.
        reason: String,
        /// The last successful check.
        last_success: Option<i64>,
    },
}

fn push_version(payload: &mut Vec<u8>, version: Version) {
    let text = version.to_string();
    payload.push(u8::try_from(text.len()).unwrap_or(0));
    payload.extend_from_slice(text.as_bytes());
}

fn push_time(payload: &mut Vec<u8>, time: Option<i64>) {
    payload.push(u8::from(time.is_some()));
    payload.extend_from_slice(&time.unwrap_or(0).to_be_bytes());
}

/// Reads fields, refusing anything short, long or malformed.
struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < count {
            return Err(bad("a message that ends early"));
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }

    fn u8(&mut self) -> io::Result<u8> {
        self.take(1)?.first().copied().ok_or_else(|| bad("a message that ends early"))
    }

    fn flag(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(bad("a flag that is not one")),
        }
    }

    fn version(&mut self) -> io::Result<Version> {
        let length = usize::from(self.u8()?);
        if length > version::MAX_TEXT {
            return Err(bad("a version too long"));
        }
        std::str::from_utf8(self.take(length)?).ok().and_then(Version::parse).ok_or_else(|| bad("a version that is not one"))
    }

    fn time(&mut self) -> io::Result<Option<i64>> {
        let present = self.flag()?;
        let time = i64::from_be_bytes(self.take(8)?.try_into().map_err(|_| bad("a time that is not one"))?);
        Ok(present.then_some(time))
    }

    fn end(&self) -> io::Result<()> {
        if self.0.is_empty() { Ok(()) } else { Err(bad("a message longer than its fields")) }
    }
}

fn bad(why: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why)
}

fn frame(kind: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.len() > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a message too long"));
    }
    let mut bytes = vec![kind];
    bytes.extend_from_slice(&u32::try_from(payload.len()).map_err(|_| io::ErrorKind::InvalidInput)?.to_be_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// One frame from `reader`: its kind and payload, within the bound.
fn read_frame(reader: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0_u8; HEADER_BYTES];
    reader.read_exact(&mut header)?;
    let [kind, a, b, c, d] = header;
    let length = usize::try_from(u32::from_be_bytes([a, b, c, d])).map_err(|_| bad("a message too long"))?;
    if length > MAX_MESSAGE {
        return Err(bad("a message too long"));
    }
    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload)?;
    Ok((kind, payload))
}

/// The bytes of a message to the updater.
///
/// # Errors
/// Never for these messages; the type is the writers'.
pub fn encode_to_updater(message: &ToUpdater) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match message {
        ToUpdater::Check => KIND_CHECK,
        ToUpdater::Download(version) => {
            push_version(&mut payload, *version);
            KIND_DOWNLOAD
        }
    };
    frame(kind, &payload)
}

/// A message to the updater from its kind and payload.
///
/// # Errors
/// On anything malformed.
pub fn decode_to_updater(kind: u8, payload: &[u8]) -> io::Result<ToUpdater> {
    let mut fields = Fields(payload);
    let message = match kind {
        KIND_CHECK => ToUpdater::Check,
        KIND_DOWNLOAD => ToUpdater::Download(fields.version()?),
        _ => return Err(bad("no such message")),
    };
    fields.end()?;
    Ok(message)
}

/// The bytes of a message from the updater.
///
/// # Errors
/// When the reason is too long.
pub fn encode_from_updater(message: &FromUpdater) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match message {
        FromUpdater::Latest { version, checked } => {
            match version {
                Some(version) => {
                    payload.push(1);
                    push_version(&mut payload, *version);
                }
                None => payload.push(0),
            }
            payload.extend_from_slice(&checked.to_be_bytes());
            KIND_LATEST
        }
        FromUpdater::Staged(version) => {
            push_version(&mut payload, *version);
            KIND_STAGED
        }
        FromUpdater::Failed { reason, last_success } => {
            let reason = truncated(reason);
            payload.extend_from_slice(&u16::try_from(reason.len()).map_err(|_| io::ErrorKind::InvalidInput)?.to_be_bytes());
            payload.extend_from_slice(reason.as_bytes());
            push_time(&mut payload, *last_success);
            KIND_FAILED
        }
    };
    frame(kind, &payload)
}

/// `reason` cut to [`MAX_REASON`] bytes at a character's boundary.
fn truncated(reason: &str) -> &str {
    let mut end = reason.len().min(MAX_REASON);
    while !reason.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    reason.get(..end).unwrap_or_default()
}

/// A message from the updater from its kind and payload, strictly.
///
/// # Errors
/// On anything malformed or out of bounds.
pub fn decode_from_updater(kind: u8, payload: &[u8]) -> io::Result<FromUpdater> {
    let mut fields = Fields(payload);
    let message = match kind {
        KIND_LATEST => {
            let version = if fields.flag()? { Some(fields.version()?) } else { None };
            let checked = i64::from_be_bytes(fields.take(8)?.try_into().map_err(|_| bad("a time that is not one"))?);
            FromUpdater::Latest { version, checked }
        }
        KIND_STAGED => FromUpdater::Staged(fields.version()?),
        KIND_FAILED => {
            let length = usize::from(u16::from_be_bytes(fields.take(2)?.try_into().map_err(|_| bad("a length that is not one"))?));
            if length > MAX_REASON {
                return Err(bad("a reason too long"));
            }
            let reason = String::from_utf8(fields.take(length)?.to_vec()).map_err(|_| bad("a reason that is not text"))?;
            FromUpdater::Failed { reason, last_success: fields.time()? }
        }
        _ => return Err(bad("no such message")),
    };
    fields.end()?;
    Ok(message)
}

/// Writes a message to the updater.
///
/// # Errors
/// When the pipe does.
pub fn write_to_updater(writer: &mut impl Write, message: &ToUpdater) -> io::Result<()> {
    writer.write_all(&encode_to_updater(message)?)?;
    writer.flush()
}

/// Reads a message to the updater.
///
/// # Errors
/// When the pipe ends or the message is malformed.
pub fn read_to_updater(reader: &mut impl Read) -> io::Result<ToUpdater> {
    let (kind, payload) = read_frame(reader)?;
    decode_to_updater(kind, &payload)
}

/// Writes a message from the updater.
///
/// # Errors
/// When the pipe does.
pub fn write_from_updater(writer: &mut impl Write, message: &FromUpdater) -> io::Result<()> {
    writer.write_all(&encode_from_updater(message)?)?;
    writer.flush()
}

/// Reads a message from the updater.
///
/// # Errors
/// When the pipe ends or the message is malformed.
pub fn read_from_updater(reader: &mut impl Read) -> io::Result<FromUpdater> {
    let (kind, payload) = read_frame(reader)?;
    decode_from_updater(kind, &payload)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn every_message_round_trips() {
        for message in [ToUpdater::Check, ToUpdater::Download(Version::new(0, 3, 0))] {
            assert_eq!(read_to_updater(&mut encode_to_updater(&message).unwrap().as_slice()).unwrap(), message);
        }
        for message in [
            FromUpdater::Latest { version: Some(Version::new(0, 3, 0)), checked: 1_792_656_300 },
            FromUpdater::Latest { version: None, checked: 0 },
            FromUpdater::Staged(Version::new(0, 3, 0)),
            FromUpdater::Failed { reason: "the timestamp has expired".into(), last_success: Some(1_792_000_000) },
            FromUpdater::Failed { reason: String::new(), last_success: None },
        ] {
            assert_eq!(read_from_updater(&mut encode_from_updater(&message).unwrap().as_slice()).unwrap(), message);
        }
    }

    #[test]
    fn a_long_reason_is_cut_and_a_malformed_message_refused() {
        let long = FromUpdater::Failed { reason: "é".repeat(MAX_REASON), last_success: None };
        let FromUpdater::Failed { reason, .. } = read_from_updater(&mut encode_from_updater(&long).unwrap().as_slice()).unwrap() else { panic!() };
        assert!(reason.len() <= MAX_REASON && reason.chars().all(|c| c == 'é'));
        assert!(decode_to_updater(KIND_CHECK, &[0]).is_err(), "long");
        assert!(decode_to_updater(KIND_DOWNLOAD, &[3, b'0', b'.', b'1']).is_err(), "not a version");
        assert!(decode_to_updater(99, &[]).is_err());
        assert!(decode_from_updater(KIND_LATEST, &[2]).is_err(), "a flag that is not one");
        assert!(decode_from_updater(KIND_FAILED, &[0xff, 0xff]).is_err(), "too long");
        assert!(decode_from_updater(KIND_STAGED, &[200]).is_err(), "a version too long");
        assert!(read_frame(&mut [KIND_CHECK, 0, 0, 0x10, 0].as_slice()).is_err(), "beyond the bound");
    }
}
