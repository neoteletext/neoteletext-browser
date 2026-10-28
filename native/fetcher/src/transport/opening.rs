//! A connection's opening, on its own thread, under one deadline from
//! the name's resolution to the end of the proxy's tunnel and the TLS
//! handshake (finding F-63), with each address the name resolves to
//! checked before it is connected to (finding F-62).

use std::collections::VecDeque;
use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use neoteletext_web::port::Security;

use super::cancel::hold;
use super::connection::read_socket;
use super::writer::Outgoing;
use super::{AllowedLocal, Connection, Failure, Opener, READS_AHEAD, WRITE_TIMEOUT, address, certificate_problem};

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
    pub(super) fn open(&self, host: &str, port: u16, security: Security) -> Result<Connection, Failure> {
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
        let outgoing = Outgoing::start(socket.try_clone().map_err(own)?, Arc::clone(&self.wake));
        let (chunks, incoming) = mpsc::sync_channel(READS_AHEAD);
        let wake = Arc::clone(&self.wake);
        std::thread::spawn(move || read_socket(reader, &chunks, &*wake));
        Ok(Connection { socket, tls, incoming, pending, outgoing, peer: address(ip) })
    }
}

/// Whether `ip` may not be connected to: the engine's rule (REQ-SEC-006),
/// as its agent applies it to a peer.
fn refused_local(ip: IpAddr, allowed: &AllowedLocal) -> bool {
    let ip = address(ip);
    ip.is_blocked() && !allowed.contains(&Some(ip))
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
