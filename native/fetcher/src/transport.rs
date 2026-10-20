//! The engine's network port over the operating system's sockets, plain
//! or with Transport Layer Security (requirement 5.8; decision 41 chose
//! `rustls`), trusting Mozilla's root certificates. A connection opens and
//! completes its handshake blocking, within the engine's own connect
//! limit, then reads without blocking, so the fetcher's loop never waits
//! on one socket while another has data.

use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{IpAddr, Shutdown, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use embedded_io::ErrorKind;
use neoteletext_web::port::{ConnectionId, LinkState, Received, Security, Transport};
use neoteletext_web::url::IpAddress;

/// How long a connection, with its TLS handshake, may take to open; the
/// agent's own limit is the engine's `DEFAULT_CONNECT_TIMEOUT_US`, ten
/// seconds.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The address the operating system resolved, as the engine writes one.
pub fn address(ip: IpAddr) -> IpAddress {
    match ip {
        IpAddr::V4(v4) => IpAddress::V4(v4.octets()),
        IpAddr::V6(v6) => IpAddress::V6(v6.segments()),
    }
}

/// One open connection.
enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn socket(&self) -> &TcpStream {
        match self {
            Self::Plain(socket) => socket,
            Self::Tls(stream) => &stream.sock,
        }
    }
}

/// An open connection and the address it reached.
type Open = Option<(Stream, IpAddress)>;

/// The fetch, push and live connections.
pub struct Tcp {
    fetch: Open,
    push: Open,
    live: Open,
    tls: Arc<rustls::ClientConfig>,
}

impl Default for Tcp {
    fn default() -> Self {
        // ring is the one provider built in, so the builder installs it.
        let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        Self { fetch: None, push: None, live: None, tls: Arc::new(tls) }
    }
}

impl Tcp {
    fn stream(&mut self, connection: ConnectionId) -> &mut Open {
        match connection {
            ConnectionId::Fetch => &mut self.fetch,
            ConnectionId::Push => &mut self.push,
            ConnectionId::Live => &mut self.live,
        }
    }

    fn open(&self, host: &str, port: u16, security: Security) -> Result<(Stream, IpAddr), ErrorKind> {
        let target = (host, port).to_socket_addrs().map_err(|_| ErrorKind::NotFound)?.next().ok_or(ErrorKind::NotFound)?;
        let socket = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT).map_err(|_| ErrorKind::ConnectionRefused)?;
        // A key record goes in its own packet at once (`interaction.md`
        // section 10).
        socket.set_nodelay(true).map_err(|_| ErrorKind::Other)?;
        socket.set_read_timeout(Some(CONNECT_TIMEOUT)).map_err(|_| ErrorKind::Other)?;
        socket.set_write_timeout(Some(CONNECT_TIMEOUT)).map_err(|_| ErrorKind::Other)?;
        let stream = match security {
            Security::Plain => Stream::Plain(socket),
            Security::Tls => {
                let name = rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|_| ErrorKind::InvalidInput)?;
                let connection = rustls::ClientConnection::new(self.tls.clone(), name).map_err(|_| ErrorKind::Other)?;
                let mut stream = rustls::StreamOwned::new(connection, socket);
                // A certificate that fails is a hard stop (requirement 7.11).
                while stream.conn.is_handshaking() {
                    stream.conn.complete_io(&mut stream.sock).map_err(|_| ErrorKind::PermissionDenied)?;
                }
                Stream::Tls(Box::new(stream))
            }
        };
        stream.socket().set_nonblocking(true).map_err(|_| ErrorKind::Other)?;
        Ok((stream, target.ip()))
    }
}

impl embedded_io::ErrorType for Tcp {
    type Error = ErrorKind;
}

impl Transport for Tcp {
    fn connect(&mut self, connection: ConnectionId, host: &str, port: u16, security: Security) -> Result<(), ErrorKind> {
        let (stream, ip) = self.open(host, port, security)?;
        *self.stream(connection) = Some((stream, address(ip)));
        Ok(())
    }

    fn state(&mut self, connection: ConnectionId) -> LinkState {
        self.stream(connection).as_ref().map_or(LinkState::Closed, |(_, ip)| LinkState::Open(*ip))
    }

    fn send(&mut self, connection: ConnectionId, bytes: &[u8]) -> Result<(), ErrorKind> {
        let (stream, _) = self.stream(connection).as_mut().ok_or(ErrorKind::NotConnected)?;
        // Written whole, blocking, within the write limit: a request or a
        // key record is small, and the socket drains it at once.
        let blocking = |stream: &Stream, on: bool| stream.socket().set_nonblocking(!on).map_err(|_| ErrorKind::Other);
        blocking(stream, true)?;
        let written = match stream {
            Stream::Plain(socket) => socket.write_all(bytes).and_then(|()| socket.flush()),
            Stream::Tls(tls) => tls.write_all(bytes).and_then(|()| tls.flush()),
        };
        blocking(stream, false)?;
        written.map_err(|_| ErrorKind::BrokenPipe)
    }

    fn receive(&mut self, connection: ConnectionId, buffer: &mut [u8]) -> Result<Received, ErrorKind> {
        let Some((stream, _)) = self.stream(connection).as_mut() else { return Ok(Received::Closed) };
        let read = match stream {
            Stream::Plain(socket) => socket.read(buffer),
            Stream::Tls(tls) => tls.read(buffer),
        };
        match read {
            Ok(0) => Ok(Received::Closed),
            Ok(count) => Ok(Received::Data(count)),
            Err(error) if error.kind() == IoErrorKind::WouldBlock => Ok(Received::Nothing),
            Err(_) => Ok(Received::Closed),
        }
    }

    fn close(&mut self, connection: ConnectionId) {
        if let Some((stream, _)) = self.stream(connection).take() {
            let _ = stream.socket().shutdown(Shutdown::Both);
        }
    }
}
