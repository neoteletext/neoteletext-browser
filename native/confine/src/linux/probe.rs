//! The sandbox tests' probes (`confine-probe`, `tests/probe.rs`): the
//! system calls a compromised helper would make, which its sandbox must
//! refuse, made here because the standard library offers none of them and
//! unsafe code is confined to this platform module (requirement 10.12).
//! Nothing the browser ships calls them.

#![expect(unsafe_code, reason = "the system calls the sandbox tests probe, which the standard library does not offer (requirement 10.12)")]

use std::io;
use std::os::fd::{FromRawFd as _, OwnedFd, RawFd};

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
