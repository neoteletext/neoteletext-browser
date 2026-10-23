//! A file the user alone may read and write (`history-and-bookmarks.md`
//! section 2.3): how every kept file is opened, and how one is written
//! whole through a temporary, and how one is read within its bound.

use std::path::Path;

/// Options for a file the user alone may read and write (section 2.3).
pub(crate) fn private_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

/// Writes `bytes` to `name` in `folder` whole, through `temporary`.
pub(crate) fn write_whole(folder: &Path, name: &str, temporary: &str, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write as _;
    std::fs::create_dir_all(folder)?;
    let temporary = folder.join(temporary);
    let mut file = private_options().create(true).truncate(true).open(&temporary)?;
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
