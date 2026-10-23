//! The trusted strip's notice (`settings-home-console-proxy-security.md`
//! section 3, requirements 19.3 and 19.10).

use std::time::Duration;

use protocol::console::Level;
use protocol::{ToInterface, clean};

use super::Core;

/// How long the strip says a notice (`settings-home-console-proxy-security.md`
/// section 3).
const NOTICE_TIME: Duration = Duration::from_secs(10);

impl Core {
    /// The strip as the session has it, with the notice while it lasts.
    pub(super) fn strip(&self) -> interface_core::browser::StripState {
        let mut strip = self.session.strip();
        if let Some((text, until)) = &self.notice
            && std::time::Instant::now() < *until
        {
            strip.notice = clean(text, protocol::MAX_STRIP_NOTICE);
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
