//! The system's file dialogs, for the chooser (DEC-1551 item 2, as
//! DEC-7168 changed it): on macOS `AppKit`'s `NSOpenPanel` and `NSSavePanel`,
//! on Windows the shell's `IFileOpenDialog` and `IFileSaveDialog`, called
//! directly. On Linux the chooser asks the desktop portal instead, and this
//! crate has nothing.
//!
//! The dialog's title, kind, starting folder and filter are the chooser's;
//! what the person chose comes back as one path, or none for a cancel. The
//! broker believes nothing more of it (DEC-1549, DEC-1551).

// No panicking constructs: a panic in the chooser would end the dialog
// with no answer. The same list is in each crate that reads input from
// outside its process; scripts/check.sh checks that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::path::Path;
#[cfg(any(target_os = "macos", windows))]
use std::path::PathBuf;

/// What the dialog chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind<'a> {
    /// One file to open.
    File,
    /// One folder.
    Folder,
    /// A file to save, offering this name.
    Save(&'a str),
}

/// One dialog.
#[derive(Debug, Clone, Copy)]
pub struct Dialog<'a> {
    /// Its title.
    pub title: &'a str,
    /// What it chooses.
    pub kind: Kind<'a>,
    /// Where it starts.
    pub folder: Option<&'a Path>,
    /// One filter: its name and the file name endings it shows.
    pub filter: Option<(&'a str, &'a [&'a str])>,
    /// On Windows, the handle of the window that owns the dialog: another
    /// process's window, which a handle names in every process. macOS cannot
    /// place a panel over another process's window, and centres it.
    pub owner: Option<isize>,
}

/// Shows `dialog` and waits for the person: the path chosen, or none when
/// the dialog was cancelled.
///
/// # Errors
///
/// When the system cannot show the dialog.
#[cfg(any(target_os = "macos", windows))]
pub fn choose(dialog: &Dialog<'_>) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    return macos::choose(dialog);
    #[cfg(windows)]
    return windows::choose(dialog);
}

/// The Windows filter's pattern: each ending as `*.ending`, separated by
/// semicolons (`COMDLG_FILTERSPEC`'s `pszSpec`).
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's alone, tested everywhere"))]
fn windows_pattern(endings: &[&str]) -> String {
    endings.iter().map(|ending| format!("*.{ending}")).collect::<Vec<_>>().join(";")
}

/// A path the Windows shell reads, without the `\\?\` prefix of a verbatim
/// path, which `SHCreateItemFromParsingName` does not take.
#[cfg_attr(not(windows), allow(dead_code, reason = "Windows's alone, tested everywhere"))]
fn shell_path(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_filter_names_each_ending() {
        assert_eq!(windows_pattern(&["png", "jpg"]), "*.png;*.jpg");
        assert_eq!(windows_pattern(&[]), "");
    }

    #[test]
    fn a_verbatim_path_loses_its_prefix() {
        assert_eq!(shell_path(r"\\?\C:\Users\a"), r"C:\Users\a");
        assert_eq!(shell_path(r"C:\Users\a"), r"C:\Users\a");
    }
}
