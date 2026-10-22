//! The interface process's reading of the phone keyboard's keys, a library
//! so that it can be tested on its own; the interface itself is `main.rs`.
//! Page images are read by `protocol::frame` (decision 61).

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

#[cfg(feature = "interaction")]
pub mod keys;
