//! One request and its answer over hyper, and the connections kept for
//! the next: HTTP/2 connections shared by the requests to their place,
//! HTTP/1.1 connections one request at a time and kept idle between. The
//! strict limits on an answer's head are held here, on top of hyper's.

use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::{Future, poll_fn};
use std::io;
use std::mem::MaybeUninit;
use std::net::IpAddr;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use hyper::body::{Body as _, Bytes, Frame, Incoming, SizeHint};
use hyper::client::conn::{http1, http2};
use protocol::web::{Header, MAX_HEADER_NAME, MAX_HEADER_VALUE, MAX_RESPONSE_HEADERS, MAX_STATUS_TEXT, Reason};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Semaphore;
use url::Url;

use super::connect::{Settings, open, resolving};
use super::limits::{IDLE_CONNECTION, MAX_IDLE_PER_PLACE, MAX_POOLED_PLACES, MAX_RESPONSE_HEAD};
use super::local::Asker;

/// The most read for hyper at once: one TLS record's plaintext, 2^14
/// bytes (RFC 8446 section 5.1).
const READ_CHUNK: usize = 1 << 14;

/// A tokio connection, for hyper. Without `unsafe`, which this crate
/// forbids: hyper offers no safe way to read into its buffer in place, so
/// what it reads is read into a chunk on the stack and copied in.
pub(crate) struct Io<T>(pub(crate) T);

impl<T: AsyncRead + Unpin> hyper::rt::Read for Io<T> {
    fn poll_read(mut self: Pin<&mut Self>, context: &mut Context<'_>, mut cursor: hyper::rt::ReadBufCursor<'_>) -> Poll<io::Result<()>> {
        let mut chunk = [MaybeUninit::<u8>::uninit(); READ_CHUNK];
        let room = cursor.remaining().min(READ_CHUNK);
        let mut read = tokio::io::ReadBuf::uninit(chunk.get_mut(..room).unwrap_or_default());
        match Pin::new(&mut self.0).poll_read(context, &mut read) {
            Poll::Ready(Ok(())) => {
                cursor.put_slice(read.filled());
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl<T: AsyncWrite + Unpin> hyper::rt::Write for Io<T> {
    fn poll_write(mut self: Pin<&mut Self>, context: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(context, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(context)
    }
}

/// Runs what HTTP/2 runs beside a connection on the web client's one
/// thread.
#[derive(Clone, Copy)]
pub(crate) struct Spawner;

impl<F: Future + 'static> hyper::rt::Executor<F> for Spawner {
    fn execute(&self, future: F) {
        tokio::task::spawn_local(async move {
            future.await;
        });
    }
}

/// A request's body: its bytes, whole, or none.
pub(crate) struct Sent(Option<Bytes>);

impl hyper::body::Body for Sent {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        Poll::Ready(self.0.take().map(|bytes| Ok(Frame::data(bytes))))
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.0.as_ref().map_or(0, |bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX)))
    }
}

/// A request as it goes to the network.
pub(crate) struct Outgoing<'a> {
    /// The method.
    pub(crate) method: &'a str,
    /// The address.
    pub(crate) url: &'a Url,
    /// Every header but `Host`, names in lower case.
    pub(crate) headers: &'a [Header],
    /// The body.
    pub(crate) body: Option<Bytes>,
    /// The page asking.
    pub(crate) page: Asker,
    /// Whether it carries credentials: a connection is kept for requests
    /// with them or for requests without, never both (the Fetch
    /// standard's connection pool key).
    pub(crate) credentials: bool,
}

/// Where connections are kept for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Place {
    secure: bool,
    host: String,
    port: u16,
    credentials: bool,
}

/// An HTTP/2 connection its place's requests share, and the address it
/// reached.
type Shared = (http2::SendRequest<Sent>, Option<IpAddr>);

/// An idle HTTP/1.1 connection.
struct Idle {
    sender: http1::SendRequest<Sent>,
    peer: Option<IpAddr>,
    since: Instant,
}

/// What an HTTP/1.1 answer's body gives back when it has been read whole:
/// its connection, for the place's next request.
struct Lease {
    pool: Rc<Pool>,
    place: Place,
    sender: http1::SendRequest<Sent>,
    peer: Option<IpAddr>,
}

/// An answer's body, as it arrives.
pub(crate) struct Body {
    incoming: Incoming,
    lease: Option<Lease>,
}

impl Body {
    /// The body's next bytes, or none at its end.
    pub(crate) async fn piece(&mut self) -> Result<Option<Bytes>, Reason> {
        loop {
            match poll_fn(|context| Pin::new(&mut self.incoming).poll_frame(context)).await {
                // Trailers are read past: the Fetch standard gives a
                // script none.
                Some(Ok(frame)) => {
                    if let Some(bytes) = frame.into_data().ok().filter(|bytes| !bytes.is_empty()) {
                        return Ok(Some(bytes));
                    }
                }
                Some(Err(error)) => return Err(reason_of(&error)),
                None => {
                    if let Some(lease) = self.lease.take() {
                        lease.pool.keep(lease.place, lease.sender, lease.peer);
                    }
                    return Ok(None);
                }
            }
        }
    }
}

/// An answer.
pub(crate) struct Answer {
    /// The status, a final one.
    pub(crate) status: u16,
    /// The status text: HTTP/1.1's reason phrase, none over HTTP/2.
    pub(crate) status_text: String,
    /// The headers, names in lower case.
    pub(crate) headers: Vec<Header>,
    /// The body.
    pub(crate) body: Body,
}

/// Why hyper failed, as the pipe says it.
fn reason_of(error: &hyper::Error) -> Reason {
    if error.is_timeout() {
        Reason::Timeout
    } else if error.is_parse() || error.is_parse_status() || error.is_incomplete_message() || error.is_body_write_aborted() {
        Reason::Protocol
    } else if error.is_user() {
        Reason::Request
    } else {
        Reason::Connection
    }
}

/// Whether a request of `method` may be sent again when a kept connection
/// turned out closed before it answered: the idempotent methods (RFC 9110
/// section 9.2.2), as the channel fetcher retries its `GET` once.
fn may_resend(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "OPTIONS" | "PUT" | "DELETE")
}

/// The request as hyper sends it: over HTTP/1.1 the path with a `Host`
/// header, over HTTP/2 the whole address.
fn request(outgoing: &Outgoing<'_>, http2: bool) -> Result<hyper::Request<Sent>, Reason> {
    let url = outgoing.url;
    let authority = url.host_str().map(|host| url.port().map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"))).ok_or(Reason::Address)?;
    let target = url.query().map_or_else(|| url.path().to_owned(), |query| format!("{}?{query}", url.path()));
    let mut builder = hyper::Request::builder().method(outgoing.method);
    builder = if http2 { builder.uri(format!("{}://{authority}{target}", url.scheme())) } else { builder.uri(target).header(hyper::header::HOST, authority) };
    for (name, value) in outgoing.headers {
        builder = builder.header(name.as_str(), value.as_slice());
    }
    builder.body(Sent(outgoing.body.clone())).map_err(|_| Reason::Request)
}

/// The answer's head, held to the web client's limits: hyper bounds the
/// head's bytes and count as it reads; each name and value is bounded
/// here, and an HTTP/1.1 answer with both `Content-Length` and
/// `Transfer-Encoding` is refused as the engine's own client refuses it
/// (RFC 9112 section 6.3, its REQ-SEC-011).
fn head(parts: &hyper::http::response::Parts) -> Result<(u16, String, Vec<Header>), Reason> {
    if parts.headers.len() > MAX_RESPONSE_HEADERS {
        return Err(Reason::TooLarge);
    }
    let mut headers = Vec::with_capacity(parts.headers.len());
    let mut total = 0_usize;
    for (name, value) in &parts.headers {
        let (name, value) = (name.as_str(), value.as_bytes());
        total = total.saturating_add(name.len()).saturating_add(value.len());
        if name.len() > MAX_HEADER_NAME || value.len() > MAX_HEADER_VALUE || total > MAX_RESPONSE_HEAD {
            return Err(Reason::TooLarge);
        }
        headers.push((name.to_owned(), value.to_vec()));
    }
    let has = |name: &str| headers.iter().any(|(header, _)| header == name);
    if parts.version < hyper::Version::HTTP_2 && has("content-length") && has("transfer-encoding") {
        return Err(Reason::Protocol);
    }
    let status = parts.status.as_u16();
    let phrase = if parts.version >= hyper::Version::HTTP_2 {
        ""
    } else {
        parts
            .extensions
            .get::<hyper::ext::ReasonPhrase>()
            .and_then(|phrase| std::str::from_utf8(phrase.as_bytes()).ok())
            .filter(|phrase| phrase.len() <= MAX_STATUS_TEXT && phrase.bytes().all(|byte| byte.is_ascii_graphic() || matches!(byte, b' ' | b'\t')))
            .or_else(|| parts.status.canonical_reason())
            .unwrap_or_default()
    };
    Ok((status, phrase.to_owned(), headers))
}

/// The connections the web client keeps. They are the process's own, and
/// a process serves one partition (the browser's DEC-7268): no connection
/// or TLS session is shared with another channel's.
pub(crate) struct Pool {
    settings: Settings,
    resolving: Arc<Semaphore>,
    idle: RefCell<HashMap<Place, Vec<Idle>>>,
    shared: RefCell<HashMap<Place, Shared>>,
}

impl Pool {
    pub(crate) fn new(settings: Settings) -> Rc<Self> {
        Rc::new(Self { settings, resolving: resolving(), idle: RefCell::default(), shared: RefCell::default() })
    }

    /// Keeps an HTTP/1.1 connection whose answer was read whole.
    fn keep(&self, place: Place, sender: http1::SendRequest<Sent>, peer: Option<IpAddr>) {
        if sender.is_closed() {
            return;
        }
        let mut idle = self.idle.borrow_mut();
        let now = Instant::now();
        idle.retain(|_, kept| {
            kept.retain(|connection| !connection.sender.is_closed() && now.saturating_duration_since(connection.since) < IDLE_CONNECTION);
            !kept.is_empty()
        });
        if !idle.contains_key(&place) && idle.len() >= MAX_POOLED_PLACES {
            // The place whose newest connection has been idle longest
            // makes room.
            let oldest = idle.iter().min_by_key(|(_, kept)| kept.iter().map(|connection| connection.since).max()).map(|(place, _)| place.clone());
            if let Some(oldest) = oldest {
                idle.remove(&oldest);
            }
        }
        let kept = idle.entry(place).or_default();
        if kept.len() < MAX_IDLE_PER_PLACE {
            kept.push(Idle { sender, peer, since: now });
        }
    }

    /// A kept connection to `place` that the page `page` may use.
    fn kept(&self, place: &Place, page: Asker) -> Option<Connection> {
        let usable = |peer: Option<IpAddr>| peer.is_none_or(|peer| self.settings.refusal(peer, page).is_none());
        {
            let mut shared = self.shared.borrow_mut();
            match shared.get(place) {
                Some((sender, peer)) if !sender.is_closed() => {
                    if usable(*peer) {
                        return Some(Connection::Http2(sender.clone()));
                    }
                }
                Some(_) => {
                    shared.remove(place);
                }
                None => {}
            }
        }
        let mut idle = self.idle.borrow_mut();
        let kept = idle.get_mut(place)?;
        let now = Instant::now();
        while let Some(connection) = kept.pop() {
            if !connection.sender.is_closed() && now.saturating_duration_since(connection.since) < IDLE_CONNECTION && usable(connection.peer) {
                return Some(Connection::Http1(connection.sender, connection.peer));
            }
        }
        None
    }

    /// A new connection to `place`.
    async fn opened(&self, place: &Place, page: Asker) -> Result<Connection, Reason> {
        let opened = open(&self.settings, &self.resolving, place.secure, &place.host, place.port, page).await?;
        let peer = opened.peer;
        if opened.http2 {
            let mut builder = http2::Builder::new(Spawner);
            builder.max_header_list_size(u32::try_from(MAX_RESPONSE_HEAD).unwrap_or(u32::MAX));
            let (sender, connection) = builder.handshake(Io(opened.stream)).await.map_err(|error| reason_of(&error))?;
            tokio::task::spawn_local(async move {
                // The connection's end, however it comes, is seen by the
                // requests on it.
                let _ = connection.await;
            });
            let mut shared = self.shared.borrow_mut();
            shared.retain(|_, (kept, _)| !kept.is_closed());
            if shared.len() < MAX_POOLED_PLACES {
                shared.insert(place.clone(), (sender.clone(), peer));
            }
            Ok(Connection::Http2(sender))
        } else {
            let mut builder = http1::Builder::new();
            // The head is read into this buffer, so it bounds the head;
            // hyper's own lowest bound on it is 8,192 bytes.
            builder.max_buf_size(MAX_RESPONSE_HEAD).max_headers(MAX_RESPONSE_HEADERS);
            let (sender, connection) = builder.handshake(Io(opened.stream)).await.map_err(|error| reason_of(&error))?;
            tokio::task::spawn_local(async move {
                let _ = connection.await;
            });
            Ok(Connection::Http1(sender, peer))
        }
    }

    /// Sends `outgoing` and reads its answer's head, on a kept connection
    /// or a new one, within the head's time limit once a connection is open.
    pub(crate) async fn send(self: &Rc<Self>, outgoing: &Outgoing<'_>) -> Result<Answer, Reason> {
        let url = outgoing.url;
        let secure = url.scheme() == "https";
        let host = match url.host().ok_or(Reason::Address)? {
            url::Host::Domain(name) => name.to_owned(),
            url::Host::Ipv4(address) => address.to_string(),
            url::Host::Ipv6(address) => address.to_string(),
        };
        let place = Place { secure, host, port: url.port_or_known_default().ok_or(Reason::Address)?, credentials: outgoing.credentials };
        let mut resent = false;
        loop {
            let (connection, reused) = match self.kept(&place, outgoing.page) {
                Some(connection) => (connection, true),
                None => (self.opened(&place, outgoing.page).await?, false),
            };
            let exchange = async {
                match connection {
                    Connection::Http2(mut sender) => {
                        poll_fn(|context| sender.poll_ready(context)).await?;
                        Ok((sender.send_request(request(outgoing, true).map_err(Failed::Reason)?).await?, None))
                    }
                    Connection::Http1(mut sender, peer) => {
                        poll_fn(|context| sender.poll_ready(context)).await?;
                        let response = sender.send_request(request(outgoing, false).map_err(Failed::Reason)?).await?;
                        Ok((response, Some(Lease { pool: Rc::clone(self), place: place.clone(), sender, peer })))
                    }
                }
            };
            match tokio::time::timeout(self.settings.head_timeout, exchange).await {
                Err(_) => return Err(Reason::Timeout),
                Ok(Err(Failed::Reason(reason))) => return Err(reason),
                // A kept connection the server had closed: once, on a new
                // one.
                Ok(Err(Failed::Hyper(error)))
                    if reused && !resent && may_resend(outgoing.method) && (error.is_canceled() || error.is_closed() || error.is_incomplete_message()) =>
                {
                    resent = true;
                }
                Ok(Err(Failed::Hyper(error))) => return Err(reason_of(&error)),
                Ok(Ok((response, lease))) => {
                    let (parts, incoming) = response.into_parts();
                    let (status, status_text, headers) = head(&parts)?;
                    return Ok(Answer { status, status_text, headers, body: Body { incoming, lease } });
                }
            }
        }
    }
}

/// A connection to send one request on.
enum Connection {
    Http1(http1::SendRequest<Sent>, Option<IpAddr>),
    Http2(http2::SendRequest<Sent>),
}

/// Why an exchange failed.
enum Failed {
    Hyper(hyper::Error),
    Reason(Reason),
}

impl From<hyper::Error> for Failed {
    fn from(error: hyper::Error) -> Self {
        Self::Hyper(error)
    }
}
