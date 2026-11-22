//! A script's `WebSocket` (RFC 6455; the WHATWG standard's "establish
//! a WebSocket connection"): the opening handshake over a connection of
//! its own, then messages each way until one side closes.
//!
//! The framing, the handshake's accept value and the reader of the
//! handshake's answer are the engine's (`neoteletext_web::ws` and
//! `neoteletext_web::http`), written in the project, without input or
//! output of their own and already fuzzed there; nothing of them is
//! written again here. A socket's connection is opened as any is
//! (`connect::open`): the same address rules for this computer and the
//! local network (DEC-7315; TODO-7316 item 11), the same deadline, the
//! same proxy and the same certificates, with HTTP/1.1 alone offered in
//! the TLS handshake, since the upgrade is HTTP/1.1's.
//!
//! What it does not do, each a departure of `docs/script-api.md` section
//! 9.4: no extension is offered or accepted, `permessage-deflate` among
//! them (D20); a message is at most [`MAX_WS_MESSAGE_BYTES`] each way, a
//! longer one closing the socket with code 1009 (D21).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use neoteletext_web::bytes::Writer;
use neoteletext_web::consts::{CLOSE_TOO_BIG, OPCODE_BINARY, OPCODE_CLOSE, OPCODE_PONG, OPCODE_TEXT};
use neoteletext_web::http::{HeadCollector, each_field, is_interim, parse_head};
use neoteletext_web::ws::{FrameReader, WsError, WsMessage, accept_for, new_key, write_frame};
use protocol::web::{FromWebClient, MAX_CHUNK_BYTES, MAX_CLOSE_REASON_BYTES, MAX_WS_MESSAGE_BYTES, MAX_WS_PROTOCOLS, Reason, Start, is_token_byte};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::mpsc;
use url::Url;

use super::connect::Stream;
use super::cookies;
use super::fetch::{Engine, is_loopback, user_agent};
use super::limits::MAX_RESPONSE_HEAD;
use super::rules::is_bad_port;
use crate::agent::Random;

/// The longest header of a frame a client writes: two bytes, eight of an
/// extended length and four of the mask (RFC 6455 section 5.2).
const MAX_FRAME_HEADER: usize = 2 + 8 + 4;
/// The bytes of a close frame's status code (RFC 6455 section 5.5.1).
const CLOSE_CODE_BYTES: usize = 2;
/// The close code of a close frame with no status (RFC 6455 section
/// 7.4.1): never sent in a frame, and what a script that gave no code is
/// taken to mean.
pub(crate) const CLOSE_NO_STATUS: u16 = 1005;
/// The close code a script is told when the connection ended without a
/// close frame (RFC 6455 section 7.4.1): never sent in a frame.
const CLOSE_ABNORMAL: u16 = 1006;
/// The close code a script gives for an ordinary end, and the codes it
/// may choose itself (the WHATWG standard, `close(code, reason)`).
const CLOSE_NORMAL: u16 = 1000;
const CLOSE_SCRIPTS_OWN: core::ops::RangeInclusive<u16> = 3000..=4999;
/// The interim answers read past before the handshake's own, as the
/// engine's HTTP/3 reader bounds them.
const MAX_INTERIM_ANSWERS: usize = 8;
/// Bytes read from the socket at a time: one TLS record's plaintext.
const READ_BYTES: usize = 1 << 14;
/// How long the server has to answer the script's close with its own
/// before the connection is dropped. RFC 6455 section 7.1.1 leaves the
/// wait to the client; a server that has gone holds nothing longer.
pub(crate) const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// The bytes of messages a script may have handed over and the socket not
/// yet taken: two of the longest message, past which the script host is
/// not keeping to its own bound and the socket is closed.
pub(crate) const MAX_QUEUED_BYTES: usize = 2 * MAX_WS_MESSAGE_BYTES;

/// What the script asks of an open socket.
pub(crate) enum Command {
    /// A piece of a message to send.
    Send {
        /// Whether binary, else text.
        binary: bool,
        /// Whether the message's last piece.
        last: bool,
        /// The piece.
        bytes: Vec<u8>,
    },
    /// The script closes the socket.
    Close {
        /// Its code: 1005 for none.
        code: u16,
        /// Its reason.
        reason: String,
    },
}

/// What a socket's task tells, and where its room is.
pub(crate) trait Told {
    /// Tells the trusted core an event of the socket.
    async fn tell(&self, event: FromWebClient);
    /// Waits for room for `count` bytes of a message.
    async fn room(&self, count: usize);
    /// Has the jar written out for the trusted core, if the handshake's
    /// answer changed it.
    async fn jar(&self);
}

/// How an open socket ended: the event the script is told.
fn closed(id: u32, code: u16, reason: &[u8], clean: bool) -> FromWebClient {
    let mut reason = String::from_utf8_lossy(reason).into_owned();
    while reason.len() > MAX_CLOSE_REASON_BYTES {
        reason.pop();
    }
    FromWebClient::SocketClosed { id, code, reason, clean }
}

/// The handshake's request, as RFC 6455 section 4.1 and the WHATWG
/// standard's "establish a WebSocket connection" write it: the upgrade's
/// headers, the origin always, the subprotocols offered, the channel's
/// cookies, and no extension.
fn request(url: &Url, origin: &str, key: &[u8], protocols: &[String], cookie: Option<&str>) -> Option<Vec<u8>> {
    let host = url.host_str()?;
    let authority = url.port().map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"));
    let target = url.query().map_or_else(|| url.path().to_owned(), |query| format!("{}?{query}", url.path()));
    let mut head =
        format!("GET {target} HTTP/1.1\r\nHost: {authority}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nOrigin: {origin}\r\n")
            .into_bytes();
    head.extend_from_slice(b"Sec-WebSocket-Key: ");
    head.extend_from_slice(key);
    head.extend_from_slice(b"\r\nUser-Agent: ");
    head.extend_from_slice(&user_agent());
    head.extend_from_slice(b"\r\n");
    if !protocols.is_empty() {
        head.extend_from_slice(format!("Sec-WebSocket-Protocol: {}\r\n", protocols.join(", ")).as_bytes());
    }
    if let Some(cookie) = cookie {
        head.extend_from_slice(format!("Cookie: {cookie}\r\n").as_bytes());
    }
    head.extend_from_slice(b"\r\n");
    Some(head)
}

/// The subprotocol the server chose in the complete answer `head`, empty
/// for none.
///
/// # Errors
/// When the answer is not the acceptance of this handshake (RFC 6455
/// section 4.1, the client's checks 1 to 6): not a 101, no `Upgrade:
/// websocket` or `Connection: Upgrade`, another accept value, any
/// extension, or a subprotocol that was not offered or is named twice.
fn accepted(head: &[u8], key: &[u8], protocols: &[String]) -> Result<String, Reason> {
    let parsed = parse_head(head).map_err(|_| Reason::Protocol)?;
    if (300..400).contains(&parsed.status) {
        // The standard asks for the redirect mode `error`.
        return Err(Reason::Redirect);
    }
    let switched = parsed.status == 101
        && parsed.upgrade_websocket
        && parsed.connection_upgrade
        && parsed.websocket_accept == Some(accept_for(key))
        && !parsed.websocket_extensions;
    let mut chosen = Vec::new();
    each_field(head, b"sec-websocket-protocol", &mut |value| chosen.push(String::from_utf8_lossy(value.trim_ascii()).into_owned()));
    match chosen.as_slice() {
        [] if switched => Ok(String::new()),
        [one] if switched && protocols.contains(one) => Ok(one.clone()),
        _ => Err(Reason::Protocol),
    }
}

/// One frame, masked, written whole.
async fn send_frame(out: &mut WriteHalf<Stream>, opcode: u8, payload: &[u8]) -> Result<(), ()> {
    let mut buffer = vec![0_u8; payload.len().saturating_add(MAX_FRAME_HEADER)];
    let mut writer = Writer::new(&mut buffer);
    write_frame(&mut writer, opcode, payload, &mut Random::new()).map_err(|_| ())?;
    let length = writer.len();
    out.write_all(buffer.get(..length).unwrap_or_default()).await.map_err(|_| ())
}

/// A close frame with `code` and `reason`, or with no body when `code`
/// is none.
async fn send_close(out: &mut WriteHalf<Stream>, code: Option<u16>, reason: &str) {
    let mut payload = Vec::with_capacity(CLOSE_CODE_BYTES.saturating_add(reason.len()));
    if let Some(code) = code {
        payload.extend_from_slice(&code.to_be_bytes());
        payload.extend_from_slice(reason.as_bytes());
    }
    // A close that cannot be written closes nothing more than the
    // connection's end will.
    let _ = send_frame(out, OPCODE_CLOSE, &payload).await;
    let _ = out.flush().await;
}

/// The code a script's close is sent with: none for 1005 (it gave none),
/// its own for the codes it may choose, and 1009 for the script host's
/// close of a socket whose message was too long (departure D21). Any
/// other is not a script's to send and goes as none.
fn close_code_to_send(code: u16) -> Option<u16> {
    (code == CLOSE_NORMAL || code == CLOSE_TOO_BIG || CLOSE_SCRIPTS_OWN.contains(&code)).then_some(code)
}

/// Where a socket's handshake goes: the address as an HTTP request's,
/// and the place to connect to.
struct Target {
    url: Url,
    secure: bool,
    host: String,
    port: u16,
}

/// The place `start` asks a socket of, held to the rules a request's
/// address is held to: a `ws` or `wss` address, no blocked port, no plain
/// socket from a channel loaded securely but to this computer itself, and
/// subprotocols that are tokens.
fn target(engine: &Engine, start: &Start) -> Result<Target, Reason> {
    let mut url = Url::parse(&start.url).map_err(|_| Reason::Address)?;
    let secure = match url.scheme() {
        "wss" => true,
        "ws" => false,
        _ => return Err(Reason::Address),
    };
    // The handshake is an HTTP request to the same place.
    url.set_scheme(if secure { "https" } else { "http" }).map_err(|()| Reason::Address)?;
    url.set_fragment(None);
    let port = url.port_or_known_default().ok_or(Reason::Address)?;
    if is_bad_port(port) {
        return Err(Reason::BadPort);
    }
    // A page opened from disk has no channel, and opens a plain socket
    // as it asks a plain address (DEC-7315 version 2).
    if engine.policy.folder().is_some_and(|folder| folder.scheme() == "https") && !secure && !is_loopback(&url) {
        return Err(Reason::MixedContent);
    }
    if start.protocols.len() > MAX_WS_PROTOCOLS || start.protocols.iter().any(|protocol| protocol.is_empty() || !protocol.bytes().all(is_token_byte)) {
        return Err(Reason::Request);
    }
    let host = match url.host().ok_or(Reason::Address)? {
        url::Host::Domain(name) => name.to_owned(),
        url::Host::Ipv4(address) => address.to_string(),
        url::Host::Ipv6(address) => address.to_string(),
    };
    Ok(Target { url, secure, host, port })
}

/// An open socket: the subprotocol the server chose, the bytes that came
/// with its answer, and the connection's two halves.
struct Open {
    protocol: String,
    early: Vec<u8>,
    input: ReadHalf<Stream>,
    out: WriteHalf<Stream>,
}

/// Connects and makes the opening handshake, within the limit on any
/// answer's head once the connection is open.
async fn open(engine: &Engine, start: &Start) -> Result<Open, Reason> {
    let Target { url, secure, host, port } = target(engine, start)?;
    // A socket carries the channel's cookies as a request with
    // credentials does (the standard's credentials mode `include`), by
    // the same SameSite rule. A page opened from disk is of no site.
    let same_site = engine.policy.folder().is_some_and(|folder| cookies::same_site(engine.jar.borrow().suffixes(), folder, &url));
    let context = cookies::Context { same_site, navigation: false, safe_method: true };
    let cookie = engine.jar.borrow().header(&url, context);
    let key = new_key(&mut Random::new());
    let head = request(&url, &engine.policy.origin(), &key, &start.protocols, cookie.as_deref()).ok_or(Reason::Address)?;
    let opened = engine.pool.open_alone(secure, &host, port, engine.asker()).await?;
    let (mut input, mut out) = tokio::io::split(opened.stream);
    let handshake = async {
        out.write_all(&head).await.map_err(|_| Reason::Connection)?;
        out.flush().await.map_err(|_| Reason::Connection)?;
        let mut buffer = vec![0_u8; READ_BYTES];
        let mut answer = vec![0_u8; MAX_RESPONSE_HEAD];
        let mut collector = HeadCollector::default();
        let mut interim = 0_usize;
        loop {
            let count = input.read(&mut buffer).await.map_err(|_| Reason::Connection)?;
            if count == 0 {
                return Err(Reason::Connection);
            }
            let mut rest = buffer.get(..count).unwrap_or_default();
            while !rest.is_empty() {
                let (used, complete) = collector.feed(&mut answer, rest).map_err(|_| Reason::TooLarge)?;
                rest = rest.get(used..).unwrap_or_default();
                if !complete {
                    continue;
                }
                let whole = answer.get(..collector.len()).unwrap_or_default();
                if is_interim(whole) && interim < MAX_INTERIM_ANSWERS {
                    interim = interim.saturating_add(1);
                    collector.reset();
                    continue;
                }
                let protocol = accepted(whole, &key, &start.protocols)?;
                // The handshake's answer sets cookies as any answer does.
                let mut jar = engine.jar.borrow_mut();
                each_field(whole, b"set-cookie", &mut |value| {
                    jar.store(&url, value, context);
                });
                return Ok((protocol, rest.to_vec()));
            }
        }
    };
    let (protocol, early) = tokio::time::timeout(engine.pool.head_timeout(), handshake).await.map_err(|_| Reason::Timeout)??;
    Ok(Open { protocol, early, input, out })
}

/// A socket's reading: the frames' reader, the room a message is put
/// together in, and whether the script has closed.
struct Reading {
    id: u32,
    reader: FrameReader,
    message: Vec<u8>,
    /// When the script has closed: the moment the wait for the server's
    /// close ends.
    closing: Option<tokio::time::Instant>,
}

impl Reading {
    /// Takes what has arrived, frame by frame: each whole message told in
    /// the pipe's pieces, a ping answered, and the server's close
    /// answered. The event that ends the socket is returned, if it ended.
    async fn take(&mut self, arrived: &[u8], told: &impl Told, out: &mut WriteHalf<Stream>) -> Option<FromWebClient> {
        let id = self.id;
        let mut rest = arrived;
        loop {
            let (used, read) = match self.reader.feed(&mut self.message, rest) {
                Ok(fed) => fed,
                Err(error) => {
                    // A frame that breaks the rules fails the connection
                    // (RFC 6455 section 7.1.7); one too long is told as
                    // such (departure D21).
                    send_close(out, Some(error.close_code()), "").await;
                    let code = if error == WsError::TooBig { CLOSE_TOO_BIG } else { CLOSE_ABNORMAL };
                    return Some(closed(id, code, b"", false));
                }
            };
            rest = rest.get(used..).unwrap_or_default();
            match read {
                Some(WsMessage::Text(length) | WsMessage::Binary(length)) if self.closing.is_none() => {
                    let binary = matches!(read, Some(WsMessage::Binary(_)));
                    let mut pieces = self.message.get(..length).unwrap_or_default().chunks(MAX_CHUNK_BYTES).peekable();
                    if pieces.peek().is_none() {
                        told.tell(FromWebClient::SocketMessage { id, binary, last: true, bytes: Vec::new() }).await;
                    }
                    while let Some(piece) = pieces.next() {
                        told.room(piece.len()).await;
                        told.tell(FromWebClient::SocketMessage { id, binary, last: pieces.peek().is_none(), bytes: piece.to_vec() }).await;
                    }
                }
                Some(WsMessage::Ping) => {
                    let payload = self.reader.control_payload().to_vec();
                    if send_frame(out, OPCODE_PONG, &payload).await.is_err() || out.flush().await.is_err() {
                        return Some(closed(id, CLOSE_ABNORMAL, b"", false));
                    }
                }
                Some(WsMessage::Close(code)) => {
                    let reason = self.reader.control_payload().get(CLOSE_CODE_BYTES..).unwrap_or_default().to_vec();
                    // The server's close answers the script's, or is
                    // answered with its own code (RFC 6455 section 5.5.1).
                    if self.closing.is_none() {
                        send_close(out, code, "").await;
                    }
                    return Some(closed(id, code.unwrap_or(CLOSE_NO_STATUS), &reason, true));
                }
                None if used == 0 || rest.is_empty() => return None,
                // A message after the script closed is read and dropped;
                // a pong asks nothing; a frame that completed nothing is
                // followed by the next.
                Some(WsMessage::Text(_) | WsMessage::Binary(_) | WsMessage::Pong | WsMessage::Skipped) | None => {}
            }
        }
    }
}

/// Opens the socket `start` asks for and runs it to its end, telling
/// `told` its opening, its messages and what was sent. The event that
/// ends it is returned: the script's `close` event.
///
/// # Errors
/// Why the socket never opened: the script's `error` and a `close` with
/// code 1006, by the reason given.
pub(crate) async fn run(
    engine: &Engine,
    start: &Start,
    told: &impl Told,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    queued: &Rc<Cell<usize>>,
) -> Result<FromWebClient, Reason> {
    let id = start.id;
    let Open { protocol, early, mut input, mut out } = open(engine, start).await?;
    told.jar().await;
    told.tell(FromWebClient::Opened { id, protocol, url: String::new() }).await;
    let mut reading = Reading { id, reader: FrameReader::new(), message: vec![0_u8; MAX_WS_MESSAGE_BYTES], closing: None };
    let mut buffer = vec![0_u8; READ_BYTES];
    let mut sending: Vec<u8> = Vec::new();
    let mut arrived = early;
    loop {
        if let Some(ended) = reading.take(&arrived, told, &mut out).await {
            return Ok(ended);
        }
        arrived.clear();
        let closing = reading.closing;
        let wait = async {
            match closing {
                Some(until) => tokio::time::sleep_until(until).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            read = input.read(&mut buffer) => match read {
                Ok(count) if count > 0 => arrived.extend_from_slice(buffer.get(..count).unwrap_or_default()),
                // The connection ended with no close frame.
                _ => return Ok(closed(id, CLOSE_ABNORMAL, b"", false)),
            },
            () = wait => return Ok(closed(id, CLOSE_ABNORMAL, b"", false)),
            command = commands.recv(), if closing.is_none() => match command {
                Some(Command::Send { binary, last, bytes }) => {
                    queued.set(queued.get().saturating_sub(bytes.len()));
                    sending.extend_from_slice(&bytes);
                    if sending.len() > MAX_WS_MESSAGE_BYTES {
                        send_close(&mut out, Some(CLOSE_TOO_BIG), "").await;
                        return Ok(closed(id, CLOSE_TOO_BIG, b"", false));
                    }
                    if last {
                        let opcode = if binary { OPCODE_BINARY } else { OPCODE_TEXT };
                        if send_frame(&mut out, opcode, &sending).await.is_err() || out.flush().await.is_err() {
                            return Ok(closed(id, CLOSE_ABNORMAL, b"", false));
                        }
                        // For the script's `bufferedAmount` (TODO-7371
                        // step 2); a message of no bytes buffered none.
                        if let Some(bytes) = u32::try_from(sending.len()).ok().filter(|&bytes| bytes > 0) {
                            told.tell(FromWebClient::Sent { id, bytes }).await;
                        }
                        sending.clear();
                    }
                }
                Some(Command::Close { code, reason }) => {
                    send_close(&mut out, close_code_to_send(code), &reason).await;
                    reading.closing = tokio::time::Instant::now().checked_add(CLOSE_WAIT);
                }
                // The loop that held the other end has gone.
                None => return Ok(closed(id, CLOSE_ABNORMAL, b"", false)),
            },
        }
    }
}
