//! The timed test's own instruments and settings (neoteletext-common's
//! `docs/page-rules.md` RULE-66), compiled only into the build the test
//! makes (`scripts/timed-test.sh`, the feature `timed-test`): no shipped
//! build and no page can reach them.
//!
//! Each process of the browser marks the moments the test times on the
//! machine's monotonic clock, which is the clock the standard library's
//! `Instant` reads on Linux (`CLOCK_MONOTONIC`) and the one the test's own
//! script reads (Python's `time.monotonic`). The standard library gives no
//! number for an `Instant`; its debugging form, `Instant { tv_sec: S,
//! tv_nsec: N }` on Linux, carries the clock's reading, and the test reads
//! that rather than take a system call library for an instrument
//! (DEC-6659 item 12). A form it cannot read is said, not guessed.

use std::time::Instant;

/// The environment variable holding the test's settings, a list of names
/// separated by commas.
pub const SETTINGS_VARIABLE: &str = "NEOTELETEXT_TIMED_TEST";
/// "Events held for the tick": the decoder's host gives each event to the
/// evaluator only after the next tick step, as before DEC-6580.
pub const EVENTS_AT_THE_TICK: &str = "events-at-the-tick";
/// "The device buffer left at 480 frames": the sound helper never asks
/// for the small buffer of RULE-97.
pub const USUAL_DEVICE_BUFFER: &str = "usual-device-buffer";
/// "The tick left unlocked, with paint points at the ticks" (RULE-98).
pub const UNLOCKED: &str = "unlocked";

/// Microseconds in a second, and nanoseconds in a microsecond.
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;
const NANOSECONDS_PER_MICROSECOND: u64 = 1_000;

/// Whether the test's setting `name` is on.
#[must_use]
pub fn setting(name: &str) -> bool {
    std::env::var(SETTINGS_VARIABLE).is_ok_and(|settings| settings.split(',').any(|setting| setting == name))
}

/// The monotonic clock's reading at `at`, in microseconds, or `None` when
/// the standard library's form is not the one read here.
#[must_use]
pub fn monotonic_us(at: Instant) -> Option<u64> {
    let form = format!("{at:?}");
    let field = |name: &str| -> Option<u64> {
        let after = form.split_once(name)?.1;
        let digits: String = after.chars().skip_while(|character| !character.is_ascii_digit()).take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let seconds = field("tv_sec")?;
    let nanoseconds = field("tv_nsec")?;
    seconds.checked_mul(MICROSECONDS_PER_SECOND)?.checked_add(nanoseconds.checked_div(NANOSECONDS_PER_MICROSECOND)?)
}

/// Writes one of the test's marks on standard error: the process, what
/// happened, the monotonic clock's reading at `at` in microseconds, and
/// what more the test matches it by.
pub fn mark(process: &str, what: &str, at: Instant, detail: core::fmt::Arguments<'_>) {
    match monotonic_us(at) {
        Some(us) => eprintln!("neoteletext-timed {process} {what} {us} {detail}"),
        None => eprintln!("neoteletext-timed {process} {what} unreadable-clock {detail}"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// RULE-66: the clock's reading is read from the standard library's
    /// own form, and two readings a millisecond apart differ by about that.
    #[test]
    fn the_monotonic_clock_is_read() {
        let first = Instant::now();
        let second = first + std::time::Duration::from_millis(1);
        let (first_us, second_us) = (monotonic_us(first).unwrap(), monotonic_us(second).unwrap());
        assert_eq!(second_us - first_us, 1_000);
    }
}
