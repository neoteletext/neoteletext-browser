//! The launcher (`installation-and-updates.md` section 3): what the user
//! starts, on every desktop. It parses no network data: its only inputs
//! are its own state file and a version's signed files.
//!
//! At each start, holding the installation's lock alone:
//!
//! 1. It reads the state (section 3.1), and on macOS makes the installed
//!    bundle the one the state names (section 4.1).
//! 2. A trial version that failed to start twice is rolled back: marked
//!    bad, and the current version, never touched, runs (step 6).
//! 3. With no trial, the newest staged version newer than the current one
//!    and not bad is accepted: complete, signed by the pinned signer, of
//!    the version its files say (section 5); it becomes the trial.
//! 4. A trial's failed starts are counted before it starts (step 5).
//!
//! It then starts the broker of the version chosen, sharing the lock, so
//! a second start runs the same version and changes nothing. When the
//! trusted core of a trial version finds it healthy (section 3.2), the
//! broker says so on its standard output and the launcher commits it. A
//! trial that ends badly, or says nothing in time, is ended with every
//! process it started and counted; the next start rolls it back.

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

mod command;
mod launcher;
pub mod launcher_release;
#[cfg(target_os = "macos")]
mod macos;
mod start;
mod state;
mod tidy;
mod trial;
#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

pub use command::run;
pub(crate) use launcher::io_text;
pub use launcher::{Chosen, Ended, HEALTH_DEADLINE, Launcher, pinned_signer};
#[cfg(target_os = "macos")]
pub use macos::{must_move, offer_move};
