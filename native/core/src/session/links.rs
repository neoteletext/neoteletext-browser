//! What hyperlinks ask of the session (`hyperlinks.md` sections 7 and 10):
//! following a link to another channel or to a page of this one, and
//! whether a channel has a page.

use neoteletext::address::Address;

use super::{Pending, Session, Visit};

impl Session {
    /// Opens another channel in the active tab because a link on a page led
    /// there (REQ-1670).
    ///
    /// # Errors
    /// When there is no tab.
    pub fn follow(&mut self, address: Address) -> Result<(), String> {
        // A link to the address shown is a reload, as in Chrome
        // (`ShouldTreatNavigationAsReload`).
        if self.is_shown(&address) {
            self.reload();
            return Ok(());
        }
        self.visit(address, true, false)
    }

    /// A link on a page of the active tab led to another page of its
    /// channel: the tab's history gains the page, and the channel stays open.
    /// Returns the address the strip now shows.
    pub fn show_page(&mut self, page: u16) -> Option<&Address> {
        let tab = self.tabs.get_mut(self.active)?;
        let address = tab.address()?.with_page(page)?;
        // A link to the page shown is a reload, which the caller makes.
        if tab.address().is_some_and(|shown| shown.text == address.text) {
            return None;
        }
        tab.push(Visit::new(address, true, self.now));
        tab.pending = Some(Pending { typed: false, drawn: true, reload: false });
        tab.address()
    }

    /// The channel's program redirected the tab holding `channel` to `page`,
    /// and `subpage` of it if it names one (`INPUT.md` section 7.11). True
    /// when the tab's address changed.
    pub fn redirect_page(&mut self, channel: u32, page: u16, subpage: Option<u16>) -> bool {
        let now = self.now;
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return false };
        let Some(address) = tab.address().and_then(|shown| shown.with_page(page)).map(|address| Address { subpage, ..address }) else { return false };
        if tab.address().is_some_and(|shown| shown.page == address.page && shown.subpage == subpage) {
            return false;
        }
        tab.push(Visit::new(address, true, now));
        tab.pending = Some(Pending { typed: false, drawn: true, reload: false });
        true
    }

    /// The viewer gave the tab holding `channel` input: a program's redirects
    /// are counted again from here (REQ-1688 item 6).
    pub fn viewer_input(&mut self, channel: u32) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.channel.id == channel) {
            tab.channel.redirects.clear();
        }
    }
}
