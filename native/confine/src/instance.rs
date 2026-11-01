//! The single-instance channel (DEC-1551 item 7): a second start hands what
//! it names to the browser already running, on a channel the sandboxed
//! processes cannot reach. On macOS and Linux that is a local-domain socket
//! in a folder only this user reaches: the helpers' profiles deny every
//! network operation on macOS, and on Linux the caller keeps it in the
//! runtime folder, which the two roles allowed local sockets do not see
//! (`linux/runtime.rs`). On Windows it is a named pipe with the default
//! access list, which gives an `AppContainer` no access.

use std::time::Duration;

/// The longest a connection is waited for: what it sends is written at once.
pub const HAND_OVER_WAIT: Duration = Duration::from_secs(2);

#[cfg(windows)]
pub use crate::windows::instance::{Instance, Stream, connect};
#[cfg(unix)]
pub use unix::{Instance, Stream, connect};

#[cfg(unix)]
mod unix {
    use super::HAND_OVER_WAIT;
    use std::io;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    /// One end of a hand-over.
    pub type Stream = UnixStream;

    /// The running browser's end of the channel.
    pub struct Instance {
        listener: UnixListener,
    }

    /// Reaches the browser already running at `place`.
    ///
    /// # Errors
    /// When none answers there.
    pub fn connect(place: &Path) -> io::Result<Stream> {
        let stream = UnixStream::connect(place)?;
        stream.set_write_timeout(Some(HAND_OVER_WAIT))?;
        Ok(stream)
    }

    impl Instance {
        /// Takes `place`, in place of one nothing answers on any more.
        ///
        /// # Errors
        /// When it cannot be taken.
        pub fn bind(place: &Path) -> io::Result<Self> {
            match std::fs::remove_file(place) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            Ok(Self { listener: UnixListener::bind(place)? })
        }

        /// The next hand-over.
        ///
        /// # Errors
        /// When the channel fails.
        pub fn next(&self) -> io::Result<Stream> {
            let (stream, _) = self.listener.accept()?;
            // macOS refuses this once the other end has gone, as a second
            // start may have, having said all it had: there is then no wait
            // to bound.
            let _ = stream.set_read_timeout(Some(HAND_OVER_WAIT));
            Ok(stream)
        }
    }
}
