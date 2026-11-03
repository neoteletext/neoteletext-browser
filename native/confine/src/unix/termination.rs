//! A request to end, by signal, heard as data (TODO-2006): `SIGTERM`,
//! `SIGINT` and `SIGHUP` each write their number to a pipe, which a thread
//! of the caller's reads, so a process that started others can end them
//! before it ends itself. A signal's own default would end it at once and
//! leave what it started running.

#![expect(unsafe_code, reason = "sigaction and the handler's write, which the standard library does not offer (requirement 10.12)")]

use std::io::{self, PipeReader};
use std::os::fd::IntoRawFd as _;
use std::sync::atomic::{AtomicI32, Ordering};

/// The signals that ask a process to end: `kill`'s default, an
/// interrupt from the terminal, and the terminal's hang-up.
pub const TERMINATION_SIGNALS: [i32; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

/// The pipe's writing end, which the handler writes to: none until
/// [`termination_requests`] has made it.
static WRITE_END: AtomicI32 = AtomicI32::new(-1);

/// The handler: the signal's number, one byte, into the pipe. `write` is
/// async-signal-safe (POSIX.1-2017, section 2.4.3); errno is kept as it
/// was, as a handler must.
extern "C" fn on_signal(signal: libc::c_int) {
    let descriptor = WRITE_END.load(Ordering::Relaxed);
    if descriptor < 0 {
        return;
    }
    let byte = u8::try_from(signal).unwrap_or(u8::MAX);
    // SAFETY: errno's location is the calling thread's own.
    let errno = unsafe { errno_location() };
    // SAFETY: as above; read before the write may change it.
    let saved = unsafe { *errno };
    // SAFETY: one byte from a live local to a descriptor this process owns
    // for its whole life; the pipe does not block, so a full one loses the
    // byte rather than holding the handler.
    let _ = unsafe { libc::write(descriptor, (&raw const byte).cast(), 1) };
    // SAFETY: as above.
    unsafe { *errno = saved };
}

/// The calling thread's errno.
#[cfg(target_os = "linux")]
unsafe fn errno_location() -> *mut libc::c_int {
    // SAFETY: the C library's own thread-local errno.
    unsafe { libc::__errno_location() }
}

/// The calling thread's errno.
#[cfg(not(target_os = "linux"))]
unsafe fn errno_location() -> *mut libc::c_int {
    // SAFETY: the C library's own thread-local errno.
    unsafe { libc::__error() }
}

/// From now on each of [`TERMINATION_SIGNALS`] is not acted on by the
/// system but said, as its number, on the pipe returned: the caller
/// reads it and ends what it started, then itself. A program started
/// after this has the signals' defaults again (`execve(2)`: caught
/// signals are reset), and the pipe is closed in it. Called once; a
/// second call is refused.
///
/// # Errors
/// When the pipe cannot be made, or a handler cannot be set.
pub fn termination_requests() -> io::Result<PipeReader> {
    let (reader, writer) = io::pipe()?;
    let descriptor = writer.into_raw_fd();
    // SAFETY: fcntl on a descriptor just made and owned here.
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    if WRITE_END.compare_exchange(-1, descriptor, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        // SAFETY: the descriptor was made here and given to nothing.
        unsafe { libc::close(descriptor) };
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "termination requests are already heard"));
    }
    for signal in TERMINATION_SIGNALS {
        // SAFETY: a zeroed sigaction is a valid value: no handler, no flags,
        // an empty mask, which are then set.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // A system call the signal interrupts is restarted, so no other
        // thread of the caller sees EINTR for it.
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: the mask is the action's own, alive for the call.
        unsafe { libc::sigemptyset(&raw mut action.sa_mask) };
        // SAFETY: a valid action, alive for the call; the old one is not wanted.
        if unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(reader)
}
