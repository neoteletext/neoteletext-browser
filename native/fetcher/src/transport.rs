//! The engine's network port over the operating system's sockets, plain
//! or with Transport Layer Security (requirement 5.8; decision 41 chose
//! `rustls`), trusting Mozilla's root certificates. A connection opens and
//! completes its handshake blocking, within the engine's own connect
//! limit. Then a thread of its own reads the socket, blocking, and wakes
//! the fetcher's loop the moment bytes arrive, so the loop sleeps until
//! something happens and never waits on one socket while another has data
//! (REQ-INT-135).
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

/// The fetch, push and live connections.
pub struct Tcp {
    fetch: Option<Connection>,
    push: Option<Connection>,
    live: Option<Connection>,
    tls: Arc<rustls::ClientConfig>,
    wake: Wake,
    /// Every connection goes through it, if set.
    proxy: Option<Proxy>,
    /// The proxy failed a connection since this was last asked.
    proxy_failed: bool,
}

impl Tcp {
    pub fn new(wake: Wake, proxy: Option<Proxy>) -> Self {
        // ring is the one provider built in, so the builder installs it.
        let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        Self { fetch: None, push: None, live: None, tls: Arc::new(tls), wake, proxy, proxy_failed: false }
    }

    /// Whether the proxy failed a connection since this was last asked.
    pub fn take_proxy_failure(&mut self) -> bool {
        std::mem::take(&mut self.proxy_failed)
    }

    /// A socket to `host` and `port`: direct, or a tunnel the proxy opened.
    /// The proxy's own address may be local: the user chose it.
    fn reach(&self, host: &str, port: u16) -> Result<(TcpStream, IpAddr), Reached> {
        let (to_host, to_port) = self.proxy.as_ref().map_or((host, port), |proxy| (proxy.host.as_str(), proxy.port));
        let target = (to_host, to_port).to_socket_addrs().map_err(|_| Reached::NotFound)?.next().ok_or(Reached::NotFound)?;
        let mut socket = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT).map_err(|_| Reached::Refused)?;
        socket.set_read_timeout(Some(CONNECT_TIMEOUT)).map_err(|_| Reached::Other)?;
        socket.set_write_timeout(Some(CONNECT_TIMEOUT)).map_err(|_| Reached::Other)?;
        if let Some(proxy) = &self.proxy {
            proxy.open_tunnel(&mut socket, host, port).map_err(|_| Reached::ProxyRefused)?;
        }
        Ok((socket, target.ip()))
    }

    fn connection(&mut self, connection: ConnectionId) -> &mut Option<Connection> {
        match connection {
            ConnectionId::Fetch => &mut self.fetch,
            ConnectionId::Push => &mut self.push,
            ConnectionId::Live => &mut self.live,
        }
    }

    fn open(&mut self, host: &str, port: u16, security: Security) -> Result<Connection, ErrorKind> {
        let (mut socket, ip) = match self.reach(host, port) {
            Ok(reached) => reached,
            Err(failure) => {
                // Any failure on the way to the proxy is the proxy's.
                self.proxy_failed |= self.proxy.is_some();
                return Err(match failure {
                    Reached::NotFound => ErrorKind::NotFound,
                    Reached::Refused | Reached::ProxyRefused => ErrorKind::ConnectionRefused,
                    Reached::Other => ErrorKind::Other,
                });
            }
        };
        // An input message goes in its own packet at once (`interaction.md`
        // section 10).
        socket.set_nodelay(true).map_err(|_| ErrorKind::Other)?;
        let tls = match security {
            Security::Plain => None,
            Security::Tls => {
                let name = rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|_| ErrorKind::InvalidInput)?;
                let mut connection = rustls::ClientConnection::new(self.tls.clone(), name).map_err(|_| ErrorKind::Other)?;
                // A certificate that fails is a hard stop (requirement 7.11).
                while connection.is_handshaking() {
                    connection.complete_io(&mut socket).map_err(|_| ErrorKind::PermissionDenied)?;
                }
                Some(Box::new(connection))
            }
        };
        // From here the reader thread waits on the socket for as long as
        // it is open; the agent's own timers decide when it has been
        // silent too long, and closing it ends the wait.
        socket.set_read_timeout(None).map_err(|_| ErrorKind::Other)?;
        let reader = socket.try_clone().map_err(|_| ErrorKind::Other)?;
        let (chunks, incoming) = mpsc::sync_channel(READS_AHEAD);
        let wake = self.wake.clone();
        std::thread::spawn(move || read_socket(reader, &chunks, &*wake));
        Ok(Connection { socket, tls, incoming, pending: VecDeque::new(), peer: address(ip) })
    }
}

/// Why no socket was reached.
enum Reached {
    NotFound,
    Refused,
    ProxyRefused,
    Other,
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
    fn connect(&mut self, connection: ConnectionId, host: &str, port: u16, security: Security) -> Result<(), ErrorKind> {
        let open = self.open(host, port, security)?;
        *self.connection(connection) = Some(open);
        Ok(())
    }

    fn state(&mut self, connection: ConnectionId) -> LinkState {
        self.connection(connection).as_ref().map_or(LinkState::Closed, |open| LinkState::Open(open.peer))
    }

    fn send(&mut self, connection: ConnectionId, bytes: &[u8]) -> Result<(), ErrorKind> {
        let open = self.connection(connection).as_mut().ok_or(ErrorKind::NotConnected)?;
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
        Ok(self.connection(connection).as_mut().map_or(Received::Closed, |open| open.receive(buffer)))
    }

    fn close(&mut self, connection: ConnectionId) {
        // Ends the reader thread's wait; the bytes it still holds are
        // dropped with the channel.
        if let Some(open) = self.connection(connection).take() {
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
                Proxying::HttpAllows | Proxying::HttpRefuses => {
                    while !asked.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap() == 1 {
                        asked.push(byte[0]);
                    }
                    let answer: &[u8] = if matches!(proxying, Proxying::HttpAllows) {
                        b"HTTP/1.1 200 Connection established\r\n\r\n"
                    } else {
                        b"HTTP/1.1 403 Forbidden\r\n\r\n"
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
        assert!(!tcp.take_proxy_failure());
        tcp.send(ConnectionId::Fetch, b"through").unwrap();
        assert_eq!(echoed(&mut tcp), b"through");
        let asked = String::from_utf8(server.join().unwrap()).unwrap();
        assert!(asked.starts_with("CONNECT news.example:80 HTTP/1.1\r\n"), "{asked}");
        assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Open(IpAddress::V4([127, 0, 0, 1]))), "the peer is the proxy");
    }

    /// Requirement 19.10: a SOCKS5 proxy, no authentication, given the
    /// host's name to resolve.
    #[test]
    fn a_connection_goes_through_a_socks5_proxy() {
        let (proxy, server) = proxy(Proxying::SocksAllows);
        let mut tcp = tcp(proxy);
        tcp.connect(ConnectionId::Fetch, "news.example", 443, Security::Plain).unwrap();
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
            assert_eq!(tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain), Err(ErrorKind::ConnectionRefused));
            assert!(tcp.take_proxy_failure());
            assert!(!tcp.take_proxy_failure(), "said once");
            assert!(matches!(tcp.state(ConnectionId::Fetch), LinkState::Closed));
            drop(server.join());
        }
        // No proxy listening at all is the proxy's failure too.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut tcp = tcp(Proxy::parse(&format!("http://127.0.0.1:{port}")).unwrap());
        assert!(tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).is_err());
        assert!(tcp.take_proxy_failure());
    }

    /// Requirement 19.10: a proxy that does not answer fails the connection
    /// within the connect limit.
    #[test]
    fn a_silent_proxy_fails_in_time() {
        let (proxy, server) = proxy(Proxying::Silent);
        let mut tcp = tcp(proxy);
        let started = Instant::now();
        assert!(tcp.connect(ConnectionId::Fetch, "news.example", 80, Security::Plain).is_err());
        assert!(started.elapsed() < CONNECT_TIMEOUT + Duration::from_secs(1), "{:?}", started.elapsed());
        assert!(tcp.take_proxy_failure());
        drop(server.join());
    }
}
