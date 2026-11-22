//! The host's half of the web APIs (work package W2; script-api.md
//! section 9 and operations 40 to 49 of section 10.2): what the prelude's
//! `fetch`, `EventSource`, `WebSocket`, `URL` and `TextDecoder` ask of
//! their host, and the web client's answers turned into the events 30 to
//! 41 of section 10.3.
//!
//! A script never touches the network and this module makes no network
//! rule (DEC-7260): it turns a request into the flat message
//! `protocol::web::Start`, which the partition's web client holds to the
//! channel's policy, and it holds the script to the rates and counts of
//! section 10.5, which the web client holds again as the authority. Each
//! operation that starts a request, an event stream or a socket is
//! counted before it does anything and the count is never given back
//! (section 10.1 rule 6): a request started and aborted has been counted.
//!
//! Every answer is the web client's word and is checked as hostile: as
//! its message's own reader checks it, against the request it names, and
//! against the room the script gave. An answer that breaks the protocol
//! fails its request and nothing else.

pub mod limits;

mod encoding;
mod url;

use std::collections::VecDeque;

use protocol::script::ContextId;
use protocol::web::{
    BodySource, CacheMode, Credentials, ErrorKind, FromWebClient, Head, MAX_CLOSE_REASON_BYTES, Mode, RedirectMode, Referrer, ReferrerPolicy, RequestKind,
    ResponseType, Start, ToWebClient,
};

use self::encoding::Decoders;
use self::limits::{
    FETCH_WINDOW_BYTES, MAX_ANSWERS_WAITING, MAX_CHUNK_BYTES, MAX_EVENT_STREAM_OPENS_PER_STEP, MAX_EVENT_STREAMS, MAX_KEEPALIVE_BODY_BYTES,
    MAX_NETWORK_BUFFER_BYTES, MAX_REQUEST_BODY_BYTES, MAX_REQUESTS_IN_FLIGHT, MAX_REQUESTS_PER_SECOND, MAX_REQUESTS_PER_STEP, MAX_REQUESTS_QUEUED,
    MAX_RESPONSE_BUFFER_BYTES, MAX_SOCKET_OPENS_PER_MINUTE, MAX_SOCKET_OPENS_PER_STEP, MAX_SOCKETS, MAX_WS_BUFFERED_BYTES, MAX_WS_MESSAGE_BYTES,
    TICKS_PER_MINUTE, WS_ABNORMAL, WS_NO_CODE, WS_TOO_BIG,
};
use super::{Host, InputPayload, Ints, LEVEL_WARN, Payload, RateWindow, Reply, range, text_of, type_error};
use crate::limits::{CONTEXTS, MAX_HOST_BYTES, MAX_URL_BYTES};
use crate::link::Context as _;
use crate::ops::{EventKind, Op};

/// The longest payload of a network event as a context is given it: an
/// answer's head, whose header values are written a character a byte and
/// so may be twice their bytes as text; a whole event of an event stream;
/// a piece of a body. The module's scratch space is this long.
pub const MAX_NETWORK_EVENT_BYTES: usize = 256 * 1024;
const _: () = assert!(
    MAX_URL_BYTES + protocol::web::MAX_STATUS_TEXT_BYTES + 2 * protocol::web::MAX_HEADER_BYTES + 2 * protocol::web::MAX_RESPONSE_HEADERS + 2
        <= MAX_NETWORK_EVENT_BYTES
);
const _: () = assert!(
    protocol::web::MAX_EVENT_TYPE_BYTES + protocol::web::MAX_EVENT_ID_BYTES + protocol::web::MAX_EVENT_DATA_BYTES <= MAX_NETWORK_EVENT_BYTES
        && MAX_CHUNK_BYTES <= MAX_NETWORK_EVENT_BYTES
);
// One call's payload may be cut into several of the web client's pieces,
// never more than a few.
const _: () = assert!(MAX_HOST_BYTES <= 4 * MAX_CHUNK_BYTES);

/// What ends each of the first five fields of operation `FETCH`'s head: a
/// NUL, which none of them may hold.
const SEPARATOR: char = '\0';
/// The referrer that names the client (Fetch standard section 2.2.1,
/// "request referrer": the default, `client`), as `Request.referrer`
/// spells it.
const REFERRER_CLIENT: &str = "about:client";
/// The method of an event stream and of a socket's handshake.
const GET: &str = "GET";
/// The header an event stream asks with (HTML section 9.2.2).
const ACCEPT_EVENT_STREAM: (&str, &[u8]) = ("Accept", b"text/event-stream");
/// What a held message is counted as costing beside its bytes.
const MESSAGE_OVERHEAD: usize = size_of::<ToWebClient>();

/// The bits of operation `FETCH`'s flags (section 10.2: "kind, mode,
/// credentials, cache, redirect, referrer policy, keepalive, body
/// follows"), each field at its shift with its mask.
pub(crate) mod fetch_flag {
    /// 0 a fetch, 1 an event stream.
    pub const KIND: (i32, i32) = (0, 1);
    /// `protocol::web::Mode`'s code.
    pub const MODE: (i32, i32) = (1, 1);
    /// `protocol::web::Credentials`'s code.
    pub const CREDENTIALS: (i32, i32) = (2, 3);
    /// `protocol::web::CacheMode`'s code.
    pub const CACHE: (i32, i32) = (4, 7);
    /// `protocol::web::RedirectMode`'s code.
    pub const REDIRECT: (i32, i32) = (7, 3);
    /// `protocol::web::ReferrerPolicy`'s code.
    pub const POLICY: (i32, i32) = (9, 15);
    /// The Fetch standard's keepalive flag.
    pub const KEEPALIVE: (i32, i32) = (13, 1);
    /// The request has a body, whose length is the call's third number.
    pub const BODY: (i32, i32) = (14, 1);
    /// The bits above every field, which must be clear.
    pub const BITS: i32 = 15;
    /// Each field's shift, by the name the prelude knows it by.
    pub const NAMES: [(&str, i32); 8] = [
        ("KIND", KIND.0),
        ("MODE", MODE.0),
        ("CREDENTIALS", CREDENTIALS.0),
        ("CACHE", CACHE.0),
        ("REDIRECT", REDIRECT.0),
        ("POLICY", POLICY.0),
        ("KEEPALIVE", KEEPALIVE.0),
        ("BODY", BODY.0),
    ];
}

/// The bits of event `FETCH_HEAD`'s third number: the response type's
/// code in the low two, then whether a redirect was followed.
const HEAD_REDIRECTED: i32 = 4;

/// One request, event stream or socket of a container.
#[derive(Debug)]
#[expect(clippy::struct_excessive_bools, reason = "five facts about a request, each read where it is named; no two of them are one state")]
struct Record {
    id: u32,
    context: ContextId,
    kind: RequestKind,
    /// The request waiting for a place or for the rate, until it starts.
    waiting: Option<Box<Start>>,
    /// Fetch: the body's declared length, and how much of it was taken.
    body_declared: usize,
    body_taken: usize,
    /// Fetch: the body is still owed pieces.
    body_owed: bool,
    /// A piece was refused for want of a start or of room: the context is
    /// told when it may try again.
    blocked: bool,
    keepalive: bool,
    /// Fetch: the head has arrived. Event stream and socket: it is open.
    open: bool,
    /// Fetch: the room the web client has left for the answer's body.
    window: usize,
    /// Socket: bytes accepted and not yet reported written.
    buffered: u32,
    /// Socket: the bytes of the message being sent, and being received.
    sending: usize,
    receiving: usize,
    /// Socket: the script has asked for its close.
    closing: bool,
}

impl Record {
    fn new(id: u32, context: ContextId, kind: RequestKind) -> Self {
        Self {
            id,
            context,
            kind,
            waiting: None,
            body_declared: 0,
            body_taken: 0,
            body_owed: false,
            blocked: false,
            keepalive: false,
            open: false,
            window: 0,
            buffered: 0,
            sending: 0,
            receiving: 0,
            closing: false,
        }
    }

    const fn started(&self) -> bool {
        self.waiting.is_none()
    }
}

/// An event of the network for a context (section 10.3, events 30 to 41).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkEvent {
    /// The context whose request it answers.
    pub context: ContextId,
    /// Its kind.
    pub kind: EventKind,
    /// Its four whole numbers.
    pub ints: [i32; 4],
    /// Its payload.
    pub payload: InputPayload,
}

/// What the host keeps of a container's network.
#[derive(Default)]
pub struct Web {
    records: Vec<Record>,
    next_id: u32,
    /// What the scripts asked of the web client, not yet taken.
    outbox: Vec<ToWebClient>,
    outbox_bytes: usize,
    /// The web client's answers, not yet given to a context.
    answers: VecDeque<FromWebClient>,
    answer_bytes: usize,
    /// Events the host itself makes: a request that may send its body
    /// now, one the host failed.
    notices: VecDeque<NetworkEvent>,
    step_requests: u32,
    second_requests: RateWindow,
    step_streams: u32,
    step_sockets: u32,
    /// The ticks of the last minute's socket opens.
    minute_sockets: VecDeque<u64>,
    decoders: [Decoders; CONTEXTS],
    /// The callback running is a context's one handling of `close`, in
    /// which alone a keepalive request is honoured (departure D9).
    pub(super) closing: bool,
    /// The contexts that have asked the web client for anything.
    used: [bool; CONTEXTS],
}

/// The bytes a message to the web client is counted as holding.
fn start_bytes(start: &Start) -> usize {
    let headers = start.headers.iter().fold(0_usize, |sum, (name, value)| sum.saturating_add(name.len()).saturating_add(value.len()));
    let protocols = start.protocols.iter().fold(0_usize, |sum, protocol| sum.saturating_add(protocol.len()));
    let referrer = if let Referrer::Url(url) = &start.referrer { url.len() } else { 0 };
    [
        start.method.len(),
        start.url.len(),
        start.client.len(),
        start.integrity.len(),
        start.last_event_id.len(),
        headers,
        protocols,
        referrer,
        size_of::<Start>(),
    ]
    .iter()
    .fold(MESSAGE_OVERHEAD, |sum, bytes| sum.saturating_add(*bytes))
}

/// The bytes an answer is counted as holding.
fn answer_bytes(answer: &FromWebClient) -> usize {
    let held = match answer {
        FromWebClient::Head(head) => {
            head.headers.iter().fold(head.url.len().saturating_add(head.status_text.len()).saturating_add(size_of::<Head>()), |sum, (name, value)| {
                sum.saturating_add(name.len()).saturating_add(value.len())
            })
        }
        FromWebClient::Body { bytes, .. } | FromWebClient::SocketMessage { bytes, .. } => bytes.len(),
        FromWebClient::Opened { protocol, url, .. } => protocol.len().saturating_add(url.len()),
        FromWebClient::Event { event, data, last_id, .. } => event.len().saturating_add(data.len()).saturating_add(last_id.len()),
        FromWebClient::SocketClosed { reason, .. } => reason.len(),
        FromWebClient::Jar(piece) => piece.bytes.len(),
        FromWebClient::Confined | FromWebClient::End(_) | FromWebClient::Failed { .. } | FromWebClient::Retrying(_) | FromWebClient::Sent { .. } => 0,
    };
    held.saturating_add(size_of::<FromWebClient>())
}

/// The request an answer names, or none for what is not a script host's.
const fn answer_id(answer: &FromWebClient) -> Option<u32> {
    match answer {
        FromWebClient::Head(head) => Some(head.id),
        FromWebClient::Body { id, .. }
        | FromWebClient::Failed { id, .. }
        | FromWebClient::Opened { id, .. }
        | FromWebClient::Event { id, .. }
        | FromWebClient::SocketMessage { id, .. }
        | FromWebClient::SocketClosed { id, .. }
        | FromWebClient::Sent { id, .. }
        | FromWebClient::End(id)
        | FromWebClient::Retrying(id) => Some(*id),
        FromWebClient::Confined | FromWebClient::Jar(_) => None,
    }
}

/// A field of `flags` at `(shift, mask)`.
fn field((shift, mask): (i32, i32), flags: i32) -> u8 {
    u8::try_from(flags.checked_shr(shift.unsigned_abs()).unwrap_or(0) & mask).unwrap_or(0)
}

/// A request's number as a context is given it.
fn as_int(id: u32) -> i32 {
    i32::try_from(id).unwrap_or(0)
}

/// Text as a context is given bytes that are no text: a character for
/// each byte (Fetch standard section 2.2.2: a header value is a byte
/// sequence, which the API gives as a `ByteString`).
fn push_isomorphic(out: &mut String, bytes: &[u8]) {
    out.extend(bytes.iter().map(|byte| char::from(*byte)));
}

/// The bytes of a `ByteString`: a byte for each character, or none when a
/// character is past U+00FF.
fn isomorphic_bytes(text: &str) -> Option<Vec<u8>> {
    text.chars().map(|character| u8::try_from(u32::from(character)).ok()).collect()
}

/// The length of `text` in UTF-16 code units, which is how the prelude
/// cuts a string.
fn utf16_length(text: &str) -> i32 {
    i32::try_from(text.encode_utf16().count()).unwrap_or(i32::MAX)
}

/// The `TypeError` of a request, stream or socket that is not the
/// calling context's own (section 10.1 rule 4).
fn stale() -> Reply {
    type_error("no such request of this context")
}

impl Web {
    /// The bytes held for the container's network.
    #[must_use]
    pub fn state_bytes(&self) -> usize {
        let records = self
            .records
            .iter()
            .fold(0_usize, |sum, record| sum.saturating_add(size_of::<Record>()).saturating_add(record.waiting.as_deref().map_or(0, start_bytes)));
        let notices = self.notices.len().saturating_mul(size_of::<NetworkEvent>());
        let decoders = self.decoders.iter().fold(0_usize, |sum, held| sum.saturating_add(held.state_bytes()));
        [records, notices, decoders, self.outbox_bytes, self.answer_bytes, self.minute_sockets.len().saturating_mul(size_of::<u64>())]
            .iter()
            .fold(0_usize, |sum, bytes| sum.saturating_add(*bytes))
    }

    /// The bytes of the network's buffers: what [`MAX_NETWORK_BUFFER_BYTES`] bounds.
    fn buffer_bytes(&self) -> usize {
        let waiting = self.records.iter().fold(0_usize, |sum, record| sum.saturating_add(record.waiting.as_deref().map_or(0, start_bytes)));
        waiting.saturating_add(self.outbox_bytes).saturating_add(self.answer_bytes)
    }

    fn has_room(&self, more: usize) -> bool {
        self.buffer_bytes().saturating_add(more) <= MAX_NETWORK_BUFFER_BYTES
    }

    fn send(&mut self, message: ToWebClient, bytes: usize) {
        self.outbox_bytes = self.outbox_bytes.saturating_add(bytes).saturating_add(MESSAGE_OVERHEAD);
        self.outbox.push(message);
    }

    /// What the scripts asked of the web client since the last call, in
    /// the order they asked. A request whose body was held back for want
    /// of room is told it may go on.
    pub fn take_requests(&mut self) -> Vec<ToWebClient> {
        self.outbox_bytes = 0;
        self.wake_blocked();
        core::mem::take(&mut self.outbox)
    }

    /// Tells each started request whose body was refused that it may try
    /// again: event `FETCH_SENT` with no bytes.
    fn wake_blocked(&mut self) {
        for record in &mut self.records {
            if record.blocked && record.started() {
                record.blocked = false;
                self.notices.push_back(NetworkEvent {
                    context: record.context,
                    kind: EventKind::FETCH_SENT,
                    ints: [as_int(record.id), 0, 0, 0],
                    payload: InputPayload::None,
                });
            }
        }
    }

    fn record(&self, context: ContextId, id: i32, kinds: &[RequestKind]) -> Option<usize> {
        let id = u32::try_from(id).ok()?;
        self.records.iter().position(|record| record.id == id && record.context == context && kinds.contains(&record.kind))
    }

    fn count(&self, kind: RequestKind, started_only: bool) -> usize {
        self.records.iter().filter(|record| record.kind == kind && (!started_only || record.started())).count()
    }

    /// The requests under way and waiting, and the sockets and event
    /// streams open, for the budget report (section 3.7).
    pub(super) fn budget(&self) -> (usize, usize) {
        (self.count(RequestKind::Fetch, false), self.count(RequestKind::WebSocket, false).saturating_add(self.count(RequestKind::EventSource, false)))
    }

    /// Whether the rates and the places let one more request start now.
    fn may_start(&mut self, tick: u64) -> bool {
        self.count(RequestKind::Fetch, true) < MAX_REQUESTS_IN_FLIGHT && self.rate_allows(tick)
    }

    fn rate_allows(&mut self, tick: u64) -> bool {
        self.step_requests < MAX_REQUESTS_PER_STEP && self.second_requests.count(tick) < MAX_REQUESTS_PER_SECOND
    }

    /// Counts a request as started (never given back) and sends its start.
    fn start(&mut self, start: Box<Start>, tick: u64) {
        self.step_requests = self.step_requests.saturating_add(1);
        self.second_requests.add(tick);
        let (id, bytes, body) = (start.id, start_bytes(&start), start.body);
        self.send(ToWebClient::Start(start), bytes);
        // A body of no bytes is complete at once.
        if body == BodySource::Bytes(0) {
            self.send(ToWebClient::RequestBodyEnd(id), 0);
        }
    }

    /// Begins a step at `tick`: the counts of the step start again, and
    /// the requests that waited start, oldest first, as far as the rates
    /// and the places allow.
    pub(super) fn begin_step(&mut self, tick: u64) {
        self.step_requests = 0;
        self.step_streams = 0;
        self.step_sockets = 0;
        while self.minute_sockets.front().is_some_and(|at| at.saturating_add(TICKS_PER_MINUTE) <= tick) {
            self.minute_sockets.pop_front();
        }
        while let Some(place) = self.records.iter().position(|record| !record.started()) {
            if !self.may_start(tick) {
                break;
            }
            let Some(start) = self.records.get_mut(place).and_then(|record| record.waiting.take()) else { break };
            self.start(start, tick);
        }
        self.wake_blocked();
    }

    /// Ends a context: its requests, streams and sockets are forgotten
    /// and the web client is told, which aborts each of them but those
    /// with keepalive (section 11.3, `ContextCloses`); its decoders go.
    pub(super) fn close(&mut self, context: ContextId, any_open: bool) {
        self.records.retain(|record| record.context != context);
        self.notices.retain(|notice| notice.context != context);
        if let Some(decoders) = self.decoders.get_mut(context.index()) {
            *decoders = Decoders::default();
        }
        if self.used.get(context.index()).copied().unwrap_or(false) {
            self.send(ToWebClient::ContextCloses(context), 0);
        }
        if let Some(used) = self.used.get_mut(context.index()) {
            *used = false;
        }
        if !any_open {
            self.answers.clear();
            self.answer_bytes = 0;
        }
    }

    /// An answer from the web client, kept for the next step. It is
    /// checked as its message's reader checks it; one that is not a
    /// message, or that names no request of this container, is dropped.
    /// When the answers waiting are past their bounds the request the
    /// answer names is failed, since a stream with a piece missing is not
    /// the stream.
    pub fn arrive(&mut self, answer: FromWebClient) {
        let Some(id) = answer_id(&answer) else { return };
        if !self.records.iter().any(|record| record.id == id && record.started()) {
            return;
        }
        let mut written = Vec::new();
        if protocol::web::write_answer(&mut written, &answer).is_err() {
            self.fail(id, "the web client's answer was not a message");
            return;
        }
        let bytes = answer_bytes(&answer);
        if self.answers.len() >= MAX_ANSWERS_WAITING || !self.has_room(bytes) {
            self.fail(id, "MAX_NETWORK_BUFFER_BYTES: the web client sent more than the script made room for");
            return;
        }
        self.answer_bytes = self.answer_bytes.saturating_add(bytes);
        self.answers.push_back(answer);
    }

    /// Fails the request `id` from the host's side: it is aborted at the
    /// web client and forgotten, and its context is told as for any
    /// network error. `why` is for the developer log alone.
    fn fail(&mut self, id: u32, why: &'static str) {
        let Some(place) = self.records.iter().position(|record| record.id == id) else { return };
        let record = self.records.remove(place);
        if record.started() {
            self.send(ToWebClient::Abort(id), 0);
        }
        self.notices.push_back(NetworkEvent {
            context: record.context,
            kind: EventKind::FETCH_FAILED,
            ints: [as_int(id), i32::from(ErrorKind::Network.code()), 0, 0],
            payload: InputPayload::Text(why.to_owned()),
        });
    }

    /// The next event of the network for a context: the host's own
    /// notices first, then the web client's answers in their order. With
    /// it, a line for the developer log where the event is a failure.
    fn next(&mut self) -> Option<(NetworkEvent, Option<String>)> {
        loop {
            if let Some(mut notice) = self.notices.pop_front() {
                // A failure's reason is the log's, never the script's.
                let line = match core::mem::take(&mut notice.payload) {
                    InputPayload::Text(why) if notice.kind == EventKind::FETCH_FAILED => Some(format!("request {} failed: {why}", notice.ints[0])),
                    other => {
                        notice.payload = other;
                        None
                    }
                };
                return Some((notice, line));
            }
            let answer = self.answers.pop_front()?;
            self.answer_bytes = self.answer_bytes.saturating_sub(answer_bytes(&answer));
            let Some(id) = answer_id(&answer) else { continue };
            let Some(place) = self.records.iter().position(|record| record.id == id && record.started()) else { continue };
            // An answer that broke the protocol failed its request, and
            // the notice of that is what the loop finds next.
            if let Some(event) = self.answered(place, answer) {
                return Some(event);
            }
        }
    }

    /// The event the answer for the record at `place` becomes; none when
    /// the answer is not one its request may be given, and the request
    /// has been failed for it.
    fn answered(&mut self, place: usize, answer: FromWebClient) -> Option<(NetworkEvent, Option<String>)> {
        let record = self.records.get_mut(place)?;
        let (id, context, kind) = (record.id, record.context, record.kind);
        let event = |kind: EventKind, ints: [i32; 4], payload: InputPayload| Some((NetworkEvent { context, kind, ints, payload }, None));
        let number = as_int(id);
        match answer {
            FromWebClient::Head(head) if kind == RequestKind::Fetch && !record.open => {
                record.open = true;
                let mut text = String::with_capacity(head.url.len().saturating_add(head.status_text.len()).saturating_add(64));
                text.push_str(&head.url);
                text.push(SEPARATOR);
                text.push_str(&head.status_text);
                text.push(SEPARATOR);
                for (name, value) in &head.headers {
                    text.push_str(name);
                    text.push(':');
                    push_isomorphic(&mut text, value);
                    text.push('\n');
                }
                let flags = i32::from(head.response_type.code()) | if head.redirected { HEAD_REDIRECTED } else { 0 };
                event(EventKind::FETCH_HEAD, [number, i32::from(head.status), flags, 0], InputPayload::Text(text))
            }
            FromWebClient::Body { bytes, .. } if kind == RequestKind::Fetch && record.open && bytes.len() <= record.window => {
                record.window = record.window.saturating_sub(bytes.len());
                event(EventKind::FETCH_CHUNK, [number, 0, 0, 0], InputPayload::Bytes(bytes))
            }
            FromWebClient::End(_) if kind == RequestKind::Fetch && record.open => {
                self.records.remove(place);
                event(EventKind::FETCH_END, [number, 0, 0, 0], InputPayload::None)
            }
            FromWebClient::Failed { error, reason, .. } => {
                self.records.remove(place);
                // The reason is for the developer log and never the
                // script's (section 9.2): a script cannot map the
                // viewer's network by it.
                let line = format!("request {id} failed: {reason:?}");
                Some((
                    NetworkEvent { context, kind: EventKind::FETCH_FAILED, ints: [number, i32::from(error.code()), 0, 0], payload: InputPayload::None },
                    Some(line),
                ))
            }
            FromWebClient::Opened { protocol, .. } if kind == RequestKind::WebSocket && !record.open => {
                record.open = true;
                event(EventKind::WS_OPENED, [number, 0, 0, 0], InputPayload::Text(protocol))
            }
            // The payload is the stream's final address, for the origin
            // of its events (HTML section 9.2.6). When the web client
            // gives none, the prelude uses the address the stream was
            // asked for.
            FromWebClient::Opened { url, .. } if kind == RequestKind::EventSource && !record.open => {
                record.open = true;
                event(EventKind::SSE_OPENED, [number, 0, 0, 0], InputPayload::Text(url))
            }
            FromWebClient::Event { event: name, data, last_id, .. } if kind == RequestKind::EventSource && record.open => {
                let lengths = [number, utf16_length(&name), utf16_length(&last_id), 0];
                let mut text = name;
                text.push_str(&last_id);
                text.push_str(&data);
                event(EventKind::SSE_EVENT, lengths, InputPayload::Text(text))
            }
            FromWebClient::Retrying(_) if kind == RequestKind::EventSource => {
                record.open = false;
                event(EventKind::SSE_RETRYING, [number, 0, 0, 0], InputPayload::None)
            }
            FromWebClient::SocketMessage { binary, last, bytes, .. } if kind == RequestKind::WebSocket && record.open => {
                record.receiving = record.receiving.saturating_add(bytes.len());
                if record.receiving > MAX_WS_MESSAGE_BYTES {
                    // Departure D21: the socket closes with code 1009.
                    self.records.remove(place);
                    self.send(ToWebClient::SocketClose { id, code: WS_TOO_BIG, reason: String::new() }, 0);
                    return event(EventKind::WS_CLOSED, [number, i32::from(WS_TOO_BIG), 0, 0], InputPayload::Text(String::new()));
                }
                if last {
                    record.receiving = 0;
                }
                event(EventKind::WS_MESSAGE, [number, i32::from(binary), i32::from(last), 0], InputPayload::Bytes(bytes))
            }
            FromWebClient::SocketClosed { code, reason, clean, .. } if kind == RequestKind::WebSocket => {
                self.records.remove(place);
                event(EventKind::WS_CLOSED, [number, i32::from(code), i32::from(clean), 0], InputPayload::Text(reason))
            }
            FromWebClient::Sent { bytes, .. } => {
                let written = i32::try_from(bytes).unwrap_or(i32::MAX);
                if kind == RequestKind::WebSocket {
                    record.buffered = record.buffered.saturating_sub(bytes);
                    event(EventKind::WS_SENT, [number, written, 0, 0], InputPayload::None)
                } else {
                    event(EventKind::FETCH_SENT, [number, written, 0, 0], InputPayload::None)
                }
            }
            _ => {
                self.fail(id, "the web client's answer was not one this request may be given");
                None
            }
        }
    }

    /// Operation `FETCH`: a fetch or an event stream begins.
    #[expect(clippy::too_many_lines, reason = "the fields of section 11.3's `Start`, each read, checked and set in the table's order")]
    fn fetch(&mut self, context: ContextId, tick: u64, client: &str, flags: i32, body_length: i32, head: &str) -> Result<Reply, Reply> {
        if flags < 0 || flags.checked_shr(fetch_flag::BITS.unsigned_abs()).unwrap_or(1) != 0 {
            return Err(type_error("no such flag of a request"));
        }
        let kind = if field(fetch_flag::KIND, flags) == 0 { RequestKind::Fetch } else { RequestKind::EventSource };
        let wrong = |_| type_error("no such mode of a request");
        let mode = Mode::of(field(fetch_flag::MODE, flags)).map_err(wrong)?;
        let credentials = Credentials::of(field(fetch_flag::CREDENTIALS, flags)).map_err(wrong)?;
        let cache = CacheMode::of(field(fetch_flag::CACHE, flags)).map_err(wrong)?;
        let redirect = RedirectMode::of(field(fetch_flag::REDIRECT, flags)).map_err(wrong)?;
        let referrer_policy = ReferrerPolicy::of(field(fetch_flag::POLICY, flags)).map_err(wrong)?;
        let has_body = field(fetch_flag::BODY, flags) != 0;
        // Honoured for requests started in the `close` handling alone
        // (departure D9); elsewhere the request is an ordinary one.
        let keepalive = field(fetch_flag::KEEPALIVE, flags) != 0 && self.closing;
        let mut fields = head.splitn(6, SEPARATOR);
        let mut next = || fields.next().ok_or_else(|| type_error("a request's head is its method, address, referrer, integrity, last event ID and headers"));
        let (method, url, referrer, integrity, last_event_id, lines) = (next()?, next()?, next()?, next()?, next()?, next()?);
        let body_length = usize::try_from(body_length).map_err(|_| type_error("a body's length is not negative"))?;
        if body_length > MAX_REQUEST_BODY_BYTES {
            return Err(range("MAX_REQUEST_BODY_BYTES", "the request's body is too long"));
        }
        if !has_body && body_length != 0 {
            return Err(type_error("a request with no body has no body length"));
        }
        let mut headers = Vec::new();
        for line in lines.split_terminator('\n') {
            let (name, value) = line.split_once(':').ok_or_else(|| type_error("a header line is its name, a colon and its value"))?;
            let value = isomorphic_bytes(value).ok_or_else(|| type_error("a header's value is bytes"))?;
            headers.push((name.to_owned(), value));
        }
        if kind == RequestKind::EventSource {
            headers.push((ACCEPT_EVENT_STREAM.0.to_owned(), ACCEPT_EVENT_STREAM.1.to_vec()));
        }
        let referrer = match referrer {
            "" => Referrer::None,
            REFERRER_CLIENT => Referrer::Client,
            url => Referrer::Url(url.to_owned()),
        };
        let id =
            self.next_id.checked_add(1).filter(|id| i32::try_from(*id).is_ok()).ok_or_else(|| type_error("the container has made every request it may"))?;
        let body = if has_body { BodySource::Bytes(u32::try_from(body_length).unwrap_or(u32::MAX)) } else { BodySource::None };
        let start = Box::new(Start {
            id,
            context,
            kind,
            method: method.to_owned(),
            url: url.to_owned(),
            client: client.to_owned(),
            headers,
            mode,
            credentials,
            cache,
            redirect,
            referrer,
            referrer_policy,
            integrity: integrity.to_owned(),
            keepalive,
            body,
            protocols: Vec::new(),
            last_event_id: last_event_id.to_owned(),
        });
        // Held to the message's own bounds before anything is counted or
        // changed: the web client would refuse what its reader refuses.
        start.check().map_err(|error| Reply::Type(format!("the request is not one the web client takes: {error}")))?;
        if kind == RequestKind::EventSource && has_body {
            return Err(type_error("an event stream has no body"));
        }
        if keepalive {
            // Fetch standard, HTTP-network-or-cache fetch step 8.5: the
            // bodies of the keepalive requests under way are 64 kibibytes
            // together at most, or the request is a network error.
            let under_way = self.records.iter().filter(|record| record.keepalive).fold(0_usize, |sum, record| sum.saturating_add(record.body_declared));
            if under_way.saturating_add(body_length) > MAX_KEEPALIVE_BODY_BYTES {
                return Err(range("MAX_KEEPALIVE_BODY_BYTES", "the keepalive requests' bodies are too long together"));
            }
        }
        let mut record = Record::new(id, context, kind);
        record.body_declared = body_length;
        record.body_owed = has_body && body_length > 0;
        record.keepalive = keepalive;
        record.window = usize::try_from(FETCH_WINDOW_BYTES).unwrap_or(0);
        let bytes = start_bytes(&start);
        if kind == RequestKind::EventSource {
            // Counted before anything else and never given back
            // (section 10.5; departure D22): past a bound the object
            // fires `error` and is closed, which the result 0 says.
            self.step_streams = self.step_streams.saturating_add(1);
            if self.step_streams > MAX_EVENT_STREAM_OPENS_PER_STEP
                || self.count(RequestKind::EventSource, false) >= MAX_EVENT_STREAMS
                || !self.rate_allows(tick)
                || !self.has_room(bytes)
            {
                return Ok(Reply::Int(0));
            }
            self.next_id = id;
            self.mark_used(context);
            self.records.push(record);
            self.start(start, tick);
            return Ok(Reply::Int(as_int(id)));
        }
        if !self.has_room(bytes) {
            return Err(range("MAX_NETWORK_BUFFER_BYTES", "the requests waiting hold as much as they may"));
        }
        if self.records.iter().all(Record::started) && self.may_start(tick) {
            self.next_id = id;
            self.mark_used(context);
            self.records.push(record);
            self.start(start, tick);
            return Ok(Reply::Int(as_int(id)));
        }
        // Past a rate or with every place taken the request waits, in its
        // turn; past the queue's bound it is refused (departure D8).
        if self.records.iter().filter(|record| !record.started()).count() >= MAX_REQUESTS_QUEUED {
            return Err(range("MAX_REQUESTS_QUEUED", "too many requests are waiting to start"));
        }
        self.next_id = id;
        self.mark_used(context);
        record.waiting = Some(start);
        self.records.push(record);
        Ok(Reply::Int(as_int(id).saturating_neg()))
    }

    fn mark_used(&mut self, context: ContextId) {
        if let Some(used) = self.used.get_mut(context.index()) {
            *used = true;
        }
    }

    /// Operation `FETCH_BODY`: the next bytes of a request's body. 1 when
    /// they were taken; 0 when the request has not started or there is no
    /// room, and then event `FETCH_SENT` with no bytes says when to try
    /// again.
    fn fetch_body(&mut self, context: ContextId, id: i32, last: i32, bytes: &[u8]) -> Result<Reply, Reply> {
        if !(0..=1).contains(&last) {
            return Err(type_error("a piece is the last (1) or is not (0)"));
        }
        let place = self.record(context, id, &[RequestKind::Fetch]).ok_or_else(stale)?;
        let room = self.has_room(bytes.len());
        let record = self.records.get_mut(place).ok_or_else(stale)?;
        if !record.body_owed {
            return Err(type_error("the request is owed no more of its body"));
        }
        if !record.started() || !room {
            record.blocked = true;
            return Ok(Reply::Int(0));
        }
        let taken = record.body_taken.saturating_add(bytes.len());
        let id = record.id;
        if taken > record.body_declared || (last == 1) != (taken == record.body_declared) || bytes.is_empty() {
            // The body is not the one the request's start declared: the
            // request cannot be completed as it was described.
            self.fail(id, "the request's body was not the length it declared");
            return Err(type_error("the request's body is not the length it declared"));
        }
        record.body_taken = taken;
        record.body_owed = last == 0;
        for piece in bytes.chunks(MAX_CHUNK_BYTES) {
            self.send(ToWebClient::RequestBody { id, bytes: piece.to_vec() }, piece.len());
        }
        if last == 1 {
            self.send(ToWebClient::RequestBodyEnd(id), 0);
        }
        Ok(Reply::Int(1))
    }

    /// Operation `FETCH_ABORT`: a fetch or an event stream is given up. A
    /// request that never started is forgotten without a word to the web
    /// client; its count stands.
    fn fetch_abort(&mut self, context: ContextId, id: i32) -> Reply {
        let Some(place) = self.record(context, id, &[RequestKind::Fetch, RequestKind::EventSource]) else { return Reply::Undefined };
        let record = self.records.remove(place);
        if record.started() {
            self.send(ToWebClient::Abort(record.id), 0);
        }
        Reply::Undefined
    }

    /// Operation `FETCH_CREDIT`: the script consumed `bytes` of an
    /// answer's body, so the web client has room for as many more.
    fn fetch_credit(&mut self, context: ContextId, id: i32, bytes: i32) -> Result<Reply, Reply> {
        let Some(place) = self.record(context, id, &[RequestKind::Fetch]) else { return Ok(Reply::Undefined) };
        let record = self.records.get_mut(place).ok_or_else(stale)?;
        let more = usize::try_from(bytes).ok().filter(|bytes| *bytes > 0).ok_or_else(|| type_error("room is made for one byte or more"))?;
        if !record.started() {
            return Err(type_error("the request has not started"));
        }
        let window = record.window.saturating_add(more);
        if window > MAX_NETWORK_BUFFER_BYTES {
            return Err(range("MAX_NETWORK_BUFFER_BYTES", "a request has no more room than this"));
        }
        record.window = window;
        let id = record.id;
        self.send(ToWebClient::More { id, bytes: u32::try_from(more).unwrap_or(0) }, 0);
        Ok(Reply::Undefined)
    }

    /// Operation `WS_OPEN`: a socket begins. Its number, or 0 when a
    /// bound of departure D22 is passed, for which the object fires
    /// `error` and closes.
    fn ws_open(&mut self, context: ContextId, tick: u64, client: &str, address_bytes: i32, payload: &str) -> Result<Reply, Reply> {
        let (url, protocols) = usize::try_from(address_bytes)
            .ok()
            .and_then(|bytes| payload.split_at_checked(bytes))
            .ok_or_else(|| type_error("a socket is asked for by its address, of the length given, and its subprotocols"))?;
        let protocols: Vec<String> = protocols.split_terminator(',').map(str::to_owned).collect();
        let id =
            self.next_id.checked_add(1).filter(|id| i32::try_from(*id).is_ok()).ok_or_else(|| type_error("the container has made every request it may"))?;
        // The handshake's request (WebSockets standard section 3.1,
        // "establish a WebSocket connection"): a GET with credentials
        // included, no cache, no referrer, and a redirect an error.
        let start = Box::new(Start {
            id,
            context,
            kind: RequestKind::WebSocket,
            method: GET.to_owned(),
            url: url.to_owned(),
            client: client.to_owned(),
            headers: Vec::new(),
            mode: Mode::Cors,
            credentials: Credentials::Include,
            cache: CacheMode::NoStore,
            redirect: RedirectMode::Error,
            referrer: Referrer::None,
            referrer_policy: ReferrerPolicy::Unset,
            integrity: String::new(),
            keepalive: false,
            body: BodySource::None,
            protocols,
            last_event_id: String::new(),
        });
        start.check().map_err(|error| Reply::Type(format!("the socket is not one the web client takes: {error}")))?;
        let bytes = start_bytes(&start);
        self.step_sockets = self.step_sockets.saturating_add(1);
        let in_minute = u32::try_from(self.minute_sockets.len()).unwrap_or(u32::MAX);
        if self.step_sockets > MAX_SOCKET_OPENS_PER_STEP
            || in_minute >= MAX_SOCKET_OPENS_PER_MINUTE
            || self.count(RequestKind::WebSocket, false) >= MAX_SOCKETS
            || !self.has_room(bytes)
        {
            return Ok(Reply::Int(0));
        }
        self.minute_sockets.push_back(tick);
        self.next_id = id;
        self.mark_used(context);
        self.records.push(Record::new(id, context, RequestKind::WebSocket));
        self.send(ToWebClient::Start(start), bytes);
        Ok(Reply::Int(as_int(id)))
    }

    /// Operation `WS_SEND`: a piece of a message. 1 when it was taken; 0
    /// when the socket is full, for which the prelude closes it
    /// (WebSocket standard section 3.1, `send`).
    fn ws_send(&mut self, context: ContextId, id: i32, binary: i32, last: i32, bytes: &[u8]) -> Result<Reply, Reply> {
        if !(0..=1).contains(&binary) || !(0..=1).contains(&last) {
            return Err(type_error("a piece is binary or text, and the last or not"));
        }
        let place = self.record(context, id, &[RequestKind::WebSocket]).ok_or_else(stale)?;
        let room = self.has_room(bytes.len());
        let record = self.records.get_mut(place).ok_or_else(stale)?;
        if !record.open || record.closing {
            return Err(type_error("the socket is not open"));
        }
        if bytes.is_empty() && last == 0 {
            return Err(type_error("only a message's last piece may be empty"));
        }
        let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let sending = record.sending.saturating_add(bytes.len());
        if !room || sending > MAX_WS_MESSAGE_BYTES || record.buffered.saturating_add(length) > MAX_WS_BUFFERED_BYTES {
            return Ok(Reply::Int(0));
        }
        record.sending = if last == 1 { 0 } else { sending };
        record.buffered = record.buffered.saturating_add(length);
        let id = record.id;
        let pieces = bytes.len().div_ceil(MAX_CHUNK_BYTES).max(1);
        let mut rest = bytes;
        for piece in 1..=pieces {
            let (now, later) = rest.split_at(rest.len().min(MAX_CHUNK_BYTES));
            rest = later;
            self.send(ToWebClient::SocketSend { id, binary: binary == 1, last: last == 1 && piece == pieces, bytes: now.to_vec() }, now.len());
        }
        Ok(Reply::Int(1))
    }

    /// Operation `WS_CLOSE`: the script closes a socket. One that is not
    /// open yet is given up, and its context told at once that it closed.
    fn ws_close(&mut self, context: ContextId, id: i32, code: i32, reason: &str) -> Result<Reply, Reply> {
        let Some(place) = self.record(context, id, &[RequestKind::WebSocket]) else { return Ok(Reply::Undefined) };
        // WebSockets standard section 3.1, `close`: 1000, or 3000 to
        // 4999; none, written as the code that is never sent; and the
        // prelude's own close of a socket that is full.
        let code = u16::try_from(code)
            .ok()
            .filter(|code| *code == 1000 || (3000..=4999).contains(code) || *code == WS_NO_CODE || *code == WS_TOO_BIG)
            .ok_or_else(|| type_error("a close code is 1000 or 3000 to 4999"))?;
        if reason.len() > MAX_CLOSE_REASON_BYTES {
            return Err(range("MAX_CLOSE_REASON_BYTES", "a close reason is 123 bytes at most"));
        }
        let record = self.records.get_mut(place).ok_or_else(stale)?;
        let (id, open) = (record.id, record.open);
        if record.closing {
            return Ok(Reply::Undefined);
        }
        if open {
            record.closing = true;
            self.send(ToWebClient::SocketClose { id, code, reason: reason.to_owned() }, reason.len());
        } else {
            self.records.remove(place);
            self.send(ToWebClient::Abort(id), 0);
            self.notices.push_back(NetworkEvent {
                context,
                kind: EventKind::WS_CLOSED,
                ints: [as_int(id), i32::from(WS_ABNORMAL), 0, 0],
                payload: InputPayload::Text(String::new()),
            });
        }
        Ok(Reply::Undefined)
    }
}

impl Host {
    /// The network's state, for the container and the link.
    pub const fn web(&mut self) -> &mut Web {
        &mut self.web
    }

    /// The next event of the network to give a context, its failure's
    /// reason written to the developer log.
    pub fn next_network_event(&mut self) -> Option<NetworkEvent> {
        let (event, line) = self.web.next()?;
        if let Some(line) = line {
            self.log_line(LEVEL_WARN, Some(event.context), &line);
        }
        Some(event)
    }

    /// Operations 40 to 49.
    pub(super) fn web_operation(&mut self, context: ContextId, op: Op, ints: &Ints<'_>, payload: Payload<'_>) -> Result<Reply, Reply> {
        let bytes_of = |payload: Payload<'_>| match payload {
            Payload::Bytes(bytes) => Ok(bytes.to_vec()),
            Payload::None => Ok(Vec::new()),
            _ => Err(Reply::Type(format!("operation {op:?} takes bytes"))),
        };
        match op {
            Op::URL_PARSE => {
                let [input_bytes] = ints.exactly()?;
                Ok(url::parse(input_bytes, text_of(op, payload)?))
            }
            Op::URL_SET => {
                let [part, address_bytes] = ints.exactly()?;
                Ok(url::set(part, address_bytes, text_of(op, payload)?))
            }
            Op::DECODE => {
                let [index, flags, id] = ints.exactly()?;
                let (text, bytes) = match payload {
                    Payload::Text(_) => (Some(text_of(op, payload)?), None),
                    Payload::Bytes(bytes) => (None, Some(bytes)),
                    Payload::None => (None, Some(&[][..])),
                    Payload::Ints(_) => return Err(type_error("a decoding takes bytes or a string")),
                };
                let decoders = self.web.decoders.get_mut(context.index()).ok_or_else(|| type_error("no such context"))?;
                Ok(decoders.operate(index, flags, id, text, bytes))
            }
            Op::FETCH | Op::WS_OPEN => {
                let held = self.context(context).ok_or_else(|| type_error("the context is not open"))?;
                // The manifest's `network` (section 3.1): without it these
                // fail at once.
                if !held.open.flags.network {
                    return Err(type_error("the script's manifest does not ask for the network"));
                }
                // The default referrer is the script's own address
                // (departure D25), which is the client the web client is told.
                let client = held.open.script_url.clone();
                let tick = self.tick;
                self.room(MAX_HOST_BYTES.min(payload_length(payload)).saturating_add(size_of::<Record>()), 0)?;
                if op == Op::WS_OPEN {
                    let [address_bytes] = ints.exactly()?;
                    return self.web.ws_open(context, tick, &client, address_bytes, text_of(op, payload)?);
                }
                let [flags, body_length] = ints.exactly()?;
                self.web.fetch(context, tick, &client, flags, body_length, text_of(op, payload)?)
            }
            Op::FETCH_BODY => {
                let [id, last] = ints.exactly()?;
                let bytes = bytes_of(payload)?;
                self.room(bytes.len(), 0)?;
                self.web.fetch_body(context, id, last, &bytes)
            }
            Op::FETCH_ABORT => {
                let [id] = ints.exactly()?;
                super::no_payload(op, payload)?;
                Ok(self.web.fetch_abort(context, id))
            }
            Op::FETCH_CREDIT => {
                let [id, bytes] = ints.exactly()?;
                super::no_payload(op, payload)?;
                self.web.fetch_credit(context, id, bytes)
            }
            Op::WS_SEND => {
                let [id, binary, last] = ints.exactly()?;
                let bytes = bytes_of(payload)?;
                self.room(bytes.len(), 0)?;
                self.web.ws_send(context, id, binary, last, &bytes)
            }
            Op::WS_CLOSE => {
                let [id, code] = ints.exactly()?;
                self.web.ws_close(context, id, code, text_of(op, payload)?)
            }
            _ => Err(type_error("not an operation of the web APIs")),
        }
    }
}

/// The bytes of a payload.
const fn payload_length(payload: Payload<'_>) -> usize {
    match payload {
        Payload::None => 0,
        Payload::Text(bytes) | Payload::Bytes(bytes) | Payload::Ints(bytes) => bytes.len(),
    }
}

/// The tables the prelude's web part is given in front of it: the flags'
/// shifts, the decoding flags, the address's components, and the bounds
/// the prelude holds or names.
pub(crate) fn prelude_tables() -> Vec<(&'static str, Vec<(&'static str, i64)>)> {
    let wide = |bytes: usize| i64::try_from(bytes).unwrap_or(i64::MAX);
    vec![
        ("FETCH_FLAG", fetch_flag::NAMES.iter().map(|(name, shift)| (*name, i64::from(*shift))).collect()),
        ("DECODE_FLAG", encoding::flag::NAMES.iter().map(|(name, bit)| (*name, i64::from(*bit))).collect()),
        ("URL_PART", url::Part::ALL.iter().map(|(name, _, number)| (*name, i64::from(*number))).collect()),
        (
            "WEB_BOUND",
            vec![
                ("MAX_URL_BYTES", wide(MAX_URL_BYTES)),
                ("MAX_HOST_BYTES", wide(MAX_HOST_BYTES)),
                ("MAX_CHUNK_BYTES", wide(MAX_CHUNK_BYTES)),
                ("MAX_REQUEST_BODY_BYTES", wide(MAX_REQUEST_BODY_BYTES)),
                ("MAX_RESPONSE_BUFFER_BYTES", wide(MAX_RESPONSE_BUFFER_BYTES)),
                ("FETCH_WINDOW_BYTES", i64::from(FETCH_WINDOW_BYTES)),
                ("MAX_WS_MESSAGE_BYTES", wide(MAX_WS_MESSAGE_BYTES)),
                ("MAX_WS_PROTOCOLS", wide(protocol::web::MAX_WS_PROTOCOLS)),
                ("MAX_CLOSE_REASON_BYTES", wide(MAX_CLOSE_REASON_BYTES)),
                ("MAX_METHOD_BYTES", wide(protocol::web::MAX_METHOD_BYTES)),
                ("MAX_REQUEST_HEADERS", wide(protocol::web::MAX_REQUEST_HEADERS)),
                ("MAX_HEADER_NAME_BYTES", wide(protocol::web::MAX_HEADER_NAME_BYTES)),
                ("MAX_HEADER_VALUE_BYTES", wide(protocol::web::MAX_HEADER_VALUE_BYTES)),
                ("MAX_HEADER_BYTES", wide(protocol::web::MAX_HEADER_BYTES)),
                ("MAX_INTEGRITY_BYTES", wide(protocol::web::MAX_INTEGRITY_BYTES)),
                ("WS_NO_CODE", i64::from(WS_NO_CODE)),
                ("WS_TOO_BIG", i64::from(WS_TOO_BIG)),
                ("WS_ABNORMAL", i64::from(WS_ABNORMAL)),
                ("URL_FIELDS", wide(url::FIELDS)),
                ("HEAD_REDIRECTED", i64::from(HEAD_REDIRECTED)),
                ("RESPONSE_BASIC", i64::from(ResponseType::Basic.code())),
                ("RESPONSE_CORS", i64::from(ResponseType::Cors.code())),
                ("RESPONSE_OPAQUE_REDIRECT", i64::from(ResponseType::OpaqueRedirect.code())),
                ("ERROR_NETWORK", i64::from(ErrorKind::Network.code())),
                ("ERROR_ABORT", i64::from(ErrorKind::Abort.code())),
                ("ERROR_TIMEOUT", i64::from(ErrorKind::Timeout.code())),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests;
