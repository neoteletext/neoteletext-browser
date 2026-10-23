//! The bytes a helper's messages may hold in the trusted core's queue of
//! events before it handles them (finding F-10, `security-analysis.md`
//! section 5.3): each helper's reading thread takes its message's bytes
//! from its own budget before it queues the message, and waits while the
//! budget is spent, so a helper that writes faster than the core handles
//! its messages stalls on its own full pipe, and the core's memory does
//! not grow with it. The core's own thread never waits on a budget: it
//! gives the bytes back when it has handled the message.

use std::sync::{Arc, Condvar, Mutex, PoisonError};

use crate::outbox::MAX_QUEUED_BYTES;

/// The most bytes of one helper's messages waiting to be handled: as many
/// as the core lets wait for a process to read (`outbox::MAX_QUEUED_BYTES`).
pub const MAX_WAITING_BYTES: usize = MAX_QUEUED_BYTES;
// Every message fits in a budget, so a reader never waits for room that
// cannot come.
const _: () = assert!(protocol::MAX_PAYLOAD < MAX_WAITING_BYTES, "a message is smaller than a budget");

#[derive(Default)]
struct Spent {
    bytes: Mutex<usize>,
    returned: Condvar,
}

/// One helper's budget, shared by its reading thread and the tickets it
/// gives out.
#[derive(Clone, Default)]
pub struct Budget {
    spent: Arc<Spent>,
}

/// The bytes of one message waiting to be handled: given back to its
/// budget when the ticket is dropped, once the message is handled.
pub struct Ticket {
    spent: Arc<Spent>,
    bytes: usize,
}

impl Budget {
    /// Takes `bytes` from the budget, waiting while that would spend more
    /// than [`MAX_WAITING_BYTES`]. A message larger than the whole budget,
    /// which no protocol bound allows, waits until nothing else is spent.
    #[must_use]
    pub fn take(&self, bytes: usize) -> Ticket {
        let mut spent = self.spent.bytes.lock().unwrap_or_else(PoisonError::into_inner);
        while *spent > 0 && spent.saturating_add(bytes) > MAX_WAITING_BYTES {
            spent = self.spent.returned.wait(spent).unwrap_or_else(PoisonError::into_inner);
        }
        *spent = spent.saturating_add(bytes);
        Ticket { spent: Arc::clone(&self.spent), bytes }
    }

    /// The bytes spent now.
    #[must_use]
    pub fn spent(&self) -> usize {
        *self.spent.bytes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut spent = self.spent.bytes.lock().unwrap_or_else(PoisonError::into_inner);
        *spent = spent.saturating_sub(self.bytes);
        self.spent.returned.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A reader waits while its budget is spent, and goes on once the core
    /// has handled a message and its ticket is dropped.
    #[test]
    fn a_spent_budget_waits_for_a_ticket_back() {
        let budget = Budget::default();
        let first = budget.take(MAX_WAITING_BYTES);
        let reader = budget.clone();
        let waiting = std::thread::spawn(move || drop(reader.take(1)));
        std::thread::sleep(Duration::from_millis(100));
        assert!(!waiting.is_finished(), "the budget is spent");
        assert_eq!(budget.spent(), MAX_WAITING_BYTES);
        drop(first);
        waiting.join().unwrap();
        assert_eq!(budget.spent(), 0);
    }
}
