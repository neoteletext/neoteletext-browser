//! Messages between the trusted core and the interface process
//! (`architecture.md` section 6.4). The framing is the engine's: one kind
//! byte, a 32-bit big-endian payload length, then the payload. Every length is bounded
//! before anything is allocated, every payload is parsed strictly to its
//! exact length, and each side accepts only the kinds meant for it, so the
//! interface can never send what only the trusted core may say.

// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

pub mod audio;
pub mod decoder;
pub mod fetcher;
#[cfg(feature = "interaction")]
pub mod interaction;
#[cfg(feature = "phone-keyboard")]
pub mod keyboard;
#[cfg(feature = "links")]
pub mod links;

use std::io::{self, Read, Write};

use interface_core::browser::{StripState, TabInfo};
use interface_core::canvas::{COLUMNS, PageKey, ROWS};
use interface_core::channel::{BANKS, Channel, PAGES, PageInfo, Status, page_index};
/// Controls, and the invisible characters that reorder or hide text, are
/// never shown from untrusted text (requirement 11.5).
use interface_core::editor::hidden;
use interface_core::theme::TAB_COLOURS;

/// The largest payload accepted, as the engine's pipe (`MAX_FRAME`).
pub const MAX_PAYLOAD: usize = 256 * 1024;
/// A channel has at most 2,048 page numbers: eight magazines of 256
/// (EN 300 706 clause 7.1.2).
pub const MAX_PAGES: usize = 8 * 256;
/// Lowest and highest teletext page numbers, magazine 8 written as 8.
pub const FIRST_PAGE: u16 = 0x100;
/// The highest teletext page number.
pub const LAST_PAGE: u16 = 0x8FF;
/// Bounds on the strip's and the view's text, in bytes of UTF-8: a
/// channel's title.
pub const MAX_TITLE: usize = 256;
/// A channel's identity, its host name.
pub const MAX_IDENTITY: usize = 256;
/// The address bar's limit, as the trusted core's address rules.
pub const MAX_ADDRESS: usize = interface_core::browser::MAX_ADDRESS;
/// A page title: one row of a teletext page is 40 characters, and each
/// takes at most four bytes of UTF-8.
pub const MAX_PAGE_TITLE: usize = 40 * 4;
/// A bank's name.
pub const MAX_BANK_NAME: usize = 64;
/// A tab's title.
pub const MAX_TAB_TITLE: usize = 128;
/// More tabs than any window shows.
pub const MAX_TABS: usize = 64;

// Trusted core to interface.
const KIND_STRIP: u8 = 2;
const KIND_FRAME: u8 = 3;
const KIND_CLOSE: u8 = 4;
const KIND_TABS: u8 = 5;
const KIND_CHANNEL: u8 = 6;
const KIND_PAGE: u8 = 8;
const KIND_UPDATED: u8 = 9;
const KIND_AUDIO: u8 = 10;
#[cfg(feature = "interaction")]
const KIND_TYPING: u8 = 40;
#[cfg(feature = "phone-keyboard")]
const KIND_PHONE: u8 = 41;
#[cfg(feature = "phone-keyboard")]
const KIND_PHONE_KEY: u8 = 42;
// Interface to trusted core: intents.
const KIND_VISIBLE: u8 = 16;
const KIND_CELL: u8 = 17;
const KIND_SELECT_TAB: u8 = 18;
const KIND_CLOSE_TAB: u8 = 19;
const KIND_NEW_TAB: u8 = 20;
const KIND_OPENED: u8 = 21;
const KIND_MUTE_TAB: u8 = 22;
const KIND_MUTE_ALL: u8 = 23;
#[cfg(feature = "links")]
const KIND_POINT: u8 = 24;
#[cfg(feature = "links")]
const KIND_TYPED: u8 = 25;
#[cfg(any(feature = "links", feature = "interaction"))]
const KIND_FOCUSED: u8 = 26;
#[cfg(feature = "interaction")]
const KIND_KEY: u8 = 27;
#[cfg(feature = "phone-keyboard")]
const KIND_PHONE_PANEL: u8 = 28;
#[cfg(feature = "phone-keyboard")]
const KIND_PHONE_FINISH: u8 = 29;
// Interface to trusted core: strip input, kinds of their own
// (`architecture.md` section 6.4).
const KIND_NAVIGATE: u8 = 32;
const KIND_BACK: u8 = 33;
const KIND_FORWARD: u8 = 34;
const KIND_BOOKMARK: u8 = 35;
#[cfg(feature = "links")]
const KIND_CONFIRM: u8 = 36;
#[cfg(feature = "links")]
const KIND_CANCEL: u8 = 37;

// The strip's flags.
const ENCRYPTED: u8 = 1;
const CAN_GO_BACK: u8 = 2;
const CAN_GO_FORWARD: u8 = 4;
const BOOKMARKED: u8 = 8;
const LOCAL: u8 = 16;
const ALL_FLAGS: u8 = ENCRYPTED | CAN_GO_BACK | CAN_GO_FORWARD | BOOKMARKED | LOCAL;
// A tab's flags.
const TAB_MUTED: u8 = 1;

const HEADER_BYTES: usize = 5;
const KEY_BYTES: usize = 6;
const FRAME_FIXED_BYTES: usize = KEY_BYTES + 4;
const CELL_BYTES: usize = KEY_BYTES + 4 + 2;

/// From the trusted core to the interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToInterface {
    /// The window's tabs and which is active.
    Tabs {
        /// Every tab, left to right.
        tabs: Vec<TabInfo>,
        /// The index of the active tab.
        active: usize,
    },
    /// What the trusted strip shows, as the trusted core formatted it.
    Strip(StripState),
    /// The active tab's channel: its banks and every page's status.
    Channel(Channel),
    /// One page's new status.
    Page {
        /// The channel.
        channel: u32,
        /// The page number.
        page: u16,
        /// Its title, cleaned.
        title: String,
    },
    /// A page has a new version: the interface shows that it changed.
    Updated {
        /// The channel.
        channel: u32,
        /// The page number.
        page: u16,
    },
    /// One page image, checked and encoded by the trusted core.
    Frame {
        /// Which page.
        key: PageKey,
        /// Counts up with each new image of the page, so an older one never replaces a newer.
        version: u32,
        /// The image, an indexed PNG at the native size.
        png: Vec<u8>,
    },
    /// A voice operation of a channel, checked by the trusted core.
    Audio {
        /// The channel whose sound it is.
        channel: u32,
        /// The operation.
        audio: audio::Audio,
    },
    /// About hyperlinks (`hyperlinks.md` section 8.1).
    #[cfg(feature = "links")]
    Link(links::Message),
    /// Whether keys go to the open page's channel, and its cursor
    /// (`interaction.md` section 11).
    #[cfg(feature = "interaction")]
    Typing(interaction::Typing),
    /// The phone keyboard's panel and state (`interaction.md` section 9).
    #[cfg(feature = "phone-keyboard")]
    Phone(keyboard::PhoneView),
    /// A key of the paired phone, which the interface takes as its own.
    #[cfg(feature = "phone-keyboard")]
    PhoneKey(keyboard::PhoneKey),
    /// The browser is closing: the interface ends.
    Close,
}

/// From the interface to the trusted core. Intents cannot name another
/// channel's address or change what is fetched (requirements 4.5 and 4.7);
/// only strip input, a separate set of message kinds, can navigate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToCore {
    /// The pages now on screen (requirement 9.5).
    Visible(Vec<PageKey>),
    /// A click on one character cell of a page (requirement 12.3).
    Cell {
        /// Which page.
        key: PageKey,
        /// The image of the page that was on screen.
        version: u32,
        /// The cell's column, 0 to 39.
        column: u8,
        /// The cell's row, 0 to 24.
        row: u8,
    },
    /// Switch to the tab at this index.
    SelectTab(u8),
    /// Close the tab at this index.
    CloseTab(u8),
    /// Open a new tab.
    NewTab,
    /// The page the user opened, zoomed on the wall, or none: its sounds
    /// play while it is open (the engine's REQ-AUD-012).
    Opened(Option<PageKey>),
    /// Mute the tab at this index, or unmute it.
    MuteTab(u8),
    /// Mute every tab, or unmute every tab.
    MuteAll(bool),
    /// The cell under the pointer or the keyboard's focus, when it is on a
    /// link, or none: the trusted strip shows where it leads.
    #[cfg(feature = "links")]
    Point(Option<links::Spot>),
    /// A page number typed that the open page or the channel links
    /// elsewhere.
    #[cfg(feature = "links")]
    Typed {
        /// The number.
        number: u16,
        /// The open page and its image's version, if a page is open.
        opened: Option<(PageKey, u32)>,
    },
    /// The window gained or lost the keyboard focus.
    #[cfg(any(feature = "links", feature = "interaction"))]
    Focused(bool),
    /// A key, text or a click on the open page, for its channel if it
    /// takes input (`interaction.md` section 5).
    #[cfg(feature = "interaction")]
    Key(interaction::KeyIntent),
    /// Strip input: open the phone keyboard's panel, or close it.
    #[cfg(feature = "phone-keyboard")]
    PhonePanel(bool),
    /// Strip input: end the phone keyboard's session.
    #[cfg(feature = "phone-keyboard")]
    PhoneFinish,
    /// Strip input: an address typed into the trusted strip.
    Navigate(String),
    /// Strip input: go back in the tab's history.
    Back,
    /// Strip input: go forward in the tab's history.
    Forward,
    /// Strip input: bookmark the channel, or remove its bookmark.
    Bookmark,
    /// Strip input: yes to the strip's question of this number.
    #[cfg(feature = "links")]
    Confirm(u32),
    /// Strip input: no to the strip's question of this number.
    #[cfg(feature = "links")]
    Cancel(u32),
}

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

/// Untrusted text made safe to show: hidden characters removed, then cut
/// to at most `max_bytes` on a character boundary.
#[must_use]
pub fn clean(text: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max_bytes));
    for character in text.chars().filter(|&character| !hidden(character)) {
        if out.len().saturating_add(character.len_utf8()) > max_bytes {
            break;
        }
        out.push(character);
    }
    out
}

/// Untrusted text sent on for the trusted core to clean: cut to at most
/// `limit` bytes on a character boundary, with a two-byte length before it.
pub(crate) fn push_raw_text(payload: &mut Vec<u8>, text: &str, limit: usize) {
    let end = (0..=text.len().min(limit)).rev().find(|&end| text.is_char_boundary(end)).unwrap_or(0);
    let cut = text.get(..end).unwrap_or_default();
    payload.extend_from_slice(&u16::try_from(cut.len()).unwrap_or(0).to_be_bytes());
    payload.extend_from_slice(cut.as_bytes());
}

fn is_clean(text: &str, max_bytes: usize) -> bool {
    text.len() <= max_bytes && !text.chars().any(hidden)
}

fn write_frame(out: &mut dyn Write, kind: u8, payload: &[u8]) -> Result<(), ProtocolError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(ProtocolError::TooLong(payload.len()));
    }
    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::TooLong(payload.len()))?;
    let mut header = [0_u8; HEADER_BYTES];
    header[0] = kind;
    header[1..].copy_from_slice(&length.to_be_bytes());
    out.write_all(&header)?;
    out.write_all(payload)?;
    out.flush()?;
    Ok(())
}

fn read_frame(input: &mut dyn Read) -> Result<Option<(u8, Vec<u8>)>, ProtocolError> {
    let mut header = [0_u8; HEADER_BYTES];
    match input.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if length > MAX_PAYLOAD {
        return Err(ProtocolError::TooLong(length));
    }
    let mut payload = vec![0_u8; length];
    input.read_exact(&mut payload)?;
    Ok(Some((header[0], payload)))
}

fn push_key(payload: &mut Vec<u8>, key: PageKey) {
    payload.extend_from_slice(&key.channel.to_be_bytes());
    payload.extend_from_slice(&key.page.to_be_bytes());
}

fn push_text(payload: &mut Vec<u8>, text: &str, max_bytes: usize) -> Result<(), ProtocolError> {
    if !is_clean(text, max_bytes) {
        return Err(ProtocolError::BadValue("text not cleaned"));
    }
    let length = u16::try_from(text.len()).map_err(|_| ProtocolError::BadValue("text length"))?;
    payload.extend_from_slice(&length.to_be_bytes());
    payload.extend_from_slice(text.as_bytes());
    Ok(())
}

fn push_keys(payload: &mut Vec<u8>, keys: &[PageKey]) -> Result<(), ProtocolError> {
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

/// Reads fields from a payload, failing on any short read.
struct Fields<'a> {
    bytes: &'a [u8],
}

impl<'a> Fields<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], ProtocolError> {
        if self.bytes.len() < count {
            return Err(ProtocolError::BadValue("payload ends early"));
        }
        let (taken, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        self.take(N)?.try_into().map_err(|_| ProtocolError::BadValue("payload ends early"))
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(u8::from_be_bytes(self.array()?))
    }

    fn u16(&mut self) -> Result<u16, ProtocolError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, ProtocolError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn key(&mut self) -> Result<PageKey, ProtocolError> {
        let channel = self.u32()?;
        let page = self.u16()?;
        if !(FIRST_PAGE..=LAST_PAGE).contains(&page) {
            return Err(ProtocolError::BadValue("page number"));
        }
        Ok(PageKey { channel, page })
    }

    fn keys(&mut self) -> Result<Vec<PageKey>, ProtocolError> {
        let count = usize::from(self.u16()?);
        if count > MAX_PAGES {
            return Err(ProtocolError::BadValue("too many pages"));
        }
        (0..count).map(|_| self.key()).collect()
    }

    fn text(&mut self, max_bytes: usize) -> Result<String, ProtocolError> {
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

    fn end(&self) -> Result<(), ProtocolError> {
        if self.bytes.is_empty() { Ok(()) } else { Err(ProtocolError::BadValue("payload longer than its fields")) }
    }
}

/// Writes a message to the interface.
///
/// # Errors
/// When writing fails, or the message breaks a bound or carries text that
/// was not cleaned.
pub fn write_to_interface(out: &mut dyn Write, message: &ToInterface) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        ToInterface::Tabs { tabs, active } => {
            if tabs.len() > MAX_TABS || (*active >= tabs.len() && !tabs.is_empty()) {
                return Err(ProtocolError::BadValue("tabs"));
            }
            payload.push(u8::try_from(*active).map_err(|_| ProtocolError::BadValue("tabs"))?);
            payload.push(u8::try_from(tabs.len()).map_err(|_| ProtocolError::BadValue("tabs"))?);
            for tab in tabs {
                payload.push(tab.colour);
                payload.extend_from_slice(&tab.channel.to_be_bytes());
                payload.push(if tab.muted { TAB_MUTED } else { 0 });
                push_text(&mut payload, &tab.title, MAX_TAB_TITLE)?;
            }
            KIND_TABS
        }
        ToInterface::Strip(strip) => {
            let flag = |set: bool, bit: u8| if set { bit } else { 0 };
            payload.push(
                flag(strip.encrypted, ENCRYPTED)
                    | flag(strip.back, CAN_GO_BACK)
                    | flag(strip.forward, CAN_GO_FORWARD)
                    | flag(strip.bookmarked, BOOKMARKED)
                    | flag(strip.local, LOCAL),
            );
            push_text(&mut payload, &strip.identity, MAX_IDENTITY)?;
            push_text(&mut payload, &strip.address, MAX_ADDRESS)?;
            KIND_STRIP
        }
        ToInterface::Channel(channel) => {
            if channel.banks.len() != BANKS || channel.pages.len() != PAGES {
                return Err(ProtocolError::BadValue("channel shape"));
            }
            payload.extend_from_slice(&channel.channel.to_be_bytes());
            push_text(&mut payload, &channel.title, MAX_TITLE)?;
            for name in &channel.banks {
                push_text(&mut payload, name, MAX_BANK_NAME)?;
            }
            let present = channel.pages.iter().flatten().count();
            payload.extend_from_slice(&u16::try_from(present).map_err(|_| ProtocolError::BadValue("pages"))?.to_be_bytes());
            for (index, page) in channel.pages.iter().enumerate() {
                let Some(page) = page else { continue };
                let number = interface_core::channel::page_number(index).ok_or(ProtocolError::BadValue("pages"))?;
                payload.extend_from_slice(&number.to_be_bytes());
                payload.push(page.status as u8);
                push_text(&mut payload, &page.title, MAX_PAGE_TITLE)?;
            }
            KIND_CHANNEL
        }
        ToInterface::Page { channel, page, title } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            payload.extend_from_slice(&page.to_be_bytes());
            push_text(&mut payload, title, MAX_PAGE_TITLE)?;
            KIND_PAGE
        }
        ToInterface::Updated { channel, page } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            payload.extend_from_slice(&page.to_be_bytes());
            KIND_UPDATED
        }
        ToInterface::Frame { key, version, png } => {
            push_key(&mut payload, *key);
            payload.extend_from_slice(&version.to_be_bytes());
            payload.extend_from_slice(png);
            KIND_FRAME
        }
        ToInterface::Audio { channel, audio } => {
            payload.extend_from_slice(&channel.to_be_bytes());
            // Checked on its own, as the reader reads it after the channel.
            let mut operation = Vec::new();
            audio::push(&mut operation, audio, audio::Sender::Core)?;
            payload.extend_from_slice(&operation);
            KIND_AUDIO
        }
        #[cfg(feature = "links")]
        ToInterface::Link(message) => links::push_message(&mut payload, message)?,
        #[cfg(feature = "interaction")]
        ToInterface::Typing(typing) => {
            interaction::push_typing(&mut payload, typing)?;
            KIND_TYPING
        }
        #[cfg(feature = "phone-keyboard")]
        ToInterface::Phone(view) => {
            keyboard::push_view(&mut payload, view)?;
            KIND_PHONE
        }
        #[cfg(feature = "phone-keyboard")]
        ToInterface::PhoneKey(key) => {
            keyboard::push_phone_key(&mut payload, key)?;
            KIND_PHONE_KEY
        }
        ToInterface::Close => KIND_CLOSE,
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind the trusted core may not send, and any payload
/// that is malformed or out of bounds.
pub fn read_from_core(input: &mut dyn Read) -> Result<Option<ToInterface>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields { bytes: &payload };
    let message = match kind {
        KIND_TABS => {
            let active = usize::from(fields.u8()?);
            let count = usize::from(fields.u8()?);
            if count > MAX_TABS || (active >= count && count > 0) {
                return Err(ProtocolError::BadValue("tabs"));
            }
            let tabs = (0..count)
                .map(|_| {
                    let colour = fields.u8()?;
                    if usize::from(colour) >= TAB_COLOURS.len() {
                        return Err(ProtocolError::BadValue("tab colour"));
                    }
                    let channel = fields.u32()?;
                    let flags = fields.u8()?;
                    if flags & !TAB_MUTED != 0 {
                        return Err(ProtocolError::BadValue("tab flags"));
                    }
                    Ok(TabInfo { colour, channel, muted: flags & TAB_MUTED != 0, title: fields.text(MAX_TAB_TITLE)? })
                })
                .collect::<Result<_, _>>()?;
            ToInterface::Tabs { tabs, active }
        }
        KIND_STRIP => {
            let flags = fields.u8()?;
            if flags & !ALL_FLAGS != 0 {
                return Err(ProtocolError::BadValue("strip flags"));
            }
            let identity = fields.text(MAX_IDENTITY)?;
            ToInterface::Strip(StripState {
                address: fields.text(MAX_ADDRESS)?,
                identity,
                encrypted: flags & ENCRYPTED != 0,
                back: flags & CAN_GO_BACK != 0,
                forward: flags & CAN_GO_FORWARD != 0,
                bookmarked: flags & BOOKMARKED != 0,
                local: flags & LOCAL != 0,
            })
        }
        KIND_CHANNEL => {
            let mut channel = Channel::empty();
            channel.channel = fields.u32()?;
            channel.title = fields.text(MAX_TITLE)?;
            for name in &mut channel.banks {
                *name = fields.text(MAX_BANK_NAME)?;
            }
            let count = usize::from(fields.u16()?);
            if count > PAGES {
                return Err(ProtocolError::BadValue("too many pages"));
            }
            for _ in 0..count {
                let index = page_index(fields.u16()?).ok_or(ProtocolError::BadValue("page number"))?;
                let status = Status::from_byte(fields.u8()?).ok_or(ProtocolError::BadValue("page status"))?;
                let title = fields.text(MAX_PAGE_TITLE)?;
                let slot = channel.pages.get_mut(index).ok_or(ProtocolError::BadValue("page number"))?;
                if slot.replace(PageInfo { title, status }).is_some() {
                    return Err(ProtocolError::BadValue("page listed twice"));
                }
            }
            ToInterface::Channel(channel)
        }
        KIND_PAGE => {
            let channel = fields.u32()?;
            let page = fields.u16()?;
            page_index(page).ok_or(ProtocolError::BadValue("page number"))?;
            ToInterface::Page { channel, page, title: fields.text(MAX_PAGE_TITLE)? }
        }
        KIND_UPDATED => {
            let channel = fields.u32()?;
            let page = fields.u16()?;
            page_index(page).ok_or(ProtocolError::BadValue("page number"))?;
            ToInterface::Updated { channel, page }
        }
        KIND_FRAME => {
            if payload.len() < FRAME_FIXED_BYTES {
                return Err(ProtocolError::BadLength { kind, length: payload.len() });
            }
            let key = fields.key()?;
            let version = fields.u32()?;
            let png = fields.take(fields.bytes.len())?.to_vec();
            ToInterface::Frame { key, version, png }
        }
        KIND_AUDIO => ToInterface::Audio { channel: fields.u32()?, audio: audio::read(&mut fields, audio::Sender::Core)? },
        #[cfg(feature = "links")]
        kind if links::is_message(kind) => ToInterface::Link(links::read_message(kind, &mut fields)?),
        #[cfg(feature = "interaction")]
        KIND_TYPING => ToInterface::Typing(interaction::read_typing(&mut fields)?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE => ToInterface::Phone(keyboard::read_view(&mut fields)?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE_KEY => ToInterface::PhoneKey(keyboard::read_phone_key(&mut fields)?),
        KIND_CLOSE => ToInterface::Close,
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// Writes a message to the trusted core.
///
/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_to_core(out: &mut dyn Write, message: &ToCore) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        ToCore::Visible(keys) => {
            push_keys(&mut payload, keys)?;
            KIND_VISIBLE
        }
        ToCore::Cell { key, version, column, row } => {
            push_key(&mut payload, *key);
            payload.extend_from_slice(&version.to_be_bytes());
            payload.extend_from_slice(&[*column, *row]);
            KIND_CELL
        }
        ToCore::SelectTab(index) => {
            payload.push(*index);
            KIND_SELECT_TAB
        }
        ToCore::CloseTab(index) => {
            payload.push(*index);
            KIND_CLOSE_TAB
        }
        ToCore::NewTab => KIND_NEW_TAB,
        ToCore::Opened(key) => {
            if let Some(key) = key {
                push_key(&mut payload, *key);
            }
            KIND_OPENED
        }
        ToCore::MuteTab(index) => {
            payload.push(*index);
            KIND_MUTE_TAB
        }
        ToCore::MuteAll(muted) => {
            payload.push(u8::from(*muted));
            KIND_MUTE_ALL
        }
        ToCore::Navigate(address) => {
            push_text(&mut payload, address, MAX_ADDRESS)?;
            KIND_NAVIGATE
        }
        ToCore::Back => KIND_BACK,
        ToCore::Forward => KIND_FORWARD,
        ToCore::Bookmark => KIND_BOOKMARK,
        #[cfg(feature = "links")]
        ToCore::Point(spot) => {
            links::push_spot(&mut payload, spot.as_ref());
            KIND_POINT
        }
        #[cfg(feature = "links")]
        ToCore::Typed { number, opened } => {
            page_index(*number).ok_or(ProtocolError::BadValue("page number"))?;
            payload.extend_from_slice(&number.to_be_bytes());
            if let Some((key, version)) = opened {
                push_key(&mut payload, *key);
                payload.extend_from_slice(&version.to_be_bytes());
            }
            KIND_TYPED
        }
        #[cfg(any(feature = "links", feature = "interaction"))]
        ToCore::Focused(focused) => {
            payload.push(u8::from(*focused));
            KIND_FOCUSED
        }
        #[cfg(feature = "interaction")]
        ToCore::Key(intent) => {
            interaction::push_key(&mut payload, intent)?;
            KIND_KEY
        }
        #[cfg(feature = "phone-keyboard")]
        ToCore::PhonePanel(open) => {
            payload.push(u8::from(*open));
            KIND_PHONE_PANEL
        }
        #[cfg(feature = "phone-keyboard")]
        ToCore::PhoneFinish => KIND_PHONE_FINISH,
        #[cfg(feature = "links")]
        ToCore::Confirm(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_CONFIRM
        }
        #[cfg(feature = "links")]
        ToCore::Cancel(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_CANCEL
        }
    };
    write_frame(out, kind, &payload)
}

/// Reads one intent from the interface, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind the interface may not send, and any payload
/// that is malformed or out of bounds.
pub fn read_from_interface(input: &mut dyn Read) -> Result<Option<ToCore>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields { bytes: &payload };
    let message = match kind {
        KIND_VISIBLE => ToCore::Visible(fields.keys()?),
        KIND_CELL => {
            if payload.len() != CELL_BYTES {
                return Err(ProtocolError::BadLength { kind, length: payload.len() });
            }
            let key = fields.key()?;
            let version = fields.u32()?;
            let (column, row) = (fields.u8()?, fields.u8()?);
            if u16::from(column) >= COLUMNS || u16::from(row) >= ROWS {
                return Err(ProtocolError::BadValue("cell outside the page"));
            }
            ToCore::Cell { key, version, column, row }
        }
        KIND_SELECT_TAB => ToCore::SelectTab(fields.u8()?),
        KIND_CLOSE_TAB => ToCore::CloseTab(fields.u8()?),
        KIND_NEW_TAB => ToCore::NewTab,
        KIND_OPENED => ToCore::Opened(if payload.is_empty() { None } else { Some(fields.key()?) }),
        KIND_MUTE_TAB => ToCore::MuteTab(fields.u8()?),
        KIND_MUTE_ALL => ToCore::MuteAll(match fields.u8()? {
            0 => false,
            1 => true,
            _ => return Err(ProtocolError::BadValue("mute")),
        }),
        KIND_NAVIGATE => ToCore::Navigate(fields.text(MAX_ADDRESS)?),
        KIND_BACK => ToCore::Back,
        KIND_FORWARD => ToCore::Forward,
        KIND_BOOKMARK => ToCore::Bookmark,
        #[cfg(feature = "links")]
        KIND_POINT => ToCore::Point(if payload.is_empty() { None } else { Some(links::read_spot(&mut fields)?) }),
        #[cfg(feature = "links")]
        KIND_TYPED => {
            let number = fields.u16()?;
            page_index(number).ok_or(ProtocolError::BadValue("page number"))?;
            let opened = if fields.bytes.is_empty() { None } else { Some((fields.key()?, fields.u32()?)) };
            ToCore::Typed { number, opened }
        }
        #[cfg(feature = "interaction")]
        KIND_KEY => ToCore::Key(interaction::read_key(&mut fields)?),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE_PANEL => ToCore::PhonePanel(match fields.u8()? {
            0 => false,
            1 => true,
            _ => return Err(ProtocolError::BadValue("phone panel")),
        }),
        #[cfg(feature = "phone-keyboard")]
        KIND_PHONE_FINISH => ToCore::PhoneFinish,
        #[cfg(any(feature = "links", feature = "interaction"))]
        KIND_FOCUSED => ToCore::Focused(match fields.u8()? {
            0 => false,
            1 => true,
            _ => return Err(ProtocolError::BadValue("focus")),
        }),
        #[cfg(feature = "links")]
        KIND_CONFIRM => ToCore::Confirm(fields.u32()?),
        #[cfg(feature = "links")]
        KIND_CANCEL => ToCore::Cancel(fields.u32()?),
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: PageKey = PageKey { channel: 7, page: 0x123 };

    fn strip() -> StripState {
        StripState {
            address: "https://example.invalid/".into(),
            identity: "example.invalid".into(),
            encrypted: true,
            back: true,
            forward: false,
            bookmarked: true,
            local: false,
        }
    }

    fn channel() -> Channel {
        let mut channel = Channel::empty();
        channel.channel = 9;
        channel.title = "Example".into();
        channel.banks[0] = "News".into();
        channel.pages[0] = Some(PageInfo { title: "Front page".into(), status: Status::Loaded });
        channel.pages[799] = Some(PageInfo { title: "Clock".into(), status: Status::Available });
        channel
    }

    #[test]
    fn every_kind_round_trips() {
        for message in [
            ToInterface::Tabs {
                tabs: vec![
                    TabInfo { title: "news".into(), colour: 1, channel: 3, muted: false },
                    TabInfo { title: "sport".into(), colour: 7, channel: 9, muted: true },
                ],
                active: 1,
            },
            ToInterface::Strip(strip()),
            ToInterface::Channel(channel()),
            ToInterface::Page { channel: 9, page: 0x899, title: "Clock".into() },
            ToInterface::Updated { channel: 9, page: 0x302 },
            ToInterface::Frame { key: KEY, version: 3, png: vec![1, 2, 3] },
            ToInterface::Close,
        ] {
            let mut bytes = Vec::new();
            write_to_interface(&mut bytes, &message).unwrap();
            assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
        for message in [
            ToCore::Visible(vec![KEY]),
            ToCore::Cell { key: KEY, version: 7, column: 39, row: 24 },
            ToCore::SelectTab(3),
            ToCore::CloseTab(0),
            ToCore::NewTab,
            ToCore::Navigate("sport.invalid".into()),
            ToCore::Back,
            ToCore::Forward,
            ToCore::Bookmark,
        ] {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &message).unwrap();
            assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
    }

    #[test]
    fn neither_side_accepts_the_other_sides_kinds() {
        // An interface sending the trusted strip's state to the trusted core.
        let mut bytes = Vec::new();
        write_to_interface(&mut bytes, &ToInterface::Strip(strip())).unwrap();
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_STRIP))));
        bytes.clear();
        write_to_core(&mut bytes, &ToCore::Navigate("x".into())).unwrap();
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_NAVIGATE))));
    }

    #[test]
    fn oversized_and_malformed_payloads_are_refused() {
        let mut bytes = vec![KIND_FRAME];
        bytes.extend_from_slice(&u32::try_from(MAX_PAYLOAD + 1).unwrap().to_be_bytes());
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::TooLong(_))));
        // A cell outside the 40 by 25 page.
        let mut bytes = vec![KIND_CELL, 0, 0, 0, 12, 0, 0, 0, 7, 0x01, 0x23, 0, 0, 0, 1, 40, 0];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("cell outside the page"))));
        // A page count larger than the payload holds.
        bytes = vec![KIND_VISIBLE, 0, 0, 0, 2, 0, 1];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("payload ends early"))));
        // Trailing bytes.
        bytes = vec![KIND_VISIBLE, 0, 0, 0, 3, 0, 0, 9];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("payload longer than its fields"))));
        // A page number outside 100 to 8FF.
        bytes = vec![KIND_VISIBLE, 0, 0, 0, 8, 0, 1, 0, 0, 0, 1, 0x09, 0x00];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
        // A new page with a hexadecimal number, and unknown strip flags.
        bytes = vec![KIND_PAGE, 0, 0, 0, 8, 0, 0, 0, 9, 0x1A, 0x00, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
        bytes = vec![KIND_STRIP, 0, 0, 0, 5, 0x20, 0, 0, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("strip flags"))));
        // A tab colour the interface has no colour for.
        bytes = vec![KIND_TABS, 0, 0, 0, 10, 0, 1, 8, 0, 0, 0, 1, 0, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("tab colour"))));
        // Unknown tab flags, and a mute that is neither on nor off.
        bytes = vec![KIND_TABS, 0, 0, 0, 10, 0, 1, 0, 0, 0, 0, 1, 2, 0, 0];
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("tab flags"))));
        bytes = vec![KIND_MUTE_ALL, 0, 0, 0, 1, 2];
        assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::BadValue("mute"))));
    }

    /// With hyperlinks compiled out, no side accepts their kinds.
    #[cfg(not(feature = "links"))]
    #[test]
    fn with_links_compiled_out_their_kinds_are_refused() {
        for kind in 11..=15 {
            let bytes = [kind, 0, 0, 0, 0];
            assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(refused)) if refused == kind));
        }
        #[cfg(not(feature = "interaction"))]
        for kind in [26, 27] {
            let bytes = [kind, 0, 0, 0, 0];
            assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(refused)) if refused == kind));
        }
        for kind in [24, 25, 36, 37] {
            let bytes = [kind, 0, 0, 0, 0];
            assert!(matches!(read_from_interface(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(refused)) if refused == kind));
        }
        let bytes = [68, 0, 0, 0, 0];
        assert!(matches!(decoder::read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(68))));
        let bytes = [83, 0, 0, 0, 0];
        assert!(matches!(fetcher::read_from_fetcher(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(83))));
    }

    #[test]
    fn a_page_listed_twice_is_refused() {
        let mut bytes = Vec::new();
        write_to_interface(&mut bytes, &ToInterface::Channel(channel())).unwrap();
        // Make the second entry's page number (899) equal the first's (100).
        let second = bytes.windows(2).rposition(|pair| pair == [0x08, 0x99]).unwrap();
        bytes[second] = 0x01;
        bytes[second + 1] = 0x00;
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page listed twice"))));
    }

    #[test]
    fn text_is_cleaned_before_sending_and_checked_on_arrival() {
        let hostile = "exa\u{202E}moc.\u{200B}elpmaxe\u{0007}";
        assert_eq!(clean(hostile, MAX_IDENTITY), "examoc.elpmaxe");
        assert_eq!(clean("ééé", 5), "éé", "cut on a character boundary");
        let mut bytes = Vec::new();
        let strip = ToInterface::Strip(StripState { identity: hostile.into(), ..StripState::default() });
        assert!(matches!(write_to_interface(&mut bytes, &strip), Err(ProtocolError::BadValue("text not cleaned"))));
        // Written by hand, as a faulty sender would, it is still refused.
        let identity = hostile.as_bytes();
        let mut payload = vec![0];
        payload.extend_from_slice(&u16::try_from(identity.len()).unwrap().to_be_bytes());
        payload.extend_from_slice(identity);
        payload.extend_from_slice(&[0, 0]);
        let mut bytes = vec![KIND_STRIP];
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&payload);
        assert!(matches!(read_from_core(&mut bytes.as_slice()), Err(ProtocolError::BadValue("text not cleaned"))));
    }
}
