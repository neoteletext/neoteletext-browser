//! The open page's controls in the trusted core (`page-controls.md`,
//! decision 67): Fastext's keys followed as the page's other links are,
//! and reveal, hold, mix and the sub-page shown, each told to the page's
//! decoder and lapsing when another page is opened, as on a set.
//!
//! Sub-pages are stepped from a place and a count, never a list (decision
//! 112 item 4): from the page's list of sub-pages when its fetcher read the
//! part that holds the next, `NNN/P.sub` (REQ-1888), which lists every
//! sub-page whether held or not, else from the neighbours its decoder says
//! of the one drawn. A sub-page of a page with a list is fetched, with
//! those either side of it, only when it is shown (decision 112 item 3;
//! REQ-1887), and the parts of the list beside it with it.

use std::collections::{BTreeMap, BTreeSet};

use interface_core::canvas::PageKey;
use interface_core::controls::{PageControl, PageControls, Rotation};
use neoteletext::subpages::{Catalogue, TUNED_EITHER_SIDE};
use neoteletext_core::limits::{SUBCODE_ANY, SUBCODE_NONE};
use neoteletext_web::subpages::PartNumber;
use protocol::ToInterface;
use protocol::decoder::{ToDecoder, View};
use protocol::fetcher::{ToFetcher, write_to_fetcher};
use protocol::subpages::{SubPageRange, SubPageStep};

use super::Core;
use super::selection::{SelectionAction, SelectionEvent, SelectionState, selection_transition};
use crate::session::Files;

/// The requests for rows that wait at once: the interface makes one at a
/// time, so more are only ones it has given up on.
const ROWS_WAITING: usize = 8;

/// What the trusted core keeps of the open page's controls.
#[derive(Default)]
pub(crate) struct State {
    /// The page open in the active tab.
    open: Option<PageKey>,
    /// How it is drawn, when the user changed it.
    view: Option<View>,
    /// The sub-pages of the pages on screen from the one drawn, as their
    /// decoders reported them.
    subpages: BTreeMap<PageKey, SubPageStep>,
    /// How far the choice of a sub-page has got (the selection machine,
    /// `selection`): pending for a page about to open, or awaited of a
    /// live channel.
    selection: SelectionState,
    /// The page and sub-page the selection machine's state is of, while
    /// it is not settled.
    choice: Option<(PageKey, u16)>,
    /// The sub-pages of the open page asked of its fetcher since it
    /// opened: each is asked for once.
    fetched: BTreeSet<u16>,
    /// The open page's list was asked for since it opened.
    catalogue_asked: bool,
    /// The requests for rows waiting on a page's list: for parts not yet
    /// held, or for the list itself on a live channel, which sends it
    /// when the page is selected. Each is answered with its own number
    /// when the list can answer it; at most [`ROWS_WAITING`], the oldest
    /// given up first (DEC-6358, fault 4: there was one place, and a
    /// second request lost the first its answer).
    rows_wanted: Vec<(PageKey, SubPageRange, u16)>,
    /// The request for rows passed to each page's decoder, by its number,
    /// until the decoder answers.
    rows_of_decoder: BTreeMap<PageKey, u16>,
    /// The controls the interface was last told of.
    sent: Option<PageControls>,
    /// What was asked of each channel's fetcher, for the tests, whose core
    /// starts no fetcher.
    #[cfg(test)]
    pub(super) told_fetchers: Vec<(u32, ToFetcher)>,
}

impl State {
    /// Whether `key` is the page open.
    #[cfg(feature = "interaction")]
    pub(super) fn is_open(&self, key: PageKey) -> bool {
        self.open == Some(key)
    }

    /// The page open, when it is of `channel`.
    pub(super) fn open_page(&self, channel: u32) -> Option<u16> {
        self.open.filter(|key| key.channel == channel).map(|key| key.page)
    }

    /// The view of `key`, as drawn now.
    pub(super) fn view_of(&self, key: PageKey) -> View {
        self.view.filter(|view| self.open == Some(key) && view.page == key.page).unwrap_or(View { page: key.page, ..View::default() })
    }
}

/// The sub-page after or before the one `view` shows, from what its
/// decoder said of the one drawn: none while another is on its way to the
/// screen, or with only one.
fn step_drawn(step: &SubPageStep, view: &View, forward: bool) -> Option<u16> {
    if view.subpage.is_some_and(|subpage| subpage != step.shown) {
        return None;
    }
    if forward { step.next } else { step.previous }
}

impl State {
    /// Sets this window's controls aside, leaving none: what the tests
    /// record of the fetchers is the core's, and stays.
    pub(super) fn park(&mut self) -> Self {
        let parked = std::mem::take(self);
        #[cfg(test)]
        let parked = {
            let mut parked = parked;
            self.told_fetchers = std::mem::take(&mut parked.told_fetchers);
            parked
        };
        parked
    }

    /// Takes up `parked`, the controls of the window now at hand.
    pub(super) fn restore(&mut self, parked: Self) {
        #[cfg(test)]
        let told = std::mem::take(&mut self.told_fetchers);
        *self = parked;
        #[cfg(test)]
        {
            self.told_fetchers = told;
        }
    }
}

/// The page `tab` shows, the sub-page its address names, and how often
/// they are asked for again, when its channel declared that and is not
/// live (REQ-704): none on the wall or the contents view.
fn refreshing(tab: &crate::session::Tab) -> Option<(u16, Option<u16>, std::time::Duration)> {
    let every = tab.channel.refresh.filter(|_| !tab.channel.live)?;
    let address = tab.address().filter(|address| !address.wall && !address.contents)?;
    let page = address.page.or(tab.channel.initial_page)?;
    Some((page, address.subpage.filter(|subcode| *subcode != SUBCODE_ANY), every))
}

/// Whether a page whose sub-page drawn has `subcode` is one of a set: a
/// page with no sub-pages has 0000 (EN 300 706 Annex A.1, display rule
/// 1), and 3F7F is not a sub-page's.
const fn of_a_set(subcode: u16) -> bool {
    subcode != SUBCODE_NONE && subcode != SUBCODE_ANY
}

impl Core {
    /// The page open in the active tab changed: what the user set on the
    /// page before lapses, and a sub-page asked for is shown.
    pub(crate) fn page_opened(&mut self, key: Option<PageKey>) -> Result<(), String> {
        if self.controls.open == key {
            return Ok(());
        }
        if let (Some(old), Some(view)) = (self.controls.open, self.controls.view.take()) {
            self.tell(old.channel, ToDecoder::View(View { page: view.page, ..View::default() }));
        }
        self.controls.open = key;
        // The page a sub-page was named for opens, or another does.
        let pending = self.controls.choice.filter(|_| self.controls.selection == SelectionState::Pending);
        let opened_it = pending.is_some_and(|(pending, _)| Some(pending) == key);
        let show = self.select_event(if opened_it { SelectionEvent::OpenedIt } else { SelectionEvent::OpenedOther }) == SelectionAction::Show;
        self.controls.fetched.clear();
        self.controls.catalogue_asked = false;
        if let Some(key) = key {
            if let Some(subpages) = self.channel_subpages(key.channel) {
                subpages.again(key.page);
            }
            self.ask_catalogue(key);
            self.ask_page(key.channel, key.page);
        }
        if let Some(channel) = key.map(|key| key.channel).or(self.active_channel()) {
            self.arm_refresh(channel);
        }
        if let Some((pending, subpage)) = pending.filter(|_| show) {
            self.address_follows(pending.page, Some(subpage))?;
            self.show_subpage(pending, View { page: pending.page, subpage: Some(subpage), ..View::default() })?;
        }
        self.send_controls()
    }

    /// The page open in the active tab, as the interface last reported it.
    pub(super) fn page_open(&self) -> Option<PageKey> {
        self.controls.open
    }

    /// The tab's address follows the sub-page chosen on its open page.
    /// `adopt_first` does not come here: a page's first sub-page is the page
    /// (REQ-1690 item 4).
    fn address_follows(&mut self, page: u16, subpage: Option<u16>) -> Result<(), String> {
        if self.session.subpage_chosen(page, subpage) {
            self.send(&ToInterface::Strip(self.strip()))?;
        }
        Ok(())
    }

    /// A page of `channel` will open at `subpage`, when it opens.
    pub(crate) fn open_at_subpage(&mut self, key: PageKey, subpage: Option<u16>) {
        match subpage {
            Some(subpage) => {
                self.select_event(SelectionEvent::ChosenBeforeOpen);
                self.controls.choice = Some((key, subpage));
            }
            None => {
                self.select_event(SelectionEvent::ChoiceWithdrawn);
            }
        }
    }

    /// Gives `event` to the selection machine; returns what to do. A
    /// settled selection is of no page.
    fn select_event(&mut self, event: SelectionEvent) -> SelectionAction {
        let (state, action) = selection_transition(self.controls.selection, event);
        self.controls.selection = state;
        if state == SelectionState::Settled {
            self.controls.choice = None;
        }
        action
    }

    /// The sub-page a page about to open was named with, when `key` is
    /// that page: its channel is told the sub-page at once, in the one
    /// selection (DEC-6358, fault 10).
    #[cfg(feature = "interaction")]
    pub(super) fn pending_subpage(&self, key: PageKey) -> Option<u16> {
        self.controls.choice.filter(|&(pending, _)| self.controls.selection == SelectionState::Pending && pending == key).map(|(_, subpage)| subpage)
    }

    /// A decoder's sub-pages of a page on screen, from the one it draws.
    pub(crate) fn subpages(&mut self, key: PageKey, step: SubPageStep) -> Result<(), String> {
        self.controls.subpages.insert(key, step);
        if self.controls.open != Some(key) {
            return Ok(());
        }
        self.session.page_drawn(key.channel);
        #[cfg(feature = "interaction")]
        self.typing.drawn(key, step.shown);
        let awaited = self.awaiting().is_some_and(|(awaited, subpage)| awaited == key && subpage == step.shown);
        if awaited && self.select_event(SelectionEvent::Drew) == SelectionAction::Say {
            self.send(&ToInterface::Strip(self.strip()))?;
        }
        self.adopt_first(key, step.shown);
        self.ask_catalogue(key);
        self.send_controls()
    }

    /// On a live channel, the first sub-page drawn of a page selected with
    /// 3F7F is the tab's selection from now on, exactly as if chosen: the
    /// view holds it, stepping goes on from it, and nothing rotates
    /// (DEC-1526).
    #[cfg(feature = "interaction")]
    fn adopt_first(&mut self, key: PageKey, shown: u16) {
        let live = self.session.all_tabs().any(|tab| tab.channel.id == key.channel && tab.channel.live);
        if !live || shown == SUBCODE_ANY || self.typing.selection(key.channel) != Some((key.page, SUBCODE_ANY)) {
            return;
        }
        self.typing.adopt(key.channel, key.page, shown);
        let view = View { subpage: Some(shown), ..self.controls.view_of(key) };
        self.controls.view = Some(view);
        self.tell(key.channel, ToDecoder::View(view));
    }

    #[cfg(not(feature = "interaction"))]
    #[expect(clippy::unused_self, reason = "SELECT is input, compiled out")]
    fn adopt_first(&mut self, _key: PageKey, _shown: u16) {}

    /// The sub-page of a live channel's open page selected and not yet
    /// drawn, if any.
    pub(super) fn awaiting(&self) -> Option<(PageKey, u16)> {
        self.controls.choice.filter(|_| self.controls.selection == SelectionState::Awaiting)
    }

    /// Forgets the sub-pages of pages no longer on screen, and of channels
    /// gone.
    pub(crate) fn keep_subpages(&mut self, visible: &[PageKey]) {
        let open = self.controls.open;
        self.controls.subpages.retain(|key, _| visible.contains(key) || Some(*key) == open);
    }

    /// Part 0 of the open page's list of sub-pages, asked of its fetcher
    /// once it opens, when it is one of a set (REQ-1888): it says whether
    /// the page has a folder, and a page with none is never asked again.
    fn ask_catalogue(&mut self, key: PageKey) {
        if self.controls.catalogue_asked || self.controls.open != Some(key) {
            return;
        }
        let drawn = self.controls.subpages.get(&key).copied();
        let known = self.channel_subpages(key.channel).is_some_and(|subpages| subpages.count(key.page) > 0);
        if !known && !drawn.is_some_and(|drawn| drawn.count > 1 || of_a_set(drawn.shown)) {
            return;
        }
        self.controls.catalogue_asked = true;
        // A live channel's lists come on its connection, unasked (REQ-703).
        if self.session.all_tabs().any(|tab| tab.channel.id == key.channel && tab.channel.live) {
            return;
        }
        self.ask_parts(key, &[PartNumber::FIRST.number()]);
    }

    /// Asks `key`'s fetcher for those of `parts` of its page's list not
    /// asked since the page opened, on a channel served as files: a live
    /// channel's lists come on its connection (REQ-703).
    pub(super) fn ask_parts(&mut self, key: PageKey, parts: &[u8]) {
        if self.session.all_tabs().any(|tab| tab.channel.id == key.channel && tab.channel.live) {
            return;
        }
        for &part in parts {
            if self.channel_subpages(key.channel).is_some_and(|subpages| subpages.ask(key.page, part)) {
                self.tell_fetcher(key.channel, &ToFetcher::FetchSubPages { page: key.page, part });
            }
        }
    }

    /// A static channel's bundle was answered, refused or found missing
    /// (REQ-703): what it held has gone to the decoder, and what its tab
    /// wants that the bundle did not hold may be asked for.
    pub(super) fn bundle_settled(&mut self, channel: u32) {
        let Some(tab) = self.session.all_tabs_mut().find(|tab| tab.channel.id == channel) else { return };
        if tab.channel.files == Files::Opening {
            tab.channel.files = Files::BundleSettled;
        }
        self.ask_wanted(channel);
    }

    /// Asks for what `channel`'s tab wants, and nothing else (REQ-703,
    /// DEC-997): the page it shows, which is the page open or else its
    /// address's or the channel's first; in the window at hand, the pages
    /// the interface shows of it; and, while its tab is in front of a
    /// window, its listing, which the page list, the contents view and the
    /// overview show.
    pub(super) fn ask_wanted(&mut self, channel: u32) {
        let Some(tab) = self.session.all_tabs().find(|tab| tab.channel.id == channel) else { return };
        let shown = tab.address().filter(|address| !address.wall && !address.contents).and_then(|address| address.page).or(tab.channel.initial_page);
        if let Some(page) = self.controls.open_page(channel).or(shown) {
            self.ask_page(channel, page);
        }
        if self.showing == Some(channel) {
            let pages = self.shown_pages.clone();
            self.ask_shown(channel, &pages);
        }
        if self.session.front_tabs().any(|tab| tab.channel.id == channel) {
            self.ask_contents(channel);
        }
    }

    /// Asks `channel`'s fetcher for `page`'s own file, when it is a static
    /// channel's whose bundle has settled and the page has neither arrived
    /// nor been asked for since it was last refused (REQ-703). A live
    /// channel's come on its connection (DEC-1467), and before its bundle
    /// settles a page may yet be in it.
    pub(super) fn ask_page(&mut self, channel: u32, page: u16) {
        let Some(tab) = self.session.all_tabs_mut().find(|tab| tab.channel.id == channel) else { return };
        let state = &mut tab.channel;
        if state.live || state.web.is_none() || state.files == Files::Opening || state.is_loaded(page) || !state.asked.insert(page) {
            return;
        }
        self.tell_fetcher(channel, &ToFetcher::FetchPage { page });
    }

    /// The pages the interface shows of `channel`, the active one, asked
    /// for: the wall's thumbnails among them (REQ-703, DEC-997).
    pub(super) fn ask_shown(&mut self, channel: u32, pages: &[u16]) {
        for &page in pages {
            self.ask_page(channel, page);
        }
    }

    /// Asks a static channel's fetcher for its contents file, once, when
    /// its bundle has settled: its listing is wanted (REQ-703).
    fn ask_contents(&mut self, channel: u32) {
        let Some(tab) = self.session.all_tabs_mut().find(|tab| tab.channel.id == channel) else { return };
        let state = &mut tab.channel;
        if state.live || state.web.is_none() || state.files != Files::BundleSettled {
            return;
        }
        state.files = Files::ContentsAsked;
        self.tell_fetcher(channel, &ToFetcher::FetchContents);
    }

    /// `channel`'s page file was refused: it is asked for again when it is
    /// next wanted.
    pub(super) fn page_refused(&mut self, channel: u32, page: u16) {
        if let Some(tab) = self.session.all_tabs_mut().find(|tab| tab.channel.id == channel) {
            tab.channel.asked.remove(&page);
        }
    }

    /// The interval `channel`'s `channel.ttx` declares (REQ-704): the page
    /// its tab shows is asked for again from now on.
    pub(super) fn refresh_declared(&mut self, channel: u32, seconds: u32) {
        if let Some(tab) = self.session.all_tabs_mut().find(|tab| tab.channel.id == channel) {
            tab.channel.refresh = Some(std::time::Duration::from_secs(u64::from(seconds)));
        }
        self.arm_refresh(channel);
    }

    /// Times `channel`'s next asking from now, or none: its tab must be in
    /// front of a window and show a page.
    fn arm_refresh(&mut self, channel: u32) {
        let now = std::time::Instant::now();
        let every = self.session.front_tabs().find(|tab| tab.channel.id == channel).and_then(refreshing).map(|(_, _, every)| every);
        if let Some(tab) = self.session.all_tabs_mut().find(|tab| tab.channel.id == channel) {
            tab.channel.next_refresh = every.and_then(|every| now.checked_add(every));
        }
    }

    /// Every tab in front of a window that refreshes and has no time yet is
    /// timed from now, and every other tab's time is dropped: after a
    /// change of tab or window.
    pub(super) fn arm_refreshes(&mut self) {
        let front: Vec<u32> = self.session.front_tabs().filter(|tab| refreshing(tab).is_some()).map(|tab| tab.channel.id).collect();
        let now = std::time::Instant::now();
        for tab in self.session.all_tabs_mut() {
            let state = &mut tab.channel;
            if !front.contains(&state.id) {
                state.next_refresh = None;
            } else if state.next_refresh.is_none() {
                state.next_refresh = state.refresh.and_then(|every| now.checked_add(every));
            }
        }
    }

    /// When a page on screen, in any window, is next asked for again.
    pub(super) fn refresh_due_at(&self) -> Option<std::time::Instant> {
        self.session.front_tabs().filter(|tab| refreshing(tab).is_some()).filter_map(|tab| tab.channel.next_refresh).min()
    }

    /// Asks again for each page on screen whose time has come, in every
    /// window, or for the sub-page its address names. The agent makes the
    /// request conditional on what it holds, and only these pages are asked
    /// for (REQ-704).
    pub(super) fn refresh_tick(&mut self, now: std::time::Instant) {
        let due: Vec<(u32, u16, Option<u16>)> = self
            .session
            .front_tabs()
            .filter(|tab| tab.channel.next_refresh.is_some_and(|due| due <= now))
            .filter_map(|tab| refreshing(tab).map(|(page, subpage, _)| (tab.channel.id, page, subpage)))
            .collect();
        for (channel, page, subpage) in due {
            self.arm_refresh(channel);
            let message = match subpage {
                Some(subcode) => ToFetcher::FetchSubPage { page, subcode },
                None => ToFetcher::FetchPage { page },
            };
            self.tell_fetcher(channel, &message);
        }
    }

    /// The sub-page state of `channel`'s tab.
    pub(super) fn channel_subpages(&mut self, channel: u32) -> Option<&mut neoteletext::subpages::SubPages> {
        self.session.all_tabs_mut().find(|tab| tab.channel.id == channel).map(|tab| &mut tab.channel.subpages)
    }

    /// What `read` makes of the catalogue of `key`'s page, when its channel
    /// holds one.
    fn with_catalogue<T>(&mut self, key: PageKey, read: impl FnOnce(&Catalogue) -> T) -> Option<T> {
        self.channel_subpages(key.channel)?.catalogue(key.page).map(read)
    }

    /// Writes `message` to `channel`'s fetcher, when it has one.
    pub(super) fn tell_fetcher(&mut self, channel: u32, message: &ToFetcher) {
        #[cfg(test)]
        self.controls.told_fetchers.push((channel, message.clone()));
        if let Some(input) = self.fetchers.get_mut(&channel).and_then(super::processes::Fetcher::input) {
            // A fetcher that cannot be written to has stopped; its reader
            // reports that.
            let _ = write_to_fetcher(input, message);
            let _ = std::io::Write::flush(input);
        }
    }

    /// Shows `view` on `key`, the open page, and fetches the sub-page it
    /// names, with those either side of it, when the page has a catalogue
    /// and they were not asked for since it opened.
    fn show_subpage(&mut self, key: PageKey, view: View) -> Result<(), String> {
        self.controls.view = Some(view);
        self.tell(key.channel, ToDecoder::View(view));
        let Some(subpage) = view.subpage else { return Ok(()) };
        // On a live channel a step or a choice is a sub-page selection,
        // which brings that sub-page alone (neoteletext-common's REQ-1502,
        // REQ-1505); a channel served as files is asked for its files. A
        // live sub-page not yet drawn is said to be loading until it is.
        self.select_view(key, subpage)?;
        let live = self.session.all_tabs().any(|tab| tab.channel.id == key.channel && tab.channel.live);
        let drawn = self.controls.subpages.get(&key).is_some_and(|step| step.shown == subpage);
        self.select_event(if live && !drawn { SelectionEvent::SelectedUndrawn } else { SelectionEvent::SelectedDrawn });
        if live && !drawn {
            self.controls.choice = Some((key, subpage));
        }
        if live {
            return Ok(());
        }
        // The parts beside it too, so that a step past the sub-pages fetched
        // with it finds its row (DOC-1869).
        let Some((around, parts)) = self.with_catalogue(key, |catalogue| {
            (catalogue.around(subpage, TUNED_EITHER_SIDE), catalogue.parts_around(subpage, TUNED_EITHER_SIDE.saturating_add(1)))
        }) else {
            return Ok(());
        };
        for subcode in around {
            if subcode != SUBCODE_ANY && self.controls.fetched.insert(subcode) {
                self.tell_fetcher(key.channel, &ToFetcher::FetchSubPage { page: key.page, subcode });
            }
        }
        self.ask_parts(key, &parts);
        Ok(())
    }

    /// Tells `key`'s channel that its tab selects `sub` of the page, a
    /// sub-code or 3F7F for the page (SELECT, REQ-INT-140), with the page
    /// parameters of an address that names that selection (REQ-1693 item 6).
    #[cfg(feature = "interaction")]
    fn select_view(&mut self, key: PageKey, sub: u16) -> Result<(), String> {
        let tab = self.session.all_tabs().find(|tab| tab.channel.id == key.channel);
        let parameters = tab.and_then(|tab| tab.address()).map(|address| address.parameters_of(key.page, sub).to_owned()).unwrap_or_default();
        let sends = self.typing.select(key.channel, key.page, sub, &parameters, self.now_us());
        self.deliver_typing(sends)
    }

    #[cfg(not(feature = "interaction"))]
    #[expect(clippy::unnecessary_wraps, clippy::unused_self, reason = "SELECT is input, compiled out")]
    fn select_view(&mut self, _key: PageKey, _sub: u16) -> Result<(), String> {
        Ok(())
    }

    /// `key` opens in the tab in front, at `subpage` if one is named: as any
    /// page the core opens, told to its channel at once, or, when it is open
    /// already, only its sub-page changes.
    pub(super) fn open_page(&mut self, key: PageKey, subpage: Option<u16>) -> Result<(), String> {
        if self.controls.open != Some(key) {
            self.open_at_subpage(key, subpage);
            self.select_view(key, subpage.unwrap_or(SUBCODE_ANY))?;
            return self.send(&ToInterface::Show(key));
        }
        let view = View { subpage, ..self.controls.view_of(key) };
        if subpage.is_some() {
            self.show_subpage(key, view)?;
        } else {
            self.controls.view = Some(view);
            self.tell(key.channel, ToDecoder::View(view));
            self.select_view(key, SUBCODE_ANY)?;
        }
        self.send_controls()
    }

    /// A link led to `key`: it opens, and its address follows a sub-page.
    #[cfg(feature = "links")]
    pub(super) fn open_followed(&mut self, key: PageKey, subpage: Option<u16>) -> Result<(), String> {
        self.address_follows(key.page, subpage)?;
        self.open_page(key, subpage)
    }

    /// A program redirected `key`'s tab to `key` and `sub`: the server took
    /// it as the tab's selection (neoteletext-app-server's REQ-1472 item 4),
    /// so the tab's is the same, with nothing sent but what was held let go.
    #[cfg(feature = "interaction")]
    pub(super) fn select_redirected(&mut self, key: PageKey, sub: u16) -> Result<(), String> {
        let tab = self.session.all_tabs().find(|tab| tab.channel.id == key.channel);
        let parameters = tab.and_then(|tab| tab.address()).map(|address| address.parameters_of(key.page, sub).to_owned()).unwrap_or_default();
        let sends = self.typing.redirected(key.channel, key.page, sub, &parameters, self.now_us());
        self.deliver_typing(sends)
    }

    #[cfg(not(feature = "interaction"))]
    #[expect(clippy::unnecessary_wraps, clippy::unused_self, reason = "SELECT is input, compiled out")]
    pub(super) fn select_redirected(&mut self, _key: PageKey, _sub: u16) -> Result<(), String> {
        Ok(())
    }

    /// Says again what `channel`'s tab selects.
    #[cfg(feature = "interaction")]
    pub(super) fn select_again(&mut self, channel: u32) -> Result<(), String> {
        let sends = self.typing.select_again(channel, self.now_us());
        self.deliver_typing(sends)
    }

    #[cfg(not(feature = "interaction"))]
    #[expect(clippy::unnecessary_wraps, clippy::unused_self, reason = "SELECT is input, compiled out")]
    pub(super) fn select_again(&mut self, _channel: u32) -> Result<(), String> {
        Ok(())
    }

    /// A part of a page's list arrived or changed: rows waiting for it are
    /// answered, and the open page's controls and the sub-page it shows
    /// follow it.
    pub(super) fn catalogue_arrived(&mut self, key: PageKey) -> Result<(), String> {
        self.answer_rows_wanted(key, false)?;
        if self.controls.open != Some(key) {
            return Ok(());
        }
        let view = self.controls.view_of(key);
        if view.subpage.is_some() {
            self.show_subpage(key, view)?;
        } else if let Some(drawn) = self.controls.subpages.get(&key).map(|drawn| drawn.shown) {
            // The page as it opened: the parts a step from it takes.
            let parts = self.with_catalogue(key, |catalogue| catalogue.parts_around(drawn, TUNED_EITHER_SIDE.saturating_add(1))).unwrap_or_default();
            self.ask_parts(key, &parts);
        }
        self.send_controls()
    }

    /// The sub-page list asks for rows of `page` of the active channel:
    /// answered from its list when any of it is held, once the parts the
    /// rows fall in have come, which are asked for (REQ-1888: the list
    /// scrolled asks for the parts its rows fall in); else by its decoder
    /// from what it holds. A page's list is first asked for only when it
    /// opens (requirement 4.7 as amended for decision 121).
    ///
    /// The request names its channel and carries a number (DEC-6358 item
    /// 3): one for a channel that is not the active one is not answered,
    /// and every answer carries the number back. On a live channel a page
    /// whose list is not held is not answered from its decoder, whose
    /// places are of what it happens to hold and not the list's: the
    /// request waits for the list, which the channel sends when the page
    /// is selected.
    pub(super) fn subpage_rows(&mut self, channel: u32, page: u16, range: SubPageRange, request: u16) -> Result<(), String> {
        if self.active_channel() != Some(channel) {
            return Ok(());
        }
        let key = PageKey { channel, page };
        let live = self.session.all_tabs().any(|tab| tab.channel.id == channel && tab.channel.live);
        let wanted = self.with_catalogue(key, |catalogue| catalogue.parts_wanted(range));
        // A list on its way is waited for, whatever the channel (the list
        // machine's `Fetching`); only a page with no list to come is
        // answered by its decoder.
        let fetching =
            self.channel_subpages(channel).is_some_and(|subpages| subpages.list_state(page) == neoteletext::subpages::list_state::ListState::Fetching);
        if wanted.is_none() && !live && !fetching {
            self.controls.rows_of_decoder.insert(key, request);
            self.tell(channel, ToDecoder::SubPageRows { page, range });
            return Ok(());
        }
        self.controls.rows_wanted.retain(|&(waiting, _, number)| (waiting, number) != (key, request));
        self.controls.rows_wanted.push((key, range, request));
        if self.controls.rows_wanted.len() > ROWS_WAITING {
            self.controls.rows_wanted.remove(0);
        }
        self.ask_parts(key, &wanted.unwrap_or_default());
        self.answer_rows_wanted(key, false)
    }

    /// Answers each request waiting on `key`'s list once the parts its rows
    /// fall in are held, or at once with what is held when `now`: a part
    /// that could not be fetched is not waited for.
    pub(super) fn answer_rows_wanted(&mut self, key: PageKey, now: bool) -> Result<(), String> {
        let waiting: Vec<(SubPageRange, u16)> =
            self.controls.rows_wanted.iter().filter(|(wanted, _, _)| *wanted == key).map(|&(_, range, request)| (range, request)).collect();
        for (range, request) in waiting {
            let Some((parts, rows)) = self.with_catalogue(key, |catalogue| (catalogue.parts_wanted(range), catalogue.rows(range))) else { return Ok(()) };
            if !parts.is_empty() && !now {
                continue;
            }
            self.controls.rows_wanted.retain(|&(waiting, _, number)| (waiting, number) != (key, request));
            if self.active_channel() == Some(key.channel) {
                self.send(&ToInterface::SubPageRows { channel: key.channel, page: key.page, request, rows })?;
            }
        }
        Ok(())
    }

    /// A decoder answered a request for rows of `key` passed to it: sent
    /// on with the request's number, unless the page's list has come
    /// meanwhile, when the list answers it instead, so that no request is
    /// left without an answer (DEC-6358, fault 4).
    pub(super) fn rows_from_decoder(&mut self, key: PageKey, mut rows: protocol::subpages::SubPageRows) -> Result<(), String> {
        let Some(request) = self.controls.rows_of_decoder.remove(&key) else { return Ok(()) };
        if self.with_catalogue(key, |_| ()).is_some() {
            let range = SubPageRange { first: rows.first, count: u16::try_from(interface_core::controls::SUBPAGE_ROWS).unwrap_or(u16::MAX) };
            self.controls.rows_wanted.push((key, range, request));
            return self.answer_rows_wanted(key, true);
        }
        if self.active_channel() != Some(key.channel) {
            return Ok(());
        }
        for (_, title) in &mut rows.rows {
            *title = protocol::clean(title, protocol::MAX_PAGE_TITLE);
        }
        self.send(&ToInterface::SubPageRows { channel: key.channel, page: key.page, request, rows })
    }

    /// Strip input: a key of the open page's controls.
    pub(crate) fn page_control(&mut self, control: PageControl) -> Result<(), String> {
        // A sub-page chosen from the page list: shown at once on the page
        // open, or when its page opens.
        if let PageControl::SubPageOf { channel, page, subcode } = control {
            // A choice made of another channel than the one in front, as in
            // the moment a tab changes, is not this channel's (DEC-6358 item 3).
            if self.active_channel() != Some(channel) {
                return Ok(());
            }
            let target = PageKey { channel, page };
            if self.controls.open != Some(target) {
                self.open_at_subpage(target, Some(subcode));
                return Ok(());
            }
            self.address_follows(page, Some(subcode))?;
            let view = View { subpage: Some(subcode), ..self.controls.view_of(target) };
            // Another sub-page chosen: its bytes are counted from nothing (REQ-6352).
            if self.controls.view_of(target).subpage != Some(subcode) {
                self.console.downloaded_restart(channel);
            }
            self.show_subpage(target, view)?;
            return self.send_controls();
        }
        let Some(key) = self.controls.open else { return Ok(()) };
        let mut view = self.controls.view_of(key);
        match control {
            #[cfg(feature = "links")]
            PageControl::Fastext(fastext) => {
                let followed = self.links.fastext(fastext, self.active_channel(), self.now());
                if let Ok((neoteletext::links::Follow::Page(page), Some(subpage))) = &followed {
                    self.open_at_subpage(PageKey { channel: key.channel, page: *page }, Some(*subpage));
                }
                return self.follow(followed.map(|(follow, _)| follow), "a Fastext key");
            }
            #[cfg(not(feature = "links"))]
            PageControl::Fastext(_) => return Ok(()),
            PageControl::Reveal => view.reveal = !view.reveal,
            PageControl::Hold => view.hold = !view.hold,
            PageControl::Mix => view.mix = !view.mix,
            PageControl::SubPage(forward) => {
                let drawn = self.controls.subpages.get(&key).copied();
                let current = view.subpage.or(drawn.map(|drawn| drawn.shown)).unwrap_or(SUBCODE_ANY);
                // The list's next row when its part is held, else the
                // decoder's.
                let next = self
                    .with_catalogue(key, |catalogue| catalogue.step(current, forward))
                    .flatten()
                    .or_else(|| drawn.and_then(|drawn| step_drawn(&drawn, &view, forward)));
                // 3F7F names no sub-page (clause 9.6.1).
                let Some(next) = next.filter(|&next| next != SUBCODE_ANY) else { return Ok(()) };
                view.subpage = Some(next);
                self.console.downloaded_restart(key.channel);
                self.address_follows(key.page, Some(next))?;
                self.show_subpage(key, view)?;
                return self.send_controls();
            }
            PageControl::Latest => {
                // A live channel's sub-pages never rotate, so there is no
                // latest to return to (DEC-1542).
                if self.session.all_tabs().any(|tab| tab.channel.id == key.channel && tab.channel.live) {
                    return Ok(());
                }
                // The return to the latest is a page selection (REQ-1502).
                view.subpage = None;
                self.address_follows(key.page, None)?;
                self.select_view(key, SUBCODE_ANY)?;
            }
            PageControl::SubPageOf { .. } => return Ok(()),
        }
        self.controls.view = Some(view);
        self.tell(key.channel, ToDecoder::View(view));
        self.send_controls()
    }

    /// What the open page shows of `channel`, when the open page is its.
    pub(super) fn selection_shown(&self, channel: u32) -> Option<neoteletext::console::Selection> {
        let key = self.controls.open.filter(|key| key.channel == channel)?;
        Some(neoteletext::console::Selection { page: key.page, sub: self.controls.view_of(key).subpage })
    }

    /// Tells the interface of the open page's controls, when they changed.
    pub(crate) fn send_controls(&mut self) -> Result<(), String> {
        let controls = match self.controls.open {
            None => PageControls::default(),
            Some(key) => {
                let view = self.controls.view_of(key);
                let drawn = self.controls.subpages.get(&key).copied().unwrap_or_default();
                // The catalogue's count and places when there is one: every
                // sub-page, held or not (the engine's REQ-NUM-030).
                let listed = self.with_catalogue(key, |catalogue| (catalogue.count(), view.subpage.and_then(|subpage| catalogue.place_of(subpage))));
                let (subpages, chosen) = listed.unwrap_or((drawn.count, view.subpage.filter(|&subpage| subpage == drawn.shown).and(drawn.place)));
                #[cfg(feature = "links")]
                let fastext = self.links.fastext_keys(key);
                #[cfg(not(feature = "links"))]
                let fastext = [false; 5];
                PageControls {
                    channel: key.channel,
                    page: Some(key.page),
                    fastext,
                    reveal: view.reveal,
                    hold: view.hold,
                    mix: view.mix,
                    subpages,
                    chosen: chosen.filter(|&chosen| chosen < subpages),
                    subcode: drawn.shown,
                    downloaded: self.selection_shown(key.channel).map_or(0, |selection| self.console.downloaded_for(key.channel, selection)),
                    rotation: if self.session.all_tabs().any(|tab| tab.channel.id == key.channel && tab.channel.live) {
                        Rotation::Never
                    } else {
                        Rotation::AsBroadcast
                    },
                }
            }
        };
        if self.controls.sent == Some(controls) {
            return Ok(());
        }
        self.controls.sent = Some(controls);
        self.send(&ToInterface::PageControls(controls))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    /// Decision 67 and decision 112 item 4: the sub-page keys step by the
    /// neighbours the decoder says of the one drawn, round the list; with
    /// one sub-page, or another on its way to the screen, nowhere.
    #[test]
    fn sub_pages_step_round() {
        let step = SubPageStep { count: 3, shown: 2, place: Some(1), previous: Some(1), next: Some(3) };
        let view = View { page: 0x100, ..View::default() };
        assert_eq!(step_drawn(&step, &view, true), Some(3));
        assert_eq!(step_drawn(&step, &view, false), Some(1));
        assert_eq!(step_drawn(&step, &View { subpage: Some(2), ..view }, true), Some(3), "the one chosen, drawn");
        assert_eq!(step_drawn(&step, &View { subpage: Some(3), ..view }, true), None, "another on its way");
        assert_eq!(step_drawn(&SubPageStep { count: 1, shown: 4, ..SubPageStep::default() }, &view, true), None);
        assert!(!of_a_set(SUBCODE_NONE) && !of_a_set(SUBCODE_ANY) && of_a_set(0x0001));
    }

    #[test]
    fn a_view_belongs_to_its_open_page() {
        let key = PageKey { channel: 1, page: 0x302 };
        let state = State { open: Some(key), view: Some(View { page: 0x302, reveal: true, ..View::default() }), ..State::default() };
        assert!(state.view_of(key).reveal);
        assert!(!state.view_of(PageKey { channel: 1, page: 0x100 }).reveal, "another page's view is its own");
    }

    /// Finding F-66: a page opened must be of the active tab's channel; a
    /// key naming another tab's channel opens nothing, so the page controls
    /// cannot reach that tab's decoder.
    #[test]
    fn only_a_page_of_the_active_channel_opens() {
        let mut test = super::super::test_support::Test::new("opened-key");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let other = test.core.session.tabs[0].channel.id;
        test.core.session.open("https://sport.invalid/teletext/").unwrap();
        let active = test.core.active_channel().unwrap();
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel: other, page: 0x100 }))).unwrap();
        assert_eq!(test.core.controls.open, None, "another tab's page");
        let key = PageKey { channel: active, page: 0x100 };
        test.core.intent(protocol::ToCore::Opened(Some(key))).unwrap();
        assert_eq!(test.core.controls.open, Some(key));
    }

    /// The parts of the list of `subcodes` as page 210's fetcher sends them.
    fn list_part(subcodes: &[u16], version: &str, part: usize) -> protocol::fetcher::FromFetcher {
        use protocol::fetcher::{FromFetcher, ROWS_PER_PART};
        use protocol::subpages::SubPageRows;
        let count = u16::try_from(subcodes.len()).unwrap();
        let rows = subcodes.chunks(ROWS_PER_PART).nth(part).unwrap_or_default().iter().map(|&subcode| (subcode, format!("Story {subcode:04X}"))).collect();
        FromFetcher::SubPages { page: 0x210, version: version.into(), part: SubPageRows { count, first: u16::try_from(part * ROWS_PER_PART).unwrap(), rows } }
    }

    /// What the core asked of page 210's fetcher since last asked.
    fn told(test: &mut super::super::test_support::Test) -> Vec<ToFetcher> {
        core::mem::take(&mut test.core.controls.told_fetchers).into_iter().map(|(_, told)| told).collect()
    }

    /// Decision 119, decision 112 item 3, REQ-1887 and REQ-1888: a page that
    /// is one of a set has part 0 of its list asked for when it opens; the
    /// list's 8,190 sub-pages are counted from it, the part a step from the
    /// page shown takes asked for with it, the list's rows answered once the
    /// parts they fall in have come, and only the sub-page shown and those
    /// either side of it asked of the fetcher, each once.
    #[test]
    fn a_list_counts_every_sub_page_and_only_the_parts_and_sub_pages_wanted_are_fetched() {
        use neoteletext_core::limits::{MAX_SUBPAGES, from_rank};
        use protocol::decoder::FromDecoder;
        let most = u16::try_from(MAX_SUBPAGES).unwrap();
        // Every sub-code but 0000 and 3F7F, in ascending order.
        let subcodes: Vec<u16> = (1..=most).map(from_rank).collect();
        let mut test = super::super::test_support::Test::new("catalogue");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let channel = test.core.active_channel().unwrap();
        let key = PageKey { channel, page: 0x210 };
        test.core.intent(protocol::ToCore::Opened(Some(key))).unwrap();
        assert!(test.core.controls.told_fetchers.is_empty(), "not asked before the page is known to be one of a set");
        let first = SubPageStep { count: 1, shown: subcodes[0], place: Some(0), previous: None, next: None };
        test.core.decoded(channel, FromDecoder::SubPages { page: 0x210, step: first }).unwrap();
        let part = |part| ToFetcher::FetchSubPages { page: 0x210, part };
        assert_eq!(told(&mut test), [part(0)]);
        test.core.decoded(channel, FromDecoder::SubPages { page: 0x210, step: first }).unwrap();
        assert!(test.core.controls.told_fetchers.is_empty(), "asked once while it is open");
        test.core.fetched(channel, list_part(&subcodes, "v1", 0)).unwrap();
        let sent = test.sent();
        assert!(sent.contains(&ToInterface::SubPageCount { channel, page: 0x210, count: most }), "part 0 alone sizes the list");
        let controls = sent.iter().rev().find_map(|message| if let ToInterface::PageControls(controls) = message { Some(*controls) } else { None });
        assert_eq!(controls.map(|controls| controls.subpages), Some(most), "every sub-page, held or not");
        assert_eq!(told(&mut test), [part(127)], "the last part, a step back from the first");
        test.core.fetched(channel, list_part(&subcodes, "v1", 127)).unwrap();
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPage(false))).unwrap();
        let last = subcodes[usize::from(most) - 1];
        assert_eq!(test.core.controls.view.and_then(|view| view.subpage), Some(last), "round from the first to the last");
        let fetch = |subcode| ToFetcher::FetchSubPage { page: 0x210, subcode };
        assert_eq!(told(&mut test), [fetch(last), fetch(subcodes[0]), fetch(subcodes[usize::from(most) - 2])], "the one shown and those either side");
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPage(true))).unwrap();
        assert_eq!(told(&mut test), [fetch(subcodes[1])], "each asked for once: only the new neighbour");
        let sent = test.sent();
        let controls = sent.iter().rev().find_map(|message| if let ToInterface::PageControls(controls) = message { Some(*controls) } else { None });
        assert_eq!(controls.and_then(|controls| controls.chosen), Some(0), "back at the first, by its place in the list");
        let range = SubPageRange { first: most - 64, count: 64 };
        test.core.intent(protocol::ToCore::SubPageRows { channel: test.core.active_channel().unwrap(), page: 0x210, range, request: 1 }).unwrap();
        assert_eq!(told(&mut test), [part(126)], "the part of the rows not held");
        let rows = |test: &mut super::super::test_support::Test| {
            test.sent().into_iter().find_map(|message| if let ToInterface::SubPageRows { rows, .. } = message { Some(rows) } else { None })
        };
        assert_eq!(rows(&mut test), None, "the rows wait for their part");
        test.core.fetched(channel, list_part(&subcodes, "v1", 126)).unwrap();
        let rows = rows(&mut test).unwrap();
        assert_eq!((rows.count, rows.first, rows.rows.len()), (most, most - 64, 64));
        assert_eq!(rows.rows[63], (last, format!("Story {last:04X}")), "answered from the list");
    }

    /// DOC-1869: a part of another version replaces the parts held, which
    /// are asked for again; a part that failed answers the rows waiting
    /// with what is held, and is not asked again while its page is open.
    #[test]
    fn a_new_version_of_a_list_is_fetched_again() {
        use neoteletext_core::limits::from_rank;
        use protocol::decoder::FromDecoder;
        use protocol::fetcher::FromFetcher;
        let subcodes: Vec<u16> = (1..=300).map(from_rank).collect();
        let mut test = super::super::test_support::Test::new("versions");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let channel = test.core.active_channel().unwrap();
        let key = PageKey { channel, page: 0x210 };
        test.core.intent(protocol::ToCore::Opened(Some(key))).unwrap();
        let first = SubPageStep { count: 1, shown: subcodes[0], place: Some(0), previous: None, next: None };
        test.core.decoded(channel, FromDecoder::SubPages { page: 0x210, step: first }).unwrap();
        test.core.fetched(channel, list_part(&subcodes, "v1", 0)).unwrap();
        test.core.fetched(channel, list_part(&subcodes, "v1", 4)).unwrap();
        told(&mut test);
        test.core
            .intent(protocol::ToCore::SubPageRows {
                channel: test.core.active_channel().unwrap(),
                page: 0x210,
                range: SubPageRange { first: 64, count: 64 },
                request: 1,
            })
            .unwrap();
        assert_eq!(told(&mut test), [ToFetcher::FetchSubPages { page: 0x210, part: 1 }]);
        test.core.fetched(channel, list_part(&subcodes, "v2", 1)).unwrap();
        let again: Vec<ToFetcher> = told(&mut test);
        assert_eq!(again, [0, 4].map(|part| ToFetcher::FetchSubPages { page: 0x210, part }), "the old version's parts, again");
        let rows = test.sent().into_iter().find_map(|message| if let ToInterface::SubPageRows { rows, .. } = message { Some(rows) } else { None });
        assert_eq!(rows.map(|rows| rows.rows.len()), Some(64), "the new part answers the rows");
        test.core
            .intent(protocol::ToCore::SubPageRows {
                channel: test.core.active_channel().unwrap(),
                page: 0x210,
                range: SubPageRange { first: 128, count: 64 },
                request: 1,
            })
            .unwrap();
        assert_eq!(told(&mut test), [ToFetcher::FetchSubPages { page: 0x210, part: 2 }]);
        test.core.fetched(channel, FromFetcher::SubPagesFailed { page: 0x210, part: 2 }).unwrap();
        let rows = test.sent().into_iter().find_map(|message| if let ToInterface::SubPageRows { rows, .. } = message { Some(rows) } else { None });
        assert_eq!(rows.map(|rows| (rows.first, rows.rows.len())), Some((128, 0)), "answered with what is held");
        test.core
            .intent(protocol::ToCore::SubPageRows {
                channel: test.core.active_channel().unwrap(),
                page: 0x210,
                range: SubPageRange { first: 128, count: 64 },
                request: 1,
            })
            .unwrap();
        assert!(told(&mut test).is_empty(), "not asked again while the page is open");
    }

    /// A tab on a live channel with page 210 open, its list of `subcodes`
    /// from a SUBPAGES sending.
    #[cfg(feature = "interaction")]
    fn live_page(name: &str, subcodes: &[u16]) -> (super::super::test_support::Test, PageKey) {
        use protocol::fetcher::FromFetcher;
        let mut test = super::super::test_support::Test::new(name);
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let channel = test.core.active_channel().unwrap();
        test.core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
        test.core.fetched(channel, FromFetcher::Connecting(false)).unwrap();
        let key = PageKey { channel, page: 0x210 };
        test.core.fetched(channel, list_part(subcodes, "1", 0)).unwrap();
        test.core.intent(protocol::ToCore::Opened(Some(key))).unwrap();
        (test, key)
    }

    #[cfg(feature = "interaction")]
    fn drawn(test: &mut super::super::test_support::Test, key: PageKey, shown: u16, count: u16, place: u16) {
        use protocol::decoder::FromDecoder;
        let step = SubPageStep { count, shown, place: Some(place), previous: None, next: None };
        test.core.decoded(key.channel, FromDecoder::SubPages { page: key.page, step }).unwrap();
    }

    // REQ-1505, DEC-1526: the list, its count and the place come from the
    // SUBPAGES record before anything beyond the first sub-page is held;
    // the first sub-page drawn of a page selected with 3F7F is the
    // selection, with nothing fetched, and another arriving changes nothing.
    #[cfg(feature = "interaction")]
    #[test]
    fn a_live_page_lists_from_its_record_and_adopts_its_first_sub_page() {
        let (mut test, key) = live_page("live list", &[0, 1, 2]);
        assert_eq!(test.core.typing.selection(key.channel), Some((0x210, SUBCODE_ANY)));
        drawn(&mut test, key, 0, 1, 0);
        assert_eq!(test.core.typing.selection(key.channel), Some((0x210, 0)), "the answer is the selection");
        assert_eq!(test.core.controls.view.and_then(|view| view.subpage), Some(0), "the view holds it");
        let sent = test.sent();
        let controls = sent.iter().rev().find_map(|message| if let ToInterface::PageControls(controls) = message { Some(*controls) } else { None });
        assert_eq!(controls.map(|controls| controls.subpages), Some(3), "every sub-page of the list, with one held");
        drawn(&mut test, key, 2, 2, 2);
        assert_eq!(test.core.typing.selection(key.channel), Some((0x210, 0)), "nothing rotates");
        assert!(test.core.controls.told_fetchers.is_empty(), "REQ-703: a live channel is asked for no file");
    }

    // DOC-1961: what the channel refuses is said on the page
    // area, by its status, while the tab still selects it. On the code
    // before, the fetcher made it a line for the console, and the strip had
    // nothing to say.
    #[cfg(feature = "interaction")]
    #[test]
    fn a_refused_selection_shows_by_its_status_until_the_next_selection() {
        use interface_core::browser::Denied;
        use protocol::fetcher::FromFetcher;
        let refused = |test: &mut super::super::test_support::Test, channel: u32, status: u16, place: &str| {
            test.core.fetched(channel, FromFetcher::Denied { status, place: place.into() }).unwrap();
            test.core.strip().denied
        };
        let (mut test, key) = live_page("denied", &[1, 2, 3]);
        for (status, denied) in [(403, Denied::Forbidden), (404, Denied::NotFound), (429, Denied::TooMany), (503, Denied::Unavailable)] {
            assert_eq!(refused(&mut test, key.channel, status, "210"), Some(denied));
        }
        assert_eq!(
            test.sent().iter().rev().find_map(|message| if let ToInterface::Strip(strip) = message { strip.denied } else { None }),
            Some(Denied::Unavailable),
            "told"
        );
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPage(true))).unwrap();
        assert_eq!(test.core.strip().denied, None, "the next selection takes it away");

        let (mut test, key) = live_page("denied elsewhere", &[1, 2, 3]);
        assert_eq!(refused(&mut test, key.channel, 404, "300"), None, "a page the tab does not have open");
        assert_eq!(refused(&mut test, key.channel, 500, "210"), None, "a status no DENIED record carries");
        assert_eq!(refused(&mut test, key.channel, 503, ""), Some(Denied::Unavailable), "the whole channel");
        drawn(&mut test, key, 1, 1, 0);
        test.core.session.page_drawn(key.channel);
        assert_eq!(test.core.strip().denied, None, "a page that drew takes it away");
    }

    // REQ-1505 item 4: a step is one selection, loading until it arrives.
    // REQ-703: and no file is asked for.
    #[cfg(feature = "interaction")]
    #[test]
    fn a_live_step_selects_the_next_sub_page_and_loads_until_it_arrives() {
        let (mut test, key) = live_page("live step", &[1, 2, 3]);
        drawn(&mut test, key, 1, 1, 0);
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPage(true))).unwrap();
        assert_eq!(test.core.typing.selection(key.channel), Some((0x210, 2)), "the next after the one adopted");
        assert_eq!(test.core.awaiting(), Some((key, 2)), "loading");
        assert_eq!(test.core.strip().notice, "Loading page 210, sub-page 0002");
        assert!(test.core.controls.told_fetchers.is_empty(), "no file");
        drawn(&mut test, key, 2, 2, 1);
        assert_eq!(test.core.awaiting(), None, "it arrived");
        assert_eq!(test.core.strip().notice, "", "and the strip stops saying so");
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPage(true))).unwrap();
        assert_eq!(test.core.typing.selection(key.channel), Some((0x210, 3)));
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPage(true))).unwrap();
        assert_eq!(test.core.typing.selection(key.channel), Some((0x210, 1)), "round the list, before the last arrived");
    }

    // REQ-1505, REQ-1502, TODO-1531 item 2: a sub-page chosen from the list
    // on a live channel is exactly one SELECT of its sub-code, on the tab's own
    // connection, the tab saying it is loading until it arrives; the first
    // sub-page adopted sends nothing, and choosing the selection again sends
    // nothing either.
    #[cfg(all(unix, feature = "interaction"))]
    #[test]
    fn a_live_choice_sends_one_select_of_its_sub_code() {
        use super::super::test_support::{selects_sent, wire};
        let (mut test, key) = live_page("live choice", &[1, 2, 3]);
        let written = wire(&mut test.core, key.channel);
        drawn(&mut test, key, 1, 1, 0);
        assert_eq!(selects_sent(&mut test.core, key.channel, &written), [], "the answer is adopted with nothing sent");
        let choose = |test: &mut super::super::test_support::Test, subcode| {
            test.core
                .intent(protocol::ToCore::PageControl(PageControl::SubPageOf { channel: test.core.active_channel().unwrap(), page: 0x210, subcode }))
                .unwrap();
        };
        choose(&mut test, 3);
        assert_eq!(selects_sent(&mut test.core, key.channel, &written), [(0x210, 3)]);
        assert_eq!(test.core.awaiting(), Some((key, 3)), "loading");
        choose(&mut test, 3);
        assert_eq!(selects_sent(&mut test.core, key.channel, &written), [], "the selection already");
    }

    /// DEC-6358, fault 10: a sub-page chosen on a page that is not open
    /// opens the page at it with one selection, of that sub-page, where
    /// the page was first selected with no sub-page and then the sub-page.
    #[cfg(feature = "interaction")]
    #[test]
    fn a_sub_page_chosen_on_a_page_not_open_is_one_selection() {
        use super::super::test_support::{selects_sent, wire};
        let (mut test, key) = live_page("one selection", &[1, 2, 3]);
        let written = wire(&mut test.core, key.channel);
        drawn(&mut test, key, 1, 1, 0);
        let _ = selects_sent(&mut test.core, key.channel, &written);
        let other = PageKey { channel: key.channel, page: 0x211 };
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPageOf { channel: key.channel, page: other.page, subcode: 2 })).unwrap();
        assert_eq!(test.core.pending_subpage(other), Some(2), "named for the page before it opens");
        test.core.intent(protocol::ToCore::Opened(Some(other))).unwrap();
        assert_eq!(selects_sent(&mut test.core, key.channel, &written), [(0x211, 2)], "one selection, of the sub-page");
        assert_eq!(test.core.awaiting(), Some((other, 2)), "and it is awaited");
        assert_eq!(test.core.pending_subpage(other), None, "no longer pending");
    }

    // REQ-1693, REQ-1502, TODO-1531 item 2: a link naming a sub-page (an
    // address's `302/0002`), to the page open or to another, sends SELECT with
    // that sub-code, once, on the tab's own connection.
    #[cfg(all(unix, feature = "interaction", feature = "links"))]
    #[test]
    fn a_link_naming_a_sub_page_selects_it() {
        use super::super::test_support::{selects_sent, wire};
        let (mut test, key) = live_page("link sub-page", &[1, 2, 3]);
        let written = wire(&mut test.core, key.channel);
        drawn(&mut test, key, 1, 1, 0);
        let other = PageKey { channel: key.channel, page: 0x302 };
        test.core.open_followed(other, Some(2)).unwrap();
        assert_eq!(selects_sent(&mut test.core, key.channel, &written), [(0x302, 2)]);
        test.core.intent(protocol::ToCore::Opened(Some(other))).unwrap();
        assert_eq!(selects_sent(&mut test.core, key.channel, &written), [], "the page opening says nothing more");
        assert_eq!(test.core.typing.selection(key.channel), Some((0x302, 2)));
        test.core.open_followed(other, Some(3)).unwrap();
        assert_eq!(selects_sent(&mut test.core, key.channel, &written), [(0x302, 3)], "another sub-page of the page open");
    }

    // DEC-1542, TODO-1531 item 2: on a live channel nothing rotates, so the
    // return to the sub-pages as they arrive does nothing and sends nothing;
    // the sub-page chosen stays the selection.
    #[cfg(all(unix, feature = "interaction"))]
    #[test]
    fn the_return_to_the_latest_does_nothing_on_a_live_channel() {
        use super::super::test_support::{selects_sent, wire};
        let (mut test, key) = live_page("live latest", &[1, 2, 3]);
        let written = wire(&mut test.core, key.channel);
        drawn(&mut test, key, 1, 1, 0);
        test.core
            .intent(protocol::ToCore::PageControl(PageControl::SubPageOf { channel: test.core.active_channel().unwrap(), page: 0x210, subcode: 2 }))
            .unwrap();
        let _ = selects_sent(&mut test.core, key.channel, &written);
        test.core.intent(protocol::ToCore::PageControl(PageControl::Latest)).unwrap();
        assert_eq!(selects_sent(&mut test.core, key.channel, &written), [], "no SELECT of 3F7F");
        assert_eq!(test.core.typing.selection(key.channel), Some((0x210, 2)));
        assert_eq!(test.core.controls.view.and_then(|view| view.subpage), Some(2), "the view keeps the sub-page chosen");
    }

    // DEC-1526: a channel served as files adopts nothing.
    #[cfg(feature = "interaction")]
    #[test]
    fn a_channel_served_as_files_adopts_no_selection() {
        let mut test = super::super::test_support::Test::new("files");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let channel = test.core.active_channel().unwrap();
        let key = PageKey { channel, page: 0x210 };
        test.core.intent(protocol::ToCore::Opened(Some(key))).unwrap();
        drawn(&mut test, key, 1, 2, 0);
        assert_eq!(test.core.typing.selection(key.channel), Some((0x210, SUBCODE_ANY)));
        assert_eq!(test.core.awaiting(), None);
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPage(true))).unwrap();
        assert_eq!(test.core.strip().notice, "", "a channel served as files says no loading");
    }

    // DEC-1467: the strip says connecting from the descriptor to the first
    // page, never that a page was not found.
    #[test]
    fn a_live_channel_says_it_is_connecting_until_its_first_page() {
        use protocol::fetcher::FromFetcher;
        let mut test = super::super::test_support::Test::new("connecting");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let channel = test.core.active_channel().unwrap();
        test.core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
        assert_eq!(test.core.strip().notice, "Connecting to the channel");
        test.core.fetched(channel, FromFetcher::Connecting(false)).unwrap();
        assert_eq!(test.core.strip().notice, "");
    }

    /// DEC-6358 item 3 and fault 4: every request for rows is answered
    /// with its own number. A live page's rows wait for its list, which
    /// the channel sends, and are not taken from the decoder; two requests
    /// waiting are both answered; and a request of a channel that is not
    /// in front is not answered from the one that is.
    #[test]
    fn each_request_for_rows_is_answered_with_its_number() {
        use protocol::fetcher::FromFetcher;
        let mut test = super::super::test_support::Test::new("rows by number");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let channel = test.core.active_channel().unwrap();
        test.core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
        test.core.fetched(channel, FromFetcher::Connecting(false)).unwrap();
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: 0x210 }))).unwrap();
        let range = |first| SubPageRange { first, count: 64 };
        let answers = |test: &mut super::super::test_support::Test| -> Vec<(u16, u16, usize)> {
            let sent = test.sent();
            sent.into_iter()
                .filter_map(
                    |message| if let ToInterface::SubPageRows { request, rows, .. } = message { Some((request, rows.first, rows.rows.len())) } else { None },
                )
                .collect()
        };
        answers(&mut test);
        // The list has not come: the requests wait, and no decoder is asked.
        test.core.intent(protocol::ToCore::SubPageRows { channel, page: 0x210, range: range(0), request: 11 }).unwrap();
        test.core.intent(protocol::ToCore::SubPageRows { channel, page: 0x210, range: range(2), request: 12 }).unwrap();
        // A request of another channel is not this one's to answer.
        test.core.intent(protocol::ToCore::SubPageRows { channel: channel + 1, page: 0x210, range: range(0), request: 13 }).unwrap();
        assert_eq!(answers(&mut test), [], "nothing to answer with yet");
        let subcodes = [1, 2, 3];
        test.core.fetched(channel, list_part(&subcodes, "1", 0)).unwrap();
        assert_eq!(answers(&mut test), [(11, 0, 3), (12, 2, 1)], "both waiting requests answered from the list, each with its number");
        // Once the list is held a request is answered at once.
        test.core.intent(protocol::ToCore::SubPageRows { channel, page: 0x210, range: range(1), request: 14 }).unwrap();
        assert_eq!(answers(&mut test), [(14, 1, 2)]);
    }

    /// DEC-6358, fault 4, on a channel served as files: a request for rows
    /// passed to the page's decoder, the page's list arriving before the
    /// decoder answers, is answered from the list with the request's
    /// number when the decoder does answer; it was dropped, and the
    /// interface never asked again.
    #[test]
    fn a_decoders_late_answer_is_answered_from_the_list_that_came() {
        use protocol::decoder::FromDecoder;
        use protocol::subpages::SubPageRows;
        let mut test = super::super::test_support::Test::new("late answer");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let channel = test.core.active_channel().unwrap();
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: 0x210 }))).unwrap();
        test.sent();
        let answers = |test: &mut super::super::test_support::Test| -> Vec<(u16, usize)> {
            let sent = test.sent();
            sent.into_iter()
                .filter_map(|message| if let ToInterface::SubPageRows { request, rows, .. } = message { Some((request, rows.rows.len())) } else { None })
                .collect()
        };
        // No list is held or on its way: the decoder is asked.
        test.core.intent(protocol::ToCore::SubPageRows { channel, page: 0x210, range: SubPageRange { first: 0, count: 64 }, request: 21 }).unwrap();
        assert_eq!(answers(&mut test), [], "the decoder has not answered");
        // The page's list comes first.
        test.core.fetched(channel, list_part(&[1, 2, 3], "v1", 0)).unwrap();
        assert_eq!(answers(&mut test), [], "no request was waiting on the list");
        // Then the decoder answers with the one sub-page it holds.
        let held = SubPageRows { count: 1, first: 0, rows: vec![(1, "One".into())] };
        test.core.decoded(channel, FromDecoder::SubPageRows { page: 0x210, rows: held }).unwrap();
        assert_eq!(answers(&mut test), [(21, 3)], "answered from the list, all three rows, with the request's number");
        // An answer the decoder gives to no request is not passed on.
        let stray = SubPageRows { count: 1, first: 0, rows: vec![(1, "One".into())] };
        test.core.decoded(channel, FromDecoder::SubPageRows { page: 0x210, rows: stray }).unwrap();
        assert_eq!(answers(&mut test), []);
    }

    /// DEC-6358, fault 5: a request for rows made of a tab is not answered
    /// once another tab is in front, and the tab's next request, when it
    /// is in front again, is.
    #[test]
    fn rows_asked_of_a_tab_are_not_answered_into_another() {
        let mut test = super::super::test_support::Test::new("tab change");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let news = test.core.active_channel().unwrap();
        test.core.fetched(news, list_part(&[1, 2, 3], "v1", 0)).unwrap();
        test.core.session.open("https://sport.invalid/teletext/").unwrap();
        let sport = test.core.active_channel().unwrap();
        assert_ne!(news, sport);
        test.sent();
        let answers = |test: &mut super::super::test_support::Test| -> Vec<(u32, u16)> {
            let sent = test.sent();
            sent.into_iter()
                .filter_map(|message| if let ToInterface::SubPageRows { channel, request, .. } = message { Some((channel, request)) } else { None })
                .collect()
        };
        // Sport is in front: a request the interface made of news, just before the tab changed, is not answered.
        test.core.intent(protocol::ToCore::SubPageRows { channel: news, page: 0x210, range: SubPageRange { first: 0, count: 64 }, request: 31 }).unwrap();
        assert_eq!(answers(&mut test), [], "not answered from, or into, the tab in front");
        // News in front again: its request is answered.
        test.core.session.active = 0;
        assert_eq!(test.core.active_channel(), Some(news));
        test.sent();
        test.core.intent(protocol::ToCore::SubPageRows { channel: news, page: 0x210, range: SubPageRange { first: 0, count: 64 }, request: 32 }).unwrap();
        assert_eq!(answers(&mut test), [(news, 32)]);
    }
}
