//! A file the user alone may read and write (`history-and-bookmarks.md`
//! section 2.3): how every kept file is opened, and how one is written
//! whole through a temporary (structure plan `20-browser-core.md` step 1).

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
