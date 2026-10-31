//! Filling the wall's missing tiles (REQ-1559): a live channel sends only
//! the selected page, so the tiles on screen that the tab's cache cannot
//! draw are selected one at a time, in the order they lie on screen, by the
//! ordinary SELECT, each after its frame or the channel's limit for
//! answering; then SELECT page 0, so no page stays live behind the wall.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use neoteletext_core::limits::SUBCODE_ANY;

use super::Core;

#[cfg(all(test, unix))]
mod tests;

/// The longest a channel's program takes to answer a selection: the
/// server's `PROGRAM_REPLY_LIMIT_MS` of 5,000 ms. A tile that has not come
/// by then gives way to the next.
const REPLY_LIMIT: Duration = Duration::from_secs(5);

/// What to do next.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Wait for the page asked for.
    Wait,
    /// Select this page.
    Select(u16),
    /// Every tile was asked for: select nothing.
    Finished,
}

/// The wall's filling, in the tab in front.
#[derive(Debug, Default)]
pub(super) struct WallFill {
    channel: u32,
    /// The pages on screen with nothing to draw, in screen order.
    missing: Vec<u16>,
    /// The page selected, and when it gives way.
    asked: Option<(u16, Instant)>,
    /// The pages selected since the wall was shown, which are not selected
    /// again.
    tried: BTreeSet<u16>,
    /// A page was selected that page 0 is still to follow.
    selected: bool,
}

impl WallFill {
    /// What follows at `now`.
    fn step(&mut self, now: Instant) -> Step {
        if let Some((page, until)) = self.asked {
            if self.missing.contains(&page) && now < until {
                return Step::Wait;
            }
            self.asked = None;
        }
        if let Some(page) = self.missing.iter().copied().find(|page| !self.tried.contains(page)) {
            self.tried.insert(page);
            self.asked = Some((page, now.checked_add(REPLY_LIMIT).unwrap_or(now)));
            self.selected = true;
            return Step::Select(page);
        }
        if core::mem::take(&mut self.selected) {
            return Step::Finished;
        }
        Step::Wait
    }
}

impl Core {
    /// `channel`'s decoder says which pages on screen it has nothing to draw
    /// for. On a live channel's wall, with no page open, they are filled.
    pub(super) fn wall_missing(&mut self, channel: u32, pages: Vec<u16>) -> Result<(), String> {
        let live = self.session.all_tabs().any(|tab| tab.channel.id == channel && tab.channel.live);
        if !live || self.opened.is_some() || self.active_channel() != Some(channel) {
            self.wall_fill = WallFill::default();
            return Ok(());
        }
        self.wall_fill.channel = channel;
        self.wall_fill.missing = pages;
        self.fill_step(Instant::now())
    }

    /// The wall's page is opened, or the tab leaves: the filling ends, and
    /// the wall shown again starts afresh.
    pub(super) fn fill_ends(&mut self) {
        self.wall_fill = WallFill::default();
    }

    fn fill_step(&mut self, now: Instant) -> Result<(), String> {
        let channel = self.wall_fill.channel;
        let sends = match self.wall_fill.step(now) {
            Step::Wait => return Ok(()),
            Step::Select(page) => self.typing.select(channel, page, SUBCODE_ANY, self.now_us()),
            Step::Finished => self.typing.selects_nothing(channel, self.now_us()),
        };
        self.deliver_typing(sends)
    }

    /// When the page asked for gives way.
    pub(super) fn fill_due_at(&self) -> Option<Instant> {
        self.wall_fill.asked.map(|(_, until)| until)
    }

    /// The page asked for has given way, if `now` is past its limit.
    pub(super) fn fill_tick(&mut self, now: Instant) -> Result<(), String> {
        if self.fill_due_at().is_some_and(|due| now >= due) { self.fill_step(now) } else { Ok(()) }
    }
}
