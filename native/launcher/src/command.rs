//! The command line: `--install` (Linux), `--uninstall-from` (Windows),
//! `--uninstall`, the macOS move to Applications, or the browser run.

use std::ffi::OsString;

use crate::Launcher;

/// Runs what the arguments ask for, and returns the exit code to end
/// with, if any.
///
/// # Errors
/// What went wrong, in words for the user.
pub fn run(arguments: &[OsString]) -> Result<Option<i32>, String> {
    let first = arguments.first().and_then(|argument| argument.to_str());
    if first == Some("--install") {
        let own = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
        #[cfg(target_os = "linux")]
        return crate::unix::install_linux(&own).map(|()| None);
        #[cfg(not(target_os = "linux"))]
        {
            let _ = own;
            return Err(String::from("--install is Linux's; on macOS drag NeoTeletext to Applications, on Windows run NeoTeletext-Setup.exe"));
        }
    }
    #[cfg(windows)]
    if first == Some(crate::windows::UNINSTALL_FROM) {
        let root = arguments.get(1).ok_or("--uninstall-from needs a folder")?;
        let and_settings = arguments.get(2).and_then(|argument| argument.to_str()) == Some(crate::windows::AND_SETTINGS);
        return crate::windows::uninstall_from(std::path::Path::new(root), and_settings).map(|()| None);
    }
    let launcher = Launcher::this()?;
    if first == Some("--uninstall") {
        #[cfg(windows)]
        return crate::windows::uninstall(&launcher).map(|()| None);
        #[cfg(not(windows))]
        return crate::unix::uninstall(&launcher).map(|()| None);
    }
    #[cfg(target_os = "macos")]
    if let Some(bundle) = &launcher.layout.bundle
        && crate::must_move(bundle)
    {
        return crate::offer_move(bundle).map(|()| None);
    }
    launcher.run(arguments)
}
