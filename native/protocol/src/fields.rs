//! The fields of a payload: the strict reader every message is parsed
//! with, which fails on any short read and refuses what runs on, and the
//! writers of the fields every pipe shares (a page key, cleaned text, a
//! list of keys).

use interface_core::canvas::PageKey;

use crate::text::is_clean;
use crate::{FIRST_PAGE, LAST_PAGE, MAX_PAGES, ProtocolError};

pub(crate) const KEY_BYTES: usize = 6;

/// Reads fields from a payload, failing on any short read.
pub(crate) struct Fields<'a> {
    bytes: &'a [u8],
}

impl<'a> Fields<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub(crate) fn take(&mut self, count: usize) -> Result<&'a [u8], ProtocolError> {
        if self.bytes.len() < count {
            return Err(ProtocolError::BadValue("payload ends early"));
        }
        let (taken, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(taken)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// How many bytes are left to read.
    pub(crate) fn remaining(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        self.take(N)?.try_into().map_err(|_| ProtocolError::BadValue("payload ends early"))
    }

    pub(crate) fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(u8::from_be_bytes(self.array()?))
    }

    pub(crate) fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(crate) fn key(&mut self) -> Result<PageKey, ProtocolError> {
        let channel = self.u32()?;
        let page = self.u16()?;
        if !(FIRST_PAGE..=LAST_PAGE).contains(&page) {
            return Err(ProtocolError::BadValue("page number"));
        }
        Ok(PageKey { channel, page })
    }

    pub(crate) fn keys(&mut self) -> Result<Vec<PageKey>, ProtocolError> {
        let count = usize::from(self.u16()?);
        if count > MAX_PAGES {
            return Err(ProtocolError::BadValue("too many pages"));
        }
        (0..count).map(|_| self.key()).collect()
    }

    pub(crate) fn text(&mut self, max_bytes: usize) -> Result<String, ProtocolError> {
        let length = usize::from(self.u16()?);
        if length > max_bytes {
            return Err(ProtocolError::BadValue("text too long"));
        }
        let text = std::str::from_utf8(self.take(length)?).map_err(|_| ProtocolError::BadValue("text not UTF-8"))?;
        if !is_clean(text, max_bytes) {
            return Err(ProtocolError::BadValue("text not cleaned"));
        }
        Ok(text.to_owned())
    }

    pub(crate) fn end(&self) -> Result<(), ProtocolError> {
        if self.bytes.is_empty() { Ok(()) } else { Err(ProtocolError::BadValue("payload longer than its fields")) }
    }
}

/// A one-byte flag: 0 or 1, nothing else.
pub(crate) fn read_flag(fields: &mut Fields<'_>) -> Result<bool, ProtocolError> {
    match fields.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ProtocolError::BadValue("flag")),
    }
}

pub(crate) fn push_key(payload: &mut Vec<u8>, key: PageKey) {
    payload.extend_from_slice(&key.channel.to_be_bytes());
    payload.extend_from_slice(&key.page.to_be_bytes());
}

pub(crate) fn push_text(payload: &mut Vec<u8>, text: &str, max_bytes: usize) -> Result<(), ProtocolError> {
    if !is_clean(text, max_bytes) {
        return Err(ProtocolError::BadValue("text not cleaned"));
    }
    let length = u16::try_from(text.len()).map_err(|_| ProtocolError::BadValue("text length"))?;
    payload.extend_from_slice(&length.to_be_bytes());
    payload.extend_from_slice(text.as_bytes());
    Ok(())
}

pub(crate) fn push_keys(payload: &mut Vec<u8>, keys: &[PageKey]) -> Result<(), ProtocolError> {
    if keys.len() > MAX_PAGES {
        return Err(ProtocolError::BadValue("too many pages"));
    }
    let count = u16::try_from(keys.len()).map_err(|_| ProtocolError::BadValue("too many pages"))?;
    payload.extend_from_slice(&count.to_be_bytes());
    for &key in keys {
        push_key(payload, key);
    }
    Ok(())
}
