//! What a tab has selected, in its address (REQ-1690): the viewer's choice
//! of a page, a sub-page or the wall, a channel's redirect, arriving at a
//! channel's bare address, and what its Back list goes by. The trusted core
//! keeps the address, at the one point every selection is reported to it.

use neoteletext::address::Address;

use super::{Pending, Session, Tab, Visit};

impl Tab {
    /// The current entry names `address` from now on, with no Back entry
    /// added, and keeps its title.
    fn rename(&mut self, address: Address, now: i64) -> bool {
        let Some(entry) = self.history.get_mut(self.position) else { return false };
        entry.address = address;
        entry.at = now;
        true
    }
}

impl Session {
    /// The viewer chose `page` in the active tab: a navigation, as a followed
    /// link is, with a Back entry and a visit when it draws (REQ-1690 item
    /// 3), which also starts a program's redirects counting again. True when
    /// the address changed. The browser's own start page is in no channel,
    /// and keeps its address. A file has no descriptor to give an initial
    /// page, so the first page it shows is where a tab at its bare address
    /// arrives, in the same entry (REQ-1693 item 9).
    pub fn page_chosen(&mut self, page: u16) -> bool {
        let now = self.now;
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        let Some(address) = tab.address().filter(|shown| shown.page != Some(page) && !shown.is_start_page()).and_then(|shown| shown.with_page(page)) else {
            return false;
        };
        if tab.address().is_some_and(|shown| shown.file.is_some() && shown.is_bare()) {
            return tab.rename(address, now);
        }
        tab.push(Visit::new(address, false, now));
        tab.channel.redirects.clear();
        tab.channel.page_selected();
        tab.pending = Some(Pending { typed: false, drawn: true, reload: false });
        true
    }

    /// The viewer chose `subpage` of `page`, the page open in the active tab,
    /// or left its sub-pages for the page alone: the current entry follows,
    /// and no Back entry is added, as a page can have thousands (REQ-1690
    /// item 4). True when the address changed.
    pub fn subpage_chosen(&mut self, page: u16, subpage: Option<u16>) -> bool {
        let now = self.now;
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        let Some(address) =
            tab.address().filter(|shown| shown.page == Some(page) && shown.subpage != subpage).and_then(|shown| shown.with_subpage(page, subpage))
        else {
            return false;
        };
        tab.rename(address, now)
    }

    /// The viewer left the page open for the wall of pages: the current
    /// entry names the wall (REQ-1690 item 5). True when the address changed.
    pub fn wall_chosen(&mut self) -> bool {
        let now = self.now;
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        let Some(address) = tab.address().filter(|shown| !shown.wall && !shown.is_start_page()).and_then(Address::with_wall) else { return false };
        tab.rename(address, now)
    }

    /// `channel` gave its initial page. A tab at its bare address arrives
    /// there: the address names the page, in the same entry (REQ-1693 item
    /// 3). True when the address changed.
    pub fn arrived(&mut self, channel: u32, page: u16) -> bool {
        let now = self.now;
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return false };
        tab.channel.initial_page = Some(page);
        let bare = |shown: &&Address| shown.is_bare() && !shown.is_start_page() && shown.file.is_none();
        let arrived = |shown: &Address| shown.with_page(page).and_then(|at| at.with_parameters(shown.parameters()));
        let Some(address) = tab.address().filter(bare).and_then(arrived) else { return false };
        tab.rename(address, now)
    }

    /// The channel's program redirected the tab holding `channel` to `page`,
    /// and `subpage` of it if it names one (`INPUT.md` section 7.11), with
    /// the page parameters of an address it named, its Back list by
    /// [`Tab::redirected`]. True when the page left is passed over from now
    /// on.
    pub fn redirect_page(&mut self, channel: u32, page: u16, subpage: Option<u16>, parameters: Option<&str>) -> bool {
        let now = self.now;
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return false };
        let Some(address) = tab.address().and_then(|shown| shown.with_subpage(page, subpage)).and_then(|at| at.with_parameters(parameters)) else {
            return false;
        };
        if tab.address().is_some_and(|shown| shown.text == address.text) {
            return false;
        }
        let typed = tab.pending.is_some_and(|pending| pending.typed);
        let marked = tab.redirected(Visit::new(address, true, now));
        tab.channel.page_selected();
        tab.pending = Some(Pending { typed, drawn: true, reload: false });
        marked
    }

    /// The viewer gave the tab holding `channel` input: a program's redirects
    /// are counted again from here (REQ-1688 item 6), the page's own input
    /// is counted, and a mark that the buttons pass over it clears
    /// (DEC-1684 item 5).
    #[cfg(feature = "interaction")]
    pub fn viewer_input(&mut self, channel: u32) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return };
        tab.channel.redirects.clear();
        tab.channel.since_selected.acted = true;
        if let Some(entry) = tab.history.get_mut(tab.position) {
            entry.passed_over = false;
        }
    }

    /// A page of the tab holding `channel` drew.
    pub fn page_drawn(&mut self, channel: u32) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.channel.id == channel) {
            tab.channel.since_selected.drawn = true;
        }
    }
}
