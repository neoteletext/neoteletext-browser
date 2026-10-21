//! The server's half of a WebSocket (RFC 6455), as small as a development
//! server needs: the opening handshake, and frames read from a client
//! (masked, section 5.3) and written to it (unmasked). Fragmented messages
//! are refused; no client of this server sends them.

use std::io::{self, Read, Write};
use std::net::TcpStream;

/// The largest message read: an input message (INPUT.md section 5.1).
const MAX_MESSAGE: usize = neoteletext_input::MAX_MESSAGE;
/// Section 5.2: the FIN bit, the opcode's bits and the mask bit.
const FIN: u8 = 0x80;
const OPCODE: u8 = 0x0F;
const MASKED: u8 = 0x80;
const LENGTH: u8 = 0x7F;
/// Section 5.2: lengths of 126 and 127 mean a 16-bit or 64-bit length follows.
const LENGTH_16: u8 = 126;
const LENGTH_64: u8 = 127;
/// Section 5.2's opcodes.
pub const TEXT: u8 = 0x1;
pub const BINARY: u8 = 0x2;
pub const CLOSE: u8 = 0x8;
pub const PING: u8 = 0x9;
pub const PONG: u8 = 0xA;

/// The `Sec-WebSocket-Accept` answer for a request head, or `None` when
/// the head is not a WebSocket upgrade.
pub fn accept(head: &str) -> Option<String> {
    let key = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("sec-websocket-key").then(|| value.trim().to_owned())
    })?;
    let accept = neoteletext_web::ws::accept_for(key.as_bytes());
    String::from_utf8(accept.to_vec()).ok()
}

/// Answers the opening handshake.
pub fn handshake(stream: &mut TcpStream, accept: &str) -> io::Result<()> {
    write!(stream, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n")?;
    stream.flush()
}

/// Writes one unmasked frame.
pub fn write(stream: &mut impl Write, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let mut frame = vec![FIN | opcode];
    match payload.len() {
        length if length < usize::from(LENGTH_16) => frame.push(u8::try_from(length).unwrap_or(0)),
        length if u16::try_from(length).is_ok() => {
            frame.push(LENGTH_16);
            frame.extend_from_slice(&u16::try_from(length).unwrap_or(0).to_be_bytes());
        }
        length => {
            frame.push(LENGTH_64);
            frame.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame)?;
    stream.flush()
}

/// A frame's opcode and its unmasked payload.
pub type Frame = (u8, Vec<u8>);

/// Bytes read so far, and the frames in them.
#[derive(Default)]
pub struct Reader {
    buffer: Vec<u8>,
}

impl Reader {
    /// Reads what the socket has (within its read timeout) and returns
    /// every whole frame: its opcode and its unmasked payload. `Ok(None)`
    /// when the client closed.
    pub fn read(&mut self, stream: &mut TcpStream) -> io::Result<Option<Vec<Frame>>> {
        let mut chunk = [0_u8; 4096];
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(None),
            Ok(count) => self.buffer.extend_from_slice(&chunk[..count]),
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(error) => return Err(error),
        }
        let mut frames = Vec::new();
        while let Some((frame, used)) = parse(&self.buffer)? {
            self.buffer.drain(..used);
            frames.push(frame);
        }
        Ok(Some(frames))
    }
}

/// One whole client frame from the front of `bytes`, and the bytes it
/// took; `None` until it has all arrived.
fn parse(bytes: &[u8]) -> io::Result<Option<(Frame, usize)>> {
    let invalid = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_owned());
    let (Some(&first), Some(&second)) = (bytes.first(), bytes.get(1)) else { return Ok(None) };
    if first & FIN == 0 {
        return Err(invalid("a fragmented message"));
    }
    if second & MASKED == 0 {
        return Err(invalid("an unmasked client frame"));
    }
    let (length, mut at) = match second & LENGTH {
        LENGTH_16 => match bytes.get(2..4) {
            Some(length) => (usize::from(u16::from_be_bytes([length[0], length[1]])), 4),
            None => return Ok(None),
        },
        LENGTH_64 => return Err(invalid("a message too long")),
        length => (usize::from(length), 2),
    };
    if length > MAX_MESSAGE {
        return Err(invalid("a message too long"));
    }
    let Some(mask) = bytes.get(at..at + 4) else { return Ok(None) };
    let mask = [mask[0], mask[1], mask[2], mask[3]];
    at += 4;
    let Some(payload) = bytes.get(at..at + length) else { return Ok(None) };
    let payload = payload.iter().enumerate().map(|(index, byte)| byte ^ mask[index % 4]).collect();
    Ok(Some(((first & OPCODE, payload), at + length)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_masked_client_frame_is_read_whole() {
        let mask = [1, 2, 3, 4];
        let payload = b"subscribe all";
        let mut frame = vec![FIN | TEXT, MASKED | u8::try_from(payload.len()).unwrap()];
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(index, byte)| byte ^ mask[index % 4]));
        assert!(parse(&frame[..5]).unwrap().is_none(), "not all there");
        let ((opcode, read), used) = parse(&frame).unwrap().unwrap();
        assert_eq!((opcode, read.as_slice(), used), (TEXT, payload.as_slice(), frame.len()));
        let mut unmasked = frame.clone();
        unmasked[1] &= !MASKED;
        assert!(parse(&unmasked).is_err());
    }

    #[test]
    fn the_accept_value_is_rfc_6455s_example() {
        // RFC 6455 section 1.3.
        let head = "GET /chat HTTP/1.1\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
        assert_eq!(accept(head).as_deref(), Some("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));
    }
}
