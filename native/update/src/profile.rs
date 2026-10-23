//! The user's data across versions (`installation-and-updates.md` section
//! 3.3, requirement 13.6): each settings format is a file of its own. A
//! version that writes a new format writes `settings-<format>.txt`, first
//! copying from the newest older format's file it finds, and never writes
//! an older format's file, so a version rolled back finds its own file as
//! it left it. Format 1 is `settings.txt`, as every release so far wrote.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The most formats looked back through for one to copy from.
const MAX_FORMATS: u32 = 64;

/// The file of format `format` of the data named `base` (`settings`).
#[must_use]
pub fn file_name(base: &str, format: u32) -> String {
    if format <= 1 { format!("{base}.txt") } else { format!("{base}-{format}.txt") }
}

/// The path of `base`'s file of `format` in `folder`, made by copying the
/// newest older format's file when it does not yet exist. Returns the
/// path and the format copied from, if one was.
///
/// # Errors
/// When the copy fails; nothing older is changed either way.
pub fn prepare(folder: &Path, base: &str, format: u32) -> io::Result<(PathBuf, Option<u32>)> {
    let path = folder.join(file_name(base, format));
    if path.exists() {
        return Ok((path, None));
    }
    let lowest = format.saturating_sub(MAX_FORMATS).max(1);
    for older in (lowest..format).rev() {
        let source = folder.join(file_name(base, older));
        if source.is_file() {
            let bytes = fs::read(&source)?;
            crate::files::write_atomically(&path, &bytes)?;
            return Ok((path, Some(older)));
        }
    }
    Ok((path, None))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// Section 13: migrate to `settings-2.txt`, roll back, and find
    /// `settings.txt` as it was.
    #[test]
    fn a_new_format_copies_and_the_old_file_is_kept_as_it_was() {
        let folder = std::env::temp_dir().join(format!("neoteletext-profile-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("settings.txt"), "neoteletext-settings 1\nhome neoteletext:home\n").unwrap();
        let (path, copied) = prepare(&folder, "settings", 2).unwrap();
        assert_eq!((path.file_name().unwrap().to_str().unwrap(), copied), ("settings-2.txt", Some(1)));
        // The newer version writes its own format.
        fs::write(&path, "neoteletext-settings 2\nhome https://news.example/\n").unwrap();
        // Rolled back: format 1's file is as the old version left it.
        let (old, copied) = prepare(&folder, "settings", 1).unwrap();
        assert_eq!(copied, None);
        assert_eq!(fs::read_to_string(old).unwrap(), "neoteletext-settings 1\nhome neoteletext:home\n");
        // Updated again: format 2's file is kept, not copied over.
        assert_eq!(prepare(&folder, "settings", 2).unwrap().1, None);
        assert_eq!(prepare(&folder, "settings", 3).unwrap().1, Some(2), "the newest older format");
        fs::remove_dir_all(&folder).unwrap();
    }
}
