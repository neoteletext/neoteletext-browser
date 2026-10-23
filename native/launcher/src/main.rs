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

/// The identity this launcher was built with, in its bytes, where the
/// release tool's `check` reads it (finding F-24).
static IDENTITY_MARK: &str = release_identity::IDENTITY_MARK;

fn main() -> ExitCode {
    os_install::restrict_dll_search();
    // Read, so the linker keeps the mark.
    let _ = std::hint::black_box(IDENTITY_MARK);
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let result = launcher::run(&arguments);
    match result {
        Ok(Some(code)) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Ok(None) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext: {error}");
            ExitCode::FAILURE
        }
    }
}
