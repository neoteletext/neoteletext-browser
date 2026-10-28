//! The interface process (design section 5.2), as a library so that its
//! parts can be tested on their own: the window's state and its event
//! loop's handler, the reader of the trusted core's messages, the system
//! window, and the reading of the phone keyboard's keys. `main.rs`
//! confines the process and runs the event loop. Page images are read by
//! `protocol::frame` (decision 61).

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

mod app;
mod incoming;
#[cfg(feature = "interaction")]
pub mod keys;
mod reader;
mod window;

pub use app::App;
pub use incoming::{Incoming, Library};
pub use reader::read_core;
pub use window::show_dock_icon;
