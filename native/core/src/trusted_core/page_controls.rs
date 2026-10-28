//! The open page's controls in the trusted core (`page-controls.md`,
//! decision 67): Fastext's keys followed as the page's other links are,
//! and reveal, hold, mix and the sub-page shown, each told to the page's
//! decoder and lapsing when another page is opened, as on a set.
//!
//! Sub-pages are stepped from a place and a count, never a list (decision
//! 112 item 4): from the page's catalogue when its fetcher read one, which
//! lists every sub-page whether held or not, else from the neighbours its
//! decoder says of the one drawn. A sub-page of a page with a catalogue is
//! fetched, with those either side of it, only when it is shown (decision
//! 112 item 3; the engine's REQ-WEB-019).

use std::collections::{BTreeMap, BTreeSet};

use interface_core::canvas::PageKey;
use interface_core::controls::{PageControl, PageControls};
use neoteletext::subpages::{Catalogue, TUNED_EITHER_SIDE};
use neoteletext_core::limits::{SUBCODE_ANY, SUBCODE_NONE};
use protocol::ToInterface;
use protocol::decoder::{ToDecoder, View};
use protocol::fetcher::{ToFetcher, write_to_fetcher};
use protocol::subpages::{SubPageRange, SubPageStep};

use super::Core;

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
    /// A page about to open at a sub-page: a Fastext link or an address
    /// named one.
    pending: Option<(PageKey, u16)>,
    /// The sub-pages of the open page asked of its fetcher since it
    /// opened: each is asked for once.
    fetched: BTreeSet<u16>,
    /// The open page's catalogue was asked for since it opened.
    catalogue_asked: bool,
    /// The controls the interface was last told of.
    sent: Option<PageControls>,
    /// What was asked of each channel's fetcher, for the tests, whose core
    /// starts no fetcher.
    #[cfg(test)]
    pub(super) told_fetchers: Vec<(u32, ToFetcher)>,
}

impl State {
    /// The view of `key`, as drawn now.
    fn view_of(&self, key: PageKey) -> View {
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
        self.controls.fetched.clear();
        self.controls.catalogue_asked = false;
        if let Some(key) = key {
            self.ask_catalogue(key);
        }
        if let Some((pending, subpage)) = self.controls.pending.take()
            && Some(pending) == key
        {
            self.show_subpage(pending, View { page: pending.page, subpage: Some(subpage), ..View::default() });
        }
        self.send_controls()
    }

    /// A page of `channel` will open at `subpage`, when it opens.
    pub(crate) fn open_at_subpage(&mut self, key: PageKey, subpage: Option<u16>) {
        self.controls.pending = subpage.map(|subpage| (key, subpage));
    }

    /// A decoder's sub-pages of a page on screen, from the one it draws.
    pub(crate) fn subpages(&mut self, key: PageKey, step: SubPageStep) -> Result<(), String> {
        self.controls.subpages.insert(key, step);
        if self.controls.open != Some(key) {
            return Ok(());
        }
        self.ask_catalogue(key);
        self.send_controls()
    }

    /// Forgets the sub-pages of pages no longer on screen, and of channels
    /// gone.
    pub(crate) fn keep_subpages(&mut self, visible: &[PageKey]) {
        let open = self.controls.open;
        self.controls.subpages.retain(|key, _| visible.contains(key) || Some(*key) == open);
    }

    /// The open page's catalogue of sub-pages, asked of its fetcher once
    /// it opens, when it is one of a set (the engine's REQ-WEB-034): a
    /// channel that publishes none is asked once a page and never again.
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
        let ask = self.channel_subpages(key.channel).is_some_and(|subpages| {
            subpages.unanswered(key.page);
            subpages.ask(key.page)
        });
        if ask {
            self.tell_fetcher(key.channel, &ToFetcher::FetchSubPages { page: key.page });
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
    fn show_subpage(&mut self, key: PageKey, view: View) {
        self.controls.view = Some(view);
        self.tell(key.channel, ToDecoder::View(view));
        let Some(subpage) = view.subpage else { return };
        let Some(around) = self.with_catalogue(key, |catalogue| catalogue.around(subpage, TUNED_EITHER_SIDE)) else { return };
        for subcode in around {
            if subcode != SUBCODE_ANY && self.controls.fetched.insert(subcode) {
                self.tell_fetcher(key.channel, &ToFetcher::FetchSubPage { page: key.page, subcode });
            }
        }
    }

    /// The open page's catalogue arrived or changed: its controls and the
    /// sub-page it shows follow it.
    pub(super) fn catalogue_arrived(&mut self, key: PageKey) -> Result<(), String> {
        if self.controls.open != Some(key) {
            return Ok(());
        }
        let view = self.controls.view_of(key);
        if view.subpage.is_some() {
            self.show_subpage(key, view);
        }
        self.send_controls()
    }

    /// The sub-page list asks for rows of `page` of the active channel:
    /// answered from its catalogue when one is held, else by its decoder
    /// from what it holds. Nothing is fetched for it: only the page open
    /// has its catalogue asked for (requirement 4.7 as amended for
    /// decision 121).
    pub(super) fn subpage_rows(&mut self, page: u16, range: SubPageRange) -> Result<(), String> {
        let Some(channel) = self.active_channel() else { return Ok(()) };
        if let Some(rows) = self.with_catalogue(PageKey { channel, page }, |catalogue| catalogue.rows(range)) {
            return self.send(&ToInterface::SubPageRows { channel, page, rows });
        }
        self.tell(channel, ToDecoder::SubPageRows { page, range });
        Ok(())
    }

    /// Strip input: a key of the open page's controls.
    pub(crate) fn page_control(&mut self, control: PageControl) -> Result<(), String> {
        // A sub-page chosen from the page list: shown at once on the page
        // open, or when its page opens.
        if let PageControl::SubPageOf { page, subcode } = control {
            let Some(channel) = self.active_channel() else { return Ok(()) };
            let target = PageKey { channel, page };
            if self.controls.open != Some(target) {
                self.open_at_subpage(target, Some(subcode));
                return Ok(());
            }
            let view = View { subpage: Some(subcode), ..self.controls.view_of(target) };
            self.show_subpage(target, view);
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
                let next = match self.with_catalogue(key, |catalogue| catalogue.step(current, forward)) {
                    Some(stepped) => stepped,
                    None => drawn.and_then(|drawn| step_drawn(&drawn, &view, forward)),
                };
                // 3F7F names no sub-page (clause 9.6.1).
                let Some(next) = next.filter(|&next| next != SUBCODE_ANY) else { return Ok(()) };
                view.subpage = Some(next);
                self.show_subpage(key, view);
                return self.send_controls();
            }
            PageControl::Latest => view.subpage = None,
            PageControl::SubPageOf { .. } => return Ok(()),
        }
        self.controls.view = Some(view);
        self.tell(key.channel, ToDecoder::View(view));
        self.send_controls()
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
                    page: Some(key.page),
                    fastext,
                    reveal: view.reveal,
                    hold: view.hold,
                    mix: view.mix,
                    subpages,
                    chosen: chosen.filter(|&chosen| chosen < subpages),
                    subcode: drawn.shown,
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

    /// Decision 119 and decision 112 item 3 (the engine's REQ-WEB-019 and
    /// REQ-WEB-034): a page that is one of a set has its catalogue asked
    /// for when it opens; the catalogue's 8,190 sub-pages are counted and
    /// stepped through, the list's rows answered from it, and only the
    /// sub-page shown and those either side of it asked of the fetcher,
    /// each once.
    #[test]
    fn a_catalogue_lists_every_sub_page_and_only_those_shown_are_fetched() {
        use neoteletext_core::limits::{MAX_SUBPAGES, from_rank};
        use protocol::decoder::FromDecoder;
        use protocol::fetcher::{CATALOGUE_PART, FromFetcher};
        use protocol::subpages::SubPageRows;
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
        assert_eq!(core::mem::take(&mut test.core.controls.told_fetchers), [(channel, ToFetcher::FetchSubPages { page: 0x210 })]);
        test.core.decoded(channel, FromDecoder::SubPages { page: 0x210, step: first }).unwrap();
        assert!(test.core.controls.told_fetchers.is_empty(), "asked once while it is open");
        for (place, part) in (0_u16..).step_by(CATALOGUE_PART).zip(subcodes.chunks(CATALOGUE_PART)) {
            let part = SubPageRows { count: most, first: place, rows: part.iter().map(|&subcode| (subcode, format!("Story {subcode:04X}"))).collect() };
            test.core.fetched(channel, FromFetcher::SubPages { page: 0x210, part }).unwrap();
        }
        let sent = test.sent();
        assert!(sent.contains(&ToInterface::SubPageCount { channel, page: 0x210, count: most }));
        let controls = sent.iter().rev().find_map(|message| if let ToInterface::PageControls(controls) = message { Some(*controls) } else { None });
        assert_eq!(controls.map(|controls| controls.subpages), Some(most), "every sub-page, held or not");
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPage(false))).unwrap();
        let last = subcodes[usize::from(most) - 1];
        assert_eq!(test.core.controls.view.and_then(|view| view.subpage), Some(last), "round from the first to the last");
        let told: Vec<ToFetcher> = core::mem::take(&mut test.core.controls.told_fetchers).into_iter().map(|(_, told)| told).collect();
        let fetch = |subcode| ToFetcher::FetchSubPage { page: 0x210, subcode };
        assert_eq!(told, [fetch(last), fetch(subcodes[0]), fetch(subcodes[usize::from(most) - 2])], "the one shown and those either side");
        test.core.intent(protocol::ToCore::PageControl(PageControl::SubPage(true))).unwrap();
        let told: Vec<ToFetcher> = core::mem::take(&mut test.core.controls.told_fetchers).into_iter().map(|(_, told)| told).collect();
        assert_eq!(told, [fetch(subcodes[1])], "each asked for once: only the new neighbour");
        let sent = test.sent();
        let controls = sent.iter().rev().find_map(|message| if let ToInterface::PageControls(controls) = message { Some(*controls) } else { None });
        assert_eq!(controls.and_then(|controls| controls.chosen), Some(0), "back at the first, by its place in the catalogue");
        let range = SubPageRange { first: most - 64, count: 64 };
        test.core.intent(protocol::ToCore::SubPageRows { page: 0x210, range }).unwrap();
        let rows = test.sent().into_iter().find_map(|message| if let ToInterface::SubPageRows { rows, .. } = message { Some(rows) } else { None }).unwrap();
        assert_eq!((rows.count, rows.first, rows.rows.len()), (most, most - 64, 64));
        assert_eq!(rows.rows[63], (last, format!("Story {last:04X}")), "answered from the catalogue");
    }
}
