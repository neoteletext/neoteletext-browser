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

fn main() -> ExitCode {
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
