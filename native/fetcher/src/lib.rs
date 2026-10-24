//! The fetcher process's logic, for its binary: the loop that holds one
//! channel's network connections through the engine's web module, the
//! sockets and the proxy, the cookie questions to the trusted core, and the
//! arguments it starts with. `main.rs` confines the process and calls
//! [`run`].

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

mod agent;
mod arguments;
mod cookies;
mod process;
mod transport;

pub use arguments::{Arguments, arguments};
pub use process::run;
