//! The engine's network port over the operating system's sockets, plain
//! or with Transport Layer Security (requirement 5.8; decision 41 chose
//! `rustls`), trusting Mozilla's root certificates. A connection is opened
//! on a thread of its own: the name is resolved, each of its addresses
//! tried, the proxy's tunnel asked for and the handshake completed there,
//! blocking, while the loop goes on and reads the outcome through
//! [`Transport::state`], as the engine's port expects. One deadline, the
//! engine's connect limit, bounds the whole opening, however a server or
//! proxy spaces its bytes, and closing a connection still opening shuts
//! its socket, so its thread ends then (finding F-63). Once open, another
//! thread reads the socket, blocking, and wakes the fetcher's loop the
//! moment bytes arrive, so the loop sleeps until something happens and
//! never waits on one socket while another has data (REQ-INT-135).
//!
//! A server on this computer or the local network is reached only when
//! the channel's arguments name its address (the engine's REQ-SEC-006):
//! each address a name resolves to is checked by the engine's rule before
//! any connection to it is made, so such a server never sees the
//! handshake, or a TLS hello naming a host of the channel's choosing.
//! The engine checks the peer again once the connection is open.
//!
//! With a proxy (requirement 19.10), every connection is a tunnel the
//! proxy opens, asked for by the engine's encoding before TLS starts, so
//! TLS runs end to end; a proxy that refuses fails the connection, and
//! nothing goes direct instead.

use std::collections::VecDeque;
use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use embedded_io::ErrorKind;
use neoteletext_web::consts::{ALLOWED_LOCAL, DEFAULT_CONNECT_TIMEOUT_US};
use neoteletext_web::port::{ConnectionId, LinkState, Received, Security, Transport};
use neoteletext_web::proxy::Proxy;
use neoteletext_web::url::IpAddress;
use protocol::fetcher::CertificateProblem;

mod cancel;

use cancel::{Cancel, hold, shut};

/// How long a connection may take to open, from the name's resolution to
/// the end of its proxy's tunnel and TLS handshake: the engine's connect
/// limit, which the agent's own timer keeps too.
const CONNECT_DEADLINE: Duration = Duration::from_micros(DEFAULT_CONNECT_TIMEOUT_US);
/// How long a write to an open connection may block: the same limit.
const WRITE_TIMEOUT: Duration = CONNECT_DEADLINE;
/// Bytes a reader thread takes from its socket at a time.
const SOCKET_READ: usize = 16 * 1024;
/// Reads a reader thread may be ahead of the loop before it waits: with
/// `SOCKET_READ`, what one connection holds in memory at most, beyond the
/// operating system's own buffer.
const READS_AHEAD: usize = 4;

/// Wakes the fetcher's loop.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The addresses on this computer or the local network the channel may
/// reach: the agent's own list (`Config::allowed_local`), so both refuse
/// the same.
pub type AllowedLocal = [Option<IpAddress>; ALLOWED_LOCAL];

/// Whether `ip` may not be connected to: the engine's rule (REQ-SEC-006),
/// as its agent applies it to a peer.
fn refused_local(ip: IpAddr, allowed: &AllowedLocal) -> bool {
    let ip = address(ip);
    ip.is_blocked() && !allowed.contains(&Some(ip))
}

/// The address the operating system resolved, as the engine writes one.
pub fn address(ip: IpAddr) -> IpAddress {
    match ip {
        IpAddr::V4(v4) => IpAddress::V4(v4.octets()),
        IpAddr::V6(v6) => IpAddress::V6(v6.segments()),
    }
}

/// One open connection.
struct Connection {
    /// For writing and for shutting down; the reader thread reads a clone.
    socket: TcpStream,
    tls: Option<Box<rustls::ClientConnection>>,
    /// What the reader thread read; the channel ends when the socket does.
    incoming: Receiver<Vec<u8>>,
    /// Bytes taken from `incoming` and not yet passed on.
    pending: VecDeque<u8>,
    peer: IpAddress,
}

/// A connection did not open; the port is not told why (the engine's
/// agent treats every failure alike), only whether the proxy is to blame,
/// whether the certificate was refused (`certificate-errors.md`), and
/// whether an address was refused as local.
struct Failure {
    proxy: bool,
    certificate: Option<CertificateProblem>,
    local: bool,
}

/// The kind of a failed secure handshake, from the TLS library's error
/// (`certificate-errors.md` section 3).
fn certificate_problem(error: &std::io::Error) -> CertificateProblem {
    use rustls::CertificateError as Certificate;
    match error.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
        Some(rustls::Error::InvalidCertificate(certificate)) => match certificate {
            Certificate::Expired | Certificate::ExpiredContext { .. } => CertificateProblem::Expired,
            Certificate::NotValidYet | Certificate::NotValidYetContext { .. } => CertificateProblem::NotYetValid,
            Certificate::UnknownIssuer => CertificateProblem::UnknownIssuer,
            Certificate::NotValidForName | Certificate::NotValidForNameContext { .. } => CertificateProblem::WrongHost,
            Certificate::Revoked => CertificateProblem::Revoked,
            _ => CertificateProblem::Invalid,
        },
        _ => CertificateProblem::Handshake,
    }
}

/// A socket whose every read and write must finish by one moment: each
/// is given only the time left, so no number of slow ones together takes
/// longer.
struct Deadline<'a> {
    socket: &'a TcpStream,
    until: Instant,
}

impl Deadline<'_> {
    fn left(&self) -> std::io::Result<Duration> {
        let left = self.until.saturating_duration_since(Instant::now());
        // A zero timeout would be refused, or mean none.
        if left.is_zero() { Err(IoErrorKind::TimedOut.into()) } else { Ok(left) }
    }
}

impl Read for Deadline<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.socket.set_read_timeout(Some(self.left()?))?;
        self.socket.read(buffer)
    }
}

impl Write for Deadline<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.socket.set_write_timeout(Some(self.left()?))?;
        self.socket.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.socket.flush()
    }
}

/// The addresses `host` resolves to, if it resolves by `until`. The
/// operating system's resolver cannot be interrupted, so it runs on a
/// thread of its own, which ends when the resolver gives up, by its own
/// limits; the connection does not wait for it.
fn resolve(host: &str, port: u16, until: Instant) -> Option<Vec<SocketAddr>> {
    let (resolved, outcome) = mpsc::sync_channel(1);
    let host = host.to_owned();
    std::thread::spawn(move || {
        let _ = resolved.send((host.as_str(), port).to_socket_addrs().map(Iterator::collect::<Vec<_>>));
    });
    outcome.recv_timeout(until.saturating_duration_since(Instant::now())).ok()?.ok()
}

/// A connection as the loop sees it.
enum Link {
    /// Being opened on its thread, which sends the outcome here, and
    /// which closing shuts.
    Connecting(Receiver<Result<Connection, Failure>>, Cancel),
    Open(Connection),
    Failed,
}

/// The fetch, push and live connections.
pub struct Tcp {
    fetch: Option<Link>,
    push: Option<Link>,
    live: Option<Link>,
    tls: Arc<rustls::ClientConfig>,
    wake: Wake,
    /// Every connection goes through it, if set.
    proxy: Option<Proxy>,
    /// The proxy failed a connection since this was last asked.
    proxy_failed: bool,
    /// A certificate was refused since this was last asked, and why.
    refused: Option<CertificateProblem>,
    /// The local addresses that may be reached directly.
    allowed_local: AllowedLocal,
    /// An address was refused as local since this was last asked.
    local_refused: bool,
}

impl Tcp {
    pub fn new(wake: Wake, proxy: Option<Proxy>, allowed_local: AllowedLocal) -> Self {
        // ring is the one provider built in, so the builder installs it.
        let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        Self { fetch: None, push: None, live: None, tls: Arc::new(tls), wake, proxy, proxy_failed: false, refused: None, allowed_local, local_refused: false }
    }

    /// Whether an address was refused as local since this was last asked.
    pub fn take_local_refusal(&mut self) -> bool {
        std::mem::take(&mut self.local_refused)
    }

    /// Whether the proxy failed a connection since this was last asked.
    pub fn take_proxy_failure(&mut self) -> bool {
        std::mem::take(&mut self.proxy_failed)
    }

    /// The certificate refused since this was last asked, and why.
    pub fn take_certificate_refusal(&mut self) -> Option<CertificateProblem> {
        self.refused.take()
    }

    fn connection(&mut self, connection: ConnectionId) -> &mut Option<Link> {
        match connection {
            ConnectionId::Fetch => &mut self.fetch,
            ConnectionId::Push => &mut self.push,
            ConnectionId::Live => &mut self.live,
        }
    }

    /// The connection, once its thread has reported; the proxy's failure
    /// is noted here.
    fn settle(&mut self, connection: ConnectionId) -> &mut Option<Link> {
        let mut proxy_failed = false;
        let mut refused = None;
        let mut local_refused = false;
        let slot = self.connection(connection);
        if let Some(Link::Connecting(outcome, _)) = slot {
            match outcome.try_recv() {
                Ok(Ok(open)) => *slot = Some(Link::Open(open)),
                Ok(Err(failure)) => {
                    proxy_failed = failure.proxy;
                    refused = failure.certificate;
                    local_refused = failure.local;
                    *slot = Some(Link::Failed);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => *slot = Some(Link::Failed),
            }
        }
        self.proxy_failed |= proxy_failed;
        self.refused = refused.or(self.refused);
        self.local_refused |= local_refused;
        self.connection(connection)
    }
}

/// What a connection's thread opens it with.
struct Opener {
    tls: Arc<rustls::ClientConfig>,
    proxy: Option<Proxy>,
    allowed: AllowedLocal,
    wake: Wake,
    cancel: Cancel,
    /// When the whole opening must be done by: `CONNECT_DEADLINE` from
    /// its start.
    until: Instant,
}

impl Opener {
    /// A socket to `host` and `port`: direct, or a tunnel the proxy
    /// opened, trying each address the name resolves to in turn, and the
    /// host's first bytes when the proxy's answer came with them (the
    /// engine's E.102). Direct, an address on this computer or the local
    /// network is skipped unless allowed (finding F-62). The proxy's own
    /// address may be local: the user chose it; the names it resolves
    /// are its own, and the agent refuses local ones before asking it
    /// (the engine's REQ-WEB-032).
    fn reach(&self, host: &str, port: u16) -> Result<(TcpStream, IpAddr, Vec<u8>), Failure> {
        let proxy = self.proxy.as_ref();
        let mut failure = Failure { proxy: proxy.is_some(), certificate: None, local: false };
        let (to_host, to_port) = proxy.map_or((host, port), |proxy| (proxy.host.as_str(), proxy.port));
        let Some(targets) = resolve(to_host, to_port, self.until) else { return Err(failure) };
        for target in targets {
            if proxy.is_none() && refused_local(target.ip(), &self.allowed) {
                failure.local = true;
                continue;
            }
            let left = self.until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let Ok(socket) = TcpStream::connect_timeout(&target, left) else { continue };
            // Closed while connecting, and no one hears how it went; or no
            // handle for closing, and the opening could not be cut short.
            if !hold(&self.cancel, &socket) {
                return Err(failure);
            }
            let early = match proxy {
                Some(proxy) => proxy.open_tunnel(&mut Deadline { socket: &socket, until: self.until }, host, port).map_err(|_| Failure {
                    proxy: true,
                    certificate: None,
                    local: false,
                })?,
                None => Vec::new(),
            };
            return Ok((socket, target.ip(), early));
        }
        Err(failure)
    }

    /// Opens a connection, blocking: on the connection's own thread.
    fn open(&self, host: &str, port: u16, security: Security) -> Result<Connection, Failure> {
        // A failure once the socket is reached is not the proxy's.
        fn own<E>(_: E) -> Failure {
            Failure { proxy: false, certificate: None, local: false }
        }
        // A failure of the secure handshake, and why.
        fn refused(error: &std::io::Error) -> Failure {
            Failure { proxy: false, certificate: Some(certificate_problem(error)), local: false }
        }
        let (socket, ip, early) = self.reach(host, port)?;
        // An input message goes in its own packet at once (`interaction.md`
        // section 10).
        socket.set_nodelay(true).map_err(own)?;
        // The host's bytes that came with the proxy's answer come first: a
        // plain connection's data, or a TLS server's records.
        let mut pending = VecDeque::new();
        let tls = match security {
            Security::Plain => {
                pending.extend(early);
                None
            }
            Security::Tls => {
                let name = rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(own)?;
                let mut connection = rustls::ClientConnection::new(Arc::clone(&self.tls), name).map_err(own)?;
                let mut early = early.as_slice();
                while !early.is_empty() {
                    if connection.read_tls(&mut early).map_err(own)? == 0 {
                        break;
                    }
                    connection.process_new_packets().map_err(|error| refused(&std::io::Error::new(std::io::ErrorKind::InvalidData, error)))?;
                }
                // A certificate that fails is a hard stop (requirement 7.11).
                let mut bounded = Deadline { socket: &socket, until: self.until };
                while connection.is_handshaking() {
                    connection.complete_io(&mut bounded).map_err(|error| refused(&error))?;
                }
                Some(Box::new(connection))
            }
        };
        // From here the reader thread waits on the socket for as long as
        // it is open; the agent's own timers decide when it has been
        // silent too long, and closing it ends the wait.
        socket.set_read_timeout(None).map_err(own)?;
        socket.set_write_timeout(Some(WRITE_TIMEOUT)).map_err(own)?;
        let reader = socket.try_clone().map_err(own)?;
        let (chunks, incoming) = mpsc::sync_channel(READS_AHEAD);
        let wake = Arc::clone(&self.wake);
        std::thread::spawn(move || read_socket(reader, &chunks, &*wake));
        Ok(Connection { socket, tls, incoming, pending, peer: address(ip) })
    }
}

/// The reader thread: passes on what the socket gives until it ends, or
/// until the connection is closed and nothing takes the bytes.
fn read_socket(mut socket: TcpStream, chunks: &SyncSender<Vec<u8>>, wake: &(dyn Fn() + Send + Sync)) {
    let mut buffer = vec![0_u8; SOCKET_READ];
    loop {
        let count = match socket.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == IoErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let Some(bytes) = buffer.get(..count) else { break };
        if chunks.send(bytes.to_vec()).is_err() {
            return;
        }
        wake();
    }
    // The loop sees the end as the channel's: this thread's sender is the
    // only one.
    wake();
}

/// Whether bytes from the reader thread are pending.
enum Refill {
    Ready,
    Waiting,
    Ended,
}

/// Takes the reader thread's next bytes when none are pending.
fn refill(pending: &mut VecDeque<u8>, incoming: &Receiver<Vec<u8>>) -> Refill {
    if !pending.is_empty() {
        return Refill::Ready;
    }
    match incoming.try_recv() {
        Ok(bytes) => {
            pending.extend(bytes);
            Refill::Ready
        }
        Err(TryRecvError::Empty) => Refill::Waiting,
        Err(TryRecvError::Disconnected) => Refill::Ended,
    }
}

impl Connection {
    fn receive(&mut self, buffer: &mut [u8]) -> Received {
        let Self { socket, tls, incoming, pending, .. } = self;
        let Some(tls) = tls.as_deref_mut() else {
            return match refill(pending, incoming) {
                Refill::Waiting => Received::Nothing,
                Refill::Ended => Received::Closed,
                Refill::Ready => pending.read(buffer).map_or(Received::Closed, Received::Data),
            };
        };
        loop {
            match tls.reader().read(buffer) {
                // The server's close_notify.
                Ok(0) => return Received::Closed,
                Ok(count) => return Received::Data(count),
                Err(error) if error.kind() == IoErrorKind::WouldBlock => {}
                Err(_) => return Received::Closed,
            }
            // Nothing decrypted waits: more records, if the socket has any.
            match refill(pending, incoming) {
                Refill::Ready => {}
                Refill::Waiting => return Received::Nothing,
                Refill::Ended => return Received::Closed,
            }
            // `pending` is not empty, which to rustls would be the end; a
            // read that takes nothing from it would never finish.
            if !matches!(tls.read_tls(pending), Ok(taken) if taken > 0) || tls.process_new_packets().is_err() {
                return Received::Closed;
            }
            // An alert or a key update may need an answer.
            if write_records(tls, socket).is_err() {
                return Received::Closed;
            }
        }
    }
}

/// Writes whatever records rustls has ready, blocking, within the write
/// limit.
fn write_records(tls: &mut rustls::ClientConnection, mut socket: &TcpStream) -> std::io::Result<()> {
    while tls.wants_write() {
        tls.write_tls(&mut socket)?;
    }
    socket.flush()
}

impl embedded_io::ErrorType for Tcp {
    type Error = ErrorKind;
}

impl Transport for Tcp {
    /// Starts opening the connection on a thread of its own, and returns
    /// at once; [`Transport::state`] says how it went.
    fn connect(&mut self, connection: ConnectionId, host: &str, port: u16, security: Security) -> Result<(), ErrorKind> {
        self.close(connection);
        let (done, outcome) = mpsc::sync_channel(1);
        let now = Instant::now();
        let opener = Opener {
            tls: Arc::clone(&self.tls),
            proxy: self.proxy.clone(),
            allowed: self.allowed_local,
            wake: Arc::clone(&self.wake),
            cancel: Cancel::default(),
            // A clock that cannot be read that far ahead leaves no time.
            until: now.checked_add(CONNECT_DEADLINE).unwrap_or(now),
        };
        let cancel = Arc::clone(&opener.cancel);
        let host = host.to_owned();
        std::thread::spawn(move || {
            let reached = opener.open(&host, port, security);
            // The connection was closed while opening: the socket goes,
            // and its reader thread with it.
            if let Err(mpsc::SendError(Ok(open))) = done.send(reached) {
                let _ = open.socket.shutdown(Shutdown::Both);
            }
            (opener.wake)();
        });
        *self.connection(connection) = Some(Link::Connecting(outcome, cancel));
        Ok(())
    }

    fn state(&mut self, connection: ConnectionId) -> LinkState {
        match self.settle(connection) {
            None => LinkState::Closed,
            Some(Link::Connecting(..)) => LinkState::Connecting,
            Some(Link::Open(open)) => LinkState::Open(open.peer),
            Some(Link::Failed) => LinkState::Failed,
        }
    }

    fn send(&mut self, connection: ConnectionId, bytes: &[u8]) -> Result<(), ErrorKind> {
        let Some(Link::Open(open)) = self.settle(connection) else { return Err(ErrorKind::NotConnected) };
        // Written whole, blocking, within the write limit: a request or a
        // input message is small, and the socket drains it at once.
        let mut socket = &open.socket;
        let written = match open.tls.as_deref_mut() {
            None => socket.write_all(bytes).and_then(|()| socket.flush()),
            Some(tls) => tls.writer().write_all(bytes).and_then(|()| write_records(tls, socket)),
        };
        written.map_err(|_| ErrorKind::BrokenPipe)
    }

    fn receive(&mut self, connection: ConnectionId, buffer: &mut [u8]) -> Result<Received, ErrorKind> {
        Ok(match self.settle(connection) {
            Some(Link::Open(open)) => open.receive(buffer),
            Some(Link::Connecting(..)) => Received::Nothing,
            None | Some(Link::Failed) => Received::Closed,
        })
    }

    fn close(&mut self, connection: ConnectionId) {
        // Open, this ends the reader thread's wait; the bytes it still
        // holds are dropped with the channel. Still opening, its thread's
        // socket is shut, and a connection it opens all the same is
        // dropped when it finds no one to give it to.
        match self.connection(connection).take() {
            Some(Link::Open(open)) => {
                let _ = open.socket.shutdown(Shutdown::Both);
            }
            Some(Link::Connecting(_, opening)) => shut(&opening),
            Some(Link::Failed) | None => {}
        }
    }
}

#[cfg(test)]
mod tests;
