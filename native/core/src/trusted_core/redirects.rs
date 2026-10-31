//! Redirects: a server's that leaves a channel (REQ-701) and a channel's
//! program's (`INPUT.md` section 7.11, REQ-1688). The fetcher says where one
//! leads, and the trusted core takes the tab there by its own rules. Nothing
//! here asks whether keys go to the channel: a tab and the server agree on
//! what it selects (neoteletext-common's DEC-1707).

use std::time::Instant;

use interface_core::canvas::PageKey;
use neoteletext::address::{Address, redirect_destination};
use neoteletext_core::limits::SUBCODE_ANY;
use protocol::console::Level;
use protocol::fetcher::MAX_NOTICE;
use protocol::{ToInterface, clean};

use super::Core;
use crate::session::RedirectBy;

#[cfg(all(test, unix))]
mod tests;

/// How long a tab shows where a redirect is taking it (REQ-1686 item 4).
const REDIRECT_NOTICE: std::time::Duration = std::time::Duration::from_secs(1);

impl Core {
    /// `channel`'s fetcher says a server's redirect leaves it.
    pub(super) fn redirected(&mut self, channel: u32, location: &str) -> Result<(), String> {
        match self.session.server_redirect(channel, location) {
            Ok(address) => self.redirect_tab(channel, address, RedirectBy::Server),
            Err(why) => self.in_window_of(channel, |core| core.failed_to_open(channel, &why)),
        }
    }

    /// Whether `channel` is a live channel's that a tab holds and that is
    /// not already leaving: what alone can redirect by a record.
    fn may_redirect(&self, channel: u32) -> bool {
        self.session.all_tabs().any(|tab| tab.channel.id == channel && tab.channel.live && tab.channel.redirecting.is_none())
    }

    /// `channel`'s program redirected its tab to `page`, and to the sub-page
    /// `sub` unless that is 3F7F (REQ-212 item 1). A page the channel does
    /// not list is no redirect: it is logged, and the server, which took it
    /// as a selection, is told again what the tab selects.
    pub(super) fn page_redirected(&mut self, channel: u32, page: u16, sub: u16) -> Result<(), String> {
        if !self.may_redirect(channel) {
            return Ok(());
        }
        let listed = self.session.all_tabs().any(|tab| tab.channel.id == channel && tab.channel.page_title(page).is_some());
        if !listed {
            self.log(Level::Warning, Some(channel), &format!("a redirect to page {page:03X}, which the channel does not list, was dropped"));
            return self.select_again(channel);
        }
        let subpage = (sub != SUBCODE_ANY).then_some(sub);
        self.in_window_of(channel, |core| {
            let marked = core.session.redirect_page(channel, page, subpage);
            core.say_passed_over(channel, marked);
            let key = PageKey { channel, page };
            if core.active_channel() != Some(channel) {
                // Its tab opens the page when it is shown.
                return core.select_redirected(key, sub);
            }
            core.send(&ToInterface::Strip(core.strip()))?;
            core.open_page(key, subpage)
        })
    }

    /// `channel`'s program redirected its tab to `location`, hostile text,
    /// by the rules a server's redirect goes by (REQ-1688 item 3). One the
    /// rules refuse is dropped and logged, and the tab stays. An address of
    /// the channel itself stays in it, so it shows no notice (REQ-1686 item
    /// 1), and goes as a page redirect.
    pub(super) fn program_redirected(&mut self, channel: u32, location: &str) -> Result<(), String> {
        if !self.may_redirect(channel) {
            return Ok(());
        }
        let address = match redirect_destination(location) {
            Ok(address) => address,
            Err(why) => {
                self.redirect_dropped(channel, &why);
                return Ok(());
            }
        };
        let own = self.session.all_tabs().find(|tab| tab.channel.id == channel).filter(|tab| tab.address().is_some_and(|shown| shown.same_channel(&address)));
        let Some(tab) = own else { return self.redirect_tab(channel, address, RedirectBy::Program) };
        match address.page.or(tab.channel.initial_page) {
            Some(page) => self.page_redirected(channel, page, address.subpage.unwrap_or(SUBCODE_ANY)),
            None => Ok(()),
        }
    }

    /// Says in the tab's console that the page a redirect left is passed
    /// over from now on (neoteletext-common's DEC-1684 item 9).
    fn say_passed_over(&mut self, channel: u32, marked: bool) {
        if marked {
            self.log(Level::Info, Some(channel), "a page moved the tab with no input from you: the Back and Forward buttons pass over it");
        }
    }

    /// Says in the tab's console that a program's redirect was not followed.
    /// A channel's address is its own to name, so it can only be cleaned.
    fn redirect_dropped(&mut self, channel: u32, why: &str) {
        self.log(Level::Warning, Some(channel), &format!("a redirect was dropped: {}", clean(why, MAX_NOTICE)));
    }

    /// Takes the tab holding `channel` to `address`, another channel: in the
    /// window holding it, the tab shows where it is going for a second, then
    /// follows, as a web browser follows a redirect, whatever sent it. A
    /// server's redirect that is refused is the channel failing to open; a
    /// program's leaves a channel that is open where it is.
    fn redirect_tab(&mut self, channel: u32, address: Address, by: RedirectBy) -> Result<(), String> {
        let now = Instant::now();
        let until = now.checked_add(REDIRECT_NOTICE).unwrap_or(now);
        self.in_window_of(channel, |core| match core.session.redirect_tab(channel, address, until, by) {
            Ok(()) if core.active_channel() == Some(channel) => core.send(&ToInterface::Strip(core.strip())),
            Ok(()) => Ok(()),
            Err(why) if by == RedirectBy::Program => {
                core.redirect_dropped(channel, &why);
                Ok(())
            }
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
                let marked = core.session.finish_redirect(channel);
                core.say_passed_over(channel, marked);
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
