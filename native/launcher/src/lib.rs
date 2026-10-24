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
//!    the version its files say (section 5); it becomes the trial. One that
//!    is refused is removed, not marked bad.
//! 4. A trial's start is counted as one that has not started yet.
//!
//! It then starts the broker of the version chosen, sharing the lock, so
//! a second start runs the same version and changes nothing. The broker
//! says `started` before it starts the trusted core, and `healthy` when the
//! core has found the version healthy (section 3.2); the launcher commits
//! it then (decision 93). A trial gets a start deadline to say `started`,
//! then a health deadline from that moment; one that misses either is
//! ended with every process it started. A start that ends badly after
//! `started`, or misses the health deadline, is a failed start; one that
//! never says `started` is counted apart; a clean quit before health
//! counts as neither. Two failed starts, or three that never started,
//! roll the trial back.

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
pub use launcher::{Chosen, Deadline, Ended, Launcher, pinned_signer};
#[cfg(target_os = "macos")]
pub use macos::{must_move, offer_move};
pub use trial::MAX_CANDIDATES_PER_START;
pub use update::layout::{HEALTH_DEADLINE, START_DEADLINE};
