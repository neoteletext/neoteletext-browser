//! Messages between the trusted core and a decoder process: the same
//! framing, bounds and strictness as the interface's messages, with kinds
//! of their own. The trusted core sends a channel's bytes as they are
//! fetched, never parsing them; the decoder sends back which pages have
//! arrived and the rendered pages it is asked for (design section 6).

use std::io::{Read, Write};

use interface_core::channel::{PAGES, page_index};

use crate::frame::PAGE_DOTS;
use crate::{Fields, MAX_PAGE_TITLE, ProtocolError, push_count, push_count_u8, push_raw_text, raw_text, read_frame, write_frame};
use kinds::{
    KIND_ARRIVED, KIND_AUDIO, KIND_BOOKMARK_FOLDER, KIND_BOOKMARK_FOLDER_END, KIND_BOOKMARK_LINK, KIND_BOOKMARKS_READ, KIND_CHANNEL_LOG, KIND_CLOSE,
    KIND_CONFINED, KIND_DATA, KIND_END, KIND_FRAME, KIND_OPENED, KIND_SHOWING, KIND_STORE_FULL, KIND_SUBPAGE_TITLES, KIND_SUBPAGES, KIND_UPDATED, KIND_VIEW,
    KIND_WHOLE,
};
#[cfg(feature = "interaction")]
use kinds::{KIND_CURSOR, KIND_INPUT_CONTROL};
#[cfg(feature = "links")]
use kinds::{KIND_LINKS, KIND_LOOK};

mod bookmarks;
mod frame;
mod kinds;
#[cfg(test)]
mod tests;
mod view;

pub use bookmarks::{BookmarkEntry, MAX_BOOKMARK_ENTRIES, MAX_BOOKMARK_FILE, MAX_BOOKMARK_NAME, NETSCAPE_DOCTYPE, is_netscape_bookmarks};
// The Colour Map of Presentation Levels 2.5 and 3.5 (EN 300 706 clause
// 12.4): every page is rendered in the engine's resolved colour mode,
// which indexes it.
pub use crate::frame::MAX_COLOURS;
pub use frame::write_frame_to_core;
pub use view::View;

/// The most bytes of a channel's data in one message.
pub const MAX_CHUNK: usize = 64 * 1024;
/// The most bytes of one whole file or message: the largest payload less
/// this message's own byte, above the engine's 256 KB page limit.
pub const MAX_WHOLE: usize = crate::MAX_PAYLOAD - 1;
/// A whole page's flags, before its bytes.
const WHOLE_FLAGS_BYTES: usize = 1;

/// The longest text of a channel's log line: the engine's
/// `neoteletext_formats::log::MAX_LOG_TEXT_BYTES`, which the decoder
/// asserts this equals.
pub const MAX_CHANNEL_LOG: usize = 200;
/// A log line's page when it followed none.
const NO_PAGE: u16 = 0;

/// The pages and sub-pages a decoder first has room for: about 0.7 MB of
/// store, enough for most channels; a decoder that fills it is started
/// again with twice the room (decision 47).
pub const FIRST_STORE_PAGES: usize = 256;
/// The most a decoder's store holds, as the engine's own sessions keep
/// (`neoteletext-app`'s `DEFAULT_PAGES`): 10.5 MB.
pub const MAX_STORE_PAGES: usize = 4096;

/// From the trusted core to a decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToDecoder {
    /// More of the channel's bytes, in the order they were fetched.
    Data(Vec<u8>),
    /// The channel's data is complete.
    End,
    /// The pages to render: those on screen. The decoder draws them again
    /// when they change and, by its own clock, when they flash.
    Showing(Vec<u16>),
    /// The channel is closed: the decoder ends.
    Close,
    /// The page the user opened, or none: its sounds play while it is open.
    Opened(Option<u16>),
    /// A whole page file, bundle or pushed message: a TTI page, or whole
    /// 42-byte packets. Its pages are complete as it ends, so they show at
    /// once (`interaction.md` section 10); a fetched copy of a page the
    /// channel has pushed is older and is dropped.
    Whole {
        /// Packets, not a TTI page.
        packets: bool,
        /// Pushed by the channel, not fetched.
        pushed: bool,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// How a page is drawn: its reveal, hold, mix and sub-page. A page with
    /// no view is drawn as it arrives.
    View(View),
    /// The look of one area of a page under the pointer, hovered or
    /// pressed, or none (INPUT.md section 9.7, the engine's decision
    /// E.127): drawn on the page's own cells.
    #[cfg(feature = "links")]
    Look {
        /// The page.
        page: u16,
        /// The area and whether it is pressed.
        look: Option<(crate::links::Area, bool)>,
    },
}

/// From a decoder to the trusted core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromDecoder {
    /// It is confined and running: a channel's decoder says this first,
    /// before it reads anything, so the trusted core can tell one that
    /// could not start from one a channel's data ended (finding F-12).
    Confined,
    /// A page has been received completely for the first time, with a
    /// title taken from its text, not yet cleaned.
    Arrived {
        /// The page number.
        page: u16,
        /// Its title.
        title: String,
    },
    /// A rendered page: palette indices, 480 by 500, into `palette`.
    Frame {
        /// The page number.
        page: u16,
        /// Red, green and blue of each colour the page uses.
        palette: Vec<[u8; 3]>,
        /// The palette index of the screen around the page (EN 300 706
        /// Table 4, Default Screen Colour; clause 12.3, Full Screen Colour).
        screen: u8,
        /// One palette index per dot, row by row.
        indices: Vec<u8>,
    },
    /// A page already reported has changed: a new version arrived.
    Updated {
        /// The page number.
        page: u16,
    },
    /// The page store is full: the channel has more pages and sub-pages
    /// than it has room for, and the oldest are being forgotten.
    StoreFull,
    /// Something found in a bookmark file the decoder was given to read
    /// (decision 71).
    Bookmark(BookmarkEntry),
    /// A log line the channel wrote for its author (the engine's
    /// REQ-INP-026, an `LG` line of a TTI page): untrusted, for the
    /// console's Log tab only (decision 70).
    ChannelLog {
        /// Its level.
        level: interface_core::console::Level,
        /// The page it was written on, when it followed a page's number.
        page: Option<u16>,
        /// Its text, at most [`MAX_CHANNEL_LOG`] bytes.
        text: String,
    },
    /// The bookmark file is read: what was found has been sent, and this
    /// many bookmarks and folders were left out.
    BookmarksRead {
        /// Left out: not addresses, too deep, or past the bound.
        skipped: u32,
    },
    /// A voice operation of the channel's audio receiver.
    Audio(crate::audio::Audio),
    /// The links found in a page, sent before its image whenever they
    /// change (`hyperlinks.md` section 8.1): untrusted, for the trusted
    /// core to check and decide.
    #[cfg(feature = "links")]
    Links {
        /// The page number.
        page: u16,
        /// A new image of the page follows: its links are for that image.
        /// Otherwise only the links changed, and they are for the image the
        /// trusted core already has.
        with_image: bool,
        /// Page numbers in its text and the runs its `LT` lines declare.
        candidates: Vec<crate::links::Candidate>,
        /// Its `LK` lines.
        numbers: Vec<crate::links::NumberLink>,
        /// Its controls (INPUT.md section 7.3), as the engine found them.
        controls: Vec<crate::links::Area>,
        /// Its coloured keys and Index, from packet X/27/0.
        fastext: crate::links::Fastext,
    },
    /// The sub-pages of a page on screen, in ascending order, and the one
    /// drawn, whenever they change (REQ-NAV-005).
    SubPages {
        /// The page number.
        page: u16,
        /// Their sub-codes, at most `interface_core::controls::MAX_SUBPAGES`.
        subcodes: Vec<u16>,
        /// The sub-code of the one drawn.
        shown: u16,
    },
    /// The sub-pages of any page that has more than one, each with a title
    /// from its text, not yet cleaned, whenever they change: for the page
    /// list's sub-pages. None means it has one sub-page again.
    SubPageTitles {
        /// The page number.
        page: u16,
        /// Each sub-page's sub-code and title, in ascending order, at most
        /// `interface_core::controls::MAX_SUBPAGES`.
        titles: Vec<(u16, String)>,
    },
    /// A cursor record the channel sent for a page, from a pushed packet or
    /// a page's `CU` line: untrusted, its body as sent, for the trusted
    /// core to check (INPUT.md section 7.1).
    #[cfg(feature = "interaction")]
    Cursor {
        /// The record's body: page, acknowledgement, row, column, width
        /// and flags.
        body: [u8; neoteletext_input::cursor::BODY],
    },
    /// An input control record the channel sent (WELCOME, `PUSH_MODE`,
    /// `POP_MODE`, `PAGE_MODES`), or a page's `IM` line as `PAGE_MODES`:
    /// untrusted, its body as sent, at most
    /// `neoteletext_input::control::MAX_BODY` bytes, for the trusted core
    /// to read (REQ-INT-111, REQ-INT-114).
    #[cfg(feature = "interaction")]
    InputControl {
        /// The control message's body.
        body: Vec<u8>,
    },
}

/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_to_decoder(out: &mut dyn Write, message: &ToDecoder) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        ToDecoder::Data(bytes) => {
            if bytes.len() > MAX_CHUNK {
                return Err(ProtocolError::TooLong(bytes.len()));
            }
            payload.reserve_exact(bytes.len());
            payload.extend_from_slice(bytes);
            KIND_DATA
        }
        ToDecoder::End => KIND_END,
        ToDecoder::Showing(pages) => {
            push_count(&mut payload, pages.len(), PAGES, "too many pages")?;
            for page in pages {
                // Only display pages, as the decoder reads them: any other
                // is refused there, and the decoder restarts (finding F-29).
                page_index(*page).ok_or(ProtocolError::BadValue("page number"))?;
                payload.extend_from_slice(&page.to_be_bytes());
            }
            KIND_SHOWING
        }
        ToDecoder::Close => KIND_CLOSE,
        ToDecoder::Opened(page) => {
            if let Some(page) = page {
                page_index(*page).ok_or(ProtocolError::BadValue("page number"))?;
                payload.extend_from_slice(&page.to_be_bytes());
            }
            KIND_OPENED
        }
        ToDecoder::Whole { packets, pushed, bytes } => {
            if bytes.len() > MAX_WHOLE {
                return Err(ProtocolError::TooLong(bytes.len()));
            }
            payload.reserve_exact(WHOLE_FLAGS_BYTES.saturating_add(bytes.len()));
            payload.push(u8::from(*packets) | (u8::from(*pushed) << 1));
            payload.extend_from_slice(bytes);
            KIND_WHOLE
        }
        ToDecoder::View(view) => {
            view::push_view(&mut payload, view)?;
            KIND_VIEW
        }
        #[cfg(feature = "links")]
        ToDecoder::Look { page, look } => {
            page_index(*page).ok_or(ProtocolError::BadValue("page number"))?;
            payload.extend_from_slice(&page.to_be_bytes());
            if let Some((area, pressed)) = look {
                crate::links::push_area(&mut payload, *area);
                payload.push(u8::from(*pressed));
            }
            KIND_LOOK
        }
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind the trusted core may not send a decoder, and
/// any payload that is malformed or out of bounds.
pub fn read_from_core(input: &mut dyn Read) -> Result<Option<ToDecoder>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        // The payload is the data: it is moved, not copied.
        KIND_DATA => {
            if payload.len() > MAX_CHUNK {
                return Err(ProtocolError::TooLong(payload.len()));
            }
            return Ok(Some(ToDecoder::Data(payload)));
        }
        KIND_END => ToDecoder::End,
        KIND_SHOWING => {
            let count = fields.count_u16(PAGES, "too many pages")?;
            ToDecoder::Showing((0..count).map(|_| fields.display_page()).collect::<Result<_, _>>()?)
        }
        KIND_CLOSE => ToDecoder::Close,
        KIND_OPENED => ToDecoder::Opened(if payload.is_empty() { None } else { Some(fields.display_page()?) }),
        KIND_WHOLE => {
            let flags = fields.u8()?;
            if flags > 3 {
                return Err(ProtocolError::BadValue("whole form"));
            }
            let length = fields.remaining();
            if length > MAX_WHOLE {
                return Err(ProtocolError::TooLong(length));
            }
            return Ok(Some(ToDecoder::Whole { packets: flags & 1 != 0, pushed: flags & 2 != 0, bytes: crate::tail(payload, WHOLE_FLAGS_BYTES) }));
        }
        KIND_VIEW => ToDecoder::View(view::read_view(&mut fields)?),
        #[cfg(feature = "links")]
        KIND_LOOK => {
            let page = fields.display_page()?;
            let look = if fields.is_empty() { None } else { Some((crate::links::read_area(&mut fields)?, fields.flag("pressed")?)) };
            ToDecoder::Look { page, look }
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_to_core(out: &mut dyn Write, message: &FromDecoder) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        FromDecoder::Arrived { page, title } => {
            payload.extend_from_slice(&page.to_be_bytes());
            // The decoder's text is untrusted; it is checked for length
            // here and cleaned by the trusted core, not trusted to be clean.
            push_raw_text(&mut payload, title, MAX_PAGE_TITLE);
            KIND_ARRIVED
        }
        FromDecoder::Frame { page, palette, screen, indices } => return write_frame_to_core(out, *page, palette, *screen, indices),
        FromDecoder::Updated { page } => {
            payload.extend_from_slice(&page.to_be_bytes());
            KIND_UPDATED
        }
        FromDecoder::Audio(audio) => {
            crate::audio::push(&mut payload, audio, crate::audio::Sender::Decoder)?;
            KIND_AUDIO
        }
        #[cfg(feature = "links")]
        FromDecoder::Links { page, with_image, candidates, numbers, controls, fastext } => {
            payload.extend_from_slice(&page.to_be_bytes());
            payload.push(u8::from(*with_image));
            crate::links::push_found(&mut payload, candidates, numbers, controls)?;
            crate::links::push_fastext(&mut payload, fastext)?;
            KIND_LINKS
        }
        FromDecoder::SubPageTitles { page, titles } => {
            if titles.iter().any(|(subcode, _)| *subcode > interface_core::controls::ANY_SUBCODE) {
                return Err(ProtocolError::BadValue("sub-pages"));
            }
            payload.extend_from_slice(&page.to_be_bytes());
            push_count_u8(&mut payload, titles.len(), interface_core::controls::MAX_SUBPAGES, "sub-pages")?;
            for (subcode, title) in titles {
                payload.extend_from_slice(&subcode.to_be_bytes());
                push_raw_text(&mut payload, title, MAX_PAGE_TITLE);
            }
            KIND_SUBPAGE_TITLES
        }
        FromDecoder::SubPages { page, subcodes, shown } => {
            if subcodes.len() > interface_core::controls::MAX_SUBPAGES
                || subcodes.iter().chain([shown]).any(|subcode| *subcode > interface_core::controls::ANY_SUBCODE)
            {
                return Err(ProtocolError::BadValue("sub-pages"));
            }
            payload.extend_from_slice(&page.to_be_bytes());
            push_count_u8(&mut payload, subcodes.len(), interface_core::controls::MAX_SUBPAGES, "sub-pages")?;
            for subcode in subcodes {
                payload.extend_from_slice(&subcode.to_be_bytes());
            }
            payload.extend_from_slice(&shown.to_be_bytes());
            KIND_SUBPAGES
        }
        #[cfg(feature = "interaction")]
        FromDecoder::Cursor { body } => {
            payload.extend_from_slice(body);
            KIND_CURSOR
        }
        #[cfg(feature = "interaction")]
        FromDecoder::InputControl { body } => {
            if body.len() > neoteletext_input::control::MAX_BODY {
                return Err(ProtocolError::TooLong(body.len()));
            }
            payload.extend_from_slice(body);
            KIND_INPUT_CONTROL
        }
        FromDecoder::Confined => KIND_CONFINED,
        FromDecoder::StoreFull => KIND_STORE_FULL,
        FromDecoder::Bookmark(entry) => bookmarks::push_entry(&mut payload, entry),
        FromDecoder::BookmarksRead { skipped } => {
            payload.extend_from_slice(&skipped.to_be_bytes());
            KIND_BOOKMARKS_READ
        }
        FromDecoder::ChannelLog { level, page, text } => {
            payload.push(crate::console::level_byte(*level));
            payload.extend_from_slice(&page.unwrap_or(NO_PAGE).to_be_bytes());
            push_raw_text(&mut payload, text, MAX_CHANNEL_LOG);
            KIND_CHANNEL_LOG
        }
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from a decoder, or `None` at a clean end. Everything
/// a decoder sends is untrusted: every bound and every index is checked.
///
/// # Errors
/// On input errors, a kind a decoder may not send, and any payload that is
/// malformed or out of bounds.
pub fn read_from_decoder(input: &mut dyn Read) -> Result<Option<FromDecoder>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_ARRIVED => {
            let page = fields.display_page()?;
            FromDecoder::Arrived { page, title: raw_text(&mut fields, MAX_PAGE_TITLE, "title too long")? }
        }
        KIND_FRAME => {
            let (page, palette, screen) = frame::read_frame(&mut fields)?;
            // The indices are the rest of the payload: moved, not copied.
            let offset = payload.len().saturating_sub(PAGE_DOTS);
            return Ok(Some(FromDecoder::Frame { page, palette, screen, indices: crate::tail(payload, offset) }));
        }
        KIND_UPDATED => FromDecoder::Updated { page: fields.display_page()? },
        KIND_AUDIO => FromDecoder::Audio(crate::audio::read(&mut fields, crate::audio::Sender::Decoder)?),
        #[cfg(feature = "links")]
        KIND_LINKS => {
            let page = fields.display_page()?;
            let with_image = fields.flag("with image")?;
            let (candidates, numbers, controls) = crate::links::read_found(&mut fields)?;
            let fastext = crate::links::read_fastext(&mut fields)?;
            FromDecoder::Links { page, with_image, candidates, numbers, controls, fastext }
        }
        KIND_SUBPAGE_TITLES => {
            let page = fields.display_page()?;
            let count = fields.count_u8(interface_core::controls::MAX_SUBPAGES, "too many sub-pages")?;
            let mut titles = Vec::with_capacity(count);
            for _ in 0..count {
                let subcode = fields.u16()?;
                if subcode > interface_core::controls::ANY_SUBCODE {
                    return Err(ProtocolError::BadValue("sub-code"));
                }
                titles.push((subcode, raw_text(&mut fields, MAX_PAGE_TITLE, "title too long")?));
            }
            FromDecoder::SubPageTitles { page, titles }
        }
        KIND_SUBPAGES => {
            let page = fields.display_page()?;
            let count = fields.count_u8(interface_core::controls::MAX_SUBPAGES, "too many sub-pages")?;
            let mut subcode = || -> Result<u16, ProtocolError> {
                let subcode = fields.u16()?;
                // A page may carry 3F7F itself, as many broadcast pages do.
                if subcode > interface_core::controls::ANY_SUBCODE { Err(ProtocolError::BadValue("sub-code")) } else { Ok(subcode) }
            };
            let subcodes = (0..count).map(|_| subcode()).collect::<Result<Vec<_>, _>>()?;
            let shown = subcode()?;
            FromDecoder::SubPages { page, subcodes, shown }
        }
        // Only its length here: the trusted core checks the fields.
        #[cfg(feature = "interaction")]
        KIND_CURSOR => FromDecoder::Cursor { body: fields.array()? },
        #[cfg(feature = "interaction")]
        KIND_INPUT_CONTROL => {
            let length = fields.remaining();
            if length > neoteletext_input::control::MAX_BODY {
                return Err(ProtocolError::TooLong(length));
            }
            FromDecoder::InputControl { body: fields.take(length)?.to_vec() }
        }
        KIND_CONFINED => FromDecoder::Confined,
        KIND_STORE_FULL => FromDecoder::StoreFull,
        KIND_BOOKMARK_FOLDER | KIND_BOOKMARK_FOLDER_END | KIND_BOOKMARK_LINK => FromDecoder::Bookmark(bookmarks::read_entry(kind, &mut fields)?),
        KIND_BOOKMARKS_READ => FromDecoder::BookmarksRead { skipped: fields.u32()? },
        KIND_CHANNEL_LOG => {
            let level = crate::console::level_from_byte(fields.u8()?)?;
            let page = match fields.u16()? {
                NO_PAGE => None,
                page => Some(page_index(page).map(|_| page).ok_or(ProtocolError::BadValue("page number"))?),
            };
            FromDecoder::ChannelLog { level, page, text: raw_text(&mut fields, MAX_CHANNEL_LOG, "log line too long")? }
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}
