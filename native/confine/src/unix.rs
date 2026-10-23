//! Passing open files between processes on macOS and Linux: a message on
//! a local-domain socket carries descriptors with it (`SCM_RIGHTS`,
//! `unix(7)`). The broker passes the trusted core the pipes of each helper
//! it starts this way. The standard library's ancillary-data interface is
//! not yet stable, so the two system calls are made here. And the local
//! time zone's offset, which the standard library does not offer, from
//! the C library's own `localtime_r`.

#![allow(unsafe_code, reason = "sendmsg and recvmsg with SCM_RIGHTS, which the standard library does not offer yet, and localtime_r (decision 56)")]

use std::io::{self, Read as _, Write as _};
use std::mem::size_of;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::ptr;

/// The most descriptors one message carries: a helper's two pipes.
pub const MAX_DESCRIPTORS: usize = 2;
/// Room for that many descriptors' control message, as `CMSG_SPACE`
/// computes it; checked against it at run time.
const CONTROL_BYTES: usize = 64;

/// A control message's length as `msghdr` holds it: a `u32` on macOS and
/// a `usize` on Linux.
#[cfg(target_os = "linux")]
#[allow(clippy::unnecessary_wraps, reason = "the same signature as macOS's, which can fail")]
fn control_length(bytes: usize) -> io::Result<usize> {
    Ok(bytes)
}
#[cfg(not(target_os = "linux"))]
fn control_length(bytes: usize) -> io::Result<libc::socklen_t> {
    libc::socklen_t::try_from(bytes).map_err(|_| io::ErrorKind::InvalidInput.into())
}

/// Sends `bytes`, with `descriptors` attached to their first byte, then
/// whatever of `bytes` the first send did not take.
pub fn send(stream: &UnixStream, bytes: &[u8], descriptors: &[RawFd]) -> io::Result<()> {
    if descriptors.len() > MAX_DESCRIPTORS {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "too many descriptors"));
    }
    let mut control = [0_u8; CONTROL_BYTES];
    let descriptor_bytes = u32::try_from(size_of::<RawFd>().saturating_mul(descriptors.len())).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: CMSG_SPACE computes a size from an integer.
    let space = unsafe { libc::CMSG_SPACE(descriptor_bytes) } as usize;
    if space > control.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "control message too large"));
    }
    let mut part = libc::iovec { iov_base: bytes.as_ptr().cast_mut().cast(), iov_len: bytes.len() };
    // SAFETY: a zeroed msghdr is valid: null pointers and zero lengths.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &raw mut part;
    message.msg_iovlen = 1;
    if !descriptors.is_empty() {
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = control_length(space)?;
        // SAFETY: msg_control points at `space` bytes of `control`, which
        // holds one header and the descriptors, as CMSG_SPACE said; the
        // header and the data are written within it.
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&raw const message);
            if header.is_null() {
                return Err(io::Error::other("no room for the control message"));
            }
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            // A u32 on macOS and a usize on Linux.
            #[allow(clippy::useless_conversion, reason = "the field's type differs between the systems")]
            let length = libc::CMSG_LEN(descriptor_bytes).try_into().map_err(|_| io::ErrorKind::InvalidInput)?;
            (*header).cmsg_len = length;
            ptr::copy_nonoverlapping(descriptors.as_ptr().cast::<u8>(), libc::CMSG_DATA(header), descriptor_bytes as usize);
        }
    }
    // SAFETY: the message's pointers are to `part`, `bytes` and `control`,
    // all alive for the call.
    let sent = unsafe { libc::sendmsg(stream.as_raw_fd(), &raw const message, 0) };
    let sent = usize::try_from(sent).map_err(|_| io::Error::last_os_error())?;
    let mut stream = stream;
    stream.write_all(bytes.get(sent..).unwrap_or_default())
}

/// Reads exactly `buffer.len()` bytes, taking the descriptors that came
/// with the first of them. Descriptors beyond `MAX_DESCRIPTORS`, or of
/// any other kind of control message, are closed and refused.
pub fn receive(stream: &UnixStream, buffer: &mut [u8]) -> io::Result<Vec<OwnedFd>> {
    let mut control = [0_u8; CONTROL_BYTES];
    let mut part = libc::iovec { iov_base: buffer.as_mut_ptr().cast(), iov_len: buffer.len() };
    // SAFETY: a zeroed msghdr is valid.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &raw mut part;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control_length(control.len())?;
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_CMSG_CLOEXEC;
    #[cfg(not(target_os = "linux"))]
    let flags = 0;
    // SAFETY: the pointers are to `part`, `buffer` and `control`, alive for
    // the call, with their true lengths.
    let received = unsafe { libc::recvmsg(stream.as_raw_fd(), &raw mut message, flags) };
    let received = usize::try_from(received).map_err(|_| io::Error::last_os_error())?;
    if received == 0 && !buffer.is_empty() {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    let mut descriptors = Vec::new();
    let mut refused = false;
    // SAFETY: the kernel wrote msg_controllen bytes of control messages
    // into `control`; CMSG_FIRSTHDR and CMSG_NXTHDR walk only within them,
    // and each SCM_RIGHTS message's data holds whole descriptors, which are
    // new and ours to own.
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&raw const message);
        while !header.is_null() {
            let data = libc::CMSG_DATA(header);
            let length = ((*header).cmsg_len as usize).saturating_sub(data.offset_from(header.cast::<u8>()).unsigned_abs());
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                for index in 0..length.checked_div(size_of::<RawFd>()).unwrap_or(0) {
                    let descriptor: RawFd = ptr::read_unaligned(data.add(index.saturating_mul(size_of::<RawFd>())).cast());
                    descriptors.push(OwnedFd::from_raw_fd(descriptor));
                }
            } else {
                refused = true;
            }
            header = libc::CMSG_NXTHDR(&raw const message, header);
        }
    }
    if refused || descriptors.len() > MAX_DESCRIPTORS || message.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected control message"));
    }
    #[cfg(not(target_os = "linux"))]
    for descriptor in &descriptors {
        // SAFETY: fcntl on a descriptor we own, with integer arguments.
        unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    let mut stream = stream;
    stream.read_exact(buffer.get_mut(received..).unwrap_or_default())?;
    Ok(descriptors)
}

// POSIX `<time.h>` (IEEE Std 1003.1-2017): sets the C library's time zone
// from `TZ`, or the system's default. The `libc` crate declares it only for
// Windows.
unsafe extern "C" {
    fn tzset();
}

/// The local time zone's offset from UTC at `utc` seconds since 1970, in
/// seconds, as the C library's `localtime_r` gives it (`tm_gmtoff`). The C
/// library reads the zone itself; `localtime_r` need not read it again
/// (POSIX), and glibc's does not, so `tzset` asks it to first: glibc then
/// reads `TZ`, or `/etc/localtime` again when it changed, and a change of
/// zone while the browser runs is seen at once.
#[allow(clippy::useless_conversion, reason = "time_t is 64 bits on the platforms built for, and 32 on some others")]
pub fn local_offset(utc: i64) -> io::Result<i32> {
    let time: libc::time_t = utc.try_into().map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `tzset` takes no arguments and only sets the C library's own
    // zone state, which `localtime_r` then reads.
    unsafe { tzset() };
    // SAFETY: an all-zero `tm` is a valid value of a plain C struct.
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to live values of the types the call takes,
    // and the result is checked before `local` is read.
    let result = unsafe { libc::localtime_r(&raw const time, &raw mut local) };
    if result.is_null() {
        return Err(io::Error::last_os_error());
    }
    i32::try_from(local.tm_gmtoff).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;
    use std::fs::File;

    /// The local offset is one a zone can have, and the broker answers
    /// it (`history-and-bookmarks.md` section 7).
    #[test]
    fn the_local_offset_is_a_zones() {
        let time = crate::local_time().unwrap();
        assert!(time.is_plausible(), "{time:?}");
        assert!(time.utc > 1_700_000_000, "a clock after 2023: {time:?}");
    }

    /// A child of this test's process, with its own environment: set to run
    /// the body of `a_change_of_zone_is_seen_at_once`.
    const ZONE_CHILD: &str = "NEOTELETEXT_ZONE_CHILD";

    /// `history-and-bookmarks.md` section 14, item 1: a change of zone while
    /// the browser runs is seen at the next reading, not at the next start.
    /// The zone is changed in a child process that runs this test alone, so
    /// no other thread reads the environment while it changes.
    #[test]
    fn a_change_of_zone_is_seen_at_once() {
        // 1 October 2026: India keeps no summer time.
        const UTC: i64 = 1_790_812_800;
        const INDIA: i32 = 5 * 3600 + 30 * 60;
        if std::env::var_os(ZONE_CHILD).is_some() {
            assert_eq!(local_offset(UTC).unwrap(), 0, "TZ=UTC0");
            // SAFETY: this process runs this one test on one thread, and
            // nothing else of it reads the environment while it changes.
            unsafe { std::env::set_var("TZ", "IST-5:30") };
            assert_eq!(local_offset(UTC).unwrap(), INDIA, "the new zone, read at once");
            return;
        }
        let test = std::env::current_exe().unwrap();
        let status = std::process::Command::new(test)
            .args(["unix::tests::a_change_of_zone_is_seen_at_once", "--exact", "--test-threads=1"])
            .env(ZONE_CHILD, "1")
            .env("TZ", "UTC0")
            .status()
            .unwrap();
        assert!(status.success(), "the child saw the zone change: {status}");
    }

    #[test]
    fn descriptors_travel_with_their_message() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let (read_end, write_end) = io::pipe().unwrap();
        let read_end = OwnedFd::from(read_end);
        send(&ours, b"hello", &[read_end.as_raw_fd()]).unwrap();
        drop(read_end);
        let mut buffer = [0_u8; 5];
        let mut descriptors = receive(&theirs, &mut buffer).unwrap();
        assert_eq!(&buffer, b"hello");
        assert_eq!(descriptors.len(), 1);
        let mut write_end = File::from(OwnedFd::from(write_end));
        write_end.write_all(b"through").unwrap();
        drop(write_end);
        let mut text = String::new();
        File::from(descriptors.remove(0)).read_to_string(&mut text).unwrap();
        assert_eq!(text, "through", "the passed descriptor is the pipe's read end");
    }
}
