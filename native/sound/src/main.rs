//! The sound helper (decision 55): it plays every channel's sound through
//! the system's output device, as the trusted core's checked voice
//! operations say, and tells the core which channels sound. It holds no
//! secrets, reaches no network, reads no file of the user's, and is the
//! only process that links the system's sound library: a Linux without
//! ALSA's library cannot start it, and the browser then runs silent
//! (requirement 16.6).
//!
//! A malformed message from the trusted core ends it.

// On Windows, a desktop program: no console window opens with it.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this program reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
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

use std::process::ExitCode;

use sound::process::run;

fn main() -> ExitCode {
    // First, before any thread: the sound output and nothing else.
    match confine::apply(confine::Role::Sound, None) {
        Ok(report) => eprintln!("neoteletext-sound: confined: {report}"),
        Err(error) => {
            eprintln!("neoteletext-sound: {error}");
            return ExitCode::FAILURE;
        }
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-sound: {error}");
            ExitCode::FAILURE
        }
    }
}
