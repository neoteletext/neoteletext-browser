//! An open connection's writes: a thread of its own writes what the loop
//! gives it, in order and blocking, so a socket the network does not drain
//! never holds the loop up, and the bytes given and not yet written are
//! counted. The agent writes a pointer's move only while the push
//! connection's are under the engine's `PUSH_UNSENT_BOUND`, and holds it
//! otherwise (REQ-RCH-035, requirement 18.31); the thread wakes the loop
//! when they fall under it, so the move held goes then.
//!
//! What the operating system holds in the socket's own buffer is not
//! counted: reading it (Linux's `SIOCOUTQNSD`, macOS's `SO_NWRITE`, or
//! `TCP_NOTSENT_LOWAT` to be woken) needs unsafe code or a library this
//! crate does not have, so the count rises only once that buffer is full.

use std::io::Write;
use std::net::{Shutdown, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use neoteletext_web::consts::{INPUT_MESSAGE_MAX, PUSH_UNSENT_BOUND};

use super::Wake;

/// The most a connection may hold given and not yet written: sixty-four
/// of the longest input message, far beyond what a viewer types while one
/// write is held up, and so what a stalled connection holds in memory at
/// most. A send past it fails, which closes the connection, as a write
/// that times out does.
pub(super) const MAX_UNSENT: usize = 64 * INPUT_MESSAGE_MAX;
const _: () = assert!(MAX_UNSENT > PUSH_UNSENT_BOUND, "moves are held before sends fail");

/// The loop's side of a connection's writer thread.
pub(super) struct Outgoing {
    queue: Sender<Vec<u8>>,
    unsent: Arc<AtomicUsize>,
}

impl Outgoing {
    /// Starts the writer thread on `socket`, which wakes the loop through
    /// `wake` when what it holds falls under the bound, or when a write
    /// fails and it shuts the socket.
    pub(super) fn start(socket: TcpStream, wake: Wake) -> Self {
        let (queue, writes) = mpsc::channel();
        let unsent = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&unsent);
        std::thread::spawn(move || write_socket(socket, &writes, &counted, &*wake));
        Self { queue, unsent }
    }

    /// Gives the writer thread `bytes`, written after those given before;
    /// false when the thread has ended or would hold more than
    /// [`MAX_UNSENT`].
    pub(super) fn give(&self, bytes: Vec<u8>) -> bool {
        let length = bytes.len();
        if self.unsent().saturating_add(length) > MAX_UNSENT {
            return false;
        }
        // Counted before it is queued, so the thread never takes off what
        // was not yet added.
        self.unsent.fetch_add(length, Ordering::AcqRel);
        if self.queue.send(bytes).is_err() {
            take_off(&self.unsent, length);
            return false;
        }
        true
    }

    /// The bytes given and not yet written to the socket.
    pub(super) fn unsent(&self) -> usize {
        self.unsent.load(Ordering::Acquire)
    }
}

/// Takes `length` bytes off the count; what it was before. Only bytes
/// [`Outgoing::give`] added are taken off, once each, so it never goes
/// under 0.
fn take_off(unsent: &AtomicUsize, length: usize) -> usize {
    unsent.fetch_sub(length, Ordering::AcqRel)
}

/// The writer thread: writes each piece given, whole, until the loop drops
/// its side or a write fails, which shuts the socket so the reader thread
/// ends and the loop sees the connection closed.
fn write_socket(mut socket: TcpStream, writes: &Receiver<Vec<u8>>, unsent: &AtomicUsize, wake: &(dyn Fn() + Send + Sync)) {
    for bytes in writes {
        let written = socket.write_all(&bytes).and_then(|()| socket.flush());
        let before = take_off(unsent, bytes.len());
        if written.is_err() {
            let _ = socket.shutdown(Shutdown::Both);
            wake();
            return;
        }
        // REQ-RCH-035: the loop hears when the count falls under the
        // bound, and not after every write.
        if before >= PUSH_UNSENT_BOUND && before.saturating_sub(bytes.len()) < PUSH_UNSENT_BOUND {
            wake();
        }
    }
}
