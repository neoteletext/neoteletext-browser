//! The engine's network port over the operating system's sockets: plain
//! TCP only. TLS is not built yet, so the agent is told none is available
//! and reports `https` addresses unreachable.

use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use embedded_io::ErrorKind;
use neoteletext_web::port::{ConnectionId, LinkState, Received, Security, Transport};
use neoteletext_web::url::IpAddress;

/// How long a connection may take to open; the agent's own limit is the
/// engine's `DEFAULT_CONNECT_TIMEOUT_US`, ten seconds.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The address the operating system resolved, as the engine writes one.
pub fn address(ip: IpAddr) -> IpAddress {
    match ip {
        IpAddr::V4(v4) => IpAddress::V4(v4.octets()),
        IpAddr::V6(v6) => IpAddress::V6(v6.segments()),
    }
}

/// An open connection and the address it reached.
type Stream = Option<(TcpStream, IpAddress)>;

/// The fetch, push and live connections.
#[derive(Default)]
pub struct Tcp {
    fetch: Stream,
    push: Stream,
    live: Stream,
}

impl Tcp {
    fn stream(&mut self, connection: ConnectionId) -> &mut Stream {
        match connection {
            ConnectionId::Fetch => &mut self.fetch,
            ConnectionId::Push => &mut self.push,
            ConnectionId::Live => &mut self.live,
        }
    }
}

impl embedded_io::ErrorType for Tcp {
    type Error = ErrorKind;
}

impl Transport for Tcp {
    fn connect(&mut self, connection: ConnectionId, host: &str, port: u16, security: Security) -> Result<(), ErrorKind> {
        if security == Security::Tls {
            return Err(ErrorKind::Unsupported);
        }
        let target = (host, port).to_socket_addrs().map_err(|_| ErrorKind::NotFound)?.next().ok_or(ErrorKind::NotFound)?;
        let stream = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT).map_err(|_| ErrorKind::ConnectionRefused)?;
        stream.set_nonblocking(true).map_err(|_| ErrorKind::Other)?;
        let _ = stream.set_nodelay(true);
        *self.stream(connection) = Some((stream, address(target.ip())));
        Ok(())
    }

    fn state(&mut self, connection: ConnectionId) -> LinkState {
        self.stream(connection).as_ref().map_or(LinkState::Closed, |(_, ip)| LinkState::Open(*ip))
    }

    fn send(&mut self, connection: ConnectionId, bytes: &[u8]) -> Result<(), ErrorKind> {
        let (stream, _) = self.stream(connection).as_mut().ok_or(ErrorKind::NotConnected)?;
        let mut rest = bytes;
        while !rest.is_empty() {
            match stream.write(rest) {
                Ok(0) => return Err(ErrorKind::BrokenPipe),
                Ok(count) => rest = rest.get(count..).unwrap_or_default(),
                // The socket is non-blocking; a full send buffer drains quickly.
                Err(error) if error.kind() == IoErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
                Err(_) => return Err(ErrorKind::BrokenPipe),
            }
        }
        Ok(())
    }

    fn receive(&mut self, connection: ConnectionId, buffer: &mut [u8]) -> Result<Received, ErrorKind> {
        let Some((stream, _)) = self.stream(connection).as_mut() else { return Ok(Received::Closed) };
        match stream.read(buffer) {
            Ok(0) => Ok(Received::Closed),
            Ok(count) => Ok(Received::Data(count)),
            Err(error) if error.kind() == IoErrorKind::WouldBlock => Ok(Received::Nothing),
            Err(_) => Ok(Received::Closed),
        }
    }

    fn close(&mut self, connection: ConnectionId) {
        *self.stream(connection) = None;
    }
}
