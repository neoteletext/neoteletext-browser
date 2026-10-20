//! The trusted core's parser of typed addresses, a library so that it can
//! be fuzzed on its own, its rules for what is heard, its frame encoder
//! (measured by the latency test), its typing rules and its hyperlinks;
//! the trusted core itself is `main.rs`.

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

pub mod address;
pub mod frames;
pub mod hearing;
#[cfg(feature = "links")]
pub mod links;
pub mod sleep;
#[cfg(feature = "interaction")]
pub mod typing;
