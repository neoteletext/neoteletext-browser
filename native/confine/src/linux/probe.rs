//! The sandbox tests' probes (`confine-probe`, `tests/probe.rs`): the
//! system calls a compromised helper would make, which its sandbox must
//! refuse, made here because the standard library offers none of them and
//! unsafe code is confined to this platform module (requirement 10.12).
//! Nothing the browser ships calls them.

#![expect(unsafe_code, reason = "the system calls the sandbox tests probe, which the standard library does not offer (requirement 10.12)")]

use std::io;
use std::os::fd::{FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt as _;

/// A result of a system call that returns -1 and sets `errno` on failure.
fn checked(result: libc::c_long) -> io::Result<libc::c_long> {
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result) }
}

/// A descriptor a system call returned, owned from here on.
fn owned(result: libc::c_long) -> io::Result<OwnedFd> {
    let descriptor = RawFd::try_from(checked(result)?).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    // SAFETY: the kernel just returned this descriptor to this process,
    // and nothing else holds it.
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
}

/// Opens a socket of `domain`, `kind` and `protocol` and closes it again:
/// whether the sandbox lets the helper have one.
///
/// # Errors
/// As `socket(2)`.
pub fn socket(domain: libc::c_int, kind: libc::c_int, protocol: libc::c_int) -> io::Result<()> {
    // SAFETY: socket takes integers only.
    let result = unsafe { libc::socket(domain, kind | libc::SOCK_CLOEXEC, protocol) };
    owned(libc::c_long::from(result)).map(drop)
}

/// A descriptor that refers to this process (`pidfd_open(2)`), taken
/// before the sandbox, for [`take_descriptor`] to use after it.
///
/// # Errors
/// As `pidfd_open(2)`.
pub fn own_process() -> io::Result<OwnedFd> {
    let pid = libc::c_long::from(std::process::id());
    // SAFETY: pidfd_open takes a process number and flags, integers only.
    owned(unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) })
}

/// Takes a copy of descriptor `target` of the process `process` refers to
/// (`pidfd_getfd(2)`), as a helper reaching into another process's files
/// would, and closes it again.
///
/// # Errors
/// As `pidfd_getfd(2)`.
pub fn take_descriptor(process: &OwnedFd, target: RawFd) -> io::Result<()> {
    use std::os::fd::AsRawFd as _;
    // SAFETY: pidfd_getfd takes two descriptors and flags, integers only;
    // what it returns is owned and closed here.
    owned(unsafe { libc::syscall(libc::SYS_pidfd_getfd, process.as_raw_fd(), target, 0) }).map(drop)
}

/// `getpid` by its x32 number, as code that slips past a filter checking
/// x86-64's numbers would call `execve`.
///
/// # Errors
/// As the system call: ENOSYS where the kernel has no x32, EPERM where
/// the sandbox refuses it.
#[cfg(target_arch = "x86_64")]
pub fn x32_getpid() -> io::Result<libc::c_long> {
    let number = libc::c_long::from(super::seccomp::X32_SYSCALL_BIT) | libc::SYS_getpid;
    // SAFETY: getpid takes no arguments, by whichever number.
    checked(unsafe { libc::syscall(number) })
}

/// A pseudo-terminal made this process's controlling terminal, as a
/// helper started from a terminal has it as its standard error: its main
/// side, and the terminal itself.
///
/// # Errors
/// When the pseudo-terminal cannot be made or made controlling.
pub fn controlling_terminal() -> io::Result<(OwnedFd, std::fs::File)> {
    use std::os::fd::AsRawFd as _;
    // SAFETY: posix_openpt takes flags only.
    let main = owned(libc::c_long::from(unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) }))?;
    // SAFETY: grantpt and unlockpt take the descriptor just made.
    checked(libc::c_long::from(unsafe { libc::grantpt(main.as_raw_fd()) }))?;
    // SAFETY: as above.
    checked(libc::c_long::from(unsafe { libc::unlockpt(main.as_raw_fd()) }))?;
    let mut name = [0_u8; libc::PATH_MAX as usize];
    // SAFETY: ptsname_r writes at most the buffer's length, NUL-terminated.
    let result = unsafe { libc::ptsname_r(main.as_raw_fd(), name.as_mut_ptr().cast(), name.len()) };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    let name = std::ffi::CStr::from_bytes_until_nul(&name).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    let terminal = std::fs::OpenOptions::new().read(true).write(true).open(std::ffi::OsStr::from_bytes(name.to_bytes()))?;
    // SAFETY: setsid takes no arguments.
    checked(libc::c_long::from(unsafe { libc::setsid() }))?;
    // SAFETY: TIOCSCTTY takes an integer argument.
    checked(libc::c_long::from(unsafe { libc::ioctl(terminal.as_raw_fd(), libc::TIOCSCTTY, 0) }))?;
    Ok((main, terminal))
}

/// Queues a byte as if typed at `terminal` (`TIOCSTI`), as a helper
/// would type a command into the user's shell.
///
/// # Errors
/// As `ioctl(2)`.
pub fn type_into(terminal: &std::fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd as _;
    let byte = b'x';
    // SAFETY: TIOCSTI reads one byte from the pointer, which is to a live
    // local.
    checked(libc::c_long::from(unsafe { libc::ioctl(terminal.as_raw_fd(), libc::TIOCSTI, &raw const byte) })).map(drop)
}

/// Whether the process `pid` ends within `ceiling`, by a descriptor
/// referring to it (`pidfd_open(2)`, readable when it has ended), so the
/// test returns as soon as it does. A process already gone has ended.
#[must_use]
pub fn ends_within(pid: u32, ceiling: std::time::Duration) -> bool {
    use std::os::fd::AsRawFd as _;
    // SAFETY: pidfd_open takes a process number and flags, integers only.
    let Ok(process) = owned(unsafe { libc::syscall(libc::SYS_pidfd_open, libc::c_long::from(pid), 0) }) else {
        return true;
    };
    let mut watched = libc::pollfd { fd: process.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    let milliseconds = libc::c_int::try_from(ceiling.as_millis()).unwrap_or(libc::c_int::MAX);
    // SAFETY: poll reads and writes the one pollfd it is given, alive for
    // the call.
    let ready = unsafe { libc::poll(&raw mut watched, 1, milliseconds) };
    ready > 0
}
