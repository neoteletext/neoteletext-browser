//! The channel's server, spoken to as a receiver speaks to it.

use super::*;
use std::io::Read;
use std::net::TcpListener;

use neoteletext_input::event::{Item, Select, write as write_item};
use neoteletext_web::consts::PUSH_SUBPROTOCOL;

/// A ceiling on the whole exchange, which takes milliseconds: the read ends
/// as soon as the page asked for has arrived.
const EXCHANGE_WITHIN: Duration = Duration::from_secs(10);
/// RFC 6455 section 5.2: the mask bit, and the lengths that say a 16-bit
/// or 64-bit length follows.
const MASKED: u8 = 0x80;
const LENGTH_16: u8 = 126;
const LENGTH_64: u8 = 127;
/// RFC 6455 section 5.3: any masking key will do for a test.
const MASK: [u8; 4] = [0x12, 0x34, 0x56, 0x78];
/// The page asked for: the typing page, which has a cursor.
const PAGE: u16 = 0x200;

/// One masked client frame (RFC 6455 section 5.3).
fn client_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x80 | opcode, MASKED | u8::try_from(payload.len()).expect("a short message")];
    frame.extend_from_slice(&MASK);
    frame.extend(payload.iter().enumerate().map(|(index, byte)| byte ^ MASK[index % MASK.len()]));
    frame
}

/// One item of the input stream, as a receiver sends it.
fn encoded(item: &Item<'_>) -> Vec<u8> {
    let mut out = [0; neoteletext_input::MAX_MESSAGE];
    let length = write_item(item, &mut out).unwrap();
    out[..length].to_vec()
}

fn read_exact(stream: &mut TcpStream, count: usize) -> Vec<u8> {
    let mut bytes = vec![0; count];
    stream.read_exact(&mut bytes).expect("the server answers within the ceiling");
    bytes
}

/// One whole unmasked server frame: its opcode and payload.
fn server_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let head = read_exact(stream, 2);
    let length = match head[1] & 0x7F {
        LENGTH_16 => usize::from(u16::from_be_bytes(read_exact(stream, 2).try_into().unwrap())),
        LENGTH_64 => usize::try_from(u64::from_be_bytes(read_exact(stream, 8).try_into().unwrap())).unwrap(),
        length => usize::from(length),
    };
    (head[0] & 0x0F, read_exact(stream, length))
}

/// Whether pushed bytes hold the header of `page`.
fn holds_header_of(bytes: &[u8], page: u16) -> bool {
    bytes.as_chunks::<42>().0.iter().any(|bytes| match neoteletext_core::packet::decode(&neoteletext_core::packet::Packet { bytes: *bytes }) {
        Ok(neoteletext_core::packet::Decoded::Header(header)) => {
            header.page.is_some_and(|number| neoteletext_core::types::PageAddress { magazine: header.magazine, page: number }.mpp() == page)
        }
        _ => false,
    })
}

// neoteletext-common's DEC-1459 and the engine's REQ-41: everything the
// channel sends on a viewer's WebSocket is a binary message, never text.
#[test]
fn every_frame_the_channel_sends_is_binary() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = serve(stream, &Settings::default());
    });
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(EXCHANGE_WITHIN)).unwrap();
    write!(
        stream,
        "GET /{CHANNEL}/push HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: {PUSH_SUBPROTOCOL}\r\n\r\n"
    )
    .unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.extend(read_exact(&mut stream, 1));
    }
    assert!(head.starts_with(b"HTTP/1.1 101 "), "{}", String::from_utf8_lossy(&head));
    let hello = encoded(&Item::Event(Event::Hello { version: neoteletext_input::VERSION, caps: super::super::tti::USED, agent: None }));
    let select = encoded(&Item::Event(Event::Select(Select::new(PAGE, neoteletext_core::limits::SUBCODE_ANY).unwrap(), "")));
    stream.write_all(&client_frame(websocket::BINARY, &hello)).unwrap();
    stream.write_all(&client_frame(websocket::BINARY, &select)).unwrap();
    let deadline = Instant::now() + EXCHANGE_WITHIN;
    let mut opcodes = Vec::new();
    loop {
        assert!(Instant::now() < deadline, "page {PAGE:03X} never came; frames {opcodes:?}");
        let (opcode, payload) = server_frame(&mut stream);
        opcodes.push(opcode);
        // A text frame ends the read at once: it is the fault looked for.
        if opcode != websocket::BINARY || holds_header_of(&payload, PAGE) {
            break;
        }
    }
    assert!(opcodes.len() >= 2, "WELCOME, then the page: {opcodes:?}");
    assert!(opcodes.iter().all(|&opcode| opcode == websocket::BINARY), "binary frames only, no text: {opcodes:?}");
}
