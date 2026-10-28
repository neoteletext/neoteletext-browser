//! An open connection's bytes: the reader thread that blocks on its
//! socket and wakes the loop when bytes arrive (REQ-INT-135), what the
//! loop takes from it, plain or through TLS, and the TLS records given to
//! the writer thread.

use std::collections::VecDeque;
use std::io::{ErrorKind as IoErrorKind, Read};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};

use neoteletext_web::port::Received;

use super::writer::Outgoing;
use super::{Connection, SOCKET_READ};

/// Whether bytes from the reader thread are pending.
enum Refill {
    Ready,
    Waiting,
    Ended,
}

impl Connection {
    pub(super) fn receive(&mut self, buffer: &mut [u8]) -> Received {
        let Self { tls, incoming, pending, outgoing, .. } = self;
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
            if !give_records(tls, outgoing) {
                return Received::Closed;
            }
        }
    }
}

/// The reader thread: passes on what the socket gives until it ends, or
/// until the connection is closed and nothing takes the bytes.
pub(super) fn read_socket(mut socket: TcpStream, chunks: &SyncSender<Vec<u8>>, wake: &(dyn Fn() + Send + Sync)) {
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

/// Gives the writer thread whatever records rustls has ready, as one
/// piece, so the messages sent together leave in one write (REQ-RCH-010);
/// false when the thread does not take them.
pub(super) fn give_records(tls: &mut rustls::ClientConnection, outgoing: &Outgoing) -> bool {
    let mut records = Vec::new();
    while tls.wants_write() {
        // Writing to memory does not fail.
        if tls.write_tls(&mut records).is_err() {
            return false;
        }
    }
    records.is_empty() || outgoing.give(records)
}
