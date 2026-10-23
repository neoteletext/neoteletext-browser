//! The framing, which is the engine's: one kind byte, a 32-bit big-endian
//! payload length, then the payload, of at most [`MAX_PAYLOAD`] bytes.

use std::io::{self, Read, Write};

use crate::{MAX_PAYLOAD, ProtocolError};

const HEADER_BYTES: usize = 5;

pub(crate) fn write_frame(out: &mut dyn Write, kind: u8, payload: &[u8]) -> Result<(), ProtocolError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(ProtocolError::TooLong(payload.len()));
    }
    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::TooLong(payload.len()))?;
    let mut header = [0_u8; HEADER_BYTES];
    header[0] = kind;
    header[1..].copy_from_slice(&length.to_be_bytes());
    // Not flushed here: a process flushes once it has handled an event,
    // before it waits for the next, so the messages of one event go in one
    // write rather than one each.
    out.write_all(&header)?;
    out.write_all(payload)?;
    Ok(())
}

pub(crate) fn read_frame(input: &mut dyn Read) -> Result<Option<(u8, Vec<u8>)>, ProtocolError> {
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
    // Read into room reserved for it, not into a payload zeroed first: a
    // page frame is 240 KB, and zeroing it was most of the cost of reading
    // one.
    let mut payload = Vec::with_capacity(length);
    input.take(u64::try_from(length).map_err(|_| ProtocolError::TooLong(length))?).read_to_end(&mut payload)?;
    if payload.len() != length {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
    }
    Ok(Some((header[0], payload)))
}

/// `payload` from `offset` on, as its own vector, moved rather than
/// copied: the payload's room is kept and its bytes shifted down.
pub(crate) fn tail(mut payload: Vec<u8>, offset: usize) -> Vec<u8> {
    payload.drain(..offset.min(payload.len()));
    payload
}
