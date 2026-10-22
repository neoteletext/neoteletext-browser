//! The interface process's reader of page images, a library so that it can
//! be fuzzed on its own; the interface itself is `main.rs`.

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

pub mod image;
#[cfg(feature = "interaction")]
pub mod keys;
