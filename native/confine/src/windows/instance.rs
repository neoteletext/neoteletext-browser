//! The single-instance channel on Windows (DEC-1551 item 7): a named pipe
//! with the default access list, which names no `AppContainer`, so no
//! sandboxed process opens it.

#![expect(unsafe_code, reason = "the named pipe's calls, which the standard library does not offer (DEC-1551 item 7)")]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::io::FromRawHandle as _;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use super::handles::{Owned, last_error, wide};

/// The buffers the system gives one hand-over: what it carries is bounded
/// by `MAX_OPENING`, and a write beyond them waits for the reader.
const PIPE_BUFFER: u32 = 64 * 1024;

/// One end of a hand-over.
pub type Stream = File;

/// The running browser's end of the channel.
pub struct Instance {
    name: Vec<u16>,
    next: std::sync::Mutex<Option<Owned>>,
}

// SAFETY: the handle it holds is a kernel object's, which any thread may use, and the
// mutex gives it to one at a time.
unsafe impl Send for Instance {}
// SAFETY: as above.
unsafe impl Sync for Instance {}

fn create(name: &[u16], first: bool) -> io::Result<Owned> {
    let access = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
    // SAFETY: the name is a live, NUL-terminated wide string, and a null security
    // attribute takes the default access list, which is the point.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            access,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE { Err(last_error()) } else { Ok(Owned::new(handle)) }
}

/// Reaches the browser already running at the pipe `place`.
///
/// # Errors
/// When none answers there.
pub fn connect(place: &Path) -> io::Result<Stream> {
    OpenOptions::new().read(true).write(true).open(place)
}

impl Instance {
    /// Takes the pipe `place`, which only one browser holds.
    ///
    /// # Errors
    /// When it cannot be taken.
    pub fn bind(place: &Path) -> io::Result<Self> {
        let name = wide(place);
        let first = create(&name, true)?;
        Ok(Self { name, next: std::sync::Mutex::new(Some(first)) })
    }

    /// The next hand-over.
    ///
    /// # Errors
    /// When the channel fails.
    pub fn next(&self) -> io::Result<Stream> {
        let waiting = self.next.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        let pipe = match waiting {
            Some(pipe) => pipe,
            None => create(&self.name, false)?,
        };
        // SAFETY: the handle is the pipe made above, owned here, and a null overlapped
        // structure is a synchronous wait.
        if unsafe { ConnectNamedPipe(pipe.raw(), ptr::null_mut()) } == 0 {
            // SAFETY: reads this thread's last error, and touches no memory.
            let error = unsafe { GetLastError() };
            if error != ERROR_PIPE_CONNECTED {
                return Err(io::Error::from_raw_os_error(error.cast_signed()));
            }
        }
        // SAFETY: the handle is owned only by `pipe`, forgotten below so the file becomes
        // its one owner.
        let file = unsafe { File::from_raw_handle(pipe.raw() as _) };
        std::mem::forget(pipe);
        Ok(file)
    }
}
