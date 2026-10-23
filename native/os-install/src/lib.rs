//! The operating system calls installing and launching need that only an
//! unsafe interface offers (`installation-and-updates.md` sections 4 and
//! 5; requirement 10.12 as amended for this crate):
//!
//! - [`ask`] and [`tell`]: a question or a notice in the system's own
//!   dialog, for the launcher, which has no window: CoreFoundation's
//!   alert on macOS (moving a translocated bundle to Applications,
//!   section 4.1), `MessageBoxW` on Windows (the uninstaller's
//!   questions, requirement 13.20). Linux asks on the terminal, where its
//!   installer runs.
//! - On Windows, [`WindowsSigner`]: Authenticode's check of every
//!   program by `WinVerifyTrust`, with the signer certificate's public key
//!   pinned (section 5); and [`uninstall_entry`]: the per-user entry
//!   Settings › Apps lists and removes the browser by (section 4.2).

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

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{UninstallEntry, WindowsSigner, remove_uninstall_entry, signer_key, uninstall_entry};

/// Asks the user `question` under `title`, with the buttons `yes` and
/// `no`; whether they chose `yes`. Where the system has no dialog for a
/// program without a window (Linux), the answer is no: Linux asks on its
/// terminal instead.
#[must_use]
pub fn ask(title: &str, question: &str, yes: &str, no: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::ask(title, question, yes, no)
    }
    #[cfg(windows)]
    {
        let _ = (yes, no);
        windows::ask(title, question)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (title, question, yes, no);
        false
    }
}

/// Tells the user `message` under `title`, and waits until it is read.
pub fn tell(title: &str, message: &str) {
    #[cfg(target_os = "macos")]
    {
        macos::tell(title, message);
    }
    #[cfg(windows)]
    {
        windows::tell(title, message);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        eprintln!("{title}: {message}");
    }
}
