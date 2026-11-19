//! The snapshot mode's tests (REQ-6884): what it reads and writes, its
//! bound, and, against a server on this computer, that nothing of the
//! viewer leaves it.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::mpsc;

use protocol::snapshot::{FromSnapshot, SnapshotEnd, ToSnapshot, read_from_fetcher, write_to_fetcher};

use super::run_with;
use crate::transport::Tcp;

/// The snapshot request for page 100 as a whole, as the engine writes it:
/// neoteletext-common's wire vector `input hello_snapshot`.
const HELLO_SNAPSHOT: &str = "ff090a8080801eff4003313030";
/// WELCOME agreeing exactly `SNAPSHOT_OFFER`: the wire vector `record
/// welcome_snapshot`.
const WELCOME_SNAPSHOT: &str = "a1ead05e000002050007ff010a8080801e013d43d7000000000000000000000000000000000000009cfb";
/// A WebSocket close frame's code for a normal close (RFC 6455 section
/// 7.4.1).
const CLOSE_NORMAL: u16 = 1000;
/// The headers that would carry the viewer, or say what the viewer already
/// holds (RFC 9110 sections 11.6.2, 10.1.3 and 13.1; RFC 6265 section 5.4).
const IDENTIFYING: [&str; 6] = ["cookie:", "authorization:", "referer:", "if-none-match:", "if-modified-since:", "proxy-authorization:"];

fn hex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap()).collect()
}

/// What a test server heard: each request's lines, and each binary
/// message the receiver sent on the push connection.
#[derive(Debug)]
enum Heard {
    Request(Vec<String>),
    Message(Vec<u8>),
    Other(u8),
}

/// A client's frame: its opcode and its payload, unmasked.
fn read_client_frame(stream: &mut dyn Read) -> Option<(u8, Vec<u8>)> {
    let mut head = [0_u8; 2];
    stream.read_exact(&mut head).ok()?;
    let mut length = u64::from(head[1] & 0x7F);
    if length == 126 {
        let mut extended = [0_u8; 2];
        stream.read_exact(&mut extended).ok()?;
        length = u64::from(u16::from_be_bytes(extended));
    } else if length == 127 {
        let mut extended = [0_u8; 8];
        stream.read_exact(&mut extended).ok()?;
        length = u64::from_be_bytes(extended);
    }
    let mut mask = [0_u8; 4];
    stream.read_exact(&mut mask).ok()?;
    let mut payload = vec![0_u8; usize::try_from(length).ok()?];
    stream.read_exact(&mut payload).ok()?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    Some((head[0] & 0x0F, payload))
}

/// A live channel on this computer: its descriptor names a push
/// connection, which answers the snapshot request with WELCOME, a page
/// and a normal close. Everything heard goes to the returned receiver.
fn live_channel() -> (u16, mpsc::Receiver<Heard>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (heard, hearing) = mpsc::channel();
    std::thread::spawn(move || {
        for socket in listener.incoming().flatten() {
            let heard = heard.clone();
            std::thread::spawn(move || {
                let mut stream = BufReader::new(socket);
                loop {
                    let mut lines = Vec::new();
                    loop {
                        let mut line = String::new();
                        if stream.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        if line.trim().is_empty() {
                            break;
                        }
                        lines.push(line.trim().to_owned());
                    }
                    let path = lines[0].split(' ').nth(1).unwrap_or_default().to_owned();
                    let _ = heard.send(Heard::Request(lines.clone()));
                    let writer = stream.get_mut();
                    if path == "/tt/push" {
                        let key = lines.iter().find_map(|line| line.strip_prefix("Sec-WebSocket-Key: ")).unwrap().to_owned();
                        let accept = neoteletext_web::ws::accept_for(key.as_bytes());
                        let upgrade = format!(
                            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Protocol: {}\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                            neoteletext_web::consts::PUSH_SUBPROTOCOL,
                            std::str::from_utf8(&accept).unwrap()
                        );
                        writer.write_all(upgrade.as_bytes()).unwrap();
                        // The request is read before anything is answered.
                        let Some((opcode, payload)) = read_client_frame(&mut stream) else { return };
                        let _ = heard.send(if opcode == neoteletext_web::consts::OPCODE_BINARY { Heard::Message(payload) } else { Heard::Other(opcode) });
                        let mut answer = hex(WELCOME_SNAPSHOT);
                        answer.extend_from_slice(&page_header());
                        let writer = stream.get_mut();
                        let mut frame = vec![0x80 | neoteletext_web::consts::OPCODE_BINARY, 126];
                        frame.extend_from_slice(&u16::try_from(answer.len()).unwrap().to_be_bytes());
                        frame.extend_from_slice(&answer);
                        writer.write_all(&frame).unwrap();
                        let mut close = vec![0x80 | neoteletext_web::consts::OPCODE_CLOSE, 2];
                        close.extend_from_slice(&CLOSE_NORMAL.to_be_bytes());
                        writer.write_all(&close).unwrap();
                        writer.flush().unwrap();
                        // Anything more the receiver sends is heard too.
                        while let Some((opcode, payload)) = read_client_frame(&mut stream) {
                            let _ = heard.send(if opcode == neoteletext_web::consts::OPCODE_BINARY { Heard::Message(payload) } else { Heard::Other(opcode) });
                        }
                        return;
                    }
                    let body = br#"{"name":"Live","push":{}}"#;
                    let answer = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len());
                    writer.write_all(answer.as_bytes()).unwrap();
                    writer.write_all(body).unwrap();
                    writer.flush().unwrap();
                }
            });
        }
    });
    (port, hearing)
}

/// Page 100's header packet, sub-code 0000, as a channel pushes it.
fn page_header() -> Vec<u8> {
    let mut packets = Vec::new();
    let mut reader = neoteletext_formats::detect::AutoReader::new(neoteletext_formats::detect::AutoConfig {
        format: Some(neoteletext_formats::detect::Format::Tti),
        encode_pages: true,
        ..neoteletext_formats::detect::AutoConfig::default()
    });
    let mut take = |item: neoteletext_formats::Item<'_>| {
        if let neoteletext_formats::Item::Packet { packet, .. } = item {
            packets.extend_from_slice(&packet.bytes);
        }
    };
    neoteletext_formats::item::Reader::feed(&mut reader, b"PN,10000\nSC,0000\nPS,8000\nOL,1,Snapshot\n", &mut take);
    neoteletext_formats::item::Reader::finish(&mut reader, &mut take);
    packets
}

/// Page 100 as a whole.
fn page_100() -> neoteletext_web::agent::SnapshotTarget {
    neoteletext_web::agent::SnapshotTarget { page: neoteletext_core::types::PageAddress::from_mpp(0x100).unwrap(), sub: neoteletext_core::types::SubCode::ANY }
}

/// Runs the snapshot mode for page 100 of `channel` on pipes, and gives
/// every message it sends until it ends, and the pipe to Close it.
fn fetch(channel: String) -> (Vec<FromSnapshot>, io::PipeWriter) {
    let (core_reads, fetcher_writes) = io::pipe().unwrap();
    let (fetcher_reads, core_writes) = io::pipe().unwrap();
    std::thread::spawn(move || {
        let local = [IpAddr::from(Ipv4Addr::LOCALHOST)];
        let config = super::snapshot_config(&local, false, page_100());
        let allowed_local = config.allowed_local;
        run_with(&channel, config, |wake| Tcp::new(wake, None, allowed_local), Box::new(fetcher_reads), Box::new(fetcher_writes))
    });
    let mut from_fetcher = BufReader::new(core_reads);
    let mut said = Vec::new();
    while let Ok(Some(message)) = read_from_fetcher(&mut from_fetcher) {
        let ended = matches!(message, FromSnapshot::Ended(_));
        said.push(message);
        if ended {
            break;
        }
    }
    (said, core_writes)
}

/// REQ-6884 items 2 and 3, the owner's DEC-6875 item 2 ("a test holds
/// this"): the server sees the descriptor's request and the push
/// connection's upgrade, neither with a cookie, an authorization, a
/// referrer or a conditional header; then one binary message, HELLO
/// offering exactly `SNAPSHOT_OFFER` with no agent text and SELECT of page
/// 100, byte for byte the engine's wire vector; and nothing else. The
/// trusted core is told it is confined, the answer's bytes and its end,
/// and nothing else.
#[test]
fn a_snapshot_connection_carries_no_viewer_state_and_no_cookie() {
    let (port, hearing) = live_channel();
    let (said, _close) = fetch(format!("http://127.0.0.1:{port}/tt/"));
    assert!(matches!(said.first(), Some(FromSnapshot::Confined)), "{said:?}");
    assert!(matches!(said.last(), Some(FromSnapshot::Ended(SnapshotEnd::Answered))), "{said:?}");
    let pieces: Vec<u8> =
        said.iter().filter_map(|message| if let FromSnapshot::Piece(bytes) = message { Some(bytes.clone()) } else { None }).flatten().collect();
    assert!(pieces.ends_with(&page_header()), "the page passed on");
    // Let the server hear whatever more the fetcher would send.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let heard: Vec<Heard> = hearing.try_iter().collect();
    let requests: Vec<&Vec<String>> = heard.iter().filter_map(|heard| if let Heard::Request(lines) = heard { Some(lines) } else { None }).collect();
    assert_eq!(requests.len(), 2, "the descriptor and the upgrade: {heard:?}");
    for lines in &requests {
        for line in *lines {
            let lower = line.to_ascii_lowercase();
            assert!(!IDENTIFYING.iter().any(|header| lower.starts_with(header)), "{line:?} carries the viewer");
        }
    }
    let messages: Vec<&Vec<u8>> = heard.iter().filter_map(|heard| if let Heard::Message(bytes) = heard { Some(bytes) } else { None }).collect();
    assert_eq!(messages, [&hex(HELLO_SNAPSHOT)], "one request, the engine's, and nothing more: {heard:?}");
    assert!(heard.iter().all(|heard| !matches!(heard, Heard::Other(opcode) if *opcode != neoteletext_web::consts::OPCODE_CLOSE)), "no ping or text: {heard:?}");
}

/// REQ-6884 item 2: the snapshot fetcher reads only `Close`, which ends
/// it; a tab fetcher's message ends it as well, having done nothing.
#[test]
fn a_snapshot_fetcher_accepts_only_close() {
    for close in [true, false] {
        let (core_reads, fetcher_writes) = io::pipe().unwrap();
        let (fetcher_reads, mut core_writes) = io::pipe().unwrap();
        // A channel that never answers: the fetch waits until told.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let channel = format!("http://127.0.0.1:{}/tt/", listener.local_addr().unwrap().port());
        let done = std::thread::spawn(move || {
            let local = [IpAddr::from(Ipv4Addr::LOCALHOST)];
            let config = super::snapshot_config(&local, false, page_100());
            let allowed_local = config.allowed_local;
            run_with(&channel, config, |wake| Tcp::new(wake, None, allowed_local), Box::new(fetcher_reads), Box::new(fetcher_writes))
        });
        let mut from_fetcher = BufReader::new(core_reads);
        assert_eq!(read_from_fetcher(&mut from_fetcher).unwrap(), Some(FromSnapshot::Confined));
        if close {
            write_to_fetcher(&mut core_writes, ToSnapshot::Close).unwrap();
        } else {
            protocol::fetcher::write_to_fetcher(&mut core_writes, &protocol::fetcher::ToFetcher::FetchContents).unwrap();
        }
        core_writes.flush().unwrap();
        assert!(done.join().unwrap().is_ok());
        assert_eq!(read_from_fetcher(&mut from_fetcher).unwrap(), None, "it ends having said nothing more");
        drop(listener);
    }
}

/// REQ-6884 item 2: everything the mode writes is of `protocol::snapshot`:
/// the trusted core's reader of a tab fetcher's messages refuses it.
#[test]
fn a_snapshot_fetcher_writes_only_its_own_messages() {
    let (port, _hearing) = live_channel();
    let (said, _close) = fetch(format!("http://127.0.0.1:{port}/tt/"));
    for message in said {
        let mut bytes = Vec::new();
        protocol::snapshot::write_to_core(&mut bytes, &message).unwrap();
        assert!(protocol::fetcher::read_from_fetcher(&mut bytes.as_slice()).is_err(), "{message:?} read as a tab fetcher's");
    }
}

/// REQ-6884 item 4: an answer past `MAX_SNAPSHOT_BYTES` ends with
/// `TooLarge` and passes on no more than the bound.
#[test]
fn a_snapshot_answer_past_its_bound_ends_too_large() {
    // A static channel whose page file is a byte past the bound.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for socket in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut stream = BufReader::new(socket);
                loop {
                    let mut first = String::new();
                    if stream.read_line(&mut first).unwrap_or(0) == 0 {
                        return;
                    }
                    loop {
                        let mut line = String::new();
                        if stream.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                            break;
                        }
                    }
                    let path = first.split(' ').nth(1).unwrap_or_default().to_owned();
                    let (media, body): (&str, Vec<u8>) = match path.as_str() {
                        "/tt/channel.ttx" => ("application/json", br#"{"name":"Big"}"#.to_vec()),
                        "/tt/100.t42" => ("application/octet-stream", page_header().repeat((protocol::snapshot::MAX_SNAPSHOT_BYTES / 42) + 1)),
                        _ => ("text/plain", Vec::new()),
                    };
                    let status = if body.is_empty() { 404 } else { 200 };
                    let head = format!("HTTP/1.1 {status} S\r\nContent-Type: {media}\r\nContent-Length: {}\r\n\r\n", body.len());
                    let writer = stream.get_mut();
                    if writer.write_all(head.as_bytes()).and_then(|()| writer.write_all(&body)).is_err() {
                        return;
                    }
                }
            });
        }
    });
    let (said, _close) = fetch(format!("http://127.0.0.1:{port}/tt/"));
    let passed: usize = said.iter().map(|message| if let FromSnapshot::Piece(bytes) = message { bytes.len() } else { 0 }).sum();
    assert!(passed <= protocol::snapshot::MAX_SNAPSHOT_BYTES);
    assert!(matches!(said.last(), Some(FromSnapshot::Ended(SnapshotEnd::TooLarge))), "{:?}", said.last());
}

/// REQ-6884 item 2: the mode's code reaches no viewer-state code: this
/// file and its parent name nothing of `state.rs`.
#[test]
fn the_snapshot_mode_does_not_use_the_viewer_state_code() {
    let source = include_str!("../snapshot.rs");
    for name in ["crate::state", "state::", "State::new", "StateAnswer", "StateChallenged"] {
        assert!(!source.contains(name), "snapshot.rs names {name}");
    }
}
