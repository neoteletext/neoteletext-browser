//! What names a file whatever its path (DEC-1551 item 1): a grant is
//! honoured only while its path still leads to this file.

use std::fs::File;
use std::io;

/// A file's identity: device and inode on macOS and Linux, volume and file
/// index on Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    /// The device or volume.
    pub volume: u64,
    /// The inode or file index.
    pub file: u64,
}

/// The identity of the open file `file`: of the descriptor, so that nothing
/// changed since it was opened can stand in for it.
///
/// # Errors
/// When the system will not say.
#[cfg(unix)]
pub fn file_identity(file: &File) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = file.metadata()?;
    Ok(FileIdentity { volume: metadata.dev(), file: metadata.ino() })
}

/// The identity of the open file `file`: of the handle, so that nothing
/// changed since it was opened can stand in for it.
///
/// # Errors
/// When the system will not say.
#[cfg(windows)]
pub fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let (volume, file) = crate::windows::identity_of(file)?;
    Ok(FileIdentity { volume, file })
}
