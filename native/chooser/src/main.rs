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
//!
//! On Linux it has one more use, as the browser's one holder of the desktop
//! portal: `neoteletext-chooser reduced-motion` prints
//! `confine::broker::REDUCED_MOTION_ANSWER` when the desktop asks for
//! reduced motion, and nothing otherwise (REQ-754). The broker asks it
//! before starting the interface, which reaches no session bus (decision
//! 86), and believes nothing but that one answer.

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

/// A window handle's base, on the command line.
#[cfg(windows)]
const HEXADECIMAL: u32 = 16;

fn title(kind: ChooseKind) -> &'static str {
    match kind {
        ChooseKind::File => "Open File",
        ChooseKind::Folder => "Open Folder",
        ChooseKind::Bookmarks => "Import Bookmarks",
    }
}

/// The system's dialog on macOS and Windows, owned on Windows by `parent`:
/// `win32:` and a window's handle in hexadecimal, or nothing. macOS centres
/// the panel on the screen, as it cannot be placed from another process.
#[cfg(not(target_os = "linux"))]
#[allow(clippy::unnecessary_wraps, reason = "the same signature as Linux's, whose portal can fail")]
#[cfg_attr(not(windows), allow(unused_variables, reason = "only a window handle of Windows means anything in another process"))]
fn choose(kind: ChooseKind, parent: &str, folder: Option<&Path>) -> Result<Option<PathBuf>, String> {
    let mut dialog = rfd::FileDialog::new().set_title(title(kind));
    if let Some(folder) = folder {
        dialog = dialog.set_directory(folder);
    }
    #[cfg(windows)]
    if let Some(window) = parent.strip_prefix("win32:").and_then(|handle| isize::from_str_radix(handle, HEXADECIMAL).ok()).and_then(confine::ForeignWindow::new)
    {
        dialog = dialog.set_parent(&window);
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

/// GNOME's own setting of animations, which the portal's Settings serves
/// beside its standard keys (`org.gnome.desktop.interface` schema, key
/// `enable-animations`, a boolean, false when the viewer turned off
/// Accessibility's animations). Read only where the portal lacks the
/// standard key, as a portal before version 1.20 of xdg-desktop-portal does.
#[cfg(target_os = "linux")]
const GNOME_INTERFACE: &str = "org.gnome.desktop.interface";
#[cfg(target_os = "linux")]
const GNOME_ENABLE_ANIMATIONS: &str = "enable-animations";
/// The `org.freedesktop.appearance` key `reduced-motion`'s value for
/// reduced motion: the portal's Settings documentation gives it as an
/// unsigned 32-bit number, 0 for no preference and 1 for reduced motion,
/// and asks that other values be read as no preference.
#[cfg(any(target_os = "linux", test))]
const PORTAL_REDUCED_MOTION: u32 = 1;

/// The desktop's preference from what the portal answered: its standard key
/// when it has one, whatever GNOME's key says; GNOME's key otherwise; no
/// preference when neither was answered.
#[cfg(any(target_os = "linux", test))]
fn reduced(appearance: Option<u32>, animations: Option<bool>) -> bool {
    match appearance {
        Some(value) => value == PORTAL_REDUCED_MOTION,
        None => animations == Some(false),
    }
}

/// Asks the desktop portal's Settings for the reduced-motion preference.
#[cfg(target_os = "linux")]
fn ask_reduced_motion() -> Result<bool, String> {
    use ashpd::desktop::settings::{APPEARANCE_NAMESPACE, REDUCED_MOTION_KEY, Settings};

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let settings = Settings::new().await.map_err(|error| error.to_string())?;
        // A key the portal does not know is an error of its; either read
        // failing is that key unanswered.
        let appearance = settings.read::<u32>(APPEARANCE_NAMESPACE, REDUCED_MOTION_KEY).await.ok();
        let animations = if appearance.is_none() { settings.read::<bool>(GNOME_INTERFACE, GNOME_ENABLE_ANIMATIONS).await.ok() } else { None };
        Ok(reduced(appearance, animations))
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
    let first = arguments.next();
    #[cfg(target_os = "linux")]
    if first.as_deref() == Some(confine::broker::REDUCED_MOTION_ARGUMENT) {
        return match ask_reduced_motion() {
            Ok(true) => {
                if std::io::stdout().write_all(confine::broker::REDUCED_MOTION_ANSWER).is_err() {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                }
            }
            Ok(false) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("neoteletext-chooser: the desktop portal's settings: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let Some(kind) = first.as_deref().and_then(ChooseKind::from_argument) else {
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

    /// REQ-754: the portal's standard key decides when it is answered,
    /// 1 alone being reduced motion; GNOME's `enable-animations` turned off
    /// decides otherwise; nothing answered is no preference.
    #[test]
    fn the_portals_answer_gives_the_preference() {
        assert!(reduced(Some(PORTAL_REDUCED_MOTION), None));
        assert!(reduced(Some(PORTAL_REDUCED_MOTION), Some(true)));
        for no_preference in [0, 2, u32::MAX] {
            assert!(!reduced(Some(no_preference), Some(false)), "{no_preference}");
        }
        assert!(reduced(None, Some(false)));
        assert!(!reduced(None, Some(true)));
        assert!(!reduced(None, None));
    }

    /// REQ-754 on Linux: the answer is the desktop's own setting. GNOME
    /// keeps it in `org.gnome.desktop.interface enable-animations`, which
    /// Settings, Accessibility, "Reduce Animation" turns off.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a GNOME desktop session with xdg-desktop-portal running, so it runs by hand there with Reduce Animation on and then off"]
    fn reduced_motion_is_the_desktops_setting() {
        let read = std::process::Command::new("gsettings").args(["get", GNOME_INTERFACE, GNOME_ENABLE_ANIMATIONS]).output().unwrap();
        let animations = String::from_utf8_lossy(&read.stdout).trim() == "true";
        assert_eq!(ask_reduced_motion(), Ok(!animations));
    }
}
