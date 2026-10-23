//! The trusted core's timed work (decisions 22, 46 and 53): the loading
//! pace, the next wake, and what falls due.

use protocol::decoder::ToDecoder;

use super::{CHUNK_BYTES, Core, LOAD_INTERVAL, SLEEP_CHECK};
use crate::source::Next;

impl Core {
    /// Hands every channel the next piece of its data, whether or not it
    /// is on screen (decision 22).
    fn load(&mut self) {
        let mut messages = Vec::new();
        for tab in self.session.all_tabs_mut() {
            // A sleeping channel's file waits where it is: what was read is
            // kept, and the rest is read when it wakes.
            if !self.decoders.contains_key(&tab.channel.id) {
                continue;
            }
            let message = match tab.channel.source.next(CHUNK_BYTES) {
                Next::Data(bytes) => ToDecoder::Data(bytes.to_vec()),
                Next::End => ToDecoder::End,
                Next::Done => continue,
            };
            messages.push((tab.channel.id, message));
        }
        for (channel, message) in messages {
            self.tell(channel, message);
        }
    }

    /// Whether anything is done at the loading pace (`LOAD_INTERVAL`): a
    /// channel's file still being fed, a channel being waited on to take
    /// keys, or the console open to be sent what comes.
    fn loading_wanted(&self) -> bool {
        let feeding = self.session.all_tabs().any(|tab| self.decoders.contains_key(&tab.channel.id) && tab.channel.source.has_more());
        #[cfg(feature = "interaction")]
        let typing = self.typing.waiting();
        #[cfg(not(feature = "interaction"))]
        let typing = false;
        feeding || typing || self.console.is_open()
    }

    /// When the trusted core next has timed work, if it has any: it waits
    /// for an event until then, and with none, for an event alone
    /// (decision 46).
    pub(super) fn next_wake(&self) -> Option<std::time::Instant> {
        let mut next = Some(self.next_sleep_check);
        if self.loading_wanted() {
            next = next.map(|at| at.min(self.next_load));
        }
        if let Some((_, until)) = &self.notice {
            next = next.map(|at| at.min(*until));
        }
        if let Some((_, _, still)) = self.window_moved {
            next = next.map(|at| at.min(still));
        }
        if let Some(due) = self.cookies.due {
            next = next.map(|at| at.min(due));
        }
        if let Some(due) = self.updates.due() {
            next = next.map(|at| at.min(due));
        }
        if let Some(due) = self.frames_due_at() {
            next = next.map(|at| at.min(due));
        }
        next
    }

    /// The timed work now due.
    pub(super) fn tick(&mut self) -> Result<(), String> {
        let now = std::time::Instant::now();
        if now >= self.next_load && self.loading_wanted() {
            self.next_load = now.checked_add(LOAD_INTERVAL).unwrap_or(now);
            self.load();
            self.typing_tick()?;
            self.send_console()?;
        }
        self.notice_tick()?;
        if now >= self.next_sleep_check {
            self.next_sleep_check = now.checked_add(SLEEP_CHECK).unwrap_or(now);
            self.match_decoders()?;
        }
        if self.window_moved.is_some_and(|(_, _, still)| now >= still) {
            self.keep_window();
        }
        if self.cookies.due.is_some_and(|due| now >= due) {
            self.keep_cookies();
        }
        if self.updates.due().is_some_and(|due| now >= due) {
            self.updates.tick(self.broker.as_deref(), now);
        }
        if self.frames_due_at().is_some_and(|due| now >= due) {
            self.send_due_frames(now)?;
        }
        Ok(())
    }

    /// Lets time pass for typing: a channel that has not answered HELLO
    /// in time takes no input (INPUT.md section 6.1).
    #[cfg_attr(not(feature = "interaction"), allow(clippy::unnecessary_wraps, clippy::unused_self, reason = "only typing has a time"))]
    fn typing_tick(&mut self) -> Result<(), String> {
        #[cfg(feature = "interaction")]
        {
            let sends = self.typing.tick(self.now());
            self.deliver_typing(sends)?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::test_support::test_core;
    use super::*;
    use std::time::Duration;

    /// Decision 53: the trusted core waits for an event until its next
    /// deadline, which is the sleep check alone while nothing is fed, no
    /// typing waits and the console is closed; the console open, it is
    /// the loading pace. Before this, two threads woke it every 200 ms and
    /// every 60 s for the life of the process.
    #[test]
    fn the_core_wakes_only_for_work_it_has() {
        let mut core = test_core();
        let now = std::time::Instant::now();
        let idle = core.next_wake().unwrap();
        assert!(idle >= now + SLEEP_CHECK.saturating_sub(Duration::from_secs(1)), "the sleep check alone: {:?}", idle - now);
        core.console.set_open(true);
        assert!(core.next_wake().unwrap() <= std::time::Instant::now(), "the console open: due now");
        core.tick().unwrap();
        let paced = core.next_wake().unwrap();
        assert!(paced > std::time::Instant::now() && paced <= std::time::Instant::now() + LOAD_INTERVAL, "then at the loading pace");
        core.console.set_open(false);
        assert!(core.next_wake().unwrap() >= now + SLEEP_CHECK.saturating_sub(Duration::from_secs(1)), "closed: the sleep check alone");
    }
}
