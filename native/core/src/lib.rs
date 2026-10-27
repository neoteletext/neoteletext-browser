//! The trusted core's library: every part of it that is pure logic or a
//! kept file's format, so that each can be tested, and the parsers of
//! outside input fuzzed, on its own. Nothing here starts a process or
//! names the trusted core. The modules: what the address field may open
//! (`address`); the bookmarks and `bookmarks.txt` (`bookmarks`); the
//! bytes a helper's messages may wait in (`budget`);
//! civil-calendar arithmetic (`calendar`); the developer console's entries
//! (`console`); the cookie store and its sealed file (`cookies`); which
//! frames are sent (`frames`, measured by the latency test); what of each
//! channel's sound is heard (`hearing`); history and `history.txt`
//! (`history`); hyperlinks (`links`); the pipes written by threads of
//! their own (`outbox`); the session file, `session.txt` (`restore`); the
//! settings and `settings.txt` (`settings`); sleeping tabs (`sleep`); the
//! start page (`start_page`); and typing into a page (`typing`). The
//! trusted core itself and the broker are the binary: `main.rs`,
//! `broker.rs` and `trusted_core.rs`.

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

pub mod address;
pub mod bookmarks;
pub mod budget;
pub mod calendar;
pub mod console;
pub mod cookies;
pub mod dropped;
pub mod frames;
pub mod hearing;
pub mod history;
#[cfg(feature = "interaction")]
pub mod input_trace;
#[cfg(feature = "links")]
pub mod links;
pub mod outbox;
mod private_file;
pub use private_file::make_folder;
pub mod restore;
pub mod settings;
pub mod sleep;
pub mod start_page;
#[cfg(feature = "interaction")]
pub mod typing;
