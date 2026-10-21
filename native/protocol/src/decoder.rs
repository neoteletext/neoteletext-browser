//! Messages between the trusted core and a decoder process: the same
//! framing, bounds and strictness as the interface's messages, with kinds
//! of their own. The trusted core sends a channel's bytes as they are
//! fetched, never parsing them; the decoder sends back which pages have
//! arrived and the rendered pages it is asked for (design section 6).

use std::io::{Read, Write};

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};
use interface_core::channel::{PAGES, page_index};

use super::{Fields, MAX_PAGE_TITLE, ProtocolError, push_raw_text, push_text, read_frame, write_frame};

/// The most bytes of a channel's data in one message.
pub const MAX_CHUNK: usize = 64 * 1024;
/// The most bytes of one whole file or message: the largest payload less
/// this message's own byte, above the engine's 256 KB page limit.
pub const MAX_WHOLE: usize = crate::MAX_PAYLOAD - 1;
/// The Colour Map of Presentation Levels 2.5 and 3.5: 32 entries (EN 300
/// 706 clause 12.4); every page is rendered in the engine's resolved
/// colour mode, which indexes it.
pub const MAX_COLOURS: usize = 32;
const PAGE_DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
const RGB_BYTES: usize = 3;
/// A frame's page number and its count of colours, before the palette.
const FRAME_HEADER_BYTES: usize = 2 + 1;
/// A whole page's flags, before its bytes.
const WHOLE_FLAGS_BYTES: usize = 1;

// Trusted core to decoder.
const KIND_DATA: u8 = 48;
const KIND_END: u8 = 49;
const KIND_SHOWING: u8 = 50;
// 51 was the trusted core's tick, removed on 21 October 2026 (decision
// 47): the decoder keeps its own clock. The number is not reused.
const KIND_CLOSE: u8 = 52;
const KIND_OPENED: u8 = 53;
const KIND_WHOLE: u8 = 54;
// Decoder to trusted core.
const KIND_ARRIVED: u8 = 64;
const KIND_FRAME: u8 = 65;
const KIND_UPDATED: u8 = 66;
const KIND_AUDIO: u8 = 67;
#[cfg(feature = "links")]
const KIND_LINKS: u8 = 68;
#[cfg(feature = "interaction")]
const KIND_CURSOR: u8 = 69;
const KIND_STORE_FULL: u8 = 70;
#[cfg(feature = "interaction")]
const KIND_INPUT_CONTROL: u8 = 71;

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
}

/// From a decoder to the trusted core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromDecoder {
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

fn display_page(fields: &mut Fields<'_>) -> Result<u16, ProtocolError> {
    let page = fields.u16()?;
    page_index(page).ok_or(ProtocolError::BadValue("page number"))?;
    Ok(page)
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
            if pages.len() > PAGES {
                return Err(ProtocolError::BadValue("too many pages"));
            }
            payload.extend_from_slice(&u16::try_from(pages.len()).map_err(|_| ProtocolError::BadValue("too many pages"))?.to_be_bytes());
            for page in pages {
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
    let mut fields = Fields { bytes: &payload };
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
            let count = usize::from(fields.u16()?);
            if count > PAGES {
                return Err(ProtocolError::BadValue("too many pages"));
            }
            ToDecoder::Showing((0..count).map(|_| display_page(&mut fields)).collect::<Result<_, _>>()?)
        }
        KIND_CLOSE => ToDecoder::Close,
        KIND_OPENED => ToDecoder::Opened(if payload.is_empty() { None } else { Some(display_page(&mut fields)?) }),
        KIND_WHOLE => {
            let flags = fields.u8()?;
            if flags > 3 {
                return Err(ProtocolError::BadValue("whole form"));
            }
            let length = fields.bytes.len();
            if length > MAX_WHOLE {
                return Err(ProtocolError::TooLong(length));
            }
            return Ok(Some(ToDecoder::Whole { packets: flags & 1 != 0, pushed: flags & 2 != 0, bytes: crate::tail(payload, WHOLE_FLAGS_BYTES) }));
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// Writes a page's frame, [`FromDecoder::Frame`], from the decoder's own
/// palette and indices: a frame is 240 KB, and this way it is not copied
/// into a message first.
///
/// # Errors
/// When writing fails or the frame is not a page's shape.
pub fn write_frame_to_core(out: &mut dyn Write, page: u16, palette: &[[u8; 3]], indices: &[u8]) -> Result<(), ProtocolError> {
    if palette.is_empty() || palette.len() > MAX_COLOURS || indices.len() != PAGE_DOTS {
        return Err(ProtocolError::BadValue("frame shape"));
    }
    let mut payload = Vec::with_capacity(FRAME_HEADER_BYTES.saturating_add(palette.len().saturating_mul(RGB_BYTES)).saturating_add(PAGE_DOTS));
    payload.extend_from_slice(&page.to_be_bytes());
    payload.push(u8::try_from(palette.len()).unwrap_or(0));
    for colour in palette {
        payload.extend_from_slice(colour);
    }
    payload.extend_from_slice(indices);
    write_frame(out, KIND_FRAME, &payload)
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
        FromDecoder::Frame { page, palette, indices } => return write_frame_to_core(out, *page, palette, indices),
        FromDecoder::Updated { page } => {
            payload.extend_from_slice(&page.to_be_bytes());
            KIND_UPDATED
        }
        FromDecoder::Audio(audio) => {
            crate::audio::push(&mut payload, audio, crate::audio::Sender::Decoder)?;
            KIND_AUDIO
        }
        #[cfg(feature = "links")]
        FromDecoder::Links { page, with_image, candidates, numbers } => {
            payload.extend_from_slice(&page.to_be_bytes());
            payload.push(u8::from(*with_image));
            crate::links::push_found(&mut payload, candidates, numbers)?;
            KIND_LINKS
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
        FromDecoder::StoreFull => KIND_STORE_FULL,
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
    let mut fields = Fields { bytes: &payload };
    let message = match kind {
        KIND_ARRIVED => {
            let page = display_page(&mut fields)?;
            let length = usize::from(fields.u16()?);
            if length > MAX_PAGE_TITLE {
                return Err(ProtocolError::BadValue("title too long"));
            }
            // Invalid UTF-8 is replaced, never trusted; the trusted core
            // cleans what remains.
            let title = String::from_utf8_lossy(fields.take(length)?).into_owned();
            FromDecoder::Arrived { page, title }
        }
        KIND_FRAME => {
            let page = display_page(&mut fields)?;
            let colours = usize::from(fields.u8()?);
            if colours == 0 || colours > MAX_COLOURS {
                return Err(ProtocolError::BadValue("palette size"));
            }
            let palette = fields.take(colours.saturating_mul(RGB_BYTES))?.as_chunks::<RGB_BYTES>().0.to_vec();
            let indices = fields.take(PAGE_DOTS)?;
            if indices.iter().any(|&index| usize::from(index) >= colours) {
                return Err(ProtocolError::BadValue("index outside the palette"));
            }
            fields.end()?;
            // The indices are the rest of the payload: moved, not copied.
            let offset = payload.len().saturating_sub(PAGE_DOTS);
            return Ok(Some(FromDecoder::Frame { page, palette, indices: crate::tail(payload, offset) }));
        }
        KIND_UPDATED => FromDecoder::Updated { page: display_page(&mut fields)? },
        KIND_AUDIO => FromDecoder::Audio(crate::audio::read(&mut fields, crate::audio::Sender::Decoder)?),
        #[cfg(feature = "links")]
        KIND_LINKS => {
            let page = display_page(&mut fields)?;
            let with_image = match fields.u8()? {
                0 => false,
                1 => true,
                _ => return Err(ProtocolError::BadValue("with image")),
            };
            let (candidates, numbers) = crate::links::read_found(&mut fields)?;
            FromDecoder::Links { page, with_image, candidates, numbers }
        }
        // Only its length here: the trusted core checks the fields.
        #[cfg(feature = "interaction")]
        KIND_CURSOR => FromDecoder::Cursor { body: fields.array()? },
        #[cfg(feature = "interaction")]
        KIND_INPUT_CONTROL => {
            let length = fields.bytes.len();
            if length > neoteletext_input::control::MAX_BODY {
                return Err(ProtocolError::TooLong(length));
            }
            FromDecoder::InputControl { body: fields.take(length)?.to_vec() }
        }
        KIND_STORE_FULL => FromDecoder::StoreFull,
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

// Kept for symmetry with the interface's messages: text the trusted core
// sends a decoder would be cleaned the same way.
#[allow(dead_code, reason = "no message to a decoder carries text yet")]
fn _text(payload: &mut Vec<u8>, text: &str) -> Result<(), ProtocolError> {
    push_text(payload, text, MAX_PAGE_TITLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_title_is_cut_on_a_character_boundary() {
        // Two-byte 'é' straddles the limit: cutting at the byte would leave
        // half a character, which arrives as U+FFFD.
        let title = format!("{}é", "a".repeat(MAX_PAGE_TITLE - 1));
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &FromDecoder::Arrived { page: 0x100, title }).unwrap();
        let arrived = read_from_decoder(&mut bytes.as_slice()).unwrap();
        assert_eq!(arrived, Some(FromDecoder::Arrived { page: 0x100, title: "a".repeat(MAX_PAGE_TITLE - 1) }));
    }

    #[test]
    fn every_kind_round_trips() {
        for message in [
            ToDecoder::Data(vec![1, 2, 3]),
            ToDecoder::Whole { packets: true, pushed: true, bytes: vec![0; 42] },
            ToDecoder::Whole { packets: false, pushed: false, bytes: b"PN,10000\n".to_vec() },
            ToDecoder::End,
            ToDecoder::Showing(vec![0x100, 0x899]),
            ToDecoder::Close,
        ] {
            let mut bytes = Vec::new();
            write_to_decoder(&mut bytes, &message).unwrap();
            assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
        let mut indices = vec![0; PAGE_DOTS];
        indices[7] = 1;
        for message in [
            FromDecoder::Arrived { page: 0x100, title: "Headlines".into() },
            FromDecoder::Updated { page: 0x302 },
            FromDecoder::StoreFull,
            FromDecoder::Frame { page: 0x123, palette: vec![[0, 0, 0], [255, 255, 255]], indices },
        ] {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &message).unwrap();
            assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
    }

    /// REQ-INT-111: a control record's body goes to the trusted core as
    /// sent, within one fragment's body.
    #[cfg(feature = "interaction")]
    #[test]
    fn an_input_control_body_round_trips_within_its_bound() {
        let message = FromDecoder::InputControl { body: vec![0xFF, 0x01, 2, 0] };
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &message).unwrap();
        assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap().unwrap(), message);
        let long = FromDecoder::InputControl { body: vec![0; neoteletext_input::control::MAX_BODY + 1] };
        assert!(write_to_core(&mut Vec::new(), &long).is_err());
        let mut bytes = vec![KIND_INPUT_CONTROL];
        bytes.extend_from_slice(&u32::try_from(neoteletext_input::control::MAX_BODY + 1).unwrap().to_be_bytes());
        bytes.extend(std::iter::repeat_n(0, neoteletext_input::control::MAX_BODY + 1));
        assert!(read_from_decoder(&mut bytes.as_slice()).is_err());
    }

    #[test]
    fn a_decoder_cannot_send_what_it_may_not() {
        // A decoder sending the trusted core's own kind.
        let mut bytes = Vec::new();
        write_to_decoder(&mut bytes, &ToDecoder::End).unwrap();
        assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::UnknownKind(KIND_END))));
        // An index outside its palette.
        let mut payload = vec![0x01, 0x00, 1, 0, 0, 0];
        payload.extend(std::iter::repeat_n(0, PAGE_DOTS));
        payload[6] = 1;
        let mut bytes = vec![KIND_FRAME];
        bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&payload);
        assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("index outside the palette"))));
        // A hexadecimal page number.
        let mut bytes = vec![KIND_ARRIVED, 0, 0, 0, 4, 0x1A, 0x00, 0, 0];
        assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("page number"))));
        // A title longer than a row.
        bytes = vec![KIND_ARRIVED, 0, 0, 0, 4, 0x01, 0x00, 0xFF, 0xFF];
        assert!(matches!(read_from_decoder(&mut bytes.as_slice()), Err(ProtocolError::BadValue("title too long"))));
    }
}
