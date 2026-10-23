//! The engine's network port over the operating system's sockets, plain
//! or with Transport Layer Security (requirement 5.8; decision 41 chose
//! `rustls`), trusting Mozilla's root certificates. A connection is opened
//! on a thread of its own: the name is resolved, each of its addresses
//! tried, the proxy's tunnel asked for and the handshake completed there,
//! blocking, while the loop goes on and reads the outcome through
//! [`Transport::state`], as the engine's port expects; the agent's own
//! connect limit bounds the whole. Once open, another thread reads the
//! socket, blocking, and wakes the fetcher's loop the moment bytes
//! arrive, so the loop sleeps until something happens and never waits on
//! one socket while another has data (REQ-INT-135).
//!
//! With a proxy (requirement 19.10), every connection is a tunnel the
//! proxy opens, asked for by the engine's encoding before TLS starts, so
//! TLS runs end to end; a proxy that refuses fails the connection, and
//! nothing goes direct instead.

use std::collections::VecDeque;
use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{IpAddr, Shutdown, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::Duration;

use embedded_io::ErrorKind;
use neoteletext_web::port::{ConnectionId, LinkState, Received, Security, Transport};
use neoteletext_web::proxy::Proxy;
use neoteletext_web::url::IpAddress;
use protocol::fetcher::CertificateProblem;

/// How long a connection, with its TLS handshake, may take to open; the
/// agent's own limit is the engine's `DEFAULT_CONNECT_TIMEOUT_US`, ten
/// seconds. A write is held to it too.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Bytes a reader thread takes from its socket at a time.
const SOCKET_READ: usize = 16 * 1024;
/// Reads a reader thread may be ahead of the loop before it waits: with
/// `SOCKET_READ`, what one connection holds in memory at most, beyond the
/// operating system's own buffer.
const READS_AHEAD: usize = 4;

/// Wakes the fetcher's loop.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

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
/// agent treats every failure alike), only whether the proxy is to blame
/// and whether the certificate was refused (`certificate-errors.md`).
struct Failure {
    proxy: bool,
    certificate: Option<CertificateProblem>,
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

/// A connection as the loop sees it.
enum Link {
    /// Being opened on its thread, which sends the outcome here.
    Connecting(Receiver<Result<Connection, Failure>>),
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
}

impl Tcp {
    pub fn new(wake: Wake, proxy: Option<Proxy>) -> Self {
        // ring is the one provider built in, so the builder installs it.
        let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        Self { fetch: None, push: None, live: None, tls: Arc::new(tls), wake, proxy, proxy_failed: false, refused: None }
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
        let slot = self.connection(connection);
        if let Some(Link::Connecting(outcome)) = slot {
            match outcome.try_recv() {
                Ok(Ok(open)) => *slot = Some(Link::Open(open)),
                Ok(Err(failure)) => {
                    proxy_failed = failure.proxy;
                    refused = failure.certificate;
                    *slot = Some(Link::Failed);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => *slot = Some(Link::Failed),
            }
        }
        self.proxy_failed |= proxy_failed;
        self.refused = refused.or(self.refused);
        self.connection(connection)
    }
}

/// A socket to `host` and `port`: direct, or a tunnel the proxy opened,
/// trying each address the name resolves to in turn, and the host's first
/// bytes when the proxy's answer came with them (the engine's E.102). The
/// proxy's own address may be local: the user chose it.
fn reach(proxy: Option<&Proxy>, host: &str, port: u16) -> Option<(TcpStream, IpAddr, Vec<u8>)> {
    let (to_host, to_port) = proxy.map_or((host, port), |proxy| (proxy.host.as_str(), proxy.port));
    for target in (to_host, to_port).to_socket_addrs().ok()? {
        let Ok(mut socket) = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT) else { continue };
        socket.set_read_timeout(Some(CONNECT_TIMEOUT)).ok()?;
        socket.set_write_timeout(Some(CONNECT_TIMEOUT)).ok()?;
        let early = match proxy {
            Some(proxy) => proxy.open_tunnel(&mut socket, host, port).ok()?,
            None => Vec::new(),
        };
        return Some((socket, target.ip(), early));
    }
    None
}

/// Opens a connection, blocking: on the connection's own thread.
fn open(tls: &Arc<rustls::ClientConfig>, proxy: Option<&Proxy>, wake: &Wake, host: &str, port: u16, security: Security) -> Result<Connection, Failure> {
    // Any failure on the way to the proxy is the proxy's.
    fn own<E>(_: E) -> Failure {
        Failure { proxy: false, certificate: None }
    }
    // A failure of the secure handshake, and why.
    fn refused(error: &std::io::Error) -> Failure {
        Failure { proxy: false, certificate: Some(certificate_problem(error)) }
    }
    let (mut socket, ip, early) = reach(proxy, host, port).ok_or(Failure { proxy: proxy.is_some(), certificate: None })?;
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
            let mut connection = rustls::ClientConnection::new(Arc::clone(tls), name).map_err(own)?;
            let mut early = early.as_slice();
            while !early.is_empty() {
                if connection.read_tls(&mut early).map_err(own)? == 0 {
                    break;
                }
                connection.process_new_packets().map_err(|error| refused(&std::io::Error::new(std::io::ErrorKind::InvalidData, error)))?;
            }
            // A certificate that fails is a hard stop (requirement 7.11).
            while connection.is_handshaking() {
                connection.complete_io(&mut socket).map_err(|error| refused(&error))?;
            }
            Some(Box::new(connection))
        }
    };
    // From here the reader thread waits on the socket for as long as
    // it is open; the agent's own timers decide when it has been
    // silent too long, and closing it ends the wait.
    socket.set_read_timeout(None).map_err(own)?;
    let reader = socket.try_clone().map_err(own)?;
    let (chunks, incoming) = mpsc::sync_channel(READS_AHEAD);
    let wake = Arc::clone(wake);
    std::thread::spawn(move || read_socket(reader, &chunks, &*wake));
    Ok(Connection { socket, tls, incoming, pending, peer: address(ip) })
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
        let (tls, proxy, wake, host) = (Arc::clone(&self.tls), self.proxy.clone(), Arc::clone(&self.wake), host.to_owned());
        std::thread::spawn(move || {
            let opened = open(&tls, proxy.as_ref(), &wake, &host, port, security);
            // The connection was closed while opening: the socket goes,
            // and its reader thread with it.
            if let Err(mpsc::SendError(Ok(open))) = done.send(opened) {
                let _ = open.socket.shutdown(Shutdown::Both);
            }
            wake();
        });
        *self.connection(connection) = Some(Link::Connecting(outcome));
        Ok(())
    }

    fn state(&mut self, connection: ConnectionId) -> LinkState {
        match self.settle(connection) {
            None => LinkState::Closed,
            Some(Link::Connecting(_)) => LinkState::Connecting,
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
            Some(Link::Connecting(_)) => Received::Nothing,
            None | Some(Link::Failed) => Received::Closed,
        })
    }

    fn close(&mut self, connection: ConnectionId) {
        // Ends the reader thread's wait; the bytes it still holds are
        // dropped with the channel. A connection still opening is dropped
        // by its thread when it finds no one to give it to.
        if let Some(Link::Open(open)) = self.connection(connection).take() {
            let _ = open.socket.shutdown(Shutdown::Both);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Instant;

    /// What the test's proxy does with a connection.
    #[derive(Clone, Copy)]
    enum Proxying {
        HttpAllows,
        /// Allows, and the host's first bytes come in the same write.
        HttpAllowsWithBytes,
        HttpRefuses,
        SocksAllows,
        SocksRefuses,
        Silent,
    }

    /// A proxy on this computer: it answers one connection as told, then
    /// echoes what the tunnel carries.
    fn proxy(proxying: Proxying) -> (Proxy, std::thread::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let kind = if matches!(proxying, Proxying::SocksAllows | Proxying::SocksRefuses) { "socks5" } else { "http" };
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut asked = Vec::new();
            let mut byte = [0_u8; 1];
            match proxying {
                Proxying::HttpAllows | Proxying::HttpAllowsWithBytes | Proxying::HttpRefuses => {
                    while !asked.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap() == 1 {
                        asked.push(byte[0]);
                    }
                    let answer: &[u8] = match proxying {
                        Proxying::HttpAllows => b"HTTP/1.1 200 Connection established\r\n\r\n",
                        Proxying::HttpAllowsWithBytes => b"HTTP/1.1 200 Connection established\r\n\r\nfirst",
                        _ => b"HTTP/1.1 403 Forbidden\r\n\r\n",
                    };
                    socket.write_all(answer).unwrap();
                }
                Proxying::SocksAllows | Proxying::SocksRefuses => {
                    let mut greeting = [0_u8; 3];
                    socket.read_exact(&mut greeting).unwrap();
                    asked.extend(greeting);
                    socket.write_all(&[5, 0]).unwrap();
                    let mut head = [0_u8; 5];
                    socket.read_exact(&mut head).unwrap();
                    let mut rest = vec![0_u8; usize::from(head[4]) + 2];
                    socket.read_exact(&mut rest).unwrap();
                    asked.extend(head);
                    asked.extend(&rest);
                    let reply = if matches!(proxying, Proxying::SocksAllows) { 0 } else { 5 };
                    socket.write_all(&[5, reply, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
                }
                // Holds the connection and says nothing, for longer than the
                // connection may take.
                Proxying::Silent => std::thread::sleep(CONNECT_TIMEOUT + Duration::from_secs(2)),
            }
            let mut echo = [0_u8; 64];
            if let Ok(count) = socket.read(&mut echo) {
                let _ = socket.write_all(&echo[..count]);
            }
            asked
        });
        (Proxy::parse(&format!("{kind}://127.0.0.1:{port}")).unwrap(), handle)
    }

    fn tcp(proxy: Proxy) -> Tcp {
        Tcp::new(Arc::new(|| {}), Some(proxy))
    }

    /// The connection's state once it is no longer opening, within the
    /// connect limit and a little.
    fn settled(tcp: &mut Tcp, connection: ConnectionId) -> LinkState {
        let started = Instant::now();
        loop {
            match tcp.state(connection) {
                LinkState::Connecting if started.elapsed() < CONNECT_TIMEOUT + Duration::from_secs(1) => std::thread::sleep(Duration::from_millis(1)),
                state => return state,
            }
        }
    }

    /// Reads what comes back through the tunnel, within a second.
    fn echoed(tcp: &mut Tcp) -> Vec<u8> {
        let started = Instant::now();
        let mut buffer = [0_u8; 64];
        while started.elapsed() < Duration::from_secs(1) {
            if let Ok(Received::Data(count)) = tcp.receive(ConnectionId::Fetch, &mut buffer) {
                return buffer[..count].to_vec();
            }
            std::thread::yield_now();
        }
        Vec::new()
    }

    /// Requirement 19.10: an HTTP proxy is asked by CONNECT for the host
    /// and port, and the connection is its tunnel.
    #[test]
    fn a_connection_goes_through_an_http_proxy() {
        let (proxy, server) = proxy(Proxying::HttpAllows);
        let mut tcp = tcp(proxy);
        tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
        assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Open(_)));
        assert!(!tcp.take_proxy_failure());
        tcp.send(ConnectionId::Fetch, b"through").unwrap();
        assert_eq!(echoed(&mut tcp), b"through");
        let asked = String::from_utf8(server.join().unwrap()).unwrap();
        assert!(asked.starts_with("CONNECT news.example:80 HTTP/1.1\r\n"), "{asked}");
        assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Open(IpAddress::V4([127, 0, 0, 1]))), "the peer is the proxy");
    }

    /// `certificate-errors.md` section 3: each of the TLS library's
    /// certificate errors is told apart, and any other handshake failure is
    /// one kind.
    #[test]
    fn certificate_failures_are_told_apart() {
        use rustls::CertificateError as Certificate;
        let of = |error: rustls::Error| certificate_problem(&std::io::Error::new(std::io::ErrorKind::InvalidData, error));
        assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::Expired)), CertificateProblem::Expired);
        assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::NotValidYet)), CertificateProblem::NotYetValid);
        assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::UnknownIssuer)), CertificateProblem::UnknownIssuer);
        assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::NotValidForName)), CertificateProblem::WrongHost);
        assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::Revoked)), CertificateProblem::Revoked);
        assert_eq!(of(rustls::Error::InvalidCertificate(Certificate::BadSignature)), CertificateProblem::Invalid);
        assert_eq!(of(rustls::Error::DecryptError), CertificateProblem::Handshake);
        assert_eq!(certificate_problem(&std::io::Error::from(std::io::ErrorKind::ConnectionReset)), CertificateProblem::Handshake);
    }

    /// A server that does not speak TLS fails the handshake: the connection
    /// fails and the refusal is reported once, as a failed handshake.
    #[test]
    fn a_failed_handshake_is_reported_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\nnot a TLS server");
        });
        let mut tcp = Tcp::new(Arc::new(|| {}), None);
        tcp.connect(ConnectionId::Fetch, "127.0.0.1", port, Security::Tls).unwrap();
        assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed));
        assert_eq!(tcp.take_certificate_refusal(), Some(CertificateProblem::Handshake));
        assert_eq!(tcp.take_certificate_refusal(), None, "once");
        assert!(!tcp.take_proxy_failure());
        server.join().unwrap();
    }

    /// The engine's E.102: the host's bytes that arrive with the proxy's
    /// answer are the connection's first, not lost.
    #[test]
    fn the_hosts_first_bytes_after_the_proxys_answer_are_kept() {
        let (proxy, server) = proxy(Proxying::HttpAllowsWithBytes);
        let mut tcp = tcp(proxy);
        tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
        assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Open(_)));
        assert_eq!(echoed(&mut tcp), b"first");
        tcp.send(ConnectionId::Fetch, b"then").unwrap();
        assert_eq!(echoed(&mut tcp), b"then");
        server.join().unwrap();
    }

    /// Requirement 19.10: a SOCKS5 proxy, no authentication, given the
    /// host's name to resolve.
    #[test]
    fn a_connection_goes_through_a_socks5_proxy() {
        let (proxy, server) = proxy(Proxying::SocksAllows);
        let mut tcp = tcp(proxy);
        tcp.connect(ConnectionId::Fetch, "news.example", 443, Security::Plain).unwrap();
        assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Open(_)));
        tcp.send(ConnectionId::Fetch, b"socks").unwrap();
        assert_eq!(echoed(&mut tcp), b"socks");
        let asked = server.join().unwrap();
        let mut expected = vec![5, 1, 0, 5, 1, 0, 3, 12];
        expected.extend(b"news.example");
        expected.extend(443_u16.to_be_bytes());
        assert_eq!(asked, expected);
    }

    /// Requirement 19.10: a proxy that refuses fails the connection, and
    /// the fetcher hears it was the proxy; nothing goes direct.
    #[test]
    fn a_refusal_fails_the_connection() {
        for proxying in [Proxying::HttpRefuses, Proxying::SocksRefuses] {
            let (proxy, server) = proxy(proxying);
            let mut tcp = tcp(proxy);
            tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
            assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed));
            assert!(tcp.take_proxy_failure());
            assert!(!tcp.take_proxy_failure(), "said once");
            drop(server.join());
        }
        // No proxy listening at all is the proxy's failure too.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut tcp = tcp(Proxy::parse(&format!("http://127.0.0.1:{port}")).unwrap());
        tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
        assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed));
        assert!(tcp.take_proxy_failure());
    }

    /// Requirement 19.10: a proxy that does not answer fails the connection
    /// within the connect limit; and the loop is never held while it is
    /// waited on (REQ-INT-135): connecting returns at once, and the other
    /// connections go on. Before this, a silent proxy held the fetcher's
    /// one loop for the whole ten seconds.
    #[test]
    fn a_silent_proxy_fails_in_time_and_holds_nothing_up() {
        let (proxy, server) = proxy(Proxying::Silent);
        let mut tcp = tcp(proxy);
        let started = Instant::now();
        tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
        assert!(started.elapsed() < Duration::from_millis(100), "connecting returned at once: {:?}", started.elapsed());
        assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Connecting));
        assert_eq!(tcp.receive(ConnectionId::Fetch, &mut [0; 8]), Ok(Received::Nothing), "nothing yet, and no waiting");
        assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Failed));
        assert!(started.elapsed() < CONNECT_TIMEOUT + Duration::from_secs(1), "{:?}", started.elapsed());
        assert!(tcp.take_proxy_failure());
        drop(server.join());
    }

    /// Requirement 5.8: the TLS handshake, now on the connection's own
    /// thread, completes against a real host, and the connection carries
    /// a request and its answer. It needs the internet, so it runs only
    /// when asked: `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs the internet"]
    fn a_tls_connection_opens_on_its_thread_and_answers() {
        let mut tcp = Tcp::new(Arc::new(|| {}), None);
        tcp.connect(ConnectionId::Fetch, "example.com", 443, Security::Tls).unwrap();
        assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Connecting | LinkState::Open(_)));
        assert!(matches!(settled(&mut tcp, ConnectionId::Fetch), LinkState::Open(_)));
        tcp.send(ConnectionId::Fetch, b"HEAD / HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n").unwrap();
        let started = Instant::now();
        let mut answer = Vec::new();
        let mut buffer = [0_u8; 1024];
        while started.elapsed() < Duration::from_secs(10) && !answer.starts_with(b"HTTP/1.1 ") {
            match tcp.receive(ConnectionId::Fetch, &mut buffer) {
                Ok(Received::Data(count)) => answer.extend_from_slice(&buffer[..count]),
                Ok(Received::Closed) => break,
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(answer.starts_with(b"HTTP/1.1 "), "{}", String::from_utf8_lossy(&answer));
    }

    /// A connection closed while it opens is dropped by its thread, and
    /// a later connection on the same slot is its own.
    #[test]
    fn a_connection_closed_while_opening_is_let_go() {
        let (proxy, _server) = proxy(Proxying::Silent);
        let mut tcp = tcp(proxy);
        tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).unwrap();
        tcp.close(ConnectionId::Fetch);
        assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Closed));
        assert!(!tcp.take_proxy_failure(), "a closed connection's failure is nobody's");
    }
}
