//! A tab going to another channel (REQ-7164): its page area is cleared and
//! says so for the session's `NAVIGATING_NOTICE` at least, and until a page
//! of the channel is ready. The session starts the
//! notice where it gives a tab another channel; here it ends, on the timer
//! or when the page's frame goes to the interface, whichever is later. It
//! locks nothing: every intent is taken as at any other time, and a
//! navigation that replaces the channel replaces its notice.

use std::time::Instant;

use interface_core::canvas::PageKey;
use protocol::ToInterface;

use super::Core;

impl Core {
    /// When the next notice's least time ends: none once every notice
    /// waits only on its page, which is no matter of time.
    pub(super) fn navigating_due_at(&self) -> Option<Instant> {
        self.session.all_tabs().filter_map(|tab| tab.channel.navigating.and_then(|notice| notice.until)).min()
    }

    /// Ends each notice whose least time is up at `now` and whose page is
    /// ready; one whose page is not waits on for it. A tab behind others
    /// runs its own time, as a redirect's notice does (REQ-1686 item 8).
    pub(super) fn navigating_tick(&mut self, now: Instant) -> Result<(), String> {
        let ended: Vec<u32> = self.session.all_tabs_mut().filter_map(|tab| tab.channel.navigating_at(now).then_some(tab.channel.id)).collect();
        ended.into_iter().try_for_each(|channel| self.navigating_ended(channel))
    }

    /// A frame of `key` went to the interface. It is what the tab of its
    /// channel waits for when it is the page open there, or when none is
    /// and the tab shows the wall.
    pub(super) fn page_ready(&mut self, key: PageKey) -> Result<(), String> {
        if self.controls.open_page(key.channel).is_some_and(|page| page != key.page) {
            return Ok(());
        }
        let ended = self.session.all_tabs_mut().find(|tab| tab.channel.id == key.channel).is_some_and(|tab| tab.channel.navigating_ready());
        if ended { self.navigating_ended(key.channel) } else { Ok(()) }
    }

    /// The notice of `channel`'s tab is over: the strip of its window says
    /// so when the tab is in front, and the interface draws the page.
    fn navigating_ended(&mut self, channel: u32) -> Result<(), String> {
        self.in_window_of(channel, |core| if core.active_channel() == Some(channel) { core.send(&ToInterface::Strip(core.strip())) } else { Ok(()) })
    }
}

#[cfg(test)]
mod tests;
