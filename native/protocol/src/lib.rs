//! Messages between the browser's processes (`architecture.md` section
//! 6.4): the trusted core and the interface, a decoder, a fetcher, the
//! keyboard and the sound helper. The framing is the engine's: one kind
//! byte, a 32-bit big-endian payload length, then the payload. Every length is bounded
//! before anything is allocated, every payload is parsed strictly to its
//! exact length, and each side accepts only the kinds meant for it, so the
//! interface can never send what only the trusted core may say.

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

pub mod audio;
mod bounds;
pub mod console;
mod controls;
pub mod cookies;
pub mod decoder;
mod error;
pub mod fetcher;
mod fields;
pub mod frame;
mod framing;
#[cfg(feature = "interaction")]
pub mod interaction;
mod interface;
#[cfg(feature = "phone-keyboard")]
pub mod keyboard;
pub mod library;
#[cfg(feature = "links")]
pub mod links;
pub mod settings;
pub mod sound;
pub mod subpages;
mod text;

pub use bounds::*;
pub use error::ProtocolError;
pub(crate) use fields::{Fields, KEY_BYTES, push_count, push_count_u8, push_flag, push_key, push_keys, push_text};
pub(crate) use framing::{read_frame, tail, write_frame};
pub use interface::{ToCore, ToInterface, problem_code, problem_of, read_from_core, read_from_interface, write_to_core, write_to_interface};
pub use text::clean;
pub(crate) use text::{push_raw_text, raw_text};
