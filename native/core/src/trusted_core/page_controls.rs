//! The open page's controls in the trusted core (`page-controls.md`,
//! decision 67): Fastext's keys followed as the page's other links are,
//! and reveal, hold, mix and the sub-page shown, each told to the page's
//! decoder and lapsing when another page is opened, as on a set.

use std::collections::BTreeMap;

use interface_core::canvas::PageKey;
use interface_core::controls::{MAX_SUBPAGES, PageControl, PageControls};
use protocol::ToInterface;
use protocol::decoder::{ToDecoder, View};

use super::Core;

/// What the trusted core keeps of the open page's controls.
#[derive(Default)]
pub(crate) struct State {
    /// The page open in the active tab.
    open: Option<PageKey>,
    /// How it is drawn, when the user changed it.
    view: Option<View>,
    /// The sub-pages of the pages on screen, and the one drawn, as their
    /// decoders reported them.
    subpages: BTreeMap<PageKey, (Vec<u16>, u16)>,
    /// A page about to open at a sub-page: a Fastext link or an address
    /// named one.
    pending: Option<(PageKey, u16)>,
    /// The controls the interface was last told of.
    sent: Option<PageControls>,
}

impl State {
    /// The view of `key`, as drawn now.
    fn view_of(&self, key: PageKey) -> View {
        self.view.filter(|view| self.open == Some(key) && view.page == key.page).unwrap_or(View { page: key.page, ..View::default() })
    }
}

/// The sub-page after or before `current` among `subcodes`, round the
/// list as a set's sub-page keys go; none when there is only one.
fn step(subcodes: &[u16], current: u16, forward: bool) -> Option<u16> {
    let count = subcodes.len();
    if count < 2 {
        return None;
    }
    let place = subcodes.iter().position(|subcode| *subcode == current).unwrap_or(0);
    let next = if forward { place.saturating_add(1).checked_rem(count).unwrap_or(0) } else { place.checked_sub(1).unwrap_or(count.saturating_sub(1)) };
    subcodes.get(next).copied()
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
        if let Some((pending, subpage)) = self.controls.pending.take()
            && Some(pending) == key
        {
            let view = View { page: pending.page, subpage: Some(subpage), ..View::default() };
            self.controls.view = Some(view);
            self.tell(pending.channel, ToDecoder::View(view));
        }
        self.send_controls()
    }

    /// A page of `channel` will open at `subpage`, when it opens.
    pub(crate) fn open_at_subpage(&mut self, key: PageKey, subpage: Option<u16>) {
        self.controls.pending = subpage.map(|subpage| (key, subpage));
    }

    /// A decoder's sub-pages of a page on screen.
    pub(crate) fn subpages(&mut self, key: PageKey, subcodes: Vec<u16>, shown: u16) -> Result<(), String> {
        self.controls.subpages.insert(key, (subcodes, shown));
        if self.controls.open == Some(key) { self.send_controls() } else { Ok(()) }
    }

    /// Forgets the sub-pages of pages no longer on screen, and of channels
    /// gone.
    pub(crate) fn keep_subpages(&mut self, visible: &[PageKey]) {
        let open = self.controls.open;
        self.controls.subpages.retain(|key, _| visible.contains(key) || Some(*key) == open);
    }

    /// Strip input: a key of the open page's controls.
    pub(crate) fn page_control(&mut self, control: PageControl) -> Result<(), String> {
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
                let Some((subcodes, shown)) = self.controls.subpages.get(&key) else { return Ok(()) };
                let Some(next) = step(subcodes, view.subpage.unwrap_or(*shown), forward) else { return Ok(()) };
                view.subpage = Some(next);
            }
            PageControl::Latest => view.subpage = None,
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
                let (subcodes, shown) = self.controls.subpages.get(&key).cloned().unwrap_or_default();
                let chosen =
                    view.subpage.and_then(|subpage| subcodes.iter().position(|subcode| *subcode == subpage)).and_then(|place| u8::try_from(place).ok());
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
                    subpages: u8::try_from(subcodes.len().min(MAX_SUBPAGES)).unwrap_or(u8::MAX),
                    chosen,
                    subcode: shown,
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
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// Decision 67: the sub-page keys go round the list; one sub-page has
    /// nowhere to go.
    #[test]
    fn sub_pages_step_round() {
        assert_eq!(step(&[1, 2, 3], 2, true), Some(3));
        assert_eq!(step(&[1, 2, 3], 3, true), Some(1));
        assert_eq!(step(&[1, 2, 3], 1, false), Some(3));
        assert_eq!(step(&[1, 2, 3], 9, true), Some(2), "one gone from the store: from the first");
        assert_eq!(step(&[4], 4, true), None);
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
}
