//! The phone keyboard process (`interaction.md` section 9, requirement
//! 18.9): it asks qrkeyboard's relay for an instance, gives the trusted
//! core the QR to show, pairs with the phone that scans it, and passes the
//! phone's keys on, bounded, with no modifiers. It alone holds the pairing
//! secret; it has the network and nothing else, and runs until the trusted
//! core closes its input or says Finish.
//!
//! Usage: `t42-keyboard [relay] [--pin]`, qrkeyboard's own relay unless
//! another is named.

// On Windows, a desktop program: no console window opens with it.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
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

#[cfg(feature = "phone-keyboard")]
mod keyboard;

use std::process::ExitCode;

#[cfg(feature = "phone-keyboard")]
fn main() -> ExitCode {
    keyboard::main()
}

#[cfg(not(feature = "phone-keyboard"))]
fn main() -> ExitCode {
    eprintln!("t42-keyboard: built without the phone keyboard");
    ExitCode::FAILURE
}
