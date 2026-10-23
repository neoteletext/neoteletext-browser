//! Writes that survive a power cut, and the exchange of two folders
//! (`installation-and-updates.md` sections 3.1 and 7). A file is written
//! whole under a new name, flushed, and renamed over the old one, and its
//! folder flushed, so it is either the old file or the new one; on
//! Windows 10 1607 and later `std::fs::rename` replaces in one call on
//! NTFS, and `File::sync_all` is `FlushFileBuffers` (section 4.2).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::fault;

/// The suffix of a file being written, before its rename.
pub const NEW_SUFFIX: &str = ".new";

/// `path` with `suffix` added to its name.
#[must_use]
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().map(std::ffi::OsStr::to_os_string).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Flushes a folder, so the names in it, a rename's above all, are on the
/// disk. Windows flushes the journal of a rename itself; opening a folder
/// there needs a flag std does not set, and the flush is left to it.
///
/// # Errors
/// When the folder cannot be opened or flushed.
pub fn sync_folder(folder: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(folder)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        fs::metadata(folder).map(|_| ())
    }
}

/// Writes `bytes` to `path` whole: to a new file, flushed, then renamed
/// over `path`, and the folder flushed (requirement 13.4).
///
/// # Errors
/// When any step fails; `path` is then as it was.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let new = with_suffix(path, NEW_SUFFIX);
    write_synced(&new, bytes, false)?;
    fault::point("state-written-before-rename");
    fs::rename(&new, path)?;
    fault::point("state-renamed");
    if let Some(folder) = path.parent() {
        sync_folder(folder)?;
    }
    Ok(())
}

/// Writes `bytes` to a new file at `path`, flushed to the disk, and
/// executable if a program.
///
/// # Errors
/// When it cannot be written.
pub fn write_synced(path: &Path, bytes: &[u8], executable: bool) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(if executable { MODE_PROGRAM } else { MODE_FILE });
    }
    #[cfg(not(unix))]
    let _ = executable;
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    {
        // A file that existed keeps its mode through `open`; set it.
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode(if executable { MODE_PROGRAM } else { MODE_FILE }))?;
    }
    file.sync_all()
}

/// A program's mode: read and run by all, written by the user.
#[cfg(unix)]
const MODE_PROGRAM: u32 = 0o755;
/// A file's mode: read by all, written by the user.
#[cfg(unix)]
const MODE_FILE: u32 = 0o644;

/// Reads at most `max` bytes of `path`, refusing a longer file rather than
/// reading part of it.
///
/// # Errors
/// When it cannot be read or is longer than `max`.
pub fn read_bounded(path: &Path, max: usize) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} is longer than {max} bytes", path.display())));
    }
    Ok(bytes)
}

/// Exchanges two folders on the same volume in one step: each name then
/// names the other's contents (requirement 13.7).
///
/// # Errors
/// When the system refuses, as a file system without the call does
/// (`EINVAL`); nothing has then changed.
#[cfg(unix)]
pub fn exchange(a: &Path, b: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(rustix::fs::CWD, a, rustix::fs::CWD, b, rustix::fs::RenameFlags::EXCHANGE).map_err(io::Error::from)
}

/// Copies the folder `from` to the new folder `to`, files flushed and
/// programs kept executable; a symbolic link, which no release has, is
/// refused, so nothing outside `from` is reached.
///
/// # Errors
/// When anything cannot be read or written, or is a link.
pub fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} is a link", entry.path().display())));
        } else if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            let bytes = fs::read(entry.path())?;
            write_synced(&target, &bytes, is_executable(&entry.metadata()?))?;
        }
    }
    sync_folder(to)
}

/// Whether a file is a program by its mode.
#[must_use]
pub fn is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

/// Removes a folder and everything in it, if it is there; links inside it
/// are removed, never followed.
///
/// # Errors
/// When something in it cannot be removed (on Windows, a program still
/// running: section 3.1 step 7 leaves it for next time).
pub fn remove_tree(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Every file under `folder`, as paths relative to it with `/` between
/// parts, in byte order; a link is refused.
///
/// # Errors
/// When the folder cannot be read, holds a link, or holds more than
/// `max` files.
pub fn list_files(folder: &Path, max: usize) -> io::Result<Vec<String>> {
    fn walk(base: &Path, folder: &Path, prefix: &str, max: usize, out: &mut Vec<String>) -> io::Result<()> {
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let name = entry.file_name().into_string().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a name that is not text"))?;
            let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if kind.is_symlink() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{relative} is a link")));
            } else if kind.is_dir() {
                walk(base, &entry.path(), &relative, max, out)?;
            } else {
                if out.len() >= max {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} holds too many files", base.display())));
                }
                out.push(relative);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(folder, folder, "", max, &mut out)?;
    out.sort();
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("neoteletext-update-files-{name}-{}", std::process::id()));
        let _ = remove_tree(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn a_file_is_replaced_whole() {
        let folder = scratch("atomic");
        let path = folder.join("state");
        write_atomically(&path, b"one\n").unwrap();
        write_atomically(&path, b"two\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two\n");
        assert!(!with_suffix(&path, NEW_SUFFIX).exists());
        assert_eq!(read_bounded(&path, 4).unwrap(), b"two\n");
        assert!(read_bounded(&path, 3).is_err(), "longer than allowed");
        remove_tree(&folder).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn two_folders_exchange_in_one_step() {
        let folder = scratch("exchange");
        let (a, b) = (folder.join("a"), folder.join("b"));
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        fs::write(a.join("which"), "a").unwrap();
        fs::write(b.join("which"), "b").unwrap();
        exchange(&a, &b).unwrap();
        assert_eq!(fs::read_to_string(a.join("which")).unwrap(), "b");
        assert_eq!(fs::read_to_string(b.join("which")).unwrap(), "a");
        assert!(exchange(&a, &folder.join("missing")).is_err());
        remove_tree(&folder).unwrap();
    }

    #[test]
    fn a_tree_is_copied_and_listed_and_a_link_refused() {
        let folder = scratch("tree");
        let from = folder.join("from");
        fs::create_dir_all(from.join("Contents/MacOS")).unwrap();
        write_synced(&from.join("Contents/MacOS/program"), b"#", true).unwrap();
        fs::write(from.join("Contents/Info.plist"), b"<plist/>").unwrap();
        copy_tree(&from, &folder.join("to")).unwrap();
        assert_eq!(list_files(&folder.join("to"), 8).unwrap(), vec!["Contents/Info.plist", "Contents/MacOS/program"]);
        assert!(list_files(&folder.join("to"), 1).is_err(), "too many");
        #[cfg(unix)]
        {
            assert!(is_executable(&fs::metadata(folder.join("to/Contents/MacOS/program")).unwrap()));
            std::os::unix::fs::symlink("/etc", from.join("link")).unwrap();
            assert!(copy_tree(&from, &folder.join("again")).is_err());
            assert!(list_files(&from, 8).is_err());
        }
        remove_tree(&folder).unwrap();
    }
}
