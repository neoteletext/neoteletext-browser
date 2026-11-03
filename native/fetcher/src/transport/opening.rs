//! A connection's opening, on its own thread, under one deadline from
//! the name's resolution to the end of the proxy's tunnel and the TLS
//! handshake (finding F-63), with each address the name resolves to
//! checked before it is connected to (finding F-62).

use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use neoteletext_transport::{SOCKET_READ, TlsFailure, start_tls, web_address};
use neoteletext_web::port::Security;

use super::cancel::hold;
use super::writer::Outgoing;
use super::{AllowedLocal, Connection, Failure, MAX_RESOLVING, Opener, WRITE_TIMEOUT, certificate_problem};

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
        let mut failure = Failure { proxy: proxy.is_some(), ..Failure::default() };
        let (to_host, to_port) = proxy.map_or((host, port), |proxy| (proxy.host.as_str(), proxy.port));
        let targets = match resolve(to_host, to_port, self.until, &self.resolving) {
            Resolution::Addresses(targets) => targets,
            Resolution::Failed => return Err(failure),
            Resolution::TimedOut => return Err(Failure { timed_out: true, ..failure }),
        };
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
                Some(proxy) => proxy
                    .open_tunnel(&mut Deadline { socket: &socket, until: self.until }, host, port)
                    .map_err(|_| Failure { proxy: true, ..Failure::default() })?,
                None => Vec::new(),
            };
            return Ok((socket, target.ip(), early));
        }
        failure.timed_out = self.out_of_time();
        Err(failure)
    }

    /// Whether the opening's deadline has passed.
    fn out_of_time(&self) -> bool {
        Instant::now() >= self.until
    }

    /// Opens a connection, blocking: on the connection's own thread.
    pub(super) fn open(&self, host: &str, port: u16, security: Security) -> Result<Connection, Failure> {
        // A failure once the socket is reached is not the proxy's.
        fn own<E>(_: E) -> Failure {
            Failure::default()
        }
        // A failure of the secure handshake, and why.
        fn refused(error: &std::io::Error) -> Failure {
            Failure { certificate: Some(certificate_problem(error)), ..Failure::default() }
        }
        // A handshake the deadline ended, by a read or write given no more
        // time, is the channel not answering in time, not its certificate
        // (TODO-1277).
        let timed_out = |error: &std::io::Error| self.out_of_time() || matches!(error.kind(), IoErrorKind::TimedOut | IoErrorKind::WouldBlock);
        let (socket, ip, early) = self.reach(host, port)?;
        // An input message goes in its own packet at once (`interaction.md`
        // section 10).
        socket.set_nodelay(true).map_err(own)?;
        // The host's bytes that came with the proxy's answer come first: a
        // plain connection's data, or a TLS server's records.
        let (tls, first) = match security {
            Security::Plain => (None, early),
            Security::Tls => {
                // A certificate that fails is a hard stop (requirement 7.11).
                let mut bounded = Deadline { socket: &socket, until: self.until };
                let connection = start_tls(Arc::clone(&self.tls), host, &early, &mut bounded).map_err(|failure| match failure {
                    TlsFailure::Handshake(error) if timed_out(&error) => Failure { timed_out: true, ..Failure::default() },
                    TlsFailure::Name(_) | TlsFailure::Start(_) | TlsFailure::Read(_) => own(failure),
                    TlsFailure::Records(error) => refused(&std::io::Error::new(IoErrorKind::InvalidData, error)),
                    TlsFailure::Handshake(error) => refused(&error),
                })?;
                (Some(Box::new(connection)), Vec::new())
            }
        };
        socket.set_write_timeout(Some(WRITE_TIMEOUT)).map_err(own)?;
        let outgoing = Outgoing::start(socket.try_clone().map_err(own)?, Arc::clone(&self.wake));
        let wake = Arc::clone(&self.wake);
        let stream = neoteletext_transport::Connection::start(socket, tls, first, SOCKET_READ, move || wake()).map_err(own)?;
        Ok(Connection { stream, outgoing, peer: web_address(ip) })
    }
}

/// Whether `ip` may not be connected to: the engine's rule (REQ-SEC-006),
/// as its agent applies it to a peer.
fn refused_local(ip: IpAddr, allowed: &AllowedLocal) -> bool {
    let ip = web_address(ip);
    ip.is_blocked() && !allowed.contains(&Some(ip))
}

/// How a name's resolution went.
#[derive(Debug, PartialEq, Eq)]
enum Resolution {
    /// The addresses it resolves to.
    Addresses(Vec<SocketAddr>),
    /// It does not resolve.
    Failed,
    /// It did not resolve by the deadline, or no resolution could start,
    /// [`MAX_RESOLVING`] being under way.
    TimedOut,
}

/// One of the fetcher's [`MAX_RESOLVING`] resolutions, given back when its
/// thread ends.
struct Resolving(Arc<AtomicUsize>);

impl Resolving {
    /// A resolution of `count`'s, if fewer than [`MAX_RESOLVING`] are under
    /// way.
    fn take(count: &Arc<AtomicUsize>) -> Option<Self> {
        let mut under_way = count.load(Ordering::SeqCst);
        loop {
            if under_way >= MAX_RESOLVING {
                return None;
            }
            match count.compare_exchange(under_way, under_way.saturating_add(1), Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Some(Self(Arc::clone(count))),
                Err(now) => under_way = now,
            }
        }
    }
}

impl Drop for Resolving {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The addresses `host` resolves to, if it resolves by `until`. The
/// operating system's resolver cannot be interrupted, so it runs on a
/// thread of its own, which ends when the resolver gives up, by its own
/// limits; the connection does not wait for it, and at most
/// [`MAX_RESOLVING`] such threads run at once.
fn resolve(host: &str, port: u16, until: Instant, resolving: &Arc<AtomicUsize>) -> Resolution {
    resolve_with(host, port, until, resolving, |host, port| (host, port).to_socket_addrs().map(Iterator::collect))
}

/// [`resolve`], with `lookup` as the resolver.
fn resolve_with(
    host: &str,
    port: u16,
    until: Instant,
    resolving: &Arc<AtomicUsize>,
    lookup: impl FnOnce(&str, u16) -> std::io::Result<Vec<SocketAddr>> + Send + 'static,
) -> Resolution {
    let Some(slot) = Resolving::take(resolving) else { return Resolution::TimedOut };
    let (resolved, outcome) = mpsc::sync_channel(1);
    let host = host.to_owned();
    std::thread::spawn(move || {
        let addresses = lookup(&host, port);
        drop(slot);
        let _ = resolved.send(addresses);
    });
    match outcome.recv_timeout(until.saturating_duration_since(Instant::now())) {
        Ok(Ok(addresses)) => Resolution::Addresses(addresses),
        Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => Resolution::Failed,
        Err(mpsc::RecvTimeoutError::Timeout) => Resolution::TimedOut,
    }
}

#[cfg(test)]
mod tests;
