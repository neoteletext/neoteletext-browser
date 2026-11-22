//! A script's WebSocket end to end (RFC 6455; TODO-7321 item 1, TODO-7371
//! steps 2 and 3), against a server in the test's process that speaks the
//! protocol by hand: the handshake and what it carries, messages each way
//! in the pipe's pieces, the closing handshake from either side, and the
//! refusals.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protocol::web::{AddressSpace, ErrorKind, FromWebClient, MAX_CHUNK_BYTES, MAX_WS_MESSAGE_BYTES, Reason, RequestKind, Start, ToWebClient};

use super::support::{Client, HERE, Reply, certificate, serve};

/// How the test's server answers the handshake.
#[derive(Clone, Copy, PartialEq)]
enum Answering {
    /// As RFC 6455 says, choosing the subprotocol `chat` when offered.
    Accepts,
    /// With an accept value for another key.
    WrongAccept,
    /// Choosing a subprotocol that was not offered.
    UnofferedProtocol,
    /// With an extension that was not asked for.
    Extension,
    /// As a web server that knows no WebSocket.
    PlainAnswer,
    /// With a redirect.
    Redirects,
}

/// A frame as the server writes it: unmasked, final.
fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        length @ 0..=125 => frame.push(u8::try_from(length).unwrap()),
        length @ 126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&u16::try_from(length).unwrap().to_be_bytes());
        }
        length => {
            frame.push(127);
            frame.extend_from_slice(&u64::try_from(length).unwrap().to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    frame
}

/// One frame a client wrote, unmasked: its opcode and payload. A client's
/// frame that is not masked fails the test.
fn read_frame(stream: &mut dyn Read) -> Option<(u8, Vec<u8>)> {
    let mut start = [0_u8; 2];
    stream.read_exact(&mut start).ok()?;
    assert_eq!(start[1] & 0x80, 0x80, "a client's frame is masked");
    let length = match start[1] & 0x7F {
        126 => {
            let mut bytes = [0_u8; 2];
            stream.read_exact(&mut bytes).ok()?;
            usize::from(u16::from_be_bytes(bytes))
        }
        127 => {
            let mut bytes = [0_u8; 8];
            stream.read_exact(&mut bytes).ok()?;
            usize::try_from(u64::from_be_bytes(bytes)).unwrap()
        }
        short => usize::from(short),
    };
    let mut mask = [0_u8; 4];
    stream.read_exact(&mut mask).ok()?;
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload).ok()?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    Some((start[0] & 0x0F, payload))
}

/// Serves one connection: the handshake as `answering` says, then every
/// message echoed, with a few texts that ask the server for something
/// else. What it read of the handshake, and each close it was sent, go to
/// `seen`.
fn converse(mut stream: impl Read + Write, answering: Answering, seen: &Mutex<Vec<String>>) {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let field = |name: &str| head.lines().find_map(|line| line.strip_prefix(name).map(|value| value.trim().to_owned()));
    seen.lock().unwrap().push(head.clone());
    let key = field("Sec-WebSocket-Key:").unwrap_or_default();
    let accept = neoteletext_web::ws::accept_for(if answering == Answering::WrongAccept { b"another key" } else { key.as_bytes() });
    let accept = String::from_utf8_lossy(&accept).into_owned();
    let offered = field("Sec-WebSocket-Protocol:").unwrap_or_default();
    let mut answer = match answering {
        Answering::PlainAnswer => "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_owned(),
        Answering::Redirects => "HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\n\r\n".to_owned(),
        _ => format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSet-Cookie: socket=1; Path=/\r\n"
        ),
    };
    match answering {
        Answering::Accepts | Answering::WrongAccept => {
            if offered.split(',').any(|protocol| protocol.trim() == "chat") {
                answer.push_str("Sec-WebSocket-Protocol: chat\r\n");
            }
            answer.push_str("\r\n");
        }
        Answering::UnofferedProtocol => answer.push_str("Sec-WebSocket-Protocol: secret\r\n\r\n"),
        Answering::Extension => answer.push_str("Sec-WebSocket-Extensions: permessage-deflate\r\n\r\n"),
        Answering::PlainAnswer | Answering::Redirects => {}
    }
    stream.write_all(answer.as_bytes()).unwrap();
    if answering != Answering::Accepts {
        return;
    }
    while let Some((opcode, payload)) = read_frame(&mut stream) {
        let reply = match (opcode, payload.as_slice()) {
            // A close is answered with the same body, and ends it.
            (8, body) => {
                seen.lock().unwrap().push(format!("close {body:?}"));
                let _ = stream.write_all(&frame(8, body));
                return;
            }
            (1, b"ping me") => [frame(9, b"are you there"), frame(1, b"pinged")].concat(),
            (10, body) => frame(1, &[b"pong: ", body].concat()),
            // In two fragments with a ping between, which RFC 6455
            // section 5.4 allows.
            (1, b"in fragments") => [vec![0x01, 3], b"one".to_vec(), frame(9, b""), vec![0x80, 3], b"two".to_vec()].concat(),
            (1, b"too big") => frame(2, &vec![7_u8; MAX_WS_MESSAGE_BYTES + 1]),
            (1, b"bye") => frame(8, &[&4001_u16.to_be_bytes()[..], b"going"].concat()),
            (1, b"masked") => vec![0x81, 0x81, 1, 2, 3, 4, b'x' ^ 1],
            (1, b"vanish") => return,
            (1, b"empty") => frame(1, b""),
            (opcode, body) => frame(opcode, body),
        };
        if stream.write_all(&reply).is_err() {
            return;
        }
    }
}

/// A WebSocket server on this computer, plain or over TLS, and what it
/// saw.
fn socket_server(answering: Answering, tls: bool) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind((HERE, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let noted = Arc::clone(&seen);
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let (socket, noted) = (socket.unwrap(), Arc::clone(&noted));
            std::thread::spawn(move || {
                if tls {
                    let (certificate, key) = certificate();
                    let mut config = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![certificate], key).unwrap();
                    // HTTP/2 is offered: a socket's handshake must not take it.
                    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
                    let connection = rustls::ServerConnection::new(Arc::new(config)).unwrap();
                    converse(rustls::StreamOwned::new(connection, socket), answering, &noted);
                } else {
                    converse(socket, answering, &noted);
                }
            });
        }
    });
    (port, seen)
}

fn as_socket(protocols: &[&str]) -> impl FnOnce(&mut Start) {
    let protocols: Vec<String> = protocols.iter().map(|&protocol| protocol.to_owned()).collect();
    move |start| (start.kind, start.protocols) = (RequestKind::WebSocket, protocols)
}

fn text(client: &mut Client, id: u32, message: &str) {
    client.send(&ToWebClient::SocketSend { id, binary: false, last: true, bytes: message.as_bytes().to_vec() });
}

/// The next event that is not a report of bytes sent.
fn next(client: &mut Client) -> FromWebClient {
    loop {
        match client.next_event() {
            FromWebClient::Sent { .. } => {}
            other => return other,
        }
    }
}

fn message(id: u32, binary: bool, bytes: &[u8]) -> FromWebClient {
    FromWebClient::SocketMessage { id, binary, last: true, bytes: bytes.to_vec() }
}

fn closed(id: u32, code: u16, reason: &str, clean: bool) -> FromWebClient {
    FromWebClient::SocketClosed { id, code, reason: reason.to_owned(), clean }
}

/// RFC 6455 sections 4.1 and 5, and the WHATWG standard's "establish
/// a WebSocket connection": the handshake carries the upgrade, the
/// channel's origin, the subprotocols offered and the channel's cookies,
/// and nothing of the script's own headers; the server's choice of
/// subprotocol is told; messages cross each way, a long one in the pipe's
/// pieces, with what was sent reported for `bufferedAmount`; a ping is
/// answered; a fragmented message is one message.
#[test]
fn a_socket_opens_and_carries_messages_each_way() {
    let (port, seen) = socket_server(Answering::Accepts, false);
    let web = serve(|_| Reply::ok("x").header("set-cookie", "sid=1; Path=/"));
    let channel = format!("http://site.test:{}/a/", web.port);
    let mut client = Client::new(&channel);
    client.get(&web.url("/a/login"));
    let address = format!("ws://site.test:{port}/a/live?room=1");
    let id = client.start(&address, |start| {
        as_socket(&["chat", "other"])(start);
        start.headers = vec![("X-Script".into(), b"no".to_vec())];
    });
    assert_eq!(client.next_event(), FromWebClient::Opened { id, protocol: "chat".into(), url: String::new() });
    let head = seen.lock().unwrap()[0].clone();
    let mut lines: Vec<&str> = head.lines().filter(|line| !line.is_empty() && !line.starts_with("Sec-WebSocket-Key:")).collect();
    lines.sort_unstable();
    let family = neoteletext_web::user_agent::os_family().unwrap();
    assert_eq!(
        lines,
        [
            "Connection: Upgrade".to_owned(),
            // The jar's cookie of the same host goes with the handshake.
            "Cookie: sid=1".to_owned(),
            "GET /a/live?room=1 HTTP/1.1".to_owned(),
            format!("Host: site.test:{port}"),
            format!("Origin: http://site.test:{}", web.port),
            "Sec-WebSocket-Protocol: chat, other".to_owned(),
            "Sec-WebSocket-Version: 13".to_owned(),
            "Upgrade: websocket".to_owned(),
            format!("User-Agent: NeoTeletext/0.1 ({family})"),
        ]
    );
    // The handshake's answer sets cookies as any answer does.
    assert!(client.jar[1].windows(8).any(|window| window == b"socket=1"));
    // Text and binary, echoed.
    text(&mut client, id, "hello \u{1F600}");
    assert_eq!(client.next_event(), FromWebClient::Sent { id, bytes: 10 });
    assert_eq!(client.next_event(), message(id, false, "hello \u{1F600}".as_bytes()));
    // A message of 100,000 bytes in two pieces each way, reported as sent
    // once it is written.
    let long = vec![9_u8; 100_000];
    client.send(&ToWebClient::SocketSend { id, binary: true, last: false, bytes: long[..MAX_CHUNK_BYTES].to_vec() });
    client.send(&ToWebClient::SocketSend { id, binary: true, last: true, bytes: long[MAX_CHUNK_BYTES..].to_vec() });
    assert_eq!(client.next_event(), FromWebClient::Sent { id, bytes: 100_000 });
    assert_eq!(client.next_event(), FromWebClient::SocketMessage { id, binary: true, last: false, bytes: long[..MAX_CHUNK_BYTES].to_vec() });
    client.send(&ToWebClient::More { id, bytes: 200_000 });
    assert_eq!(client.next_event(), FromWebClient::SocketMessage { id, binary: true, last: true, bytes: long[MAX_CHUNK_BYTES..].to_vec() });
    // A message of no bytes is one empty last piece, and reports nothing
    // sent.
    text(&mut client, id, "empty");
    assert_eq!(next(&mut client), message(id, false, b""));
    client.send(&ToWebClient::SocketSend { id, binary: true, last: true, bytes: Vec::new() });
    assert_eq!(client.next_event(), message(id, true, b""));
    // The server's ping is answered with its own bytes, and the script
    // hears nothing of it.
    text(&mut client, id, "ping me");
    assert_eq!(next(&mut client), message(id, false, b"pinged"));
    assert_eq!(next(&mut client), message(id, false, b"pong: are you there"));
    text(&mut client, id, "in fragments");
    assert_eq!(next(&mut client), message(id, false, b"onetwo"));
    assert_eq!(next(&mut client), message(id, false, b"pong: "), "the ping between the fragments was answered too");
    // The script closes: the server's close answers, and the closing is
    // clean.
    client.send(&ToWebClient::SocketClose { id, code: 1000, reason: "done".into() });
    assert_eq!(next(&mut client), closed(id, 1000, "done", true));
    assert_eq!(seen.lock().unwrap().last().unwrap(), &format!("close {:?}", [&1000_u16.to_be_bytes()[..], b"done"].concat()));
    client.send(&ToWebClient::SocketSend { id, binary: false, last: true, bytes: b"late".to_vec() });
    assert_eq!(client.next_within(Duration::from_millis(200)), None, "a closed socket takes no more");
    client.finish().unwrap();
}

/// DEC-7315 version 2 (TODO-7395 step 1): a page opened from disk opens
/// a socket as the origin `null`, plain or not, with no cookie.
#[test]
fn a_file_pages_socket_says_the_origin_null() {
    let (port, seen) = socket_server(Answering::Accepts, false);
    let mut page = Client::with(protocol::web::FILE_PAGE_IDENTITY, |setup| setup.space = protocol::web::FILE_PAGE_SPACE);
    let id = page.start(&format!("ws://localhost:{port}/live"), as_socket(&[]));
    assert_eq!(page.next_event(), FromWebClient::Opened { id, protocol: String::new(), url: String::new() });
    let head = seen.lock().unwrap()[0].clone();
    assert!(head.contains("\r\nOrigin: null\r\n") && !head.contains("Cookie:"), "{head}");
    assert!(page.jar.iter().all(Vec::is_empty), "the handshake's Set-Cookie is not kept");
    text(&mut page, id, "hello");
    assert_eq!(next(&mut page), message(id, false, b"hello"));
    page.send(&ToWebClient::SocketClose { id, code: 1000, reason: String::new() });
    assert_eq!(next(&mut page), closed(id, 1000, "", true));
    page.finish().unwrap();
}

/// RFC 6455 sections 5.5.1 and 7, with TODO-7371 step 3: a script's close
/// with no code is a close frame with no body; the script host's 1009 is
/// sent as it is; a code that is no script's goes as none; the server's
/// own close is told with its code and reason; a connection that ends
/// with no close frame, a message past the bound, and a frame that breaks
/// the rules each end the socket uncleanly.
#[test]
fn a_socket_closes_as_each_side_asks() {
    let (port, seen) = socket_server(Answering::Accepts, false);
    let mut client = Client::new(&format!("http://site.test:{port}/a/"));
    let address = format!("ws://site.test:{port}/a/live");
    let open = |client: &mut Client| {
        let id = client.start(&address, as_socket(&[]));
        assert_eq!(client.next_event(), FromWebClient::Opened { id, protocol: String::new(), url: String::new() });
        id
    };
    let last_close = |seen: &Mutex<Vec<String>>| seen.lock().unwrap().iter().rev().find(|line| line.starts_with("close")).cloned().unwrap();
    for (code, sent, told) in
        [(1005, "close []", 1005), (1009, "close [3, 241]", 1009), (3000, "close [11, 184]", 3000), (1006, "close []", 1005), (1001, "close []", 1005)]
    {
        let id = open(&mut client);
        client.send(&ToWebClient::SocketClose { id, code, reason: String::new() });
        assert_eq!(client.next_event(), closed(id, told, "", true), "code {code}");
        assert_eq!(last_close(&seen), sent, "code {code}");
    }
    let id = open(&mut client);
    text(&mut client, id, "bye");
    assert_eq!(next(&mut client), closed(id, 4001, "going", true));
    assert_eq!(last_close(&seen), "close [15, 161]", "the server's close is answered with its code");
    let id = open(&mut client);
    text(&mut client, id, "vanish");
    assert_eq!(next(&mut client), closed(id, 1006, "", false));
    let id = open(&mut client);
    text(&mut client, id, "too big");
    assert_eq!(next(&mut client), closed(id, 1009, "", false), "a message past the bound (departure D21)");
    let id = open(&mut client);
    text(&mut client, id, "masked");
    assert_eq!(next(&mut client), closed(id, 1006, "", false), "a server's frame that is masked breaks the protocol");
    // A message the script sends past the bound closes the socket too.
    let id = open(&mut client);
    for _ in 0..=MAX_WS_MESSAGE_BYTES / MAX_CHUNK_BYTES {
        client.send(&ToWebClient::SocketSend { id, binary: true, last: false, bytes: vec![0; MAX_CHUNK_BYTES] });
    }
    assert_eq!(next(&mut client), closed(id, 1009, "", false));
    // An abort gives a socket up, open or not.
    let id = open(&mut client);
    client.send(&ToWebClient::Abort(id));
    assert_eq!(client.next_event(), FromWebClient::Failed { id, error: ErrorKind::Abort, reason: Reason::Aborted });
    client.finish().unwrap();
}

/// RFC 6455 section 4.1, the client's checks of the server's answer, and
/// the rules a socket shares with a fetch: an answer that is not this
/// handshake's acceptance opens nothing; a redirect is not followed; a
/// bad port, a plain socket from a channel loaded securely, and this
/// computer asked by a public channel (DEC-7315, TODO-7316 item 11) are
/// each refused by name.
#[test]
fn a_socket_that_may_not_open_does_not() {
    for (answering, reason) in [
        (Answering::WrongAccept, Reason::Protocol),
        (Answering::UnofferedProtocol, Reason::Protocol),
        (Answering::Extension, Reason::Protocol),
        (Answering::PlainAnswer, Reason::Protocol),
        (Answering::Redirects, Reason::Redirect),
    ] {
        let (port, _) = socket_server(answering, false);
        let mut client = Client::new(&format!("http://site.test:{port}/a/"));
        assert_eq!(client.fetch(&format!("ws://site.test:{port}/a/live"), as_socket(&["chat"])).reason(), reason);
        client.finish().unwrap();
    }
    let (port, seen) = socket_server(Answering::Accepts, false);
    let channel = format!("http://site.test:{port}/a/");
    let mut client = Client::new(&channel);
    assert_eq!(client.fetch("ws://site.test:25/", as_socket(&[])).reason(), Reason::BadPort);
    let closed_port = TcpListener::bind((HERE, 0)).unwrap().local_addr().unwrap().port();
    assert_eq!(client.fetch(&format!("ws://site.test:{closed_port}/"), as_socket(&[])).reason(), Reason::Connection);
    client.finish().unwrap();
    let mut public = Client::with(&channel, |setup| setup.space = AddressSpace::Public);
    assert_eq!(public.fetch(&format!("ws://site.test:{port}/a/live"), as_socket(&[])).reason(), Reason::LocalRefused);
    assert_eq!(public.fetch("ws://192.168.1.1/", as_socket(&[])).reason(), Reason::LocalRefused);
    public.finish().unwrap();
    let mut secure_channel = Client::new("https://localhost:8443/a/");
    assert_eq!(secure_channel.fetch("ws://elsewhere.test/live", as_socket(&[])).reason(), Reason::MixedContent);
    secure_channel.finish().unwrap();
    assert!(seen.lock().unwrap().is_empty(), "none of them reached the server");
}

/// `wss`: the same socket over TLS, by the same certificates as any
/// connection, with HTTP/1.1 alone taken in the TLS handshake though the
/// server offers HTTP/2; a certificate for another name opens nothing.
#[test]
fn a_secure_socket_opens_over_tls() {
    let (port, seen) = socket_server(Answering::Accepts, true);
    let mut client = Client::new(&format!("https://localhost:{port}/a/"));
    let id = client.start(&format!("wss://localhost:{port}/a/live"), as_socket(&["chat"]));
    assert_eq!(client.next_event(), FromWebClient::Opened { id, protocol: "chat".into(), url: String::new() });
    assert!(seen.lock().unwrap()[0].contains(&format!("Origin: https://localhost:{port}\r\n")));
    text(&mut client, id, "over tls");
    assert_eq!(next(&mut client), message(id, false, b"over tls"));
    client.send(&ToWebClient::SocketClose { id, code: 1000, reason: String::new() });
    assert_eq!(next(&mut client), closed(id, 1000, "", true));
    assert_eq!(client.fetch(&format!("wss://site.test:{port}/a/live"), as_socket(&[])).reason(), Reason::Certificate);
    client.finish().unwrap();
}
