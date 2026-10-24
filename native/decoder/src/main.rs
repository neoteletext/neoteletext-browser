//! The decoder process: it reads a channel's bytes and requests from the
//! trusted core on standard input, and writes which pages arrived and the
//! pages it renders on standard output. It opens no file and no network
//! connection; on macOS the trusted core starts it in a sandbox that
//! denies both.

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this crate reads input from outside the
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

use decoder::process::{Mode, mode, read_bookmarks, run};

fn main() -> ExitCode {
    // First, before any thread: its pipes and nothing else (requirement 3.4).
    match confine::apply(confine::Role::Decoder, None) {
        Ok(report) => eprintln!("neoteletext-decoder: confined: {report}"),
        Err(error) => {
            eprintln!("neoteletext-decoder: {error}");
            return ExitCode::FAILURE;
        }
    }
    let ran = match mode() {
        Ok(Mode::Channel(pages)) => run(pages),
        Ok(Mode::Bookmarks) => read_bookmarks(),
        Err(usage) => {
            eprintln!("neoteletext-decoder: {usage}");
            return ExitCode::FAILURE;
        }
    };
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-decoder: {error}");
            ExitCode::FAILURE
        }
    }
}
