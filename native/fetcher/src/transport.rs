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
//! never waits on one socket while another has data (REQ-INT-135); and a
//! third writes to it, so the loop never waits on a socket the network
//! does not drain, and counts what it has not yet written (REQ-RCH-035).
//!
//! Every connection shares one TLS configuration (REQ-RCH-014, requirement
//! 18.33): a reconnection to a server resumes the session of the last
//! connection to it where the server allows, saving a round trip, and no
//! early data (TLS 0-RTT) is ever sent, since an input message in it could
//! be replayed. TCP Fast Open is never used: the standard library does not
//! offer it.
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
//!
//! The reading of an open connection, the TLS start and the configuration
//! are the engine's `neoteletext-transport`, which its own programs use.
//! The opening's one deadline, its address checks and its cancelling, and
//! the writer thread, are this process's alone (the engine's DEC-1906).

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use neoteletext_transport::{CONNECT_TIMEOUT, client_config, mozilla_roots};
use neoteletext_web::consts::ALLOWED_LOCAL;
use neoteletext_web::port::ConnectionId;
use neoteletext_web::proxy::Proxy;
use neoteletext_web::url::IpAddress;
use protocol::fetcher::CertificateProblem;

mod cancel;
mod connection;
mod opening;
mod port;
mod writer;

use cancel::Cancel;
use writer::Outgoing;

/// How long a connection may take to open, from the name's resolution to
/// the end of its proxy's tunnel and TLS handshake: the engine's connect
/// limit, which the agent's own timer keeps too.
const CONNECT_DEADLINE: Duration = CONNECT_TIMEOUT;
/// How long a write to an open connection may block: the same limit.
const WRITE_TIMEOUT: Duration = CONNECT_DEADLINE;
/// The most names a fetcher resolves at once (finding F-62's residual,
/// TODO-1277): one for each connection it holds, the fetch, push and live
/// connections. The system's resolver cannot be interrupted, so a
/// resolution outlives its connection's deadline on a thread of its own
/// until the resolver gives up; a name whose servers never answer, tried
/// again at the agent's rate, held a thread more at every try. A
/// resolution beyond this fails at once, as one out of time.
const MAX_RESOLVING: usize = 3;

/// Wakes the fetcher's loop.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The addresses on this computer or the local network the channel may
/// reach: the agent's own list (`Config::allowed_local`), so both refuse
/// the same.
pub type AllowedLocal = [Option<IpAddress>; ALLOWED_LOCAL];

/// One open connection.
struct Connection {
    /// The reader thread and what it read, plain or through TLS.
    stream: neoteletext_transport::Connection,
    /// The writer thread, which writes everything sent once the
    /// connection is open, the TLS records of alerts among it, in order.
    outgoing: Outgoing,
    peer: IpAddress,
}

/// A connection did not open; the port is not told why (the engine's
/// agent treats every failure alike), only whether the proxy is to blame,
/// whether the certificate was refused (`certificate-errors.md`), and
/// whether an address was refused as local, and whether the opening ran
/// out of its deadline.
#[derive(Default)]
struct Failure {
    proxy: bool,
    certificate: Option<CertificateProblem>,
    local: bool,
    /// The opening ran out of [`CONNECT_DEADLINE`], or a resolution could
    /// not start for [`MAX_RESOLVING`]: the channel did not answer in time,
    /// which is not a certificate's fault (TODO-1277).
    timed_out: bool,
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
    /// An opening ran out of its deadline since this was last asked.
    timed_out: bool,
    /// How many names are being resolved, of at most [`MAX_RESOLVING`].
    resolving: Arc<AtomicUsize>,
}

impl Tcp {
    pub fn new(wake: Wake, proxy: Option<Proxy>, allowed_local: AllowedLocal) -> Self {
        Self::with_roots(mozilla_roots(), wake, proxy, allowed_local)
    }

    /// The transport trusting `roots`: Mozilla's, or a test's own.
    fn with_roots(roots: rustls::RootCertStore, wake: Wake, proxy: Option<Proxy>, allowed_local: AllowedLocal) -> Self {
        Self {
            fetch: None,
            push: None,
            live: None,
            tls: Arc::new(client_config(roots)),
            wake,
            proxy,
            proxy_failed: false,
            refused: None,
            allowed_local,
            local_refused: false,
            timed_out: false,
            resolving: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Whether an address was refused as local since this was last asked.
    pub fn take_local_refusal(&mut self) -> bool {
        std::mem::take(&mut self.local_refused)
    }

    /// Whether the proxy failed a connection since this was last asked.
    pub fn take_proxy_failure(&mut self) -> bool {
        std::mem::take(&mut self.proxy_failed)
    }

    /// Whether an opening ran out of its deadline since this was last
    /// asked.
    pub fn take_timeout(&mut self) -> bool {
        std::mem::take(&mut self.timed_out)
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
        let mut timed_out = false;
        let slot = self.connection(connection);
        if let Some(Link::Connecting(outcome, _)) = slot {
            match outcome.try_recv() {
                Ok(Ok(open)) => *slot = Some(Link::Open(open)),
                Ok(Err(failure)) => {
                    proxy_failed = failure.proxy;
                    refused = failure.certificate;
                    local_refused = failure.local;
                    timed_out = failure.timed_out;
                    *slot = Some(Link::Failed);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => *slot = Some(Link::Failed),
            }
        }
        self.proxy_failed |= proxy_failed;
        self.refused = refused.or(self.refused);
        self.local_refused |= local_refused;
        self.timed_out |= timed_out;
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
    /// The fetcher's count of names being resolved.
    resolving: Arc<AtomicUsize>,
}

#[cfg(test)]
mod tests;
