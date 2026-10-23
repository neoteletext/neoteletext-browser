//! An update's marks: how strongly a page just updated still glows, and
//! "Updated 12s", its age, for a minute after it; read by the wall, the
//! list and the overview.

use alloc::format;
use alloc::string::String;

/// An update glows for 3 seconds, and is marked with its age for a minute.
pub const UPDATE_GLOW_MILLISECONDS: u64 = 3_000;
/// How long an update is marked with its age.
pub const UPDATE_MARK_MILLISECONDS: u64 = 60_000;
const MILLISECONDS_PER_SECOND: u64 = 1_000;
/// The width of an updated page's glowing frame, at its brightest.
pub(super) const GLOW_WIDTH: f64 = 4.0;

/// How strongly an update still shows, from 1 just after it to 0 once
/// `UPDATE_GLOW_MILLISECONDS` have passed.
#[must_use]
pub fn glow(updated: Option<u64>, now: u64) -> f64 {
    updated.map_or(0.0, |at| {
        #[allow(clippy::cast_precision_loss, reason = "a few seconds")]
        let share = now.saturating_sub(at) as f64 / UPDATE_GLOW_MILLISECONDS as f64;
        (1.0 - share).max(0.0)
    })
}

/// "Updated 12s": how long ago a page was updated, for a minute after it
/// was.
#[must_use]
pub fn since(updated: Option<u64>, now: u64) -> Option<String> {
    let elapsed = now.saturating_sub(updated?);
    (elapsed < UPDATE_MARK_MILLISECONDS).then(|| format!("Updated {}s", elapsed / MILLISECONDS_PER_SECOND))
}
