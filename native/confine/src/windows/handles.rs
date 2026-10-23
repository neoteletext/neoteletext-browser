//! The Win32 handles and SIDs the sandbox makes, each freed when dropped,
//! and the handles passed to the trusted core (decision 56).

#![expect(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::FromRawHandle as _;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, GetLastError, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LocalFree, PSID, SetHandleInformation,
};
use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows_sys::Win32::Security::{FreeSid, SECURITY_ATTRIBUTES};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// A handle closed when dropped.
pub(super) struct Owned(pub(super) HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: a handle this value owns, valid and closed once, here.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A SID from its text, freed when dropped.
pub(super) struct Sid(pub(super) PSID);

impl Sid {
    pub(super) fn parse(text: &str) -> io::Result<Self> {
        let mut sid: PSID = ptr::null_mut();
        // SAFETY: the text is NUL-terminated and alive for the call; the SID is written to a
        // live local and owned by the `Sid` returned.
        if unsafe { ConvertStringSidToSidW(wide(text).as_ptr(), &raw mut sid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(sid))
    }
}

impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: ConvertStringSidToSidW allocated this SID; it is freed once, here.
        unsafe { LocalFree(self.0.cast()) };
    }
}

/// A SID of an `AppContainer`, freed with `FreeSid`.
pub(super) struct ContainerSid(pub(super) PSID);

impl Drop for ContainerSid {
    fn drop(&mut self) {
        // SAFETY: the container calls allocated this SID, to be freed with FreeSid; once, here.
        unsafe { FreeSid(self.0) };
    }
}

/// `text` as a zero-terminated UTF-16 string.
pub(super) fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

/// The last error, as an `io::Error`.
pub(super) fn last_error() -> io::Error {
    // SAFETY: GetLastError takes nothing and reads this thread's last error.
    io::Error::from_raw_os_error(i32::try_from(unsafe { GetLastError() }).unwrap_or(0))
}

/// A pipe: the end the helper gets inherits; ours does not.
pub(super) fn pipe() -> io::Result<(Owned, Owned)> {
    let mut read: HANDLE = 0;
    let mut write: HANDLE = 0;
    let attributes =
        SECURITY_ATTRIBUTES { nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0), lpSecurityDescriptor: ptr::null_mut(), bInheritHandle: 1 };
    // SAFETY: both handles are written to live locals and owned by `Owned`; the security
    // attributes are a live local.
    if unsafe { CreatePipe(&raw mut read, &raw mut write, &raw const attributes, 0) } == 0 {
        return Err(last_error());
    }
    Ok((Owned(read), Owned(write)))
}

pub(super) fn keep_ours(handle: HANDLE) -> io::Result<()> {
    // SAFETY: `handle` is a pipe end the caller made and owns.
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 { Err(last_error()) } else { Ok(()) }
}

/// Duplicates `file`'s handle into the process `into` and returns its
/// value there, for the broker to send the core (decision 56).
pub(crate) fn duplicate_into(file: &File, into: HANDLE) -> io::Result<u64> {
    use std::os::windows::io::AsRawHandle as _;
    let mut theirs: HANDLE = 0;
    // SAFETY: the source is the handle `file` owns, alive for the call; `into` is the
    // core's process handle, owned by the caller's `Process`; the copy's value is
    // written to a live local and belongs to that process.
    if unsafe { DuplicateHandle(GetCurrentProcess(), file.as_raw_handle() as HANDLE, into, &raw mut theirs, 0, 0, DUPLICATE_SAME_ACCESS) } == 0 {
        return Err(last_error());
    }
    u64::try_from(theirs).map_err(|_| io::ErrorKind::InvalidData.into())
}

/// The file of a handle value the broker duplicated into this process.
pub(crate) fn file_from_handle_value(value: u64) -> io::Result<File> {
    let handle = HANDLE::try_from(value).map_err(|_| io::ErrorKind::InvalidData)?;
    if handle == 0 || handle == INVALID_HANDLE_VALUE {
        return Err(io::ErrorKind::InvalidData.into());
    }
    // The broker made this handle for this process and sent its value
    // once; it is ours alone to own.
    // SAFETY: the broker made this handle for this process with DuplicateHandle and sent
    // its value once over the channel, so nothing else here owns it; 0 and
    // INVALID_HANDLE_VALUE are refused above. The broker is the trusted side (decision 56).
    Ok(unsafe { File::from_raw_handle(handle as _) })
}
