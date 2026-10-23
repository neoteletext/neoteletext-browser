//! The updater process (`installation-and-updates.md` section 3):
//! `neoteletext-updater [--proxy URL]`, started by the trusted core
//! through the broker, confined to the update host's port (or the
//! proxy's) and its staging folder (requirement 13.18). It reads the
//! core's messages on its standard input and answers on its standard
//! output (`update::wire`).

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

use std::process::ExitCode;

fn main() -> ExitCode {
    match updater::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-updater: {error}");
            ExitCode::FAILURE
        }
    }
}
