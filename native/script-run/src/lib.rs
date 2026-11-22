//! The standalone runner (work package H1 of
//! `neoteletext-common/docs/script-api.md`, section 16; DEC-7262): a
//! channel folder, one page of it or one script is run with no browser,
//! on a clock that is stepped and never read, with a fixed seed, so that
//! two runs give the same pictures, sound, log and budget byte for byte.
//!
//! It runs scripts through [`neoteletext_script::ScriptLink`], the one
//! interface the browser's script host also runs them through (section 1
//! principle 6), draws with the decoder's own engine ([`screen`]) and
//! mixes with the sound helper's own mixer ([`sound`]), so that a picture
//! or a sound is what the browser would give.
//!
//! - [`args`]: the command line;
//! - [`files`]: what is read from disk for a run, by the unconfined
//!   process, and [`blobs`], how it reaches the confined one;
//! - [`manifest`]: a script's manifest (section 3.1);
//! - [`input`]: the input script (`--input`), every event of section 4.2;
//! - [`network`]: the network modes (`--network`), over the web client's
//!   own messages;
//! - [`screen`] and [`sound`]: snapshots as PNG and WAV;
//! - [`run`]: the stepped loop;
//! - [`report`]: the budget, the times and the counts, as text and JSON;
//! - [`expect`]: the kept results (`--expect`, `--bless`);
//! - [`worker`]: a run from the files given to the files made.
//!
//! A script, its manifest, a page file, an input script and a network
//! file are all untrusted. Every bound is a named constant in [`limits`].

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

pub mod args;
pub mod blobs;
pub mod expect;
pub mod files;
pub mod input;
pub mod json;
pub mod limits;
pub mod manifest;
pub mod network;
pub mod report;
pub mod run;
pub mod screen;
pub mod sound;
pub mod worker;

/// How a run ended, as the process's exit status (section 16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// It ran, and matched what was kept if anything was.
    Matched = 0,
    /// A snapshot, the sound, the log or the budget differed from what
    /// was kept.
    Differed = 1,
    /// The script was stopped: a limit, a trap or too many failures.
    Stopped = 2,
    /// The script or its manifest was refused.
    Refused = 3,
    /// Bad arguments or files.
    Usage = 4,
}

impl Status {
    /// Every status, in the order of their codes.
    pub const ALL: [Self; 5] = [Self::Matched, Self::Differed, Self::Stopped, Self::Refused, Self::Usage];

    /// The exit status.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// The status with exit status `code`.
    #[must_use]
    pub fn of(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.code() == code)
    }
}

/// A run that could not be made, or ended early: its status and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// The exit status.
    pub status: Status,
    /// Why, for the person who ran it.
    pub message: String,
}

impl Failure {
    /// Bad arguments or files.
    pub fn usage(message: impl Into<String>) -> Self {
        Self { status: Status::Usage, message: message.into() }
    }

    /// A script or manifest refused.
    pub fn refused(message: impl Into<String>) -> Self {
        Self { status: Status::Refused, message: message.into() }
    }
}

impl core::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl core::error::Error for Failure {}
