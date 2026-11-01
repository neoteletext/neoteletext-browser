//! The trusted strip's notice (`settings-home-console-proxy-security.md`
//! section 3, requirements 19.3 and 19.10).

use std::time::Duration;

use interface_core::browser::Denied;
#[cfg(feature = "interaction")]
use interface_core::canvas::PageKey;
#[cfg(feature = "interaction")]
use neoteletext_core::limits::SUBCODE_ANY;
use protocol::console::Level;
use protocol::{ToInterface, clean};

use super::Core;

/// How long the strip says a notice (`settings-home-console-proxy-security.md`
/// section 3).
const NOTICE_TIME: Duration = Duration::from_secs(10);
/// What the strip says while a live channel opens its connection.
const CONNECTING: &str = "Connecting to the channel";

impl Core {
    /// The strip as the session has it, with the notice while it lasts.
    pub(super) fn strip(&self) -> interface_core::browser::StripState {
        let mut strip = self.session.strip();
        if let Some((text, until)) = &self.notice
            && std::time::Instant::now() < *until
        {
            strip.notice = clean(text, protocol::MAX_STRIP_NOTICE);
        } else if self.session.active_tab().is_some_and(|tab| tab.channel.connecting) {
            // A live channel opening its connection is connecting, never a
            // page not found (DEC-1467).
            strip.notice = String::from(CONNECTING);
        } else if let Some((key, subpage)) = self.awaiting()
            && self.active_channel() == Some(key.channel)
        {
            strip.notice = format!("Loading page {:03X}, sub-page {subpage:04X}", key.page);
        }
        strip.denied = self.denial_shown();
        strip
    }

    /// The refusal the active tab shows: one of what it still selects.
    #[cfg(feature = "interaction")]
    fn denial_shown(&self) -> Option<Denied> {
        let (denied, selection) = self.session.active_tab()?.channel.denied?;
        (self.typing.selection(self.active_channel()?) == selection).then_some(denied)
    }

    #[cfg(not(feature = "interaction"))]
    #[expect(clippy::unused_self, reason = "a selection is input, compiled out")]
    fn denial_shown(&self) -> Option<Denied> {
        None
    }

    /// The channel refused a selection, in a record its fetcher passed on,
    /// untrusted: a status a DENIED record does not carry, or a place that is
    /// not the page the tab has open, shows nothing, as a refusal of a page
    /// the wall asked for by itself should not. The console says each.
    pub(super) fn denied(&mut self, channel: u32, status: u16, place: &str) -> Result<(), String> {
        let place = clean(place, protocol::fetcher::MAX_DENIED_PLACE);
        let shown = if place.is_empty() { "the channel" } else { place.as_str() };
        self.log(Level::Warning, Some(channel), &format!("channel {channel}: {shown} was refused: status {status}"));
        let Some(denied) = Denied::of_status(status) else { return Ok(()) };
        let selection = self.selected_place(channel, &place);
        if !place.is_empty() && selection.is_none() {
            return Ok(());
        }
        if let Some(tab) = self.session.all_tabs_mut().find(|tab| tab.channel.id == channel) {
            tab.channel.denied = Some((denied, selection));
        }
        self.send(&ToInterface::Strip(self.strip()))
    }

    /// What the tab of `channel` selects, when it is the page open there and
    /// `place` names it, or `place` names nothing.
    #[cfg(feature = "interaction")]
    fn selected_place(&self, channel: u32, place: &str) -> Option<(u16, u16)> {
        let (page, sub) = self.typing.selection(channel)?;
        let target = if sub == SUBCODE_ANY { format!("{page:03X}") } else { format!("{page:03X}/{sub:04X}") };
        (place.is_empty() || (place == target && self.controls.is_open(PageKey { channel, page }))).then_some((page, sub))
    }

    #[cfg(not(feature = "interaction"))]
    #[expect(clippy::unused_self, reason = "a selection is input, compiled out")]
    fn selected_place(&self, _channel: u32, _place: &str) -> Option<(u16, u16)> {
        None
    }

    /// Says `text` in the strip for a while, and in the log.
    pub(super) fn say(&mut self, text: String) -> Result<(), String> {
        self.log(Level::Warning, None, &text);
        let now = std::time::Instant::now();
        self.notice = Some((text, now.checked_add(NOTICE_TIME).unwrap_or(now)));
        self.send(&ToInterface::Strip(self.strip()))
    }

    /// The notice's time is up: the strip shows the address again.
    pub(super) fn notice_tick(&mut self) -> Result<(), String> {
        if self.notice.as_ref().is_some_and(|(_, until)| std::time::Instant::now() >= *until) {
            self.notice = None;
            self.send(&ToInterface::Strip(self.strip()))?;
        }
        Ok(())
    }
}
