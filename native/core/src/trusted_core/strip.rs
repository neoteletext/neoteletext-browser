//! The trusted strip's notice (`settings-home-console-proxy-security.md`
//! section 3, requirements 19.3 and 19.10).

use std::time::Duration;

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
        strip
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
