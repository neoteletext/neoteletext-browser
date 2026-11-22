//! The script library's core (work package F2 of
//! `neoteletext-common/docs/script-api.md`; DEC-7262, DEC-7246, DEC-7256,
//! DEC-7287): a channel's JavaScript runs in QuickJS compiled to
//! WebAssembly ([`machine`]), interpreted by wasmi under a memory cap and
//! wall-clock limits, in a [`container`] holding a channel context and a
//! page context, and reaches its host only through one flat call
//! ([`host`], [`ops`]): whole numbers and at most one bounded string or
//! typed array. The API it sees is `nt` version [`API_VERSION`], whose
//! shared core is here: events, timers, the clock, keys and the pointer as
//! state, scopes, the shared store, the budget report, random bytes,
//! sound and synthesizers. The renderers' APIs, the components and the
//! web APIs are later work packages' and add their operations beside
//! these.
//!
//! One library, two programs (section 1 principle 6): the browser's
//! script-host program and the standalone runner both run scripts through
//! [`link::ScriptLink`], and nothing else implements the API.
//!
//! A script is exactly as untrusted as the channel that served it. Every
//! bound on what it may ask is a named constant in [`limits`].

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

pub mod container;
pub mod host;
pub mod limits;
pub mod link;
pub mod machine;
pub mod ops;
pub mod probe;
pub mod random;
pub mod watchdog;

/// The API's version: a whole number, raised by every change, never
/// withdrawn (DEC-7208 item 3).
pub const API_VERSION: u16 = 1;

pub use container::{Container, Costs, Settings, TimeLimits};
pub use link::{InProcess, LinkError, ScriptLink};
pub use machine::{Engine, Stop};
pub use watchdog::Watchdog;
