//! Waiting for the other end of a socket to close, without reading from
//! it (TODO-2006): the trusted core learns that its broker ended, while
//! the replies on the same socket stay for the thread that asked.

#![expect(unsafe_code, reason = "poll(2), which the standard library does not offer (requirement 10.12)")]

use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// How long to wait before looking again while a reply waits unread, which
/// the thread that asked for it reads within microseconds.
const READABLE_PAUSE: Duration = Duration::from_millis(50);

/// Returns once the other end of `stream` has closed. Reading is asked
/// for, since macOS's `poll` reports a hang-up only on a descriptor
/// watched for something (it is built on `kqueue`, whose read filter
/// carries the end of the stream); data waiting to be read is left to its
/// reader, and looked at again after [`READABLE_PAUSE`].
pub(crate) fn wait_for_hang_up(stream: &UnixStream) {
    loop {
        let mut entry = libc::pollfd { fd: stream.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd, a live local, for the call; no time limit.
        let result = unsafe { libc::poll(&raw mut entry, 1, -1) };
        if result < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // A socket that cannot be watched serves nothing any more.
            return;
        }
        if entry.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            return;
        }
        std::thread::sleep(READABLE_PAUSE);
    }
}
