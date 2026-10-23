//! The processes' sandboxes (requirements 3.3, 3.4, 3.6 and 3.7; decision
//! 52): every helper of the trusted core runs with only what it is for,
//! on every platform, by the platform's own means.
//!
//! - **macOS:** the trusted core starts each helper under a `sandbox-exec`
//!   profile that starts from "deny everything" (`macos`).
//! - **Windows:** the trusted core starts each helper in a job object,
//!   with the process mitigation policies, and in an `AppContainer`, or
//!   for the interface, which needs the desktop, with a restricted token
//!   at low integrity (`windows`).
//! - **Linux:** each helper confines itself first thing: an empty network
//!   namespace for those without the network, Landlock and a seccomp
//!   system-call filter (`linux`).
//!
//! Every layer is required (decision 58: security is paramount, and is
//! never weakened or downgraded). A machine that cannot give a process
//! every layer its role names gets an error and no process, never a
//! process with less; there is no fallback and no switch to one.
//!
//! The two halves are [`spawn`], the parent's side, and [`apply`], a
//! process's own side; each is a no-op on the platforms where the other
//! does the work. The helpers receive everything over their pipes, so no
//! sandbox here has to let anything through on request: they only deny.
//!
//! The parent is the broker (decision 56, `broker`): the one unconfined
//! process, which starts the trusted core confined and every helper for
//! it, so that the core too is confined on every platform.

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

pub mod broker;
mod environment;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod process;
mod role;
mod spawn;
mod time;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub use environment::environment;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub use linux::probe;
pub use process::{Input, Output, Process};
pub use role::{Report, Role};
pub use spawn::{Spawn, apply, spawn};
pub use time::{LocalTime, MAX_OFFSET, local_time};
#[cfg(windows)]
pub use windows::{grant_packages_read, remove_containers};
