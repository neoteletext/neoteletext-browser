//! The input trace's line (the owner, 27 October 2026): with the trusted
//! core's `--input-trace`, each input event it sends a channel is shown on
//! the open page's row 0, so what reaches the channel, and how often, can
//! be seen. The trusted core writes the line; the interface only draws it.

use alloc::string::String;

/// The longest description: a page row's 40 cells.
pub const MAX_TRACE: usize = 40;

/// The last input event sent to a channel, as the trusted core described it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceLine {
    /// What went, cleaned text: "MOUSE move b0 12,5".
    pub text: String,
    /// How many times running the same event went: 1 for a new one.
    pub repeats: u32,
    /// Events sent since the browser started.
    pub sent: u64,
}
