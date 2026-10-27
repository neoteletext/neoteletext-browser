//! What hyperlinks ask of the session (`hyperlinks.md` sections 7 and 10):
//! following a link to another channel or to a page of this one, and
//! whether a channel has a page.

use neoteletext::address::Address;

use super::{Pending, Session, Visit};

impl Session {
    /// Opens another channel in the active tab because a link on a page led
    /// there and the user confirmed it in the trusted strip.
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
}
