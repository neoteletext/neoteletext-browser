//! A file the user alone may read and write (`history-and-bookmarks.md`
//! section 2.3): how every kept file is opened, and how one is written
//! whole through a temporary, and how one is read within its bound.

use std::path::Path;

/// Options for a file the user alone may read and write (section 2.3),
/// never opened through a link (finding F-35).
pub(crate) fn private_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
}

/// Makes `folder`, and any folder above it that is missing, the user's
/// alone (finding F-35).
///
/// # Errors
/// When a folder cannot be made.
pub fn make_folder(folder: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(folder)
}

/// Makes a kept file that already exists the user's alone: one written
/// before its mode was set, or by another program.
/// Windows keeps a file's access by the folder's, the user's own.
#[cfg(unix)]
pub(crate) fn keep_private(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// Writes `bytes` to `name` in `folder` whole, through `temporary`: a new
/// file, never one found there, so a link put at its name is not
/// followed (finding F-35).
pub(crate) fn write_whole(folder: &Path, name: &str, temporary: &str, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write as _;
    make_folder(folder)?;
    let temporary = folder.join(temporary);
    match std::fs::remove_file(&temporary) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut file = private_options().create_new(true).open(&temporary)?;
    file.write_all(bytes.as_ref())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, folder.join(name))
}

/// What a kept file holds, as its `load` needs it.
pub(crate) enum Read {
    /// There is no file.
    Missing,
    /// The file is longer than its bound, and none of it was read.
    TooLong,
    /// The file's text, any bytes that are not UTF-8 replaced.
    Text(String),
}

/// Reads the kept file at `path` if it is there and at most `max` bytes:
/// each caller turns a missing file into its default and an overlong one
/// into a dropped line, never into an error.
///
/// # Errors
/// When the file exists but cannot be read.
pub(crate) fn read_text(path: &Path, max: usize) -> std::io::Result<Read> {
    match std::fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Read::Missing),
        Err(error) => return Err(error),
        Ok(metadata) if metadata.len() > u64::try_from(max).unwrap_or(u64::MAX) => return Ok(Read::TooLong),
        Ok(_) => {}
    }
    let bytes = std::fs::read(path)?;
    // Valid text is kept as it was read; only other bytes are copied.
    Ok(Read::Text(String::from_utf8(bytes).unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())))
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    /// The mode bits of a path's permissions.
    const MODE_BITS: u32 = 0o777;

    /// Finding F-35: a link put at a kept file's temporary name is not
    /// followed, so nothing outside the folder is written through it; the
    /// folder made is the user's alone, and so is the file.
    #[test]
    fn a_link_at_the_temporary_name_is_not_followed() {
        let root = std::env::temp_dir().join(format!("neoteletext-private-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let victim = root.join("victim.txt");
        std::fs::write(&victim, b"the user's own").unwrap();
        let folder = root.join("settings");
        write_whole(&folder, "kept.txt", "kept.txt.new", b"first").unwrap();
        assert_eq!(std::fs::metadata(&folder).unwrap().permissions().mode() & MODE_BITS, 0o700, "the folder made");
        std::os::unix::fs::symlink(&victim, folder.join("kept.txt.new")).unwrap();
        write_whole(&folder, "kept.txt", "kept.txt.new", b"second").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"the user's own", "the link was not followed");
        assert_eq!(std::fs::read(folder.join("kept.txt")).unwrap(), b"second");
        assert_eq!(std::fs::metadata(folder.join("kept.txt")).unwrap().permissions().mode() & MODE_BITS, 0o600);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
