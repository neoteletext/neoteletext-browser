//! The updater (`installation-and-updates.md` sections 3 and 8): a helper
//! of the trusted core, confined to the update host and the staging
//! folder (requirement 13.18). It walks The Update Framework's client
//! workflow with `tough` (requirement 13.8), says which version is the
//! newest the repository names for this platform, and, when the user has
//! accepted one, downloads each of its files, checks it against the
//! targets metadata (by `tough`) and against the release's manifest (its
//! length and SHA-256 once decompressed), and stages the version whole:
//! the files are written and flushed in `staging/<version>.partial`, which
//! is then renamed to `staging/<version>`. It switches nothing: the
//! launcher accepts a staged version at the next start, by its own check.
//!
//! The repository's layout on the host: `/metadata/` holds the four roles'
//! metadata, `/targets/` the release files, each
//! `<version>/<platform>/<path>.gz`, the manifest among them.

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

mod client;
mod helper;
mod staging;
pub mod transport;

pub use client::{Config, MAX_ATTEMPTS, METADATA, newest, now, target_name};
pub use helper::run;
pub use staging::{COMPRESSED, PARTIAL};
pub use transport::Https;
