//! The engine's network port as `Tcp` keeps it: a connection is opened
//! on a thread of its own and its state read back, bytes sent and taken,
//! and a connection closed, open or still opening.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use embedded_io::ErrorKind;
use neoteletext_web::port::{ConnectionId, LinkState, Received, Security, Transport};

use super::cancel::{Cancel, shut};
use super::connection::give_records;
use super::{CONNECT_DEADLINE, Failure, Link, Opened, Opener, Tcp};

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
        // The agent asked for its push connection as a stream of the
        // channel's shared connection (REQ-6989): opened on a thread of
        // its own as a TCP connection is, under the same deadline.
        if connection == ConnectionId::Push
            && let Some(request) = self.stream_request.take()
        {
            let (shared, wake) = (self.shared.clone(), Arc::clone(&self.wake));
            let until = now.checked_add(CONNECT_DEADLINE).unwrap_or(now);
            std::thread::spawn(move || {
                let opened = match &shared {
                    Some(shared) => shared.open(&request, until, &wake),
                    None => Err(String::from("this fetcher has no shared connection")),
                };
                // Closed while opening: the stream goes with its handle.
                let _ = done.send(opened.map(Opened::Stream).map_err(|why| Failure { no_stream: Some(why), ..Failure::default() }));
                wake();
            });
            *self.connection(connection) = Some(Link::Connecting(outcome, Cancel::default()));
            return Ok(());
        }
        let opener = Opener {
            tls: Arc::clone(&self.tls),
            proxy: self.proxy.clone(),
            allowed: self.allowed_local,
            wake: Arc::clone(&self.wake),
            cancel: Cancel::default(),
            // A clock that cannot be read that far ahead leaves no time.
            until: now.checked_add(CONNECT_DEADLINE).unwrap_or(now),
            resolving: Arc::clone(&self.resolving),
        };
        let cancel = Arc::clone(&opener.cancel);
        let host = host.to_owned();
        std::thread::spawn(move || {
            let reached = opener.open(&host, port, security).map(Opened::Own);
            // The connection was closed while opening: the socket goes,
            // and its reader thread with it.
            if let Err(mpsc::SendError(Ok(Opened::Own(open)))) = done.send(reached) {
                open.stream.shutdown();
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
            Some(Link::Stream(link)) => LinkState::Open(link.peer),
            Some(Link::Failed) => LinkState::Failed,
        }
    }

    fn send(&mut self, connection: ConnectionId, bytes: &[u8]) -> Result<(), ErrorKind> {
        let open = match self.settle(connection) {
            Some(Link::Open(open)) => open,
            // One send is one DATA frame on the stream, given whole: the
            // messages the agent sends together leave together
            // (REQ-RCH-010).
            Some(Link::Stream(link)) => return if link.stream.send(bytes) { Ok(()) } else { Err(ErrorKind::BrokenPipe) },
            _ => return Err(ErrorKind::NotConnected),
        };
        // Given whole to the writer thread, as one write: the messages the
        // agent sends together leave together (REQ-RCH-010), and a socket
        // the network does not drain holds up only that thread.
        let given = match open.stream.tls_mut() {
            None => open.outgoing.give(bytes.to_vec()),
            Some(tls) => std::io::Write::write_all(&mut tls.writer(), bytes).is_ok() && give_records(tls, &open.outgoing),
        };
        if given { Ok(()) } else { Err(ErrorKind::BrokenPipe) }
    }

    /// What the writer thread has been given and has not yet written; the
    /// operating system's own buffer is not seen (`writer.rs`).
    fn unsent(&mut self, connection: ConnectionId) -> usize {
        match self.connection(connection) {
            Some(Link::Open(open)) => open.outgoing.unsent(),
            Some(Link::Stream(link)) => link.stream.unsent(),
            _ => 0,
        }
    }

    fn receive(&mut self, connection: ConnectionId, buffer: &mut [u8]) -> Result<Received, ErrorKind> {
        Ok(match self.settle(connection) {
            Some(Link::Open(open)) => open.receive(buffer),
            Some(Link::Stream(link)) => link.stream.receive(buffer),
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
            Some(Link::Open(open)) => open.stream.shutdown(),
            Some(Link::Connecting(_, opening)) => shut(&opening),
            // A stream closes with its handle: what it was given is sent,
            // then its sending side finished.
            Some(Link::Stream(_) | Link::Failed) | None => {}
        }
    }
}

impl Drop for Tcp {
    /// A tab that leaves its process closes its connections (REQ-6980):
    /// their threads end with them, where once the process's end ended
    /// them.
    fn drop(&mut self) {
        for connection in ConnectionId::ALL {
            self.close(connection);
        }
    }
}
