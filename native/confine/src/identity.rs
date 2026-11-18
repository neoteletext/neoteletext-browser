//! What names a file whatever its path (DEC-1551 item 1): a grant is
//! honoured only while its path still leads to this file.
//!
//! Device and inode alone do not name a file over time: ext4 gives a file
//! made just after another was removed the inode number just freed, so a
//! different file put at a granted path had the granted file's identity
//! and was read through its grant (TODO-6629, finding F-03). The identity
//! therefore holds when the file was made, which a new file at a reused
//! inode cannot share.

use std::fs::File;
use std::io;
use std::time::{SystemTime, UNIX_EPOCH};

/// A file's identity: device and inode on macOS and Linux, volume and file
/// index on Windows, and when the file was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    /// The device or volume.
    pub volume: u64,
    /// The inode or file index.
    pub file: u64,
    /// When the file was made, in nanoseconds since 1970: its birth time
    /// where the file system records one (statx on Linux, the birth time
    /// on macOS, the creation time on Windows), else the time its inode
    /// last changed, which is stricter: a grant then also ends when the
    /// file's metadata changes, never when another file takes its inode.
    pub born: u128,
}

/// Nanoseconds in a second.
#[cfg(unix)]
const NANOSECONDS_PER_SECOND: u128 = 1_000_000_000;

/// Nanoseconds since 1970 of `time`, 0 for a time before it.
fn nanoseconds(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos())
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
    // With no birth time on this file system, the inode's change time.
    let changed = || {
        let seconds = u128::try_from(metadata.ctime()).unwrap_or(0);
        let nanos = u128::try_from(metadata.ctime_nsec()).unwrap_or(0);
        seconds.saturating_mul(NANOSECONDS_PER_SECOND).saturating_add(nanos)
    };
    let born = metadata.created().map_or_else(|_| changed(), nanoseconds);
    Ok(FileIdentity { volume: metadata.dev(), file: metadata.ino(), born })
}

/// The path the system resolved the open file `file` by, every link and
/// junction followed, in the form `std::fs::canonicalize` gives: compared
/// with the canonical path opened, it shows a junction swapped in for a
/// folder of that path after it was resolved, which opening the last
/// component as a reparse point does not (TODO-1278 item 1, decision 88).
///
/// # Errors
/// When the system will not say.
#[cfg(windows)]
pub fn opened_path(file: &File) -> io::Result<std::path::PathBuf> {
    crate::windows::final_path_of(file)
}

/// The identity of the open file `file`: of the handle, so that nothing
/// changed since it was opened can stand in for it.
///
/// # Errors
/// When the system will not say.
#[cfg(windows)]
pub fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let born = file.metadata()?.created().map(nanoseconds).unwrap_or(0);
    let (volume, file) = crate::windows::identity_of(file)?;
    Ok(FileIdentity { volume, file, born })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// TODO-6629, finding F-03: a file made where another was removed is
    /// another file, even when the file system gives it the inode number
    /// just freed. ext4 does at once; the test makes and removes files in
    /// the build's own folder, on the disk, until it does, and then holds
    /// that the two identities differ. On the code before, which knew only
    /// device and inode, they were equal.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_file_made_at_a_freed_inode_is_another_file() {
        use std::os::unix::fs::MetadataExt as _;
        /// Tries before giving up on a file system that does not reuse
        /// inode numbers at once.
        const TRIES: usize = 64;
        let folder = std::env::current_exe().unwrap().parent().unwrap().join(format!("identity-reuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("granted");
        let mut reused = false;
        for _ in 0..TRIES {
            std::fs::write(&path, b"granted").unwrap();
            let granted = file_identity(&File::open(&path).unwrap()).unwrap();
            std::fs::remove_file(&path).unwrap();
            std::fs::write(&path, b"another").unwrap();
            let another = File::open(&path).unwrap();
            if another.metadata().unwrap().ino() == granted.file {
                reused = true;
                assert_ne!(file_identity(&another).unwrap(), granted, "another file at the granted file's inode");
            }
            std::fs::remove_file(&path).unwrap();
            if reused {
                break;
            }
        }
        std::fs::remove_dir_all(&folder).unwrap();
        assert!(reused, "the build folder's file system gave no freed inode again in {TRIES} tries: this test needs one that does, as ext4 does");
    }
}
