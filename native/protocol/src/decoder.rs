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
/// The Colour Map of Presentation Levels 2.5 and 3.5: 32 entries (EN 300
/// 706 clause 12.4); every page is rendered in the engine's resolved
/// colour mode, which indexes it.
pub const MAX_COLOURS: usize = 32;
const PAGE_DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
const RGB_BYTES: usize = 3;

// Trusted core to decoder.
const KIND_DATA: u8 = 48;
const KIND_END: u8 = 49;
const KIND_SHOWING: u8 = 50;
const KIND_TICK: u8 = 51;
const KIND_CLOSE: u8 = 52;
const KIND_OPENED: u8 = 53;
const KIND_AUDIBLE: u8 = 54;
// Decoder to trusted core.
const KIND_ARRIVED: u8 = 64;
const KIND_FRAME: u8 = 65;
const KIND_UPDATED: u8 = 66;
const KIND_AUDIO: u8 = 67;

/// From the trusted core to a decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToDecoder {
    /// More of the channel's bytes, in the order they were fetched.
    Data(Vec<u8>),
    /// The channel's data is complete.
    End,
    /// The pages to render now and at every tick: those on screen.
    Showing(Vec<u16>),
    /// Time has passed: render the shown pages again for flash.
    Tick,
    /// The channel is closed: the decoder ends.
    Close,
    /// The page the user opened, or none: its sounds play while it is open.
    Opened(Option<u16>),
    /// Whether the channel is the one heard. Becoming heard, the decoder
    /// sends its sounds again: the interface forgot them.
    Audible(bool),
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
    /// A voice operation of the channel's audio receiver.
    Audio(crate::audio::Audio),
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
        ToDecoder::Tick => KIND_TICK,
        ToDecoder::Close => KIND_CLOSE,
        ToDecoder::Opened(page) => {
            if let Some(page) = page {
                page_index(*page).ok_or(ProtocolError::BadValue("page number"))?;
                payload.extend_from_slice(&page.to_be_bytes());
            }
            KIND_OPENED
        }
        ToDecoder::Audible(heard) => {
            payload.push(u8::from(*heard));
            KIND_AUDIBLE
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
        KIND_DATA => {
            if payload.len() > MAX_CHUNK {
                return Err(ProtocolError::TooLong(payload.len()));
            }
            ToDecoder::Data(fields.take(payload.len())?.to_vec())
        }
        KIND_END => ToDecoder::End,
        KIND_SHOWING => {
            let count = usize::from(fields.u16()?);
            if count > PAGES {
                return Err(ProtocolError::BadValue("too many pages"));
            }
            ToDecoder::Showing((0..count).map(|_| display_page(&mut fields)).collect::<Result<_, _>>()?)
        }
        KIND_TICK => ToDecoder::Tick,
        KIND_CLOSE => ToDecoder::Close,
        KIND_OPENED => ToDecoder::Opened(if payload.is_empty() { None } else { Some(display_page(&mut fields)?) }),
        KIND_AUDIBLE => ToDecoder::Audible(match fields.u8()? {
            0 => false,
            1 => true,
            _ => return Err(ProtocolError::BadValue("audible")),
        }),
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
        FromDecoder::Frame { page, palette, indices } => {
            if palette.is_empty() || palette.len() > MAX_COLOURS || indices.len() != PAGE_DOTS {
                return Err(ProtocolError::BadValue("frame shape"));
            }
            payload.extend_from_slice(&page.to_be_bytes());
            payload.push(u8::try_from(palette.len()).unwrap_or(0));
            for colour in palette {
                payload.extend_from_slice(colour);
            }
            payload.extend_from_slice(indices);
            KIND_FRAME
        }
        FromDecoder::Updated { page } => {
            payload.extend_from_slice(&page.to_be_bytes());
            KIND_UPDATED
        }
        FromDecoder::Audio(audio) => {
            crate::audio::push(&mut payload, audio, crate::audio::Sender::Decoder)?;
            KIND_AUDIO
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
            let indices = fields.take(PAGE_DOTS)?.to_vec();
            if indices.iter().any(|&index| usize::from(index) >= colours) {
                return Err(ProtocolError::BadValue("index outside the palette"));
            }
            FromDecoder::Frame { page, palette, indices }
        }
        KIND_UPDATED => FromDecoder::Updated { page: display_page(&mut fields)? },
        KIND_AUDIO => FromDecoder::Audio(crate::audio::read(&mut fields, crate::audio::Sender::Decoder)?),
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
        for message in [ToDecoder::Data(vec![1, 2, 3]), ToDecoder::End, ToDecoder::Showing(vec![0x100, 0x899]), ToDecoder::Tick, ToDecoder::Close] {
            let mut bytes = Vec::new();
            write_to_decoder(&mut bytes, &message).unwrap();
            assert_eq!(read_from_core(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
        let mut indices = vec![0; PAGE_DOTS];
        indices[7] = 1;
        for message in [
            FromDecoder::Arrived { page: 0x100, title: "Headlines".into() },
            FromDecoder::Updated { page: 0x302 },
            FromDecoder::Frame { page: 0x123, palette: vec![[0, 0, 0], [255, 255, 255]], indices },
        ] {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &message).unwrap();
            assert_eq!(read_from_decoder(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
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
