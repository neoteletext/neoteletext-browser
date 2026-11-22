//! The watchdog (script-api.md section 3.2 rule 5(b); the security
//! review's findings S-04, TODO-7270, and S-18 item 1, TODO-7285): a
//! thread that stands behind the container's own clock checks. It is told
//! the deadline of every entry into the module and told again when the
//! entry returns; if an entry is still running [`WATCHDOG_GRACE`] after
//! its deadline, it acts. It does not depend on the interpreter returning,
//! and it holds no lock the interpreter can hold: the interpreter's thread
//! takes the watchdog's one lock only to write a deadline, before it
//! enters the module and after it has left, never while it is inside.
//!
//! What it does is the program's choice. The interpreter cannot be
//! interrupted from outside, so the script-host program ends its process
//! ([`Watchdog::ending_the_process`]); the library's tests count instead.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use crate::limits::WATCHDOG_GRACE;

/// The exit status of a script-host process its watchdog ended.
pub const WATCHDOG_EXIT_STATUS: i32 = 70;

/// What the two threads share.
struct Watched {
    /// The deadline of the entry running now; none between entries.
    deadline: Option<Instant>,
    /// Counts entries, so that the watchdog acts once for each.
    entry: u64,
    /// The watchdog's owner is gone: the thread ends.
    closed: bool,
}

struct Shared {
    watched: Mutex<Watched>,
    changed: Condvar,
}

impl Shared {
    /// The lock, whatever became of a thread that held it: the state is
    /// three plain values, valid at every moment.
    fn lock(&self) -> MutexGuard<'_, Watched> {
        self.watched.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A handle to a watchdog thread. Dropping the last one ends the thread.
#[derive(Clone)]
pub struct Watchdog {
    shared: Arc<Shared>,
    _owner: Arc<Owner>,
}

/// Ends the thread when the last handle is dropped.
struct Owner(Arc<Shared>);

impl Drop for Owner {
    fn drop(&mut self) {
        self.0.lock().closed = true;
        self.0.changed.notify_all();
    }
}

impl Watchdog {
    /// Starts a watchdog thread that calls `late` when an entry is still
    /// running [`WATCHDOG_GRACE`] after its deadline, once for that entry.
    ///
    /// # Errors
    /// When the thread cannot be started.
    pub fn start(late: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let shared = Arc::new(Shared { watched: Mutex::new(Watched { deadline: None, entry: 0, closed: false }), changed: Condvar::new() });
        let watching = Arc::clone(&shared);
        thread::Builder::new().name("script-watchdog".into()).spawn(move || watch(&watching, &late))?;
        Ok(Self { _owner: Arc::new(Owner(Arc::clone(&shared))), shared })
    }

    /// A watchdog that ends the process with [`WATCHDOG_EXIT_STATUS`]: the
    /// script-host program's. Nothing is unwound and nothing flushed: the
    /// interpreter's thread is stuck inside the module, and the process's
    /// state is not trusted.
    ///
    /// # Errors
    /// When the thread cannot be started.
    pub fn ending_the_process() -> std::io::Result<Self> {
        Self::start(|| std::process::exit(WATCHDOG_EXIT_STATUS))
    }

    /// An entry begins: it must have returned by `deadline`, an instant
    /// of the scripts' clock ([`crate::clock`]).
    pub fn arm(&self, deadline: Instant) {
        let mut watched = self.shared.lock();
        watched.deadline = Some(deadline);
        watched.entry = watched.entry.wrapping_add(1);
        drop(watched);
        self.shared.changed.notify_all();
    }

    /// The entry returned.
    pub fn disarm(&self) {
        self.shared.lock().deadline = None;
        self.shared.changed.notify_all();
    }
}

/// The watchdog thread: waits on the deadline, never polls.
fn watch(shared: &Shared, late: &dyn Fn()) {
    let mut acted_for = 0_u64;
    let mut watched = shared.lock();
    loop {
        if watched.closed {
            return;
        }
        let Some(limit) = watched.deadline.and_then(|deadline| deadline.checked_add(WATCHDOG_GRACE)) else {
            watched = shared.changed.wait(watched).unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        // On the scripts' clock (`crate::clock`): where that counts the
        // process's work, an entry that was merely given no processor is
        // not late, and the wait below, which is of the wall clock, ends
        // no later than the clock can reach the limit and is then made
        // again for what is left.
        let now = crate::clock::now();
        if now >= limit {
            if watched.entry == acted_for {
                watched = shared.changed.wait(watched).unwrap_or_else(PoisonError::into_inner);
            } else {
                acted_for = watched.entry;
                drop(watched);
                late();
                watched = shared.lock();
            }
            continue;
        }
        watched = shared.changed.wait_timeout(watched, limit.saturating_duration_since(now)).unwrap_or_else(PoisonError::into_inner).0;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use super::*;

    /// Section 3.2 rule 5(b): an entry still running past its deadline and
    /// the grace is acted on, once; one that returns in time is not.
    #[test]
    fn it_acts_once_on_a_late_entry_and_not_on_a_timely_one() {
        let late = Arc::new(AtomicU32::new(0));
        let counted = Arc::clone(&late);
        let watchdog = Watchdog::start(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        // In time: armed for 50 ms ahead and disarmed at once.
        watchdog.arm(Instant::now() + Duration::from_millis(50));
        watchdog.disarm();
        thread::sleep(Duration::from_millis(50) + WATCHDOG_GRACE * 3);
        assert_eq!(late.load(Ordering::SeqCst), 0);
        // Late: armed for 5 ms ahead and left running.
        let began = Instant::now();
        watchdog.arm(began + Duration::from_millis(5));
        while late.load(Ordering::SeqCst) == 0 && began.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(1));
        }
        let waited = began.elapsed();
        assert_eq!(late.load(Ordering::SeqCst), 1);
        assert!(waited >= Duration::from_millis(5) + WATCHDOG_GRACE, "acted after {waited:?}, before the grace had passed");
        // Once only, however long the entry stays.
        thread::sleep(WATCHDOG_GRACE * 3);
        assert_eq!(late.load(Ordering::SeqCst), 1);
        watchdog.disarm();
    }
}
