//! Linux: memory the interface shares with its display server, mapped
//! into the interface. For X11 it is a POSIX shared-memory object
//! (`shm_open(3)`, in the interface's own empty `/dev/shm`, sandboxing.md:
//! DEC-6659 item 24), unlinked at once, whose descriptor the X server is
//! handed (`MIT-SHM` 1.2's `ShmAttachFd`); for Wayland a sealed memory
//! file (`memfd_create(2)`), whose descriptor the compositor is handed
//! (`wl_shm.create_pool`).
#![allow(unsafe_code, reason = "mapping shared memory, and reading it as pixels, has no safe interface: each block has its reason")]

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::ptr::NonNull;

use rustix::fs::{MemfdFlags, Mode, SealFlags};
use rustix::mm::{MapFlags, ProtFlags};
use rustix::shm::OFlags;

use crate::{BYTES_PER_PIXEL, Error};

/// How many names a shared-memory object is tried under before giving up:
/// a name already taken is another's, never opened.
const NAME_TRIES: u32 = 8;

/// Where the memory comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A POSIX shared-memory object, for the X server.
    Posix,
    /// A sealed memory file, for a Wayland compositor.
    Sealed,
}

/// A file of shared memory and its mapping, unmapped when dropped.
#[derive(Debug)]
pub struct Mapping {
    file: OwnedFd,
    pointer: NonNull<u32>,
    bytes: usize,
}

// SAFETY: the mapping is owned by this value alone, which is used from one
// thread at a time through `&mut self` or `&self`; the pointer is not
// shared otherwise.
unsafe impl Send for Mapping {}

/// A new name for a shared-memory object: this process's, and a count
/// and the clock's nanoseconds, so that two windows or two processes do not
/// collide; a collision is refused by `O_EXCL` and tried again.
fn name(attempt: u32) -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static MADE: AtomicU32 = AtomicU32::new(0);
    let made = MADE.fetch_add(1, Ordering::Relaxed);
    let nanoseconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.subsec_nanos());
    format!("/neoteletext-{}-{made}-{attempt}-{nanoseconds}", std::process::id())
}

fn file(kind: Kind) -> Result<OwnedFd, Error> {
    match kind {
        Kind::Posix => {
            for attempt in 0..NAME_TRIES {
                let name = name(attempt);
                match rustix::shm::open(name.as_str(), OFlags::RDWR | OFlags::CREATE | OFlags::EXCL, Mode::RUSR | Mode::WUSR) {
                    Ok(file) => {
                        // Unlinked at once: the descriptor alone reaches it.
                        let _ = rustix::shm::unlink(name.as_str());
                        return Ok(file);
                    }
                    Err(rustix::io::Errno::EXIST) => {}
                    Err(error) => return Err(Error::new(format!("shared memory: {error}"))),
                }
            }
            Err(Error::new("shared memory: every name tried was taken"))
        }
        Kind::Sealed => {
            let file = rustix::fs::memfd_create(c"neoteletext", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)
                .map_err(|error| Error::new(format!("shared memory: {error}")))?;
            // The compositor may count on the memory never shrinking under
            // it (`wl_shm`'s description of the pool).
            rustix::fs::fcntl_add_seals(&file, SealFlags::SHRINK | SealFlags::SEAL).map_err(|error| Error::new(format!("sealing shared memory: {error}")))?;
            Ok(file)
        }
    }
}

impl Mapping {
    /// Shared memory of `pixels` pixels.
    pub fn new(kind: Kind, pixels: usize) -> Result<Self, Error> {
        let bytes = pixels.checked_mul(BYTES_PER_PIXEL).filter(|&bytes| bytes > 0).ok_or_else(|| Error::new("shared memory of no pixels"))?;
        let file = file(kind)?;
        rustix::fs::ftruncate(&file, u64::try_from(bytes).unwrap_or(u64::MAX)).map_err(|error| Error::new(format!("sizing shared memory: {error}")))?;
        let pointer = map(&file, bytes)?;
        Ok(Self { file, pointer, bytes })
    }

    /// Its size in bytes.
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// Grows it to hold `pixels` pixels, mapped again; the file is the same,
    /// so what the display server holds of it stays valid.
    pub fn grow(&mut self, pixels: usize) -> Result<(), Error> {
        let bytes = pixels.checked_mul(BYTES_PER_PIXEL).ok_or_else(|| Error::new("shared memory too large"))?;
        if bytes <= self.bytes {
            return Ok(());
        }
        rustix::fs::ftruncate(&self.file, u64::try_from(bytes).unwrap_or(u64::MAX)).map_err(|error| Error::new(format!("sizing shared memory: {error}")))?;
        let pointer = map(&self.file, bytes)?;
        // SAFETY: the old mapping was made by `map` with this length, and
        // nothing refers to it once this returns.
        unsafe { unmap(self.pointer, self.bytes) };
        self.pointer = pointer;
        self.bytes = bytes;
        Ok(())
    }

    /// The first `pixels` pixels.
    pub fn pixels(&self, pixels: usize) -> &[u32] {
        let pixels = pixels.min(self.bytes / BYTES_PER_PIXEL);
        // SAFETY: the mapping is `self.bytes` long, readable and aligned
        // for `u32` (a page), and lives as long as `self`; the display
        // server writes nothing into it.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), pixels) }
    }

    /// The first `pixels` pixels, to draw into.
    pub fn pixels_mut(&mut self, pixels: usize) -> &mut [u32] {
        let pixels = pixels.min(self.bytes / BYTES_PER_PIXEL);
        // SAFETY: as above, and writable; `&mut self` makes this the only
        // reference into the mapping in this process. The display server
        // reads it only between a present and the wait the next frame
        // makes before drawing (`crate::x11`, `crate::wayland`).
        unsafe { std::slice::from_raw_parts_mut(self.pointer.as_ptr(), pixels) }
    }
}

impl AsFd for Mapping {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

fn map(file: &OwnedFd, bytes: usize) -> Result<NonNull<u32>, Error> {
    // SAFETY: a new shared mapping of a file at least `bytes` long, at an
    // address the kernel chooses, so nothing already mapped is replaced.
    let pointer = unsafe { rustix::mm::mmap(std::ptr::null_mut(), bytes, ProtFlags::READ | ProtFlags::WRITE, MapFlags::SHARED, file, 0) }
        .map_err(|error| Error::new(format!("mapping shared memory: {error}")))?;
    NonNull::new(pointer.cast::<u32>()).ok_or_else(|| Error::new("mapping shared memory gave nothing"))
}

/// # Safety
/// `pointer` and `bytes` are a mapping `map` made, referred to by nothing.
unsafe fn unmap(pointer: NonNull<u32>, bytes: usize) {
    // SAFETY: the caller's.
    let _ = unsafe { rustix::mm::munmap(pointer.as_ptr().cast(), bytes) };
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: the mapping is this value's, and ends with it.
        unsafe { unmap(self.pointer, self.bytes) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both kinds of shared memory hold the pixels written, grow without
    /// losing them, and are refused at no size.
    #[test]
    fn shared_memory_holds_what_is_drawn_and_grows() {
        for kind in [Kind::Posix, Kind::Sealed] {
            let mut mapping = Mapping::new(kind, 16).unwrap();
            mapping.pixels_mut(16).iter_mut().enumerate().for_each(|(at, pixel)| *pixel = u32::try_from(at).unwrap());
            mapping.grow(1 << 20).unwrap();
            assert_eq!(mapping.bytes(), 4 << 20);
            assert!(mapping.pixels(16).iter().enumerate().all(|(at, &pixel)| pixel == u32::try_from(at).unwrap()), "{kind:?}: kept across the growth");
            assert_eq!(mapping.pixels(usize::MAX).len(), 1 << 20, "{kind:?}: never more than it holds");
            assert!(Mapping::new(kind, 0).is_err());
        }
    }
}
