//! Messages from a fetcher process to the trusted core: the same framing,
//! bounds and strictness as the other messages, with kinds of their own.
//! The fetcher reads the network formats (HTTP, server-sent events, the
//! contents file) through the engine's web module; what it sends is
//! untrusted, and every bound is checked on arrival. The trusted core
//! passes page bytes to the channel's decoder without reading them.

use std::io::{Read, Write};

pub use interface_core::browser::CertificateProblem;

use crate::subpages::SubPageRows;
use crate::{Fields, MAX_ADDRESS, MAX_PAGE_TITLE, MAX_PAYLOAD, ProtocolError, push_count, push_raw_text, raw_text, read_frame, write_frame};
use interface_core::channel::page_index;
#[cfg(feature = "links")]
use kinds::KIND_LINKS;
use kinds::{
    KIND_CERTIFICATE_REFUSED, KIND_CLEAR_SITE_DATA, KIND_CONFINED, KIND_CONTENTS, KIND_COOKIE_ANSWER, KIND_COOKIE_QUESTION, KIND_COOKIE_SET, KIND_EXCHANGE,
    KIND_FETCH_SUBPAGE, KIND_FETCH_SUBPAGES, KIND_NOTICE, KIND_PAGE, KIND_PROXY_FAILED, KIND_PUSHING, KIND_REACHED_ANSWER, KIND_REACHED_QUESTION,
    KIND_SUBPAGES, KIND_SUBPAGES_MISSING,
};
#[cfg(feature = "interaction")]
use kinds::{KIND_INPUT, KIND_SEND};

mod cookies;
mod kinds;
#[cfg(test)]
mod tests;

pub use cookies::{CookieSite, MAX_COOKIE_HEADER, MAX_SET_COOKIE};

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
/// The entries of a page's catalogue of sub-pages one message carries: a
/// catalogue of the standard's 8,190 is eight messages, each well within
/// a payload with every title at its longest (decision 112 item 3).
pub const CATALOGUE_PART: usize = 1024;
/// A catalogue entry at its longest: its sub-code, its title's length and
/// the title.
const CATALOGUE_ENTRY_BYTES: usize = 2 + 2 + MAX_PAGE_TITLE;
/// A catalogue message's page and its three counts.
const CATALOGUE_HEAD_BYTES: usize = 2 + 2 + 2 + 2;
const _: () = assert!(CATALOGUE_HEAD_BYTES + CATALOGUE_PART * CATALOGUE_ENTRY_BYTES <= MAX_PAYLOAD, "a catalogue part fits one message");
/// HTTP status codes are three digits from 100 (RFC 9110 section 15).
const STATUS_CODES: core::ops::RangeInclusive<u16> = 100..=599;

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
    /// It is confined and running: a fetcher says this first, before it
    /// opens its channel, so the trusted core can tell one that could not
    /// start from one a channel's server ended (finding F-12).
    Confined,
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
    /// A connection of the channel was refused because of its certificate,
    /// and why (`certificate-errors.md`, decision 83). It names no host:
    /// the trusted core knows the fetcher's channel.
    CertificateRefused(CertificateProblem),
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
    /// Asks for the `Cookie` header of a request the agent is about to
    /// send (`cookies.md` section 2): the trusted core keeps every cookie.
    CookieQuestion {
        /// Matches the answer to the question.
        id: u32,
        /// The request's host.
        host: String,
        /// The request's path.
        path: String,
        /// The request goes over an encrypted connection.
        secure: bool,
        /// The kind of request.
        site: CookieSite,
    },
    /// Asks whether the host was reached over an encrypted connection
    /// that set a cookie still live (the engine's REQ-SEC-007).
    ReachedQuestion {
        /// Matches the answer to the question.
        id: u32,
        /// The host asked about.
        host: String,
    },
    /// A `Set-Cookie` value a response carried, as received.
    CookieSet {
        /// The response's host.
        host: String,
        /// The path of the request it answered.
        path: String,
        /// The channel's folder, which keeps the cookie inside it.
        scope: Option<String>,
        /// The response came over an encrypted connection.
        secure: bool,
        /// The header's value, as received.
        line: Vec<u8>,
    },
    /// A response from `host` over an encrypted connection said
    /// `Clear-Site-Data: "cookies"` (`cookies.md` section 8).
    ClearSiteData(String),
    /// Part of a page's catalogue of sub-pages, `NNN.sub`, as the engine's
    /// reader read it (REQ-WEB-034): the catalogue's count, the place of
    /// the part's first entry and at most [`CATALOGUE_PART`] entries, each
    /// a sub-code and its title, not yet cleaned, in the page's order. The
    /// parts of one catalogue come in order, the first at place 0.
    SubPages {
        /// The page.
        page: u16,
        /// The count, the first entry's place and the entries.
        part: SubPageRows,
    },
    /// The page has no catalogue (404 or 410): its sub-pages, if any, are
    /// in its page file.
    SubPagesMissing {
        /// The page.
        page: u16,
    },
}

/// From the trusted core to a fetcher: input messages (requirement 4.7
/// and its one exception, 18.4), the answers to its cookie questions, and
/// the catalogues and sub-pages the user's page needs (decision 112).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToFetcher {
    /// One input message for the channel's push WebSocket: whole items of
    /// the input stream, at most 4 096 bytes, read by the engine's reader on
    /// both sides (REQ-INT-133).
    #[cfg(feature = "interaction")]
    Send(Vec<u8>),
    /// The `Cookie` header of a request, empty for none.
    CookieAnswer {
        /// The question's.
        id: u32,
        /// The header's value.
        header: Vec<u8>,
    },
    /// Whether the host was reached securely.
    ReachedAnswer {
        /// The question's.
        id: u32,
        /// Reached securely, with a cookie still live.
        reached: bool,
    },
    /// Fetch a page's catalogue of sub-pages, `NNN.sub` (the engine's
    /// `Input::FetchSubPages`, REQ-WEB-034), conditionally when it was
    /// fetched before.
    FetchSubPages {
        /// The page.
        page: u16,
    },
    /// Fetch one sub-page: its own file, `NNN-SSSS.t42`, once the page's
    /// catalogue has answered, else the page's file (the engine's
    /// `Input::FetchPage` with a sub-code, REQ-WEB-019).
    FetchSubPage {
        /// The page.
        page: u16,
        /// The sub-page, any of the thirteen bits' values but 3F7F.
        subcode: u16,
    },
}

/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_to_fetcher(out: &mut dyn Write, message: &ToFetcher) -> Result<(), ProtocolError> {
    match message {
        #[cfg(feature = "interaction")]
        ToFetcher::Send(record) => {
            crate::interaction::check_message(record)?;
            write_frame(out, KIND_SEND, record)
        }
        ToFetcher::CookieAnswer { .. } | ToFetcher::ReachedAnswer { .. } => {
            let mut payload = Vec::new();
            let kind = cookies::push_answer(&mut payload, message)?;
            write_frame(out, kind, &payload)
        }
        ToFetcher::FetchSubPages { page } => {
            page_index(*page).ok_or(ProtocolError::BadValue("page number"))?;
            write_frame(out, KIND_FETCH_SUBPAGES, &page.to_be_bytes())
        }
        ToFetcher::FetchSubPage { page, subcode } => {
            page_index(*page).ok_or(ProtocolError::BadValue("page number"))?;
            if !crate::decoder::names_one_subpage(*subcode) {
                return Err(ProtocolError::BadValue("sub-code"));
            }
            let mut payload = page.to_be_bytes().to_vec();
            payload.extend_from_slice(&subcode.to_be_bytes());
            write_frame(out, KIND_FETCH_SUBPAGE, &payload)
        }
    }
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors, any other kind, and anything out of bounds.
pub fn read_to_fetcher(input: &mut dyn Read) -> Result<Option<ToFetcher>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        #[cfg(feature = "interaction")]
        KIND_SEND => {
            crate::interaction::check_message(&payload)?;
            return Ok(Some(ToFetcher::Send(payload)));
        }
        KIND_COOKIE_ANSWER | KIND_REACHED_ANSWER => cookies::read_answer(kind, &mut fields)?,
        KIND_FETCH_SUBPAGES => ToFetcher::FetchSubPages { page: fields.display_page()? },
        KIND_FETCH_SUBPAGE => {
            let page = fields.display_page()?;
            let subcode = fields.u16()?;
            if !crate::decoder::names_one_subpage(subcode) {
                return Err(ProtocolError::BadValue("sub-code"));
            }
            ToFetcher::FetchSubPage { page, subcode }
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_to_core(out: &mut dyn Write, message: &FromFetcher) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        FromFetcher::Contents(entries) => {
            push_count(&mut payload, entries.len(), MAX_ENTRIES, "too many entries")?;
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
        FromFetcher::Confined => KIND_CONFINED,
        FromFetcher::ProxyFailed => KIND_PROXY_FAILED,
        FromFetcher::CertificateRefused(problem) => {
            payload.push(crate::problem_code(*problem));
            KIND_CERTIFICATE_REFUSED
        }
        FromFetcher::CookieQuestion { .. } | FromFetcher::ReachedQuestion { .. } | FromFetcher::CookieSet { .. } | FromFetcher::ClearSiteData(_) => {
            cookies::push_cookie_message(&mut payload, message)?
        }
        FromFetcher::SubPages { page, part } => {
            page_index(*page).ok_or(ProtocolError::BadValue("page number"))?;
            payload.extend_from_slice(&page.to_be_bytes());
            crate::subpages::push_rows(&mut payload, part, CATALOGUE_PART, |payload, title| {
                push_raw_text(payload, title, MAX_PAGE_TITLE);
                Ok(())
            })?;
            KIND_SUBPAGES
        }
        FromFetcher::SubPagesMissing { page } => {
            page_index(*page).ok_or(ProtocolError::BadValue("page number"))?;
            payload.extend_from_slice(&page.to_be_bytes());
            KIND_SUBPAGES_MISSING
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
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_CONTENTS => {
            let count = fields.count_u16(MAX_ENTRIES, "too many entries")?;
            let entries = (0..count)
                .map(|_| match fields.u8()? {
                    ENTRY_SECTION => Ok(Entry::Section(raw_text(&mut fields, MAX_PAGE_TITLE, "text too long")?)),
                    ENTRY_PAGE => {
                        let page = fields.display_page()?;
                        Ok(Entry::Page { page, title: raw_text(&mut fields, MAX_PAGE_TITLE, "text too long")? })
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
            FromFetcher::Page { origin, bytes: fields.take(fields.remaining())?.to_vec() }
        }
        KIND_NOTICE => FromFetcher::Notice(raw_text(&mut fields, MAX_NOTICE, "text too long")?),
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
        KIND_PUSHING => FromFetcher::Pushing(fields.flag("pushing")?),
        KIND_EXCHANGE => {
            let method = raw_text(&mut fields, MAX_METHOD, "text too long")?;
            if method.is_empty() || !method.bytes().all(|byte| byte.is_ascii_uppercase()) {
                return Err(ProtocolError::BadValue("method"));
            }
            let url = raw_text(&mut fields, MAX_EXCHANGE_URL, "text too long")?;
            let status = fields.u16()?;
            if !STATUS_CODES.contains(&status) {
                return Err(ProtocolError::BadValue("status"));
            }
            let bytes = u64::from_be_bytes(fields.array()?);
            let elapsed_us = u64::from_be_bytes(fields.array()?);
            FromFetcher::Exchange { method, url, status, bytes, elapsed_us }
        }
        KIND_CONFINED => FromFetcher::Confined,
        KIND_PROXY_FAILED => FromFetcher::ProxyFailed,
        KIND_CERTIFICATE_REFUSED => FromFetcher::CertificateRefused(crate::problem_of(fields.u8()?).ok_or(ProtocolError::BadValue("certificate problem"))?),
        KIND_COOKIE_QUESTION | KIND_REACHED_QUESTION | KIND_COOKIE_SET | KIND_CLEAR_SITE_DATA => cookies::read_cookie_message(kind, &mut fields)?,
        KIND_SUBPAGES => {
            let page = fields.display_page()?;
            let part = crate::subpages::read_rows(&mut fields, CATALOGUE_PART, |fields| raw_text(fields, MAX_PAGE_TITLE, "title too long"))?;
            FromFetcher::SubPages { page, part }
        }
        KIND_SUBPAGES_MISSING => FromFetcher::SubPagesMissing { page: fields.display_page()? },
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}
