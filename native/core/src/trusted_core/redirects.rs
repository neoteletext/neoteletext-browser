//! A redirect that leaves a channel (REQ-701): the fetcher says where it
//! leads, and the trusted core takes the tab there by its own rules.

use std::time::Instant;

use neoteletext::address::Address;
use protocol::console::Level;
use protocol::fetcher::MAX_NOTICE;
use protocol::{ToInterface, clean};

use super::Core;

#[cfg(all(test, unix))]
mod tests;

/// How long a tab shows where a redirect is taking it (REQ-1686 item 4).
const REDIRECT_NOTICE: std::time::Duration = std::time::Duration::from_secs(1);

impl Core {
    /// `channel`'s fetcher says a server's redirect leaves it.
    pub(super) fn redirected(&mut self, channel: u32, location: &str) -> Result<(), String> {
        match self.session.server_redirect(channel, location) {
            Ok(address) => self.redirect_tab(channel, address),
            Err(why) => self.in_window_of(channel, |core| core.failed_to_open(channel, &why)),
        }
    }

    /// Takes the tab holding `channel` to `address`, another channel: in the
    /// window holding it, the tab shows where it is going for a second, then
    /// follows, as a web browser follows a redirect, whatever sent it. A
    /// redirect that is refused is the channel failing to open.
    fn redirect_tab(&mut self, channel: u32, address: Address) -> Result<(), String> {
        let now = Instant::now();
        let until = now.checked_add(REDIRECT_NOTICE).unwrap_or(now);
        self.in_window_of(channel, |core| match core.session.redirect_tab(channel, address, until) {
            Ok(()) if core.active_channel() == Some(channel) => core.send(&ToInterface::Strip(core.strip())),
            Ok(()) => Ok(()),
            Err(why) => core.failed_to_open(channel, &why),
        })
    }

    /// Says why `channel` could not open: in the strip of its tab when that
    /// is in front, and in the log.
    fn failed_to_open(&mut self, channel: u32, why: &str) -> Result<(), String> {
        let sentence = format!("The channel could not open: {}", clean(why, MAX_NOTICE));
        if self.active_channel() == Some(channel) {
            return self.say(sentence);
        }
        self.log(Level::Warning, Some(channel), &sentence);
        Ok(())
    }

    /// When the next notice's second ends.
    pub(super) fn redirect_due_at(&self) -> Option<Instant> {
        self.session.all_tabs().filter_map(|tab| tab.channel.redirecting.as_ref()).map(|redirect| redirect.until).min()
    }

    /// Follows each redirect whose second is up, in the window holding its
    /// tab: a tab behind others runs its own second (REQ-1686 item 8).
    pub(super) fn finish_redirects(&mut self, now: Instant) -> Result<(), String> {
        let due: Vec<u32> = self
            .session
            .all_tabs()
            .filter(|tab| tab.channel.redirecting.as_ref().is_some_and(|redirect| redirect.until <= now))
            .map(|tab| tab.channel.id)
            .collect();
        for channel in due {
            self.in_window_of(channel, |core| {
                let active = core.active_channel() == Some(channel);
                core.session.finish_redirect(channel);
                if active {
                    return core.send_all();
                }
                core.match_decoders()?;
                let (tabs, active) = (core.session.tab_infos(), core.session.active);
                core.send(&ToInterface::Tabs { tabs, active })
            })?;
        }
        Ok(())
    }
}
