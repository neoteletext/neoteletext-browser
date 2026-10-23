//! The link messages (`hyperlinks.md` section 8.1). A decoder sends the
//! links it finds in each page it renders, and a fetcher the aliases and
//! named services of its channel's descriptor; both are untrusted, so their
//! targets arrive as raw text for the trusted core's own parser. The
//! trusted core sends the interface only where each link of a page lies and
//! what kind of place it leads to, never the target itself.

mod fastext;
mod found;
mod interface;
#[cfg(test)]
mod tests;

use interface_core::canvas::{COLUMNS, PageKey, ROWS};

use crate::{Fields, MAX_ADDRESS, ProtocolError, push_key};
use interface::{push_numbers, push_page_links, push_pointing, push_prompt, read_numbers, read_page_links, read_pointing, read_prompt};

pub use fastext::{Fastext, FastextTarget};
pub(crate) use fastext::{push_fastext, read_fastext};
pub use found::{Alias, Candidate, Found, NumberLink, Service};
pub(crate) use found::{push_channel_links, push_found, read_channel_links, read_found};
pub use interface_core::controls::ANY_SUBCODE;
pub use interface_core::links::{LinkKind, LinkRun, PageLinks, Pointing, Prompt, PromptKind};

/// The most links a page has: beyond this the trusted core gives it none
/// (`hyperlinks.md` section 8.2).
pub const MAX_RUNS: usize = 32;
/// The most page numbers found in one page's text (`hyperlinks.md`
/// section 6.1).
pub const MAX_FOUND: usize = 24;
/// The most `LT` and `LK` lines a page keeps (the engine's `TEXT_LINKS`
/// and `NUMBER_LINKS`, TTI Link Extension section E.1).
pub const MAX_TEXT_LINKS: usize = 8;
/// The most `LK` lines a page keeps.
pub const MAX_NUMBER_LINKS: usize = 8;
/// A target as a page file writes it: at most 256 bytes (the engine's
/// `LINK_ADDRESS_BYTES`, TTI Link Extension section E.1), a `web:` target
/// included.
pub const MAX_TARGET: usize = 256;
/// The most aliases a descriptor has (the engine's REQ-WEB-002).
pub const MAX_ALIASES: usize = 16;
/// The most named services a descriptor lists (the engine's
/// `MAX_LINKED_SERVICES`).
pub const MAX_SERVICES: usize = 16;
/// A named service's short name (the engine's `MAX_SHORT_NAME`).
pub const MAX_SERVICE_NAME: usize = 32;
/// Page numbers a page remaps: its `LK` lines and the channel's aliases.
pub const MAX_REMAPPED: usize = MAX_NUMBER_LINKS + MAX_ALIASES;
/// A destination as the trusted strip shows it: an address and a few
/// words about it.
pub const MAX_DESTINATION: usize = MAX_ADDRESS + 64;

// Interface to trusted core (`interface/kinds.rs` lists every kind of the
// pipe).
pub(crate) const KIND_POINT: u8 = 24;
pub(crate) const KIND_TYPED: u8 = 25;
pub(crate) const KIND_CONFIRM: u8 = 36;
pub(crate) const KIND_CANCEL: u8 = 37;
// Trusted core to interface.
const KIND_PAGE_LINKS: u8 = 11;
const KIND_POINTING: u8 = 12;
const KIND_PROMPT: u8 = 13;
const KIND_SHOW: u8 = 14;
const KIND_ALIASES: u8 = 15;

/// What the trusted core tells the interface about hyperlinks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Where the links of one image of a page lie: no target, only places
    /// and kinds.
    Page {
        /// Which page.
        key: PageKey,
        /// Its links, for the image of their version.
        links: PageLinks,
    },
    /// What the trusted strip shows of the link under the pointer or the
    /// keyboard's focus, or nothing (requirement 17.6).
    Pointing(Option<Pointing>),
    /// The question the trusted strip asks before a link leaves, or none
    /// (requirements 8.4 and 17.8).
    Prompt(Option<Prompt>),
    /// Open this page of the active channel: a link was followed.
    Show(PageKey),
    /// The page numbers the channel's descriptor links elsewhere: typed,
    /// they go to the trusted core.
    Aliases {
        /// The channel.
        channel: u32,
        /// The page numbers.
        numbers: Vec<u16>,
    },
}

/// Writes a message to the interface's payload; returns its kind.
pub(crate) fn push_message(payload: &mut Vec<u8>, message: &Message) -> Result<u8, ProtocolError> {
    Ok(match message {
        Message::Page { key, links } => {
            push_page_links(payload, *key, links)?;
            KIND_PAGE_LINKS
        }
        Message::Pointing(pointing) => {
            push_pointing(payload, pointing.as_ref())?;
            KIND_POINTING
        }
        Message::Prompt(prompt) => {
            push_prompt(payload, prompt.as_ref())?;
            KIND_PROMPT
        }
        Message::Show(key) => {
            push_key(payload, *key);
            KIND_SHOW
        }
        Message::Aliases { channel, numbers } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            push_numbers(payload, numbers)?;
            KIND_ALIASES
        }
    })
}

/// Whether `kind` is a link message from the trusted core.
pub(crate) fn is_message(kind: u8) -> bool {
    (KIND_PAGE_LINKS..=KIND_ALIASES).contains(&kind)
}

/// Reads a link message from the trusted core of `kind`.
pub(crate) fn read_message(kind: u8, fields: &mut Fields<'_>) -> Result<Message, ProtocolError> {
    let empty = fields.is_empty();
    Ok(match kind {
        KIND_PAGE_LINKS => {
            let (key, links) = read_page_links(fields)?;
            Message::Page { key, links }
        }
        KIND_POINTING => Message::Pointing(read_pointing(fields, empty)?),
        KIND_PROMPT => Message::Prompt(read_prompt(fields, empty)?),
        KIND_SHOW => Message::Show(fields.key()?),
        KIND_ALIASES => Message::Aliases { channel: fields.u32()?, numbers: read_numbers(fields)? },
        other => return Err(ProtocolError::UnknownKind(other)),
    })
}

/// A cell of a page image: where the pointer or the keyboard is, or a
/// click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    /// The page.
    pub key: PageKey,
    /// The image of the page that was on screen.
    pub version: u32,
    /// The column, 0 to 39.
    pub column: u8,
    /// The row, 0 to 24.
    pub row: u8,
}

/// A cell of a page image, or none.
pub(crate) fn push_spot(payload: &mut Vec<u8>, spot: Option<&Spot>) {
    if let Some(spot) = spot {
        push_key(payload, spot.key);
        payload.extend_from_slice(&spot.version.to_be_bytes());
        payload.extend_from_slice(&[spot.column, spot.row]);
    }
}

pub(crate) fn read_spot(fields: &mut Fields<'_>) -> Result<Spot, ProtocolError> {
    let key = fields.key()?;
    let version = fields.u32()?;
    let (column, row) = (fields.u8()?, fields.u8()?);
    if u16::from(column) >= COLUMNS || u16::from(row) >= ROWS {
        return Err(ProtocolError::BadValue("cell outside the page"));
    }
    Ok(Spot { key, version, column, row })
}
