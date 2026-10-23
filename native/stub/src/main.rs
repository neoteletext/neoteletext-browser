//! The Windows stub (`installation-and-updates.md` section 4.2), packaged
//! as `NeoTeletext.exe`: it reads `launcher`, one byte, `a` or `b`, beside
//! itself, starts `launcher-a.exe` or `launcher-b.exe` with its own
//! arguments, and ends as the launcher does. It is never replaced, so a
//! launcher release (section 7) writes the other copy and switches the
//! byte, and Start, the taskbar and the uninstaller keep pointing here. It
//! is kept this small so it never needs to change.

// A desktop program: no console window opens with it.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::{Command, ExitCode};

/// The file naming the launcher copy in use (`update::layout`'s
/// `WINDOWS_LAUNCHER_CHOICE`; the stub depends on nothing).
const CHOICE: &str = "launcher";
/// The copies' letters; anything else is the first.
const COPIES: [u8; 2] = *b"ab";

fn main() -> ExitCode {
    let Some(folder) = std::env::current_exe().ok().and_then(|own| own.parent().map(std::path::Path::to_path_buf)) else {
        return ExitCode::FAILURE;
    };
    let choice = std::fs::read(folder.join(CHOICE)).ok().and_then(|bytes| bytes.first().copied()).filter(|choice| COPIES.contains(choice)).unwrap_or(b'a');
    let launcher = folder.join(format!("launcher-{}{}", char::from(choice), std::env::consts::EXE_SUFFIX));
    match Command::new(launcher).args(std::env::args_os().skip(1)).status() {
        Ok(status) => ExitCode::from(status.code().and_then(|code| u8::try_from(code).ok()).unwrap_or(1)),
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stub names the files the launcher writes, defined there.
    #[test]
    fn the_stub_reads_what_the_launcher_writes() {
        assert_eq!(CHOICE, update::layout::WINDOWS_LAUNCHER_CHOICE);
        #[cfg(windows)]
        assert_eq!(COPIES, launcher::windows::CHOICES);
    }
}
