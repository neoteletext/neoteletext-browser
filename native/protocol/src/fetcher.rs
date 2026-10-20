//! Messages from a fetcher process to the trusted core: the same framing,
//! bounds and strictness as the other messages, with kinds of their own.
//! The fetcher reads the network formats (HTTP, server-sent events, the
//! contents file) through the engine's web module; what it sends is
//! untrusted, and every bound is checked on arrival. The trusted core
//! passes page bytes to the channel's decoder without reading them.

use std::io::{Read, Write};

use interface_core::channel::page_index;

use super::{Fields, MAX_PAGE_TITLE, MAX_PAYLOAD, ProtocolError, push_raw_text, read_frame, write_frame};

/// The engine's contents file limits (`neoteletext-web` `contents`): at
/// most 1000 lines, so at most 1000 entries, and 800 pages.
pub const MAX_ENTRIES: usize = 1000;
/// A notice for the log.
pub const MAX_NOTICE: usize = 512;

const KIND_CONTENTS: u8 = 80;
const KIND_PAGE: u8 = 81;
const KIND_NOTICE: u8 = 82;
#[cfg(feature = "links")]
const KIND_LINKS: u8 = 83;

const ENTRY_SECTION: u8 = 1;
const ENTRY_PAGE: u8 = 2;

/// One entry of a channel's contents file, in its order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A heading that starts a section of the contents.
    Section(String),
    /// A page the channel has.
    Page {
        /// Its page number.
        page: u16,
        /// Its title, not yet cleaned.
        title: String,
    },
}

/// From a fetcher to the trusted core. A fetcher is untrusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromFetcher {
    /// The channel's contents file, read.
    Contents(Vec<Entry>),
    /// Page bytes, TTI or T42, as fetched or pushed.
    Page {
        /// Sent by the channel's push connection, not fetched in order.
        pushed: bool,
        /// The page file's bytes.
        bytes: Vec<u8>,
    },
    /// Something for the log, not yet cleaned.
    Notice(String),
    /// The page numbers the channel's descriptor links elsewhere, and the
    /// services it names for `name:mpp` targets (`hyperlinks.md` section
    /// 6.4), untrusted.
    #[cfg(feature = "links")]
    Links {
        /// Its aliases.
        aliases: Vec<crate::links::Alias>,
        /// Its named services.
        services: Vec<crate::links::Service>,
    },
}

fn raw_text(fields: &mut Fields<'_>, limit: usize) -> Result<String, ProtocolError> {
    let length = usize::from(fields.u16()?);
    if length > limit {
        return Err(ProtocolError::BadValue("text too long"));
    }
    Ok(String::from_utf8_lossy(fields.take(length)?).into_owned())
}

/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_to_core(out: &mut dyn Write, message: &FromFetcher) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        FromFetcher::Contents(entries) => {
            if entries.len() > MAX_ENTRIES {
                return Err(ProtocolError::BadValue("too many entries"));
            }
            payload.extend_from_slice(&u16::try_from(entries.len()).unwrap_or(0).to_be_bytes());
            for entry in entries {
                match entry {
                    Entry::Section(title) => {
                        payload.push(ENTRY_SECTION);
                        push_raw_text(&mut payload, title, MAX_PAGE_TITLE);
                    }
                    Entry::Page { page, title } => {
                        payload.push(ENTRY_PAGE);
                        payload.extend_from_slice(&page.to_be_bytes());
                        push_raw_text(&mut payload, title, MAX_PAGE_TITLE);
                    }
                }
            }
            KIND_CONTENTS
        }
        FromFetcher::Page { pushed, bytes } => {
            if bytes.len() >= MAX_PAYLOAD {
                return Err(ProtocolError::TooLong(bytes.len()));
            }
            payload.push(u8::from(*pushed));
            payload.extend_from_slice(bytes);
            KIND_PAGE
        }
        FromFetcher::Notice(text) => {
            push_raw_text(&mut payload, text, MAX_NOTICE);
            KIND_NOTICE
        }
        #[cfg(feature = "links")]
        FromFetcher::Links { aliases, services } => {
            crate::links::push_channel_links(&mut payload, aliases, services)?;
            KIND_LINKS
        }
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from a fetcher, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind a fetcher may not send, and any payload that is
/// malformed or out of bounds.
pub fn read_from_fetcher(input: &mut dyn Read) -> Result<Option<FromFetcher>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields { bytes: &payload };
    let message = match kind {
        KIND_CONTENTS => {
            let count = usize::from(fields.u16()?);
            if count > MAX_ENTRIES {
                return Err(ProtocolError::BadValue("too many entries"));
            }
            let entries = (0..count)
                .map(|_| match fields.u8()? {
                    ENTRY_SECTION => Ok(Entry::Section(raw_text(&mut fields, MAX_PAGE_TITLE)?)),
                    ENTRY_PAGE => {
                        let page = fields.u16()?;
                        page_index(page).ok_or(ProtocolError::BadValue("page number"))?;
                        Ok(Entry::Page { page, title: raw_text(&mut fields, MAX_PAGE_TITLE)? })
                    }
                    _ => Err(ProtocolError::BadValue("contents entry")),
                })
                .collect::<Result<_, _>>()?;
            FromFetcher::Contents(entries)
        }
        KIND_PAGE => {
            let pushed = match fields.u8()? {
                0 => false,
                1 => true,
                _ => return Err(ProtocolError::BadValue("page origin")),
            };
            FromFetcher::Page { pushed, bytes: fields.take(fields.bytes.len())?.to_vec() }
        }
        KIND_NOTICE => FromFetcher::Notice(raw_text(&mut fields, MAX_NOTICE)?),
        #[cfg(feature = "links")]
        KIND_LINKS => {
            let (aliases, services) = crate::links::read_channel_links(&mut fields)?;
            FromFetcher::Links { aliases, services }
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips() {
        for message in [
            FromFetcher::Contents(vec![Entry::Section("News".into()), Entry::Page { page: 0x101, title: "Headlines".into() }]),
            FromFetcher::Page { pushed: true, bytes: b"PN,10100\\nOL,1,Hello\\n".to_vec() },
            FromFetcher::Notice("Opened".into()),
        ] {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &message).unwrap();
            assert_eq!(read_from_fetcher(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
    }

    #[test]
    fn malformed_contents_are_refused() {
        // A hexadecimal page number.
        let bytes = [KIND_CONTENTS, 0, 0, 0, 7, 0, 1, ENTRY_PAGE, 0x1A, 0x00, 0, 0];
        assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
        // An unknown entry kind.
        let bytes = [KIND_CONTENTS, 0, 0, 0, 3, 0, 1, 9];
        assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::BadValue("contents entry"))));
        // A fetcher sending a decoder's kind.
        let bytes = [65, 0, 0, 0, 0];
        assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(65))));
    }
}
