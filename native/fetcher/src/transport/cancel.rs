//! Closing a connection still opening (finding F-63): its thread holds
//! the socket here as soon as it has one, and `close` shuts it, so the
//! thread's read or write fails and the thread ends at once, rather than
//! when the opening's deadline passes.

use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};

/// What `close` reaches of a connection still opening: whether it was
/// closed, and the socket its thread is on, to shut.
#[derive(Default)]
pub(super) struct Opening {
    closed: bool,
    socket: Option<TcpStream>,
}

/// A connection's opening, shared by its thread and the loop.
pub(super) type Cancel = Arc<Mutex<Opening>>;

/// The opening as it stands; a thread that panicked holding it changed
/// nothing that matters here.
fn opening(cancel: &Cancel) -> std::sync::MutexGuard<'_, Opening> {
    cancel.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Gives `close` a handle to `socket`; false when the connection was
/// closed already, and the socket is to be let go.
pub(super) fn hold(cancel: &Cancel, socket: &TcpStream) -> bool {
    let mut opening = opening(cancel);
    if opening.closed {
        return false;
    }
    opening.socket = socket.try_clone().ok();
    opening.socket.is_some()
}

/// Shuts a connection still opening: its thread's next read or write, or
/// the one it is blocked in, fails, and the thread ends.
pub(super) fn shut(cancel: &Cancel) {
    let mut opening = opening(cancel);
    opening.closed = true;
    if let Some(socket) = opening.socket.take() {
        let _ = socket.shutdown(Shutdown::Both);
    }
}
