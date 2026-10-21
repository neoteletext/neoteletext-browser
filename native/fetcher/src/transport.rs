//! The engine's network port over the operating system's sockets, plain
//! or with Transport Layer Security (requirement 5.8; decision 41 chose
//! `rustls`), trusting Mozilla's root certificates. A connection opens and
//! completes its handshake blocking, within the engine's own connect
//! limit. Then a thread of its own reads the socket, blocking, and wakes
//! the fetcher's loop the moment bytes arrive, so the loop sleeps until
//! something happens and never waits on one socket while another has data
//! (REQ-INT-135).

use std::collections::VecDeque;
use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{IpAddr, Shutdown, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::Duration;

use embedded_io::ErrorKind;
use neoteletext_web::port::{ConnectionId, LinkState, Received, Security, Transport};
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
}

impl Tcp {
    pub fn new(wake: Wake) -> Self {
        // ring is the one provider built in, so the builder installs it.
        let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        Self { fetch: None, push: None, live: None, tls: Arc::new(tls), wake }
    }

    fn connection(&mut self, connection: ConnectionId) -> &mut Option<Connection> {
        match connection {
            ConnectionId::Fetch => &mut self.fetch,
            ConnectionId::Push => &mut self.push,
            ConnectionId::Live => &mut self.live,
        }
    }

    fn open(&self, host: &str, port: u16, security: Security) -> Result<Connection, ErrorKind> {
        let target = (host, port).to_socket_addrs().map_err(|_| ErrorKind::NotFound)?.next().ok_or(ErrorKind::NotFound)?;
        let mut socket = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT).map_err(|_| ErrorKind::ConnectionRefused)?;
        // An input message goes in its own packet at once (`interaction.md`
        // section 10).
        socket.set_nodelay(true).map_err(|_| ErrorKind::Other)?;
        socket.set_read_timeout(Some(CONNECT_TIMEOUT)).map_err(|_| ErrorKind::Other)?;
        socket.set_write_timeout(Some(CONNECT_TIMEOUT)).map_err(|_| ErrorKind::Other)?;
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
        Ok(Connection { socket, tls, incoming, pending: VecDeque::new(), peer: address(target.ip()) })
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
