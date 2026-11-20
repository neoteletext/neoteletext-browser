//! The channel's shared HTTP/3 connection (REQ-6989, neoteletext-common's
//! DEC-6979): one QUIC connection for the whole process, on which each
//! tab's push WebSocket is a stream of its own. It is opened when the first
//! tab's agent asks for a stream, joined by the tabs that ask after, and
//! made again when it has been lost. A connection that cannot be made is
//! remembered for [`HTTP3_RETRY_AFTER`], and every tab is told meanwhile
//! that no stream is available, so its agent opens its own HTTP/1.1
//! connection at once.
//!
//! The connection itself is the engine's
//! `neoteletext_transport::quic::Http3Connection`. What is this process's
//! alone is here: which addresses may be reached, checked as for a TCP
//! connection before any datagram is sent (the engine's REQ-SEC-006,
//! finding F-62), and the failure remembered. Nothing here is used with a
//! proxy or with sharing turned off: the process is then started with no
//! shared connection at all (requirement 19.10, REQ-6982).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use neoteletext_transport::quic::{ConnectionState, Http3Connection, Lost, QuicTls, StreamRequest, StreamState, WebSocketStream, quic_tls};
use neoteletext_transport::web_address;
use neoteletext_web::consts::PUSH_UNSENT_BOUND;
use neoteletext_web::url::IpAddress;

use super::opening::{Resolution, refused_local, resolve};
use super::writer::MAX_UNSENT;
use super::{AllowedLocal, Wake};

/// How long a connection that could not be made is not tried again
/// (REQ-6989 item 5, DEC-6990 item 5): long enough that a path which does
/// not carry UDP costs a viewer one wait and not one for every tab, short
/// enough that a server which gains HTTP/3, or a network left behind, is
/// found again within the same sitting.
pub(crate) const HTTP3_RETRY_AFTER: Duration = Duration::from_mins(5);

/// What a tab's agent asked for: the request of its push stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PushRequest {
    /// The channel's host: the name resolved, and the name its certificate
    /// is checked against.
    pub(crate) host: String,
    /// The channel's port, the one UDP port the process may reach.
    pub(crate) port: u16,
    /// `:authority`.
    pub(crate) authority: Vec<u8>,
    /// `:path`.
    pub(crate) path: String,
    /// The subprotocol offered.
    pub(crate) subprotocol: String,
    /// `user-agent`.
    pub(crate) user_agent: Vec<u8>,
}

/// A tab's stream, answered by the server.
pub(super) struct StreamLink {
    pub(super) stream: WebSocketStream,
    /// The address the shared connection reached.
    pub(super) peer: IpAddress,
    /// The answer's status and subprotocol, until the tab's agent is given
    /// them.
    pub(super) answer: Option<(u16, Option<Vec<u8>>)>,
}

/// Why no stream was opened, in words for the console.
pub(super) type Unavailable = String;

struct State {
    connection: Option<Http3Connection>,
    /// No connection is tried before this moment: the last could not be
    /// made.
    failed_until: Option<Instant>,
}

/// The process's shared connection, and whether it may have one.
pub(crate) struct Shared {
    tls: QuicTls,
    allowed: AllowedLocal,
    state: Mutex<State>,
    /// The process's count of names being resolved is each tab's own; the
    /// shared connection's resolutions are counted apart, one at a time
    /// under the state's lock.
    resolving: Arc<AtomicUsize>,
    /// How many connections this process has started.
    made: AtomicUsize,
}

/// A tab's handle on the process's shared connection; `None` in a process
/// that may have none.
pub(crate) type SharedConnection = Option<Arc<Shared>>;

/// Why an attempt at a stream gave none.
enum Ended {
    /// The connection it was asked for on had been ready and ended.
    WithItsConnection,
    /// Anything else, in words.
    Because(Unavailable),
}

impl Ended {
    fn said(self) -> Unavailable {
        match self {
            Self::WithItsConnection => String::from("the shared connection ended"),
            Self::Because(why) => why,
        }
    }
}

/// Wakes a thread waiting for a stream's answer, and the tab's loop.
struct Arrival {
    came: Mutex<bool>,
    signal: Condvar,
}

impl Shared {
    /// The shared connection of a process that may have one, trusting
    /// `roots` as its TCP connections do; `None` when the TLS library
    /// cannot serve QUIC, which the `ring` provider always can.
    pub(crate) fn new(roots: rustls::RootCertStore, allowed: AllowedLocal) -> SharedConnection {
        let tls = quic_tls(roots).ok()?;
        Some(Arc::new(Self {
            tls,
            allowed,
            state: Mutex::new(State { connection: None, failed_until: None }),
            resolving: Arc::new(AtomicUsize::new(0)),
            made: AtomicUsize::new(0),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a stream may be asked for now: no failure is remembered.
    pub(crate) fn usable(&self) -> bool {
        self.lock().failed_until.is_none_or(|until| Instant::now() >= until)
    }

    /// Forgets a remembered failure: the channel's server could not be
    /// reached over TCP either, so what was remembered says nothing of the
    /// path's UDP, and the next push connection may be a stream again.
    pub(crate) fn forget_failure(&self) {
        self.lock().failed_until = None;
    }

    /// Remembers that the connection could not be made.
    fn remember_failure(&self, failed: &Http3Connection) {
        let mut state = self.lock();
        // Only the connection that failed: a tab that waited on it may
        // report after another has already made the next.
        if state.connection.as_ref().is_some_and(|held| same(held, failed)) {
            state.connection = None;
        }
        let now = Instant::now();
        state.failed_until = Some(now.checked_add(HTTP3_RETRY_AFTER).unwrap_or(now));
    }

    /// The connection to open a stream on: the one held while it is of
    /// use, else a new one to the first address of `host` that may be
    /// reached.
    fn connection(&self, request: &PushRequest, until: Instant) -> Result<Http3Connection, Unavailable> {
        let mut state = self.lock();
        if state.failed_until.is_some_and(|retry| Instant::now() < retry) {
            return Err(String::from("its last attempt failed and is not yet tried again"));
        }
        if let Some(held) = &state.connection {
            if !matches!(held.state(), ConnectionState::Lost(_)) {
                return Ok(held.clone());
            }
            // It was ready and has ended: the next is made at once
            // (REQ-6989 item 4).
            state.connection = None;
        }
        let targets = match resolve(&request.host, request.port, until, &self.resolving) {
            Resolution::Addresses(targets) => targets,
            Resolution::Failed => return Err(format!("{} does not resolve", request.host)),
            Resolution::TimedOut => return Err(format!("{} did not resolve in time", request.host)),
        };
        // Each address is checked before a datagram is sent to it, as
        // before a TCP connection (finding F-62).
        let target: SocketAddr = targets
            .into_iter()
            .find(|target| !refused_local(target.ip(), &self.allowed))
            .ok_or_else(|| format!("{} leads only to addresses this channel may not reach", request.host))?;
        let connection = Http3Connection::connect(&self.tls, target, &request.host, Arc::new(|| {}))
            .map_err(|error| format!("no QUIC connection could be started: {error}"))?;
        state.connection = Some(connection.clone());
        self.made.fetch_add(1, Ordering::Relaxed);
        Ok(connection)
    }

    /// Opens a tab's push stream and waits for the server's answer, until
    /// `until`: on the tab's opening thread, as a TCP connection is opened.
    /// `wake` wakes the tab's loop for everything the stream brings.
    pub(super) fn open(&self, request: &PushRequest, until: Instant, wake: &Wake) -> Result<StreamLink, Unavailable> {
        // The connection held may have ended without this side yet knowing,
        // as when its server restarted: a stream that ends with it is asked
        // for once more, on the connection made in its place (REQ-6989
        // item 4).
        match self.open_on(request, until, wake) {
            Err(Ended::WithItsConnection) => self.open_on(request, until, wake).map_err(Ended::said),
            opened => opened.map_err(Ended::said),
        }
    }

    /// One attempt at [`Self::open`], on the connection held or a new one.
    fn open_on(&self, request: &PushRequest, until: Instant, wake: &Wake) -> Result<StreamLink, Ended> {
        let connection = self.connection(request, until).map_err(Ended::Because)?;
        let arrival = Arc::new(Arrival { came: Mutex::new(false), signal: Condvar::new() });
        let (arrived, tab) = (Arc::clone(&arrival), Arc::clone(wake));
        let stream = connection
            .open_websocket(&StreamRequest {
                authority: &request.authority,
                path: request.path.as_bytes(),
                subprotocol: request.subprotocol.as_bytes(),
                user_agent: &request.user_agent,
                wake: Arc::new(move || {
                    *arrived.came.lock().unwrap_or_else(PoisonError::into_inner) = true;
                    arrived.signal.notify_all();
                    tab();
                }),
                // As a TCP connection's writer thread: the loop hears when
                // the unsent bytes fall under the bound at which the agent
                // holds a pointer's moves (REQ-RCH-035), and a send past
                // the most a stalled connection may hold fails.
                wake_under: PUSH_UNSENT_BOUND,
                max_unsent: MAX_UNSENT,
            })
            .ok_or(Ended::WithItsConnection)?;
        loop {
            match stream.state() {
                StreamState::Answered { status, subprotocol } => {
                    return Ok(StreamLink { stream, peer: web_address(connection.peer().ip()), answer: Some((status, subprotocol)) });
                }
                StreamState::Ended => {
                    return Err(match connection.state() {
                        ConnectionState::Lost(Lost::Unreachable) => {
                            self.remember_failure(&connection);
                            Ended::Because(String::from("no QUIC connection was made in time"))
                        }
                        ConnectionState::Lost(Lost::NoExtendedConnect) => {
                            self.remember_failure(&connection);
                            Ended::Because(String::from("the server does not offer extended CONNECT over HTTP/3"))
                        }
                        ConnectionState::Lost(Lost::Ended) => Ended::WithItsConnection,
                        ConnectionState::Connecting | ConnectionState::Ready => Ended::Because(String::from("the stream ended before it was answered")),
                    });
                }
                StreamState::Opening => {}
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Ended::Because(String::from("the stream was not answered in time")));
            }
            let mut came = arrival.came.lock().unwrap_or_else(PoisonError::into_inner);
            if !*came {
                came = arrival.signal.wait_timeout(came, left).unwrap_or_else(PoisonError::into_inner).0;
            }
            *came = false;
        }
    }

    /// How many connections this process has started: one, until one is
    /// lost and made again.
    #[cfg(test)]
    pub(crate) fn connections_made(&self) -> usize {
        self.made.load(Ordering::Relaxed)
    }

    /// Closes the connection as the process ends: what its streams were
    /// last given leaves first, within the engine's `CLOSE_WAIT`.
    pub(crate) fn close(&self) {
        let held = self.lock().connection.take();
        if let Some(connection) = held {
            connection.close();
        }
    }
}

/// Whether two handles are of one connection: one process holds one at a
/// time, to one peer, so a connection is known by when it was made, which
/// its handles share.
fn same(held: &Http3Connection, other: &Http3Connection) -> bool {
    held.is(other)
}
