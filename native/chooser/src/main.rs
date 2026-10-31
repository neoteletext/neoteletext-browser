//! The file dialog (DEC-1551 item 1). The broker starts this program for one
//! choice: `neoteletext-chooser <kind> <parent> <folder>`, the kind what is
//! chosen, the parent the window to place it over or nothing, the folder
//! where it starts or nothing. It shows the system's dialog, prints the path
//! chosen on its standard output and ends; printing nothing is a cancel.
//!
//! The broker checks what it prints and opens nothing on its word alone, so
//! a compromised chooser can only name a path, as the user could. It runs
//! beside the broker, unconfined, because the dialog needs the user's
//! folders: the macOS panel draws in a process of the system's, and the
//! portal in the desktop's, but the Windows dialog runs here, with the
//! shell's extensions, which is why it is not in the broker.
//!
//! Its title is fixed here: neither the trusted core nor the interface can
//! reword it. The system's dialogs do not offer files and folders in one,
//! so a folder is a kind of its own (DEC-1705).

// No panicking constructs: this program reads its arguments from outside. The
// same list is in each such crate; scripts/check.sh checks that they agree.
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

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use confine::broker::ChooseKind;

fn title(kind: ChooseKind) -> &'static str {
    match kind {
        ChooseKind::File => "Open File",
        ChooseKind::Folder => "Open Folder",
        ChooseKind::Bookmarks => "Import Bookmarks",
    }
}

/// The system's dialog on macOS and Windows.
#[cfg(not(target_os = "linux"))]
#[allow(clippy::unnecessary_wraps, reason = "the same signature as Linux's, whose portal can fail")]
fn choose(kind: ChooseKind, _parent: &str, folder: Option<&Path>) -> Result<Option<PathBuf>, String> {
    let mut dialog = rfd::FileDialog::new().set_title(title(kind));
    if let Some(folder) = folder {
        dialog = dialog.set_directory(folder);
    }
    Ok(if kind == ChooseKind::Folder { dialog.pick_folder() } else { dialog.pick_file() })
}

/// The desktop portal's dialog on Linux, parented by `parent`: `x11:` and a
/// window's number in hexadecimal, or nothing.
#[cfg(target_os = "linux")]
fn choose(kind: ChooseKind, parent: &str, folder: Option<&Path>) -> Result<Option<PathBuf>, String> {
    use ashpd::desktop::file_chooser::OpenFileRequest;
    use ashpd::{Error, WindowIdentifier};

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let mut request = OpenFileRequest::default().title(title(kind)).directory(kind == ChooseKind::Folder).multiple(false);
        if let Some(window) = parent.strip_prefix("x11:").and_then(|number| u32::from_str_radix(number, 16).ok()) {
            request = request.identifier(WindowIdentifier::from_xid(u64::from(window)));
        }
        if let Some(folder) = folder {
            request = request.current_folder(folder).map_err(|error| error.to_string())?;
        }
        let response = request.send().await.map_err(|error| error.to_string())?.response();
        match response {
            Ok(files) => Ok(files.uris().first().and_then(|uri| path_of_uri(uri.as_str()))),
            Err(Error::Response(ashpd::desktop::ResponseError::Cancelled)) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    })
}

/// The path a portal's `file://` URI names ("All URIs have the `file://`
/// scheme": the `FileChooser` portal's documentation), its escapes undone.
#[cfg(any(target_os = "linux", test))]
fn path_of_uri(uri: &str) -> Option<PathBuf> {
    const HEXADECIMAL: u32 = 16;
    let mut bytes = uri.strip_prefix("file://")?.bytes();
    let mut path = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(HEXADECIMAL)?;
            let low = char::from(bytes.next()?).to_digit(HEXADECIMAL)?;
            path.push(u8::try_from(high.checked_mul(HEXADECIMAL)?.checked_add(low)?).ok()?);
        } else {
            path.push(byte);
        }
    }
    String::from_utf8(path).ok().map(PathBuf::from)
}

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    let Some(kind) = arguments.next().as_deref().and_then(ChooseKind::from_argument) else {
        eprintln!("neoteletext-chooser: the browser's broker starts this program");
        return ExitCode::FAILURE;
    };
    let parent = arguments.next().unwrap_or_default();
    let folder = arguments.next().filter(|folder| !folder.is_empty()).map(PathBuf::from);
    let chosen = match choose(kind, &parent, folder.as_deref()) {
        Ok(chosen) => chosen,
        Err(error) => {
            eprintln!("neoteletext-chooser: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Some(path) = chosen.as_deref().and_then(Path::to_str) else { return ExitCode::SUCCESS };
    if std::io::stdout().write_all(path.as_bytes()).is_err() { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn a_uri_gives_its_path_with_its_escapes_undone() {
        assert_eq!(path_of_uri("file:///home/a/news%20of%20today.t42"), Some(PathBuf::from("/home/a/news of today.t42")));
        assert_eq!(path_of_uri("file:///caf%C3%A9.tti"), Some(PathBuf::from("/caf\u{e9}.tti")));
        for refused in ["https://a.example/", "file:///a%2", "file:///a%zz", "file:///%FF"] {
            assert_eq!(path_of_uri(refused), None, "{refused}");
        }
    }

    #[test]
    fn each_kind_has_its_own_title() {
        assert_eq!(title(ChooseKind::File), "Open File");
        assert_eq!(title(ChooseKind::Bookmarks), "Import Bookmarks");
    }
}
