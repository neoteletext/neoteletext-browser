//! The clock the scripts' time limits are counted on (script-api.md
//! section 3.2 rules 1 to 6; DEC-7357 as revised on 23 November 2026).
//!
//! The limits bound a script's work: a top level, a callback, a step, a
//! second of steps. Counted on the wall clock they also counted every
//! moment the script host was ready to run and had no processor, so that
//! on a busy machine a script was stopped "for time" it never had:
//! measured on 23 November 2026, thirty-two script hosts started at once
//! on twenty processors each waited 1.7 to 1.8 seconds for a processor in
//! their first 2.5.
//!
//! So the program that runs the library may give it the processor time of
//! its own process ([`install`]), and every limit is then counted on
//! that: an instant of this clock is a fixed origin plus the processor
//! time the process has used. A script cannot make this clock stand still
//! while it holds the thread: nothing it can call sleeps or waits (no
//! host call blocks; there are no threads, no `Atomics.wait`, no
//! synchronous request), so whatever it does is work, and work is
//! counted. With nothing installed the clock is the wall clock, as the
//! standalone runner and the library's own tests have it.
//!
//! Reading the processor time is a system call, and the clock is checked
//! before and after every host call. [`passed`] therefore asks only when
//! it must: from the last reading it knows the most the clock can have
//! advanced since (as much as the wall clock has), and while even that
//! does not reach the deadline the deadline has not passed.

use std::cell::Cell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Gives the processor time the process has used; none when the system
/// does not say.
pub type ProcessorTime = fn() -> Option<Duration>;

/// The source installed, and the clock's origin: the wall clock's instant
/// that stands for no processor time used.
static SOURCE: OnceLock<(ProcessorTime, Instant)> = OnceLock::new();

thread_local! {
    /// This thread's last reading: the wall clock's instant it was made
    /// at, and what the clock said.
    static READING: Cell<Option<(Instant, Instant)>> = const { Cell::new(None) };
}

/// Counts every time limit of this process's scripts on `source`, the
/// processor time the process has used, from now on. Called once, by the
/// program, before any container is made. False, and nothing changes,
/// when `source` gives nothing or a source is installed already: the
/// limits then stay on the clock they were on.
pub fn install(source: ProcessorTime) -> bool {
    source().is_some() && SOURCE.set((source, Instant::now())).is_ok()
}

/// Whether the limits are counted on the process's processor time.
#[must_use]
pub fn counts_work() -> bool {
    SOURCE.get().is_some()
}

/// The clock now, read whole.
#[must_use]
pub fn now() -> Instant {
    let wall = Instant::now();
    let Some((source, origin)) = SOURCE.get() else { return wall };
    // A system that stops saying leaves the clock where it last stood:
    // the limits then pass no more, and the watchdog's and the trusted
    // core's bounds on the wall clock are what is left.
    let last = READING.get().map(|(_, said)| said);
    let said = source().and_then(|used| origin.checked_add(used)).or(last).unwrap_or(*origin);
    READING.set(Some((wall, said)));
    said
}

/// Whether `deadline`, an instant of this clock, has passed.
#[must_use]
pub fn passed(deadline: Instant) -> bool {
    if SOURCE.get().is_none() {
        return Instant::now() > deadline;
    }
    // The most the clock can say now: its last reading, and as much again
    // as the wall clock has moved since.
    let most = READING.get().and_then(|(at, said)| said.checked_add(at.elapsed()));
    if most.is_some_and(|most| most <= deadline) {
        return false;
    }
    now() > deadline
}

/// How long on this clock since `began`, an instant of it.
#[must_use]
pub fn since(began: Instant) -> Duration {
    now().saturating_duration_since(began)
}

/// The instant of this clock `limit` from now; none when there is none.
#[must_use]
pub fn after(limit: Duration) -> Option<Instant> {
    now().checked_add(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With nothing installed the clock is the wall clock: what the
    /// runner and the library's tests have. (A source cannot be taken
    /// out again, so the clock on processor time is tested in the
    /// script-host program, where one is installed.)
    #[test]
    fn with_no_source_the_clock_is_the_wall_clock() {
        assert!(!counts_work());
        let before = Instant::now();
        let said = now();
        assert!(said >= before && said <= Instant::now());
        let soon = after(Duration::from_millis(20)).unwrap();
        assert!(!passed(soon));
        std::thread::sleep(Duration::from_millis(25));
        assert!(passed(soon));
        assert!(since(before) >= Duration::from_millis(25));
    }
}
