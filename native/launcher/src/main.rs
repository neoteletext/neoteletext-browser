//! The launcher: what the user starts (`installation-and-updates.md`
//! section 3). `neoteletext-launcher [address]...` runs the browser;
//! `--install` (Linux, from the tarball) installs it, `--uninstall`
//! removes it.

// On Windows, a desktop program: no console window opens with it.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::ffi::OsString;
use std::process::ExitCode;

use launcher::{Launcher, install};

fn main() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let result = run(&arguments);
    match result {
        Ok(Some(code)) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Ok(None) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[OsString]) -> Result<Option<i32>, String> {
    let first = arguments.first().and_then(|argument| argument.to_str());
    if first == Some("--install") {
        let own = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
        #[cfg(target_os = "linux")]
        return install::install_linux(&own).map(|()| None);
        #[cfg(not(target_os = "linux"))]
        {
            let _ = own;
            return Err(String::from("--install is Linux's; on macOS drag NeoTeletext to Applications, on Windows run NeoTeletext-Setup.exe"));
        }
    }
    #[cfg(windows)]
    if first == Some(launcher::windows::UNINSTALL_FROM) {
        let root = arguments.get(1).ok_or("--uninstall-from needs a folder")?;
        let and_settings = arguments.get(2).and_then(|argument| argument.to_str()) == Some(launcher::windows::AND_SETTINGS);
        return launcher::windows::uninstall_from(std::path::Path::new(root), and_settings).map(|()| None);
    }
    let launcher = Launcher::this()?;
    if first == Some("--uninstall") {
        #[cfg(windows)]
        return launcher::windows::uninstall(&launcher).map(|()| None);
        #[cfg(not(windows))]
        return install::uninstall(&launcher).map(|()| None);
    }
    #[cfg(target_os = "macos")]
    if let Some(bundle) = &launcher.layout.bundle
        && launcher::must_move(bundle)
    {
        return launcher::offer_move(bundle).map(|()| None);
    }
    launcher.run(arguments)
}
