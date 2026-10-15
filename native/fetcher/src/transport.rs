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
/// One slot each for the fetch, push and live connections.
const CONNECTIONS: usize = 3;

fn slot(connection: ConnectionId) -> usize {
    match connection {
        ConnectionId::Fetch => 0,
        ConnectionId::Push => 1,
        ConnectionId::Live => 2,
    }
}

pub fn address(ip: IpAddr) -> IpAddress {
    match ip {
        IpAddr::V4(v4) => IpAddress::V4(v4.octets()),
        IpAddr::V6(v6) => IpAddress::V6(v6.segments()),
    }
}

#[derive(Default)]
pub struct Tcp {
    streams: [Option<(TcpStream, IpAddress)>; CONNECTIONS],
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
        self.streams[slot(connection)] = Some((stream, address(target.ip())));
        Ok(())
    }

    fn state(&mut self, connection: ConnectionId) -> LinkState {
        self.streams[slot(connection)].as_ref().map_or(LinkState::Closed, |(_, ip)| LinkState::Open(*ip))
    }

    fn send(&mut self, connection: ConnectionId, bytes: &[u8]) -> Result<(), ErrorKind> {
        let (stream, _) = self.streams[slot(connection)].as_mut().ok_or(ErrorKind::NotConnected)?;
        let mut sent = 0;
        while sent < bytes.len() {
            match stream.write(&bytes[sent..]) {
                Ok(0) => return Err(ErrorKind::BrokenPipe),
                Ok(count) => sent += count,
                // The socket is non-blocking; a full send buffer drains quickly.
                Err(error) if error.kind() == IoErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(1)),
                Err(_) => return Err(ErrorKind::BrokenPipe),
            }
        }
        Ok(())
    }

    fn receive(&mut self, connection: ConnectionId, buffer: &mut [u8]) -> Result<Received, ErrorKind> {
        let Some((stream, _)) = self.streams[slot(connection)].as_mut() else { return Ok(Received::Closed) };
        match stream.read(buffer) {
            Ok(0) => Ok(Received::Closed),
            Ok(count) => Ok(Received::Data(count)),
            Err(error) if error.kind() == IoErrorKind::WouldBlock => Ok(Received::Nothing),
            Err(_) => Ok(Received::Closed),
        }
    }

    fn close(&mut self, connection: ConnectionId) {
        self.streams[slot(connection)] = None;
    }
}
