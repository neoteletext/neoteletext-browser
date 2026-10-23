//! Passing open files between processes on macOS and Linux: a message on
//! a local-domain socket carries descriptors with it (`SCM_RIGHTS`,
//! `unix(7)`), as the broker passes the trusted core each helper's pipes.

#![expect(unsafe_code, reason = "sendmsg and recvmsg with SCM_RIGHTS, which the standard library does not offer yet (decision 56)")]

use std::io::{self, Read as _, Write as _};
use std::mem::size_of;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::ptr;

/// The most descriptors one message carries: a helper's two pipes.
pub const MAX_DESCRIPTORS: usize = 2;
/// Room for that many descriptors' control message, as `CMSG_SPACE`
/// computes it.
const CONTROL_BYTES: usize = 64;

/// The control buffer, aligned for `cmsghdr`, which the kernel and the
/// `CMSG_*` macros read and write through it (finding F-56): a bare
/// `[u8; N]` has alignment 1, and casting it to `*mut cmsghdr`
/// (alignment 8 on Linux, 4 on macOS) and reading its fields is undefined
/// behaviour, which the compiler may assume away.
#[repr(C, align(8))]
struct Control([u8; CONTROL_BYTES]);

impl Control {
    /// A zeroed buffer.
    fn new() -> Self {
        Self([0; CONTROL_BYTES])
    }
}

/// The buffer is large enough for the most descriptors a message carries,
/// as `CMSG_SPACE` computes that room (finding F-56): checked here, once,
/// at compile time.
/// The descriptors' bytes, as `CMSG_SPACE` takes them: two pointers'
/// worth, well within a `c_uint`.
#[allow(clippy::cast_possible_truncation, reason = "MAX_DESCRIPTORS * 4 bytes is 8, far within a c_uint")]
const DESCRIPTOR_BYTES: libc::c_uint = (MAX_DESCRIPTORS * size_of::<RawFd>()) as libc::c_uint;
const _: () = {
    // SAFETY: CMSG_SPACE computes a size from an integer, touching no
    // memory; it is a const fn.
    let needed = unsafe { libc::CMSG_SPACE(DESCRIPTOR_BYTES) } as usize;
    assert!(CONTROL_BYTES >= needed, "the control buffer is too small for its descriptors");
};

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
    let mut control = Control::new();
    let descriptor_bytes = u32::try_from(size_of::<RawFd>().saturating_mul(descriptors.len())).map_err(|_| io::ErrorKind::InvalidInput)?;
    // SAFETY: CMSG_SPACE computes a size from an integer.
    let space = unsafe { libc::CMSG_SPACE(descriptor_bytes) } as usize;
    if space > control.0.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "control message too large"));
    }
    let mut part = libc::iovec { iov_base: bytes.as_ptr().cast_mut().cast(), iov_len: bytes.len() };
    // SAFETY: a zeroed msghdr is valid: null pointers and zero lengths.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &raw mut part;
    message.msg_iovlen = 1;
    if !descriptors.is_empty() {
        message.msg_control = control.0.as_mut_ptr().cast();
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
    let mut control = Control::new();
    let mut part = libc::iovec { iov_base: buffer.as_mut_ptr().cast(), iov_len: buffer.len() };
    // SAFETY: a zeroed msghdr is valid.
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &raw mut part;
    message.msg_iovlen = 1;
    message.msg_control = control.0.as_mut_ptr().cast();
    message.msg_controllen = control_length(control.0.len())?;
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
    // Truncated control data means a descriptor was dropped, and on macOS
    // the header is copied with its original length though the data was
    // cut (finding F-57): refuse before wrapping anything, so no descriptor
    // of ours is taken from garbage.
    if message.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the control message was truncated"));
    }
    let mut descriptors = Vec::new();
    let mut refused = false;
    // The control data the kernel actually wrote, which bounds every
    // header and its data (finding F-57): `cmsg_len` is not trusted past it.
    let control_end = control.0.as_ptr().wrapping_add(message.msg_controllen as usize);
    // SAFETY: the kernel wrote msg_controllen bytes of control messages
    // into `control`; CMSG_FIRSTHDR and CMSG_NXTHDR walk only within them,
    // and each SCM_RIGHTS message's data, clamped to that length, holds
    // whole descriptors, which are new and ours to own.
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&raw const message);
        while !header.is_null() {
            let data = libc::CMSG_DATA(header);
            // The data runs from `data` to the header's end, but never past
            // the control data the kernel wrote.
            let claimed = (*header).cmsg_len as usize;
            let before_data = data.offset_from(header.cast::<u8>()).unsigned_abs();
            let within_buffer = control_end.offset_from(data).max(0).unsigned_abs();
            let length = claimed.saturating_sub(before_data).min(within_buffer);
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
    if refused || descriptors.len() > MAX_DESCRIPTORS {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected control message"));
    }
    // macOS has no MSG_CMSG_CLOEXEC, so the flag is set here; a failure
    // leaves a descriptor that could be inherited by a later child, so it
    // is an error (finding F-57). Linux set it atomically in recvmsg.
    #[cfg(not(target_os = "linux"))]
    for descriptor in &descriptors {
        // SAFETY: fcntl on a descriptor we own, with integer arguments.
        if unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    let mut stream = stream;
    stream.read_exact(buffer.get_mut(received..).unwrap_or_default())?;
    Ok(descriptors)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;
    use std::fs::File;

    /// Finding F-56: the control buffer is aligned at least as `cmsghdr`
    /// needs, which the `CMSG_*` macros assume when they read it through a
    /// `*mut cmsghdr`. A bare byte array (the code before) had alignment 1.
    #[test]
    fn the_control_buffer_is_aligned_for_its_header() {
        assert!(align_of::<Control>() >= align_of::<libc::cmsghdr>(), "{} < {}", align_of::<Control>(), align_of::<libc::cmsghdr>());
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
