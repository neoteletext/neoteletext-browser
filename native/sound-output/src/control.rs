//! What the owner of a stream wants of the thread that writes it, where a
//! platform's stream is written from a thread of its own (ALSA, WASAPI):
//! stopped, playing or closed. The thread waits on it while stopped, so
//! a stopped stream wakes nothing (decision 46: a program waits on what it
//! needs), and reads it after each buffer while playing.

use std::sync::{Condvar, Mutex, PoisonError};

/// What the owner wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    /// Stopped.
    Paused,
    /// Playing.
    Playing,
    /// Closed: the thread ends.
    Closing,
}

/// The owner's wish and how often it changed.
#[derive(Debug)]
struct State {
    wanted: Wanted,
    changes: u64,
}

/// Shared between the owner and the thread.
#[derive(Debug)]
pub struct Control {
    state: Mutex<State>,
    changed: Condvar,
}

impl Control {
    /// Stopped, until told otherwise.
    pub const fn new() -> Self {
        Self { state: Mutex::new(State { wanted: Wanted::Paused, changes: 0 }), changed: Condvar::new() }
    }

    /// The owner's wish. A wish to close is never replaced.
    pub fn set(&self, wanted: Wanted) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.wanted != Wanted::Closing {
            state.wanted = wanted;
            state.changes = state.changes.wrapping_add(1);
        }
        drop(state);
        self.changed.notify_all();
    }

    /// The owner's wish now, and how many changes it has seen.
    pub fn wanted(&self) -> (Wanted, u64) {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        (state.wanted, state.changes)
    }

    /// Waits until the wish changes from the one seen at `seen` changes,
    /// and returns it with its count.
    pub fn changed_since(&self, seen: u64) -> (Wanted, u64) {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let state = self.changed.wait_while(state, |state| state.changes == seen && state.wanted != Wanted::Closing).unwrap_or_else(PoisonError::into_inner);
        (state.wanted, state.changes)
    }
}

/// How the thread that writes a stream goes from one wish to the next: it
/// follows the owner, and after a failure the device cannot recover from
/// it waits for the owner's next wish before trying again, so that a lost
/// device is never retried in a busy loop.
#[derive(Debug)]
pub struct Follower {
    seen: u64,
    broken: bool,
}

impl Follower {
    /// Following from the start.
    pub const fn new() -> Self {
        Self { seen: 0, broken: false }
    }

    /// The device failed and cannot go on until the owner wishes again.
    pub const fn broke(&mut self) {
        self.broken = true;
    }

    /// What to do next. A wish the thread has already followed to a stop,
    /// or any wish after a failure, is waited on until the owner's next:
    /// the stopped thread waits here.
    pub fn next(&mut self, control: &Control) -> Wanted {
        let (mut wanted, mut changes) = control.wanted();
        if wanted != Wanted::Closing && changes == self.seen && (self.broken || wanted == Wanted::Paused) {
            (wanted, changes) = control.changed_since(self.seen);
        }
        self.broken = false;
        self.seen = changes;
        wanted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    /// A stopped thread waits for the owner's wish, a playing one goes on,
    /// and a wish to close ends it whatever came after.
    #[test]
    fn the_thread_follows_the_owner() {
        let control = Arc::new(Control::new());
        let mut follower = Follower::new();
        control.set(Wanted::Playing);
        assert_eq!(follower.next(&control), Wanted::Playing);
        assert_eq!(follower.next(&control), Wanted::Playing, "playing goes on without waiting");
        control.set(Wanted::Paused);
        assert_eq!(follower.next(&control), Wanted::Paused, "told to stop, it stops");
        let owner = Arc::clone(&control);
        let started = std::time::Instant::now();
        let waker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            owner.set(Wanted::Playing);
        });
        // Stopped, it waits for the change.
        assert_eq!(follower.next(&control), Wanted::Playing);
        assert!(started.elapsed() >= Duration::from_millis(50), "it waited for the owner");
        waker.join().unwrap();
        control.set(Wanted::Closing);
        control.set(Wanted::Playing);
        assert_eq!(follower.next(&control), Wanted::Closing, "closing is never undone");
    }

    /// After a failure the thread does not try again until the owner
    /// wishes again, though the owner still wants it playing.
    #[test]
    fn a_broken_device_waits_for_the_owner() {
        let control = Arc::new(Control::new());
        let mut follower = Follower::new();
        control.set(Wanted::Playing);
        assert_eq!(follower.next(&control), Wanted::Playing);
        follower.broke();
        let owner = Arc::clone(&control);
        let started = std::time::Instant::now();
        let waker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            owner.set(Wanted::Playing);
        });
        assert_eq!(follower.next(&control), Wanted::Playing, "it tries again");
        assert!(started.elapsed() >= Duration::from_millis(50), "only once the owner wished again");
        waker.join().unwrap();
    }
}
