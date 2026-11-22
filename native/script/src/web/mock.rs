//! A mock web client (work package W2's step 7; script-api.md section 16,
//! "answers the network"): what stands where the partition's web client
//! stands, with no socket, so that a script's `fetch`, `EventSource` and
//! `WebSocket` can be run against answers a test or the standalone
//! runner states. It speaks the web client's own messages
//! (`protocol::web`) and keeps the web client's flow control: an
//! answer's body is sent only as far as the script has made room.
//!
//! It makes no rule of the network. What the real web client refuses
//! (another origin without CORS, the local network, a bad port) is for a
//! responder to say, as [`Answer::Fail`] with the reason the web client
//! would give; the script then sees what it would see.

use std::collections::VecDeque;

use protocol::script::ContextId;
use protocol::web::{
    BodySource, ErrorKind, FETCH_WINDOW_BYTES, FromWebClient, Head, Header, MAX_CHUNK_BYTES, Reason, RequestKind, ResponseType, Start, ToWebClient,
};

use crate::container::Container;
use crate::host::web::limits::WS_ABNORMAL;
use crate::link::ScriptLink;

/// The status of an answer that says nothing else.
const STATUS_OK: u16 = 200;

/// A request as a responder is shown it: its start, and its body whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requested {
    /// The request's start, as the script host wrote it.
    pub start: Start,
    /// Its body; empty for none.
    pub body: Vec<u8>,
}

/// An answer to a fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The status.
    pub status: u16,
    /// The status text.
    pub status_text: String,
    /// The headers the script may read.
    pub headers: Vec<Header>,
    /// The body.
    pub body: Vec<u8>,
    /// The response type.
    pub response_type: ResponseType,
    /// Whether a redirect was followed, and then the final address.
    pub redirected_to: Option<String>,
    /// The most bytes of the body in one piece.
    pub piece: usize,
}

impl Response {
    /// A 200 answer with `body` and a `Content-Type` of `content_type`.
    #[must_use]
    pub fn ok(content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: STATUS_OK,
            status_text: "OK".to_owned(),
            headers: vec![("content-type".to_owned(), content_type.as_bytes().to_vec())],
            body: body.into(),
            response_type: ResponseType::Basic,
            redirected_to: None,
            piece: MAX_CHUNK_BYTES,
        }
    }

    /// The same answer with `status` and its text.
    #[must_use]
    pub fn status(mut self, status: u16, text: &str) -> Self {
        self.status = status;
        text.clone_into(&mut self.status_text);
        self
    }

    /// The same answer with one more header.
    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.as_bytes().to_vec()));
        self
    }

    /// The same answer with its body in pieces of at most `piece` bytes.
    #[must_use]
    pub fn in_pieces(mut self, piece: usize) -> Self {
        self.piece = piece.clamp(1, MAX_CHUNK_BYTES);
        self
    }
}

/// One thing an event stream or a socket does, in its order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Nothing for this many steps.
    Wait(u32),
    /// An event stream: an event with its type (empty for `message`), its
    /// data and its last event ID.
    Event {
        /// The type.
        event: String,
        /// The data.
        data: String,
        /// The last event ID.
        last_id: String,
    },
    /// An event stream: the connection is lost and made again.
    Reconnect,
    /// A socket: a message.
    Message {
        /// Whether binary, else text.
        binary: bool,
        /// Its bytes.
        bytes: Vec<u8>,
    },
    /// A socket: the server closes it.
    Close {
        /// The code.
        code: u16,
        /// The reason.
        reason: String,
    },
    /// The stream or socket fails, as the web client's `Failed` says.
    Fail(Reason),
}

/// What a responder answers a request with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// A fetch's answer.
    Response(Response),
    /// The request fails: the error the script is given, and the reason
    /// the developer log is given.
    Fail(ErrorKind, Reason),
    /// An event stream opens and does these, in order.
    EventStream(Vec<Item>),
    /// A socket opens with this subprotocol and does these, in order;
    /// when `echo`, each message sent to it is sent back.
    Socket {
        /// The subprotocol chosen; empty for none.
        protocol: String,
        /// Whether messages are echoed.
        echo: bool,
        /// What the server does.
        items: Vec<Item>,
    },
    /// Nothing, ever: the request stays under way.
    Hold,
}

/// A request under way at the mock.
struct Live {
    id: u32,
    context: ContextId,
    kind: RequestKind,
    keepalive: bool,
    /// The start and the body so far, until the body is complete.
    asked: Option<Requested>,
    /// A fetch's body still to send, and the room for it.
    body: VecDeque<Vec<u8>>,
    room: usize,
    ending: bool,
    /// What an event stream or a socket still does, and the steps it waits.
    items: VecDeque<Item>,
    wait: u32,
    echo: bool,
    /// The message a socket is being sent, piece by piece.
    incoming: Vec<u8>,
}

/// A mock web client: see the module.
pub struct MockWebClient {
    responder: Box<dyn FnMut(&Requested) -> Answer>,
    live: Vec<Live>,
    out: Vec<FromWebClient>,
    seen: Vec<ToWebClient>,
}

impl MockWebClient {
    /// A mock that asks `responder` for each request's answer, once the
    /// request's body is complete.
    #[must_use]
    pub fn new(responder: impl FnMut(&Requested) -> Answer + 'static) -> Self {
        Self { responder: Box::new(responder), live: Vec::new(), out: Vec::new(), seen: Vec::new() }
    }

    /// A mock that answers by a table: each request whose method and
    /// address are a row's is given that row's answer; any other fails as
    /// a connection that could not be made.
    #[must_use]
    pub fn with_routes(routes: Vec<(String, String, Answer)>) -> Self {
        Self::new(move |requested| {
            routes
                .iter()
                .find(|(method, url, _)| *method == requested.start.method && *url == requested.start.url)
                .map_or(Answer::Fail(ErrorKind::Network, Reason::Connection), |(_, _, answer)| answer.clone())
        })
    }

    /// Every message the script host has sent, in order.
    #[must_use]
    pub fn seen(&self) -> &[ToWebClient] {
        &self.seen
    }

    /// The starts among them.
    #[must_use]
    pub fn starts(&self) -> Vec<&Start> {
        self.seen.iter().filter_map(|message| if let ToWebClient::Start(start) = message { Some(&**start) } else { None }).collect()
    }

    /// The requests under way.
    #[must_use]
    pub fn under_way(&self) -> usize {
        self.live.len()
    }

    /// An answer of the test's own making, sent as it is with the next step's.
    pub fn push(&mut self, answer: FromWebClient) {
        self.out.push(answer);
    }

    fn place(&self, id: u32) -> Option<usize> {
        self.live.iter().position(|live| live.id == id)
    }

    /// One message from the script host.
    pub fn take(&mut self, message: ToWebClient) {
        self.seen.push(message.clone());
        match message {
            ToWebClient::Start(start) => {
                let (id, context, kind, keepalive, body) = (start.id, start.context, start.kind, start.keepalive, start.body);
                self.live.push(Live {
                    id,
                    context,
                    kind,
                    keepalive,
                    asked: Some(Requested { start: *start, body: Vec::new() }),
                    body: VecDeque::new(),
                    room: usize::try_from(FETCH_WINDOW_BYTES).unwrap_or(0),
                    ending: false,
                    items: VecDeque::new(),
                    wait: 0,
                    echo: false,
                    incoming: Vec::new(),
                });
                if body == BodySource::None {
                    self.respond(id);
                }
            }
            ToWebClient::RequestBody { id, bytes } => {
                if let Some(asked) = self.place(id).and_then(|place| self.live.get_mut(place)).and_then(|live| live.asked.as_mut()) {
                    asked.body.extend_from_slice(&bytes);
                }
            }
            ToWebClient::RequestBodyEnd(id) => self.respond(id),
            ToWebClient::Abort(id) => {
                if let Some(place) = self.place(id) {
                    self.live.remove(place);
                    self.out.push(FromWebClient::Failed { id, error: ErrorKind::Abort, reason: Reason::Aborted });
                }
            }
            ToWebClient::More { id, bytes } => {
                if let Some(live) = self.place(id).and_then(|place| self.live.get_mut(place)) {
                    live.room = live.room.saturating_add(usize::try_from(bytes).unwrap_or(0));
                }
            }
            ToWebClient::SocketSend { id, binary, last, bytes } => {
                let Some(live) = self.place(id).and_then(|place| self.live.get_mut(place)) else { return };
                if let Ok(sent) = u32::try_from(bytes.len())
                    && sent > 0
                {
                    self.out.push(FromWebClient::Sent { id, bytes: sent });
                }
                live.incoming.extend_from_slice(&bytes);
                if last {
                    let whole = core::mem::take(&mut live.incoming);
                    if live.echo {
                        live.items.push_back(Item::Message { binary, bytes: whole });
                    }
                }
            }
            ToWebClient::SocketClose { id, code, reason } => {
                if let Some(place) = self.place(id) {
                    self.live.remove(place);
                    // The closing handshake completes with the script's
                    // own code; one that gave none is told 1005 (RFC 6455
                    // section 7.1.5), which is how the host writes none.
                    self.out.push(FromWebClient::SocketClosed { id, code, reason, clean: true });
                }
            }
            ToWebClient::ContextCloses(context) => self.live.retain(|live| live.context != context || live.keepalive),
            ToWebClient::TabCloses => self.live.retain(|live| live.keepalive),
            ToWebClient::JarKept(_) | ToWebClient::ForgetJar => {}
        }
    }

    /// Asks the responder for the answer to request `id`, whose body is complete.
    fn respond(&mut self, id: u32) {
        let Some(place) = self.place(id) else { return };
        let Some(asked) = self.live.get_mut(place).and_then(|live| live.asked.take()) else { return };
        let answer = (self.responder)(&asked);
        let Some(live) = self.live.get_mut(place) else { return };
        match answer {
            Answer::Response(response) if live.kind == RequestKind::Fetch => {
                let redirected = response.redirected_to.is_some();
                let url = response.redirected_to.unwrap_or_else(|| asked.start.url.split('#').next().unwrap_or_default().to_owned());
                let unreadable = response.response_type == ResponseType::OpaqueRedirect;
                self.out.push(FromWebClient::Head(Box::new(Head {
                    id,
                    response_type: response.response_type,
                    status: if unreadable { 0 } else { response.status },
                    status_text: if unreadable { String::new() } else { response.status_text },
                    url: if unreadable { String::new() } else { url },
                    redirected,
                    headers: if unreadable { Vec::new() } else { response.headers },
                })));
                live.body = response.body.chunks(response.piece.clamp(1, MAX_CHUNK_BYTES)).map(<[u8]>::to_vec).collect();
                live.ending = true;
            }
            Answer::EventStream(items) if live.kind == RequestKind::EventSource => {
                self.out.push(FromWebClient::Opened { id, protocol: String::new() });
                live.items = items.into();
            }
            Answer::Socket { protocol, echo, items } if live.kind == RequestKind::WebSocket => {
                self.out.push(FromWebClient::Opened { id, protocol });
                live.items = items.into();
                live.echo = echo;
            }
            Answer::Hold => {}
            Answer::Fail(error, reason) => {
                self.live.remove(place);
                self.out.push(FromWebClient::Failed { id, error, reason });
            }
            // An answer of the wrong kind for the request: the request
            // fails as one the web client could not make.
            Answer::Response(_) | Answer::EventStream(_) | Answer::Socket { .. } => {
                self.live.remove(place);
                self.out.push(FromWebClient::Failed { id, error: ErrorKind::Network, reason: Reason::Unsupported });
            }
        }
    }

    /// One step: everything the mock has to say now, in order. A fetch's
    /// body goes as far as its room; an event stream and a socket do
    /// what they do until they wait.
    pub fn step(&mut self) -> Vec<FromWebClient> {
        let mut index = 0;
        while let Some(live) = self.live.get_mut(index) {
            let id = live.id;
            let mut ended = false;
            while let Some(piece) = live.body.front() {
                if piece.len() > live.room {
                    break;
                }
                live.room = live.room.saturating_sub(piece.len());
                if let Some(bytes) = live.body.pop_front() {
                    self.out.push(FromWebClient::Body { id, bytes });
                }
            }
            if live.ending && live.body.is_empty() {
                self.out.push(FromWebClient::End(id));
                ended = true;
            }
            if live.wait > 0 {
                live.wait = live.wait.saturating_sub(1);
            } else {
                while let Some(item) = live.items.pop_front() {
                    match item {
                        Item::Wait(steps) => {
                            live.wait = steps;
                            break;
                        }
                        Item::Event { event, data, last_id } => self.out.push(FromWebClient::Event { id, event, data, last_id }),
                        Item::Reconnect => {
                            self.out.push(FromWebClient::Retrying(id));
                            self.out.push(FromWebClient::Opened { id, protocol: String::new() });
                        }
                        Item::Message { binary, bytes } => {
                            // In the pipe's pieces; an empty message is one empty last piece.
                            let pieces = bytes.len().div_ceil(MAX_CHUNK_BYTES).max(1);
                            let mut rest = bytes.as_slice();
                            for piece in 1..=pieces {
                                let (now, later) = rest.split_at(rest.len().min(MAX_CHUNK_BYTES));
                                rest = later;
                                self.out.push(FromWebClient::SocketMessage { id, binary, last: piece == pieces, bytes: now.to_vec() });
                            }
                        }
                        Item::Close { code, reason } => {
                            self.out.push(FromWebClient::SocketClosed { id, code, reason, clean: code != WS_ABNORMAL });
                            ended = true;
                            break;
                        }
                        Item::Fail(reason) => {
                            self.out.push(FromWebClient::Failed { id, error: ErrorKind::Network, reason });
                            ended = true;
                            break;
                        }
                    }
                }
            }
            if ended {
                self.live.remove(index);
            } else {
                index = index.saturating_add(1);
            }
        }
        core::mem::take(&mut self.out)
    }

    /// Carries what `link`'s scripts asked to the mock, and the mock's
    /// answers back to the link, for its next step. Called between steps.
    pub fn exchange(&mut self, link: &mut impl ScriptLink) {
        for message in link.take_requests() {
            self.take(message);
        }
        for answer in self.step() {
            link.network(answer);
        }
    }

    /// As [`Self::exchange`], with a container used directly.
    pub fn exchange_with(&mut self, container: &mut Container) {
        for message in container.take_requests() {
            self.take(message);
        }
        for answer in self.step() {
            container.network(answer);
        }
    }
}
