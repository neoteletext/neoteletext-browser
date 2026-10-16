//! The framed pipe protocol, shaped like `neoteletext-app/src/pipe.rs`:
//! one kind byte, a 32-bit big-endian payload length, then the payload.
//! Lengths are bounded before anything is allocated, unknown kinds are
//! rejected, and every payload is parsed strictly to its exact length.

use std::io::{self, Read, Write};

use crate::page::{COLUMNS_DOTS, Frame, MAX_COLOURS, ROWS_DOTS, Rgb};

/// The largest payload accepted, as in `neoteletext-app` (`MAX_FRAME`).
pub const MAX_PAYLOAD: usize = 256 * 1024;

const KIND_SHOW: u8 = 1;
const KIND_TICK: u8 = 2;
const KIND_FRAME: u8 = 3;
const KIND_CLOSE: u8 = 4;

const HEADER_BYTES: usize = 5;
const SHOW_BYTES: usize = 8;
const TICK_BYTES: usize = 1;
// channel 4, page 2, flash 1, colours 1, width 2, height 2
const FRAME_FIXED_BYTES: usize = 12;
const RGB_BYTES: usize = 3;

/// Lowest and highest teletext page numbers: magazine 1 page 00 to
/// magazine 8 page FF, magazine 8 being written as 0 on air (EN 300 706
/// clause 7.1.2) and as 8 in page numbers.
pub const FIRST_PAGE: u16 = 0x100;
pub const LAST_PAGE: u16 = 0x8FF;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Trusted core to decoder: render this page of this channel.
    Show { channel: u32, page: u16, subcode: u16 },
    /// Trusted core to decoder: the flash phase changed.
    Tick { flash_on: bool },
    /// Decoder to trusted core: a rendered page.
    Frame(Frame),
    /// Either way: finish.
    Close,
}

#[derive(Debug)]
pub enum PipeError {
    Io(io::Error),
    TooLong(usize),
    UnknownKind(u8),
    BadLength { kind: u8, length: usize },
    BadValue(&'static str),
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipeError::Io(error) => write!(f, "pipe input or output failed: {error}"),
            PipeError::TooLong(length) => write!(f, "payload of {length} bytes exceeds {MAX_PAYLOAD}"),
            PipeError::UnknownKind(kind) => write!(f, "unknown message kind {kind}"),
            PipeError::BadLength { kind, length } => write!(f, "message kind {kind} cannot be {length} bytes"),
            PipeError::BadValue(what) => write!(f, "invalid value: {what}"),
        }
    }
}

impl std::error::Error for PipeError {}

impl From<io::Error> for PipeError {
    fn from(error: io::Error) -> Self {
        PipeError::Io(error)
    }
}

/// Writes one message.
///
/// # Errors
/// Fails when writing fails or the message would exceed [`MAX_PAYLOAD`].
pub fn write(out: &mut dyn Write, message: &Message) -> Result<(), PipeError> {
    let (kind, payload) = encode(message)?;
    if payload.len() > MAX_PAYLOAD {
        return Err(PipeError::TooLong(payload.len()));
    }
    let length = u32::try_from(payload.len()).map_err(|_| PipeError::TooLong(payload.len()))?;
    let mut header = [0_u8; HEADER_BYTES];
    header[0] = kind;
    header[1..].copy_from_slice(&length.to_be_bytes());
    out.write_all(&header)?;
    out.write_all(&payload)?;
    out.flush()?;
    Ok(())
}

/// Reads one message, or `None` at a clean end of input.
///
/// # Errors
/// Fails on input errors, oversized or malformed payloads, and unknown kinds.
pub fn read(input: &mut dyn Read) -> Result<Option<Message>, PipeError> {
    let mut header = [0_u8; HEADER_BYTES];
    match input.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let kind = header[0];
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if length > MAX_PAYLOAD {
        return Err(PipeError::TooLong(length));
    }
    let mut payload = vec![0_u8; length];
    input.read_exact(&mut payload)?;
    decode(kind, &payload).map(Some)
}

fn encode(message: &Message) -> Result<(u8, Vec<u8>), PipeError> {
    Ok(match message {
        Message::Show { channel, page, subcode } => {
            let mut payload = Vec::with_capacity(SHOW_BYTES);
            payload.extend_from_slice(&channel.to_be_bytes());
            payload.extend_from_slice(&page.to_be_bytes());
            payload.extend_from_slice(&subcode.to_be_bytes());
            (KIND_SHOW, payload)
        }
        Message::Tick { flash_on } => (KIND_TICK, vec![u8::from(*flash_on)]),
        Message::Close => (KIND_CLOSE, Vec::new()),
        Message::Frame(frame) => {
            let colours = u8::try_from(frame.palette.len()).map_err(|_| PipeError::BadValue("palette size"))?;
            let width = u16::try_from(frame.width).map_err(|_| PipeError::BadValue("width"))?;
            let height = u16::try_from(frame.height).map_err(|_| PipeError::BadValue("height"))?;
            let mut payload = Vec::with_capacity(FRAME_FIXED_BYTES + frame.palette.len() * RGB_BYTES + frame.indices.len());
            payload.extend_from_slice(&frame.channel.to_be_bytes());
            payload.extend_from_slice(&frame.page.to_be_bytes());
            payload.push(u8::from(frame.flash_on));
            payload.push(colours);
            payload.extend_from_slice(&width.to_be_bytes());
            payload.extend_from_slice(&height.to_be_bytes());
            for colour in &frame.palette {
                payload.extend_from_slice(&[colour.red, colour.green, colour.blue]);
            }
            payload.extend_from_slice(&frame.indices);
            (KIND_FRAME, payload)
        }
    })
}

fn decode(kind: u8, payload: &[u8]) -> Result<Message, PipeError> {
    let bad_length = || PipeError::BadLength { kind, length: payload.len() };
    let u16_at = |at: usize| u16::from_be_bytes([payload[at], payload[at + 1]]);
    match kind {
        KIND_SHOW => {
            if payload.len() != SHOW_BYTES {
                return Err(bad_length());
            }
            let channel = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let page = u16_at(4);
            if !(FIRST_PAGE..=LAST_PAGE).contains(&page) {
                return Err(PipeError::BadValue("page number"));
            }
            Ok(Message::Show { channel, page, subcode: u16_at(6) })
        }
        KIND_TICK => match payload {
            [0] => Ok(Message::Tick { flash_on: false }),
            [1] => Ok(Message::Tick { flash_on: true }),
            _ if payload.len() == TICK_BYTES => Err(PipeError::BadValue("flash phase")),
            _ => Err(bad_length()),
        },
        KIND_CLOSE if payload.is_empty() => Ok(Message::Close),
        KIND_CLOSE => Err(bad_length()),
        KIND_FRAME => decode_frame(payload).ok_or_else(bad_length).and_then(|frame| frame),
        other => Err(PipeError::UnknownKind(other)),
    }
}

// Returns None for a length mismatch, Some(Err) for a bad value.
fn decode_frame(payload: &[u8]) -> Option<Result<Message, PipeError>> {
    if payload.len() < FRAME_FIXED_BYTES {
        return None;
    }
    let channel = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let page = u16::from_be_bytes([payload[4], payload[5]]);
    let flash_on = match payload[6] {
        0 => false,
        1 => true,
        _ => return Some(Err(PipeError::BadValue("flash phase"))),
    };
    let colours = usize::from(payload[7]);
    let width = usize::from(u16::from_be_bytes([payload[8], payload[9]]));
    let height = usize::from(u16::from_be_bytes([payload[10], payload[11]]));
    let palette_end = FRAME_FIXED_BYTES + colours * RGB_BYTES;
    if payload.len() != palette_end + width * height {
        return None;
    }
    if colours == 0 || colours > MAX_COLOURS || width != COLUMNS_DOTS || height != ROWS_DOTS {
        return Some(Err(PipeError::BadValue("frame shape")));
    }
    let palette = payload[FRAME_FIXED_BYTES..palette_end]
        .as_chunks::<RGB_BYTES>()
        .0
        .iter()
        .map(|&[red, green, blue]| Rgb { red, green, blue })
        .collect();
    let indices = payload[palette_end..].to_vec();
    if indices.iter().any(|&index| usize::from(index) >= colours) {
        return Some(Err(PipeError::BadValue("palette index out of range")));
    }
    Some(Ok(Message::Frame(Frame { channel, page, flash_on, width, height, palette, indices })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::render;

    fn round_trip(message: &Message) -> Message {
        let mut bytes = Vec::new();
        write(&mut bytes, message).unwrap();
        read(&mut bytes.as_slice()).unwrap().unwrap()
    }

    #[test]
    fn every_kind_round_trips() {
        for message in [
            Message::Show { channel: 7, page: 0x123, subcode: 1 },
            Message::Tick { flash_on: true },
            Message::Close,
            Message::Frame(render(7, 0x100, false)),
        ] {
            assert_eq!(round_trip(&message), message);
        }
    }

    #[test]
    fn oversized_length_is_refused_before_reading_the_payload() {
        let mut bytes = vec![KIND_FRAME];
        bytes.extend_from_slice(&u32::try_from(MAX_PAYLOAD + 1).unwrap().to_be_bytes());
        assert!(matches!(read(&mut bytes.as_slice()), Err(PipeError::TooLong(_))));
    }

    #[test]
    fn unknown_kind_and_bad_values_are_refused() {
        let mut bytes = vec![99, 0, 0, 0, 0];
        assert!(matches!(read(&mut bytes.as_slice()), Err(PipeError::UnknownKind(99))));
        bytes = vec![KIND_SHOW, 0, 0, 0, 8, 0, 0, 0, 1, 0x09, 0x00, 0, 0];
        assert!(matches!(read(&mut bytes.as_slice()), Err(PipeError::BadValue("page number"))));
        bytes = vec![KIND_TICK, 0, 0, 0, 1, 2];
        assert!(matches!(read(&mut bytes.as_slice()), Err(PipeError::BadValue("flash phase"))));
    }

    #[test]
    fn frame_with_an_index_outside_its_palette_is_refused() {
        let mut frame = render(1, 0x100, false);
        frame.indices[0] = u8::try_from(frame.palette.len()).unwrap();
        let mut bytes = Vec::new();
        write(&mut bytes, &Message::Frame(frame)).unwrap();
        assert!(matches!(read(&mut bytes.as_slice()), Err(PipeError::BadValue("palette index out of range"))));
    }

    #[test]
    fn clean_end_of_input_is_none() {
        assert!(read(&mut [].as_slice()).unwrap().is_none());
    }
}
