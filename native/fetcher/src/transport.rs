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
use std::net::{IpAddr, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use neoteletext_web::consts::{ALLOWED_LOCAL, DEFAULT_CONNECT_TIMEOUT_US};
use neoteletext_web::port::ConnectionId;
use neoteletext_web::proxy::Proxy;
use neoteletext_web::url::IpAddress;
use protocol::fetcher::CertificateProblem;

mod cancel;
mod connection;
mod opening;
mod port;

use cancel::Cancel;

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

#[cfg(test)]
mod tests;
