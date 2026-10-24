//! The engine's network port as `Tcp` keeps it: a connection is opened
//! on a thread of its own and its state read back, bytes sent and taken,
//! and a connection closed, open or still opening.

use std::io::Write;
use std::net::Shutdown;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use embedded_io::ErrorKind;
use neoteletext_web::port::{ConnectionId, LinkState, Received, Security, Transport};

use super::cancel::{Cancel, shut};
use super::connection::write_records;
use super::{CONNECT_DEADLINE, Link, Opener, Tcp};

impl embedded_io::ErrorType for Tcp {
    type Error = ErrorKind;
}

impl Transport for Tcp {
    /// Starts opening the connection on a thread of its own, and returns
    /// at once; [`Transport::state`] says how it went.
    fn connect(&mut self, connection: ConnectionId, host: &str, port: u16, security: Security) -> Result<(), ErrorKind> {
        self.close(connection);
        let (done, outcome) = mpsc::sync_channel(1);
        let now = Instant::now();
        let opener = Opener {
            tls: Arc::clone(&self.tls),
            proxy: self.proxy.clone(),
            allowed: self.allowed_local,
            wake: Arc::clone(&self.wake),
            cancel: Cancel::default(),
            // A clock that cannot be read that far ahead leaves no time.
            until: now.checked_add(CONNECT_DEADLINE).unwrap_or(now),
        };
        let cancel = Arc::clone(&opener.cancel);
        let host = host.to_owned();
        std::thread::spawn(move || {
            let reached = opener.open(&host, port, security);
            // The connection was closed while opening: the socket goes,
            // and its reader thread with it.
            if let Err(mpsc::SendError(Ok(open))) = done.send(reached) {
                let _ = open.socket.shutdown(Shutdown::Both);
            }
            (opener.wake)();
        });
        *self.connection(connection) = Some(Link::Connecting(outcome, cancel));
        Ok(())
    }

    fn state(&mut self, connection: ConnectionId) -> LinkState {
        match self.settle(connection) {
            None => LinkState::Closed,
            Some(Link::Connecting(..)) => LinkState::Connecting,
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
            Some(Link::Connecting(..)) => Received::Nothing,
            None | Some(Link::Failed) => Received::Closed,
        })
    }

    fn close(&mut self, connection: ConnectionId) {
        // Open, this ends the reader thread's wait; the bytes it still
        // holds are dropped with the channel. Still opening, its thread's
        // socket is shut, and a connection it opens all the same is
        // dropped when it finds no one to give it to.
        match self.connection(connection).take() {
            Some(Link::Open(open)) => {
                let _ = open.socket.shutdown(Shutdown::Both);
            }
            Some(Link::Connecting(_, opening)) => shut(&opening),
            Some(Link::Failed) | None => {}
        }
    }
}
