//! Messages from a fetcher process to the trusted core: the same framing,
//! bounds and strictness as the other messages, with kinds of their own.
//! The fetcher reads the network formats (HTTP, server-sent events, the
//! contents file) through the engine's web module; what it sends is
//! untrusted, and every bound is checked on arrival. The trusted core
//! passes page bytes to the channel's decoder without reading them.

use std::io::{Read, Write};

use interface_core::channel::page_index;

use super::{Fields, MAX_ADDRESS, MAX_PAGE_TITLE, MAX_PAYLOAD, ProtocolError, push_raw_text, read_frame, write_frame};

/// The engine's contents file limits (`neoteletext-web` `contents`): at
/// most 1000 lines, so at most 1000 entries, and 800 pages.
pub const MAX_ENTRIES: usize = 1000;
/// A notice for the log.
pub const MAX_NOTICE: usize = 512;
/// The longest method an exchange reports: `GET` and `POST` are the
/// engine's (its `http::Method`).
pub const MAX_METHOD: usize = 8;
/// The longest address an exchange reports: the engine's `MAX_URL_LENGTH`,
/// which is the address field's limit too.
pub const MAX_EXCHANGE_URL: usize = MAX_ADDRESS;
/// HTTP status codes are three digits from 100 (RFC 9110 section 15).
const STATUS_CODES: core::ops::RangeInclusive<u16> = 100..=599;

const KIND_CONTENTS: u8 = 80;
const KIND_PAGE: u8 = 81;
const KIND_NOTICE: u8 = 82;
#[cfg(feature = "links")]
const KIND_LINKS: u8 = 83;
#[cfg(feature = "interaction")]
const KIND_INPUT: u8 = 84;
const KIND_PUSHING: u8 = 85;
const KIND_EXCHANGE: u8 = 86;
const KIND_PROXY_FAILED: u8 = 87;
#[cfg(feature = "interaction")]
const KIND_SEND: u8 = 96;

const ENTRY_SECTION: u8 = 1;
const ENTRY_PAGE: u8 = 2;

/// Where page bytes came from, and so how the decoder reads them. Each
/// is whole: a page file, a bundle or one pushed message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Fetched, of a format the decoder detects (`text/plain`, or no
    /// media type the engine names).
    Fetched,
    /// Fetched as TTI.
    FetchedPage,
    /// Fetched as T42 packets.
    FetchedPackets,
    /// A WebSocket text message or a server-sent event: one TTI page,
    /// whole (the engine's REQ-WEB-005).
    PushedPage,
    /// A WebSocket binary message: whole 42-byte packets.
    PushedPackets,
}

const ORIGIN_FETCHED: u8 = 0;
const ORIGIN_PUSHED_PAGE: u8 = 1;
const ORIGIN_PUSHED_PACKETS: u8 = 2;
const ORIGIN_FETCHED_PAGE: u8 = 3;
const ORIGIN_FETCHED_PACKETS: u8 = 4;

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
        /// Fetched, or pushed and in which form.
        origin: Origin,
        /// The page file's bytes.
        bytes: Vec<u8>,
    },
    /// Something for the log, not yet cleaned.
    Notice(String),
    /// The channel's push connection opened (true) or was lost: a sleeping
    /// tab keeps its fetcher while it holds one (decision 47).
    Pushing(bool),
    /// One exchange the agent made (the engine's REQ-WEB-033), for the
    /// developer console (requirement 19.9), not yet cleaned.
    Exchange {
        /// The request's method.
        method: String,
        /// The address asked for.
        url: String,
        /// The response's status code.
        status: u16,
        /// The body's bytes.
        bytes: u64,
        /// Microseconds from the request to the end of its response.
        elapsed_us: u64,
    },
    /// The proxy refused, failed or did not answer a connection
    /// (requirement 19.10). It carries nothing: the trusted core says it in
    /// its own words, so a fetcher cannot write the strip.
    ProxyFailed,
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
    /// The channel's push WebSocket opened or closed, when its descriptor
    /// says it takes input (INPUT.md section 4): a session may start now,
    /// and whether they go encrypted.
    #[cfg(feature = "interaction")]
    Input {
        /// The socket is open.
        open: bool,
        /// It is `wss`.
        secure: bool,
    },
}

/// From the trusted core to a fetcher: the only thing a fetcher takes
/// (requirement 4.7 and its one exception, 18.4).
#[cfg(feature = "interaction")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToFetcher {
    /// One input message for the channel's push WebSocket: whole items of
    /// the input stream, at most 4 096 bytes, read by the engine's reader on
    /// both sides (REQ-INT-133).
    Send(Vec<u8>),
}

/// # Errors
/// When writing fails or the message is not whole items of the input stream.
#[cfg(feature = "interaction")]
pub fn write_to_fetcher(out: &mut dyn Write, message: &ToFetcher) -> Result<(), ProtocolError> {
    let ToFetcher::Send(record) = message;
    crate::interaction::check_message(record)?;
    write_frame(out, KIND_SEND, record)
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors, any other kind, and anything but an input message.
#[cfg(feature = "interaction")]
pub fn read_to_fetcher(input: &mut dyn Read) -> Result<Option<ToFetcher>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    if kind != KIND_SEND {
        return Err(ProtocolError::UnknownKind(kind));
    }
    crate::interaction::check_message(&payload)?;
    Ok(Some(ToFetcher::Send(payload)))
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
        FromFetcher::Page { origin, bytes } => {
            if bytes.len() >= MAX_PAYLOAD {
                return Err(ProtocolError::TooLong(bytes.len()));
            }
            payload.push(match origin {
                Origin::Fetched => ORIGIN_FETCHED,
                Origin::FetchedPage => ORIGIN_FETCHED_PAGE,
                Origin::FetchedPackets => ORIGIN_FETCHED_PACKETS,
                Origin::PushedPage => ORIGIN_PUSHED_PAGE,
                Origin::PushedPackets => ORIGIN_PUSHED_PACKETS,
            });
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
        #[cfg(feature = "interaction")]
        FromFetcher::Input { open, secure } => {
            payload.push(u8::from(*open) | (u8::from(*secure) << 1));
            KIND_INPUT
        }
        FromFetcher::Pushing(open) => {
            payload.push(u8::from(*open));
            KIND_PUSHING
        }
        FromFetcher::Exchange { method, url, status, bytes, elapsed_us } => {
            if !STATUS_CODES.contains(status) {
                return Err(ProtocolError::BadValue("status"));
            }
            push_raw_text(&mut payload, method, MAX_METHOD);
            push_raw_text(&mut payload, url, MAX_EXCHANGE_URL);
            payload.extend_from_slice(&status.to_be_bytes());
            payload.extend_from_slice(&bytes.to_be_bytes());
            payload.extend_from_slice(&elapsed_us.to_be_bytes());
            KIND_EXCHANGE
        }
        FromFetcher::ProxyFailed => KIND_PROXY_FAILED,
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
            let origin = match fields.u8()? {
                ORIGIN_FETCHED => Origin::Fetched,
                ORIGIN_FETCHED_PAGE => Origin::FetchedPage,
                ORIGIN_FETCHED_PACKETS => Origin::FetchedPackets,
                ORIGIN_PUSHED_PAGE => Origin::PushedPage,
                ORIGIN_PUSHED_PACKETS => Origin::PushedPackets,
                _ => return Err(ProtocolError::BadValue("page origin")),
            };
            FromFetcher::Page { origin, bytes: fields.take(fields.bytes.len())?.to_vec() }
        }
        KIND_NOTICE => FromFetcher::Notice(raw_text(&mut fields, MAX_NOTICE)?),
        #[cfg(feature = "links")]
        KIND_LINKS => {
            let (aliases, services) = crate::links::read_channel_links(&mut fields)?;
            FromFetcher::Links { aliases, services }
        }
        #[cfg(feature = "interaction")]
        KIND_INPUT => {
            let flags = fields.u8()?;
            if flags > 3 {
                return Err(ProtocolError::BadValue("input flags"));
            }
            FromFetcher::Input { open: flags & 1 != 0, secure: flags & 2 != 0 }
        }
        KIND_PUSHING => FromFetcher::Pushing(match fields.u8()? {
            0 => false,
            1 => true,
            _ => return Err(ProtocolError::BadValue("pushing")),
        }),
        KIND_EXCHANGE => {
            let method = raw_text(&mut fields, MAX_METHOD)?;
            if method.is_empty() || !method.bytes().all(|byte| byte.is_ascii_uppercase()) {
                return Err(ProtocolError::BadValue("method"));
            }
            let url = raw_text(&mut fields, MAX_EXCHANGE_URL)?;
            let status = fields.u16()?;
            if !STATUS_CODES.contains(&status) {
                return Err(ProtocolError::BadValue("status"));
            }
            let bytes = u64::from_be_bytes(fields.array()?);
            let elapsed_us = u64::from_be_bytes(fields.array()?);
            FromFetcher::Exchange { method, url, status, bytes, elapsed_us }
        }
        KIND_PROXY_FAILED => FromFetcher::ProxyFailed,
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
            FromFetcher::Page { origin: Origin::PushedPage, bytes: b"PN,10100\\nOL,1,Hello\\n".to_vec() },
            FromFetcher::Page { origin: Origin::PushedPackets, bytes: vec![0; 42] },
            FromFetcher::Page { origin: Origin::Fetched, bytes: vec![1, 2, 3] },
            FromFetcher::Page { origin: Origin::FetchedPage, bytes: b"PN,10000\\n".to_vec() },
            FromFetcher::Page { origin: Origin::FetchedPackets, bytes: vec![0; 84] },
            FromFetcher::Notice("Opened".into()),
            FromFetcher::Pushing(true),
            FromFetcher::Exchange { method: "GET".into(), url: "https://a.example/100.tti".into(), status: 200, bytes: 512, elapsed_us: 1_234 },
            FromFetcher::ProxyFailed,
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
        // An exchange's status out of range, and a method that is not one.
        let exchange = |method: &[u8], status: u16| {
            let mut payload = vec![0, u8::try_from(method.len()).unwrap()];
            payload.extend_from_slice(method);
            payload.extend_from_slice(&[0, 1, b'/']);
            payload.extend_from_slice(&status.to_be_bytes());
            payload.extend_from_slice(&[0; 16]);
            let mut bytes = vec![KIND_EXCHANGE, 0, 0, 0, u8::try_from(payload.len()).unwrap()];
            bytes.extend(payload);
            read_from_fetcher(&mut bytes.as_slice())
        };
        assert!(exchange(b"GET", 200).is_ok());
        assert!(matches!(exchange(b"GET", 99), Err(ProtocolError::BadValue("status"))));
        assert!(matches!(exchange(b"GET", 600), Err(ProtocolError::BadValue("status"))));
        assert!(matches!(exchange(b"get\n", 200), Err(ProtocolError::BadValue("method"))));
        assert!(matches!(exchange(b"", 200), Err(ProtocolError::BadValue("method"))));
        assert!(matches!(exchange(b"VERYLONGMETHOD", 200), Err(ProtocolError::BadValue("text too long"))));
        let bytes = [KIND_PROXY_FAILED, 0, 0, 0, 1, 0];
        assert!(read_from_fetcher(&mut bytes.as_slice()).is_err(), "nothing after it");
        // A fetcher sending a decoder's kind.
        let bytes = [65, 0, 0, 0, 0];
        assert!(matches!(read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(65))));
    }
}
