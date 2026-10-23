//! The fields of a payload: the strict reader every message is parsed
//! with, which fails on any short read and refuses what runs on, and the
//! writers of the fields every pipe shares (a page key, cleaned text, a
//! list of keys).

use interface_core::canvas::PageKey;
use interface_core::channel::page_index;

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
        let count = self.count_u16(MAX_PAGES, "too many pages")?;
        (0..count).map(|_| self.key()).collect()
    }

    /// A display page number, 100 to 8FF.
    pub(crate) fn display_page(&mut self) -> Result<u16, ProtocolError> {
        let page = self.u16()?;
        page_index(page).ok_or(ProtocolError::BadValue("page number"))?;
        Ok(page)
    }

    /// A one-byte flag: 0 or 1, nothing else; `reason` names what was
    /// neither.
    pub(crate) fn flag(&mut self, reason: &'static str) -> Result<bool, ProtocolError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProtocolError::BadValue(reason)),
        }
    }

    /// A one-byte count of at most `max`.
    pub(crate) fn count_u8(&mut self, max: usize, reason: &'static str) -> Result<usize, ProtocolError> {
        let count = usize::from(self.u8()?);
        if count > max { Err(ProtocolError::BadValue(reason)) } else { Ok(count) }
    }

    /// A two-byte count of at most `max`.
    pub(crate) fn count_u16(&mut self, max: usize, reason: &'static str) -> Result<usize, ProtocolError> {
        let count = usize::from(self.u16()?);
        if count > max { Err(ProtocolError::BadValue(reason)) } else { Ok(count) }
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
    push_count(payload, keys.len(), MAX_PAGES, "too many pages")?;
    for &key in keys {
        push_key(payload, key);
    }
    Ok(())
}

/// A one-byte flag.
pub(crate) fn push_flag(payload: &mut Vec<u8>, flag: bool) {
    payload.push(u8::from(flag));
}

/// A two-byte count of at most `max`.
pub(crate) fn push_count(payload: &mut Vec<u8>, count: usize, max: usize, reason: &'static str) -> Result<(), ProtocolError> {
    if count > max {
        return Err(ProtocolError::BadValue(reason));
    }
    payload.extend_from_slice(&u16::try_from(count).map_err(|_| ProtocolError::BadValue(reason))?.to_be_bytes());
    Ok(())
}

/// A one-byte count of at most `max`.
pub(crate) fn push_count_u8(payload: &mut Vec<u8>, count: usize, max: usize, reason: &'static str) -> Result<(), ProtocolError> {
    if count > max {
        return Err(ProtocolError::BadValue(reason));
    }
    payload.push(u8::try_from(count).map_err(|_| ProtocolError::BadValue(reason))?);
    Ok(())
}
