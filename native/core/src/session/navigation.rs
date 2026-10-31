//! A tab's history (requirements 11.2 and 20.10): an address opened in
//! it, the channel reloaded, Back and Forward by one step or several, and
//! the lists of each way with their chapter stops.

use interface_core::library::{MAX_CHAPTER_STOPS, MAX_TAB_LIST};
use neoteletext::address::{Address, FRAGMENT, parse, redirect_destination};
use std::time::Instant;

use super::{MAX_REDIRECTS, Pending, Redirecting, Session, Tab, Visit, Went};

impl Session {
    /// Opens `typed` in the active tab, after its current address in
    /// history; `in_field` when the user typed it in the address field.
    ///
    /// # Errors
    /// When the address cannot be opened, or there is no tab.
    pub fn navigate(&mut self, typed: &str, in_field: bool) -> Result<(), String> {
        let address = parse(typed)?;
        // The address shown, entered again, is a reload, as in Chrome
        // (`chrome.history`'s "reload" transition: "pressing Enter in the
        // address bar").
        if self.is_shown(&address) {
            self.reload();
            return Ok(());
        }
        self.visit(address, false, in_field)
    }

    /// Whether `address` is the one the active tab shows.
    #[must_use]
    pub fn is_shown(&self, address: &Address) -> bool {
        self.active_tab().and_then(Tab::address).is_some_and(|shown| shown.text == address.text)
    }

    pub(super) fn visit(&mut self, address: Address, by_page: bool, typed: bool) -> Result<(), String> {
        let channel = self.channel_for(&address);
        let now = self.now;
        let tab = self.tabs.get_mut(self.active).ok_or("no tab is open")?;
        tab.push(Visit::new(address, by_page, now));
        tab.channel = channel;
        tab.pending = Some(Pending { typed, drawn: false, reload: false });
        Ok(())
    }

    /// Where a server's redirect of `channel` leads, by the address rules.
    /// Only the request that opens a channel can move its tab: a page
    /// fetched later is no navigation, whatever an untrusted fetcher says. A
    /// fragment goes with a redirect that names none (RFC 9110 section
    /// 10.2.2).
    ///
    /// # Errors
    /// Why it is not followed, a sentence for the user.
    pub fn server_redirect(&self, channel: u32, location: &str) -> Result<Address, String> {
        let tab = self.tabs.iter().find(|tab| tab.channel.id == channel).ok_or("its tab has closed")?;
        if tab.channel.initial_page.is_some() {
            return Err("a page of it redirected elsewhere".into());
        }
        let destination = redirect_destination(location)?;
        let inherited = tab.address().and_then(|shown| shown.text.split_once(FRAGMENT)).filter(|_| !location.contains(FRAGMENT));
        match inherited {
            Some((_, fragment)) => parse(&format!("{}{FRAGMENT}{fragment}", destination.text)),
            None => Ok(destination),
        }
    }

    /// The tab holding `channel` is being redirected to `address`, whoever
    /// sent it: it is followed at `until`, when its notice has been shown
    /// (REQ-701, REQ-1686). A redirect already waiting stays as it is, so
    /// that saying it again does not put its second off.
    ///
    /// # Errors
    /// Why it is not followed, a sentence for the user. The tab stays.
    pub fn redirect_tab(&mut self, channel: u32, address: Address, until: Instant) -> Result<(), String> {
        let tab = self.tabs.iter_mut().find(|tab| tab.channel.id == channel).ok_or("its tab has closed")?;
        if tab.channel.redirecting.is_some() {
            return Ok(());
        }
        let shown = tab.address().ok_or("its tab has closed")?;
        let mut chain = tab.channel.redirects.clone();
        chain.push(shown.channel_text().to_owned());
        if chain.len() > MAX_REDIRECTS {
            return Err("too many redirects".into());
        }
        if chain.iter().any(|earlier| earlier == address.channel_text()) {
            return Err("its redirects in a loop".into());
        }
        tab.channel.redirecting = Some(Redirecting { address, until, chain });
        Ok(())
    }

    /// The notice of `channel`'s redirect is over: the destination takes the
    /// place of the address that redirected, so Back returns to the page
    /// before it, as in a web browser, with a channel of its own that
    /// remembers the chain (REQ-701).
    pub fn finish_redirect(&mut self, channel: u32) {
        let Some(Redirecting { address, chain, .. }) =
            self.tabs.iter_mut().find(|tab| tab.channel.id == channel).and_then(|tab| tab.channel.redirecting.take())
        else {
            return;
        };
        let mut next = self.channel_for(&address);
        next.redirects = chain;
        let now = self.now;
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return };
        let typed = tab.pending.is_some_and(|pending| pending.typed);
        if let Some(visit) = tab.history.get_mut(tab.position) {
            *visit = Visit::new(address, true, now);
        }
        tab.channel = next;
        tab.pending = Some(Pending { typed, drawn: false, reload: false });
    }

    /// Opens the active tab's channel afresh where the tab is in its
    /// history: a new decoder and fetcher, its pages read again (requirement
    /// 11.2). The history is as it was. False when no tab is open.
    pub fn reload(&mut self) -> bool {
        let Some(address) = self.active_tab().and_then(Tab::address).cloned() else { return false };
        let channel = self.channel_for(&address);
        let now = self.now;
        let Some(tab) = self.tabs.get_mut(self.active) else { return false };
        tab.channel = channel;
        if let Some(visit) = tab.history.get_mut(tab.position) {
            visit.at = now;
        }
        tab.pending = Some(Pending { typed: false, drawn: false, reload: true });
        true
    }

    /// Moves the active tab back or forward in its history. Another page of
    /// the same channel keeps the channel open.
    pub fn go(&mut self, forward: bool) -> Went {
        self.go_to(forward, 1)
    }

    /// Moves the active tab `steps` back or forward in its history, as
    /// Back or Forward pressed that many times (requirement 20.10).
    pub fn go_to(&mut self, forward: bool, steps: usize) -> Went {
        let Some(tab) = self.tabs.get(self.active) else { return Went::Nowhere };
        let position = if forward { tab.position.checked_add(steps) } else { tab.position.checked_sub(steps) };
        let Some(position) = position.filter(|_| steps > 0) else { return Went::Nowhere };
        let Some(visit) = tab.history.get(position).cloned() else { return Went::Nowhere };
        let same = tab.address().is_some_and(|current| current.same_channel(&visit.address));
        let channel = (!same).then(|| self.channel_for(&visit.address));
        let now = self.now;
        let Some(tab) = self.tabs.get_mut(self.active) else { return Went::Nowhere };
        tab.position = position;
        if let Some(visit) = tab.history.get_mut(position) {
            visit.at = now;
        }
        tab.pending = Some(Pending { typed: false, drawn: same, reload: false });
        tab.channel.redirecting = None;
        match channel {
            Some(channel) => {
                tab.channel = channel;
                Went::Channel
            }
            None => Went::Page(visit.address.page),
        }
    }

    /// The active tab's entries one way, the nearest first, at most
    /// [`MAX_TAB_LIST`] (requirement 20.10).
    #[must_use]
    pub fn tab_list(&self, forward: bool) -> Vec<(usize, &Visit)> {
        let Some(tab) = self.active_tab() else { return Vec::new() };
        (1..=MAX_TAB_LIST)
            .map_while(|steps| {
                let position = if forward { tab.position.checked_add(steps) } else { tab.position.checked_sub(steps) }?;
                tab.history.get(position).map(|visit| (steps, visit))
            })
            .collect()
    }

    /// After a full list, at most [`MAX_CHAPTER_STOPS`] entries beyond it
    /// where the host changes, with their steps, as Chrome's
    /// `BackForwardMenuModel::GetChapterStopCount` and `FindChapterStop`.
    #[must_use]
    pub fn chapter_stops(&self, forward: bool) -> Vec<(usize, &Visit)> {
        let Some(tab) = self.active_tab() else { return Vec::new() };
        if self.tab_list(forward).len() < MAX_TAB_LIST {
            return Vec::new();
        }
        let start = if forward { tab.position.checked_add(MAX_TAB_LIST) } else { tab.position.checked_sub(MAX_TAB_LIST) };
        let mut stops = Vec::new();
        let mut at = start;
        while stops.len() < MAX_CHAPTER_STOPS {
            let Some(stop) = at.and_then(|at| tab.next_chapter_stop(at, forward)) else { break };
            if let Some(visit) = tab.history.get(stop) {
                stops.push((stop.abs_diff(tab.position), visit));
            }
            at = Some(stop);
        }
        stops
    }
}
