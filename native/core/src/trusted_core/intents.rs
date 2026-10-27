//! The interface's intents (requirements 4.5 and 12.3): each acted on by
//! the trusted core's own rules, and what the interface is sent afresh
//! after a change of tab or channel.

use interface_core::canvas::PageKey;
use neoteletext::frames;
use protocol::console::Level;
use protocol::decoder::ToDecoder;
#[cfg(feature = "links")]
use protocol::links::{Message as LinkMessage, Spot};
use protocol::{ToCore, ToInterface};

use super::startup::open_home;
use super::{Core, WINDOW_STILL};
use crate::session::Went;

impl Core {
    /// Sends everything the interface shows: tabs, strip and channel. The
    /// interface then reports what is visible afresh.
    pub(super) fn send_all(&mut self) -> Result<(), String> {
        self.match_decoders()?;
        // Another tab or channel: the keys leave the page they were going
        // to; the interface reports what it opens next.
        #[cfg(feature = "interaction")]
        {
            self.opened = None;
            let sends = self.typing.opened(self.active_channel(), None, self.now());
            self.deliver_typing(sends)?;
        }
        self.frames = frames::Frames::default();
        if let Some(previous) = self.showing.take() {
            self.tell(previous, ToDecoder::Showing(Vec::new()));
        }
        let tabs = self.session.tab_infos();
        let active = self.session.active;
        self.send(&ToInterface::Tabs { tabs, active })?;
        self.send(&ToInterface::Strip(self.strip()))?;
        if let Some(tab) = self.session.active_tab() {
            let channel = tab.channel.view();
            self.send(&ToInterface::Channel(channel))?;
            self.send_subpages()?;
        }
        self.send_console()?;
        #[cfg(feature = "links")]
        self.send_links_afresh()?;
        self.send_mutes();
        // An address that names a sub-page opens its page there (decision 67).
        if let Some(tab) = self.session.active_tab()
            && let Some(address) = tab.address()
            && let Some(page) = address.page
        {
            let key = PageKey { channel: tab.channel.id, page };
            self.open_at_subpage(key, address.subpage);
        }
        Ok(())
    }

    /// Tells the interface each tab's channel's mute, where it changed: a
    /// muted tab's new channel starts muted.
    fn send_mutes(&mut self) {
        let tabs: Vec<(u32, bool)> = self.session.all_tabs().map(|tab| (tab.channel.id, tab.muted)).collect();
        let sends = tabs.into_iter().filter_map(|(channel, muted)| self.hearing.mute(channel, muted)).collect();
        self.deliver(sends);
    }

    /// The tabs again, after a change of mute, and the mutes themselves.
    fn send_tabs(&mut self) -> Result<(), String> {
        let tabs = self.session.tab_infos();
        let active = self.session.active;
        self.send(&ToInterface::Tabs { tabs, active })?;
        self.send_mutes();
        Ok(())
    }

    /// Returns false when the browser should end.
    pub(super) fn intent(&mut self, intent: ToCore) -> Result<bool, String> {
        if matches!(intent, ToCore::From(_) | ToCore::NewWindow { .. } | ToCore::TabToWindow { .. } | ToCore::CloseWindow) {
            return self.window_intent(&intent);
        }
        if self.from_closed {
            return Ok(true);
        }
        match intent {
            ToCore::From(_) | ToCore::NewWindow { .. } | ToCore::TabToWindow { .. } | ToCore::CloseWindow => {}
            ToCore::Visible(visible) => {
                let Some(channel) = self.active_channel() else { return Ok(true) };
                self.visible(channel, visible);
            }
            ToCore::Cell { key, version, column, row } => self.cell(key, version, column, row)?,
            ToCore::SelectTab(index) => self.select_tab(usize::from(index))?,
            ToCore::CloseTab(index) => {
                self.session.close(usize::from(index));
                // A window's last tab closes the window; the last window's,
                // the browser.
                if self.session.tabs.is_empty() {
                    return self.close_window();
                }
                self.send_all()?;
                self.send_history()?;
            }
            ToCore::NewTab => self.new_tab()?,
            // The start page's bookmark pages open their bookmarks, as the
            // user's own navigation (requirement 19.7).
            ToCore::Opened(Some(key)) if self.session.start_page_bookmark(key.channel, key.page).is_some() => self.open_start_page_bookmark(key)?,
            ToCore::Opened(key) => self.opened(key)?,
            ToCore::MuteTab(_) | ToCore::MuteAll(_) | ToCore::MoveTab { .. } => self.tab_intent(&intent)?,
            ToCore::Navigate(typed) => self.navigate(&typed)?,
            ToCore::Back | ToCore::Forward => {
                let went = self.session.go(matches!(intent, ToCore::Forward));
                self.went(went)?;
            }
            ToCore::Reload | ToCore::PageControl(_) => self.page_intent(&intent)?,
            ToCore::Bookmark => self.star()?,
            ToCore::Home | ToCore::ChangeSetting(_) | ToCore::Console(_) | ToCore::Drawn | ToCore::InstallUpdate => self.settings_intent(intent)?,
            ToCore::History(_)
            | ToCore::Bookmarks(_)
            | ToCore::Star(_)
            | ToCore::Bar(_)
            | ToCore::Suggest(_)
            | ToCore::Suggestion { .. }
            | ToCore::TabList(_)
            | ToCore::GoTo { .. }
            | ToCore::NewPrivateTab => self.library_intent(intent)?,
            ToCore::Window(window) => self.window_reported(window),
            ToCore::Cookies(request) => self.cookies_request(request)?,
            #[cfg(feature = "links")]
            ToCore::Point(_) | ToCore::Typed { .. } | ToCore::Confirm(_) | ToCore::Cancel(_) => self.link_intent(&intent)?,
            #[cfg(any(feature = "links", feature = "interaction"))]
            ToCore::Focused(focused) => self.focused(focused)?,
            #[cfg(feature = "interaction")]
            ToCore::Input(_) | ToCore::Editing(_) => self.input_intent(intent)?,
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhonePanel(_) | ToCore::PhoneFinish => self.input_intent(intent)?,
        }
        // Another page of a channel already drawing is a visit at once.
        self.record_drawn()?;
        self.private_cookies_closed();
        self.keep_session();
        Ok(true)
    }

    /// The pages the interface shows of `channel`, the active one: only
    /// the active channel's pages can be asked for.
    fn visible(&mut self, channel: u32, visible: Vec<PageKey>) {
        let visible: Vec<PageKey> = visible.into_iter().filter(|key| key.channel == channel).collect();
        let pages = visible.iter().map(|key| key.page).collect();
        #[cfg(feature = "links")]
        self.links.retain(&visible);
        self.keep_subpages(&visible);
        self.frames.set_visible(visible);
        self.shown_pages.clone_from(&pages);
        self.tell(channel, ToDecoder::Showing(pages));
        self.showing = Some(channel);
    }

    /// A click means only "this cell"; the trusted core decides whether a
    /// link is there (requirement 12.3).
    #[cfg_attr(not(feature = "links"), allow(clippy::unnecessary_wraps, reason = "only following a link can fail here"))]
    fn cell(&mut self, key: PageKey, version: u32, column: u8, row: u8) -> Result<(), String> {
        #[cfg(feature = "links")]
        {
            let (now, active) = (self.now(), self.active_channel());
            let followed = self.links.click(Spot { key, version, column, row }, active, now);
            self.follow(followed, "a click")?;
        }
        #[cfg(not(feature = "links"))]
        self.log(Level::Warning, None, &format!("cell {column},{row} of image {version} of page {:03X} on channel {}", key.page, key.channel));
        Ok(())
    }

    /// Tab `index` becomes the active one, if there is such a tab.
    fn select_tab(&mut self, index: usize) -> Result<(), String> {
        if index < self.session.tabs.len() {
            self.session.active = index;
            self.send_all()?;
        }
        Ok(())
    }

    /// A new tab at the home page, if there is room for one.
    fn new_tab(&mut self) -> Result<(), String> {
        if let Err(reason) = self.session.room_for_a_tab() {
            self.say(reason)?;
        } else {
            let home = self.settings.home.clone();
            open_home(&mut self.session, &home)?;
            self.send_all()?;
        }
        Ok(())
    }

    /// An address typed in the address field opened in the active tab.
    fn navigate(&mut self, typed: &str) -> Result<(), String> {
        if let Err(reason) = self.session.navigate(typed, true) {
            self.log(Level::Warning, self.log_place(), &reason);
        }
        self.send_all()
    }

    /// The window's place and size, kept once it has stayed still.
    fn window_reported(&mut self, window: protocol::library::WindowGeometry) {
        let at_hand = self.session.window;
        self.window_moved = std::time::Instant::now().checked_add(WINDOW_STILL).map(|still| (at_hand, window, still));
    }

    /// A tab's sound, every tab's, and a tab moved along the strip
    /// (decision 69): the tabs are sent again when they changed.
    fn tab_intent(&mut self, intent: &ToCore) -> Result<(), String> {
        let changed = match *intent {
            ToCore::MuteTab(index) => self.session.toggle_mute(usize::from(index)),
            ToCore::MuteAll(muted) => {
                self.session.mute_all(muted);
                true
            }
            ToCore::MoveTab { from, to } => self.session.move_tab(usize::from(from), usize::from(to)),
            _ => false,
        };
        if changed { self.send_tabs() } else { Ok(()) }
    }

    /// Reload, and the open page's controls (decision 67).
    fn page_intent(&mut self, intent: &ToCore) -> Result<(), String> {
        match intent {
            ToCore::Reload => self.reload(),
            ToCore::PageControl(control) => self.page_control(*control),
            _ => Ok(()),
        }
    }

    /// The page open in the active tab changed: its keys, its links, its
    /// sounds and its controls follow it.
    fn opened(&mut self, key: Option<PageKey>) -> Result<(), String> {
        // Only a page of the active tab's channel opens: a key of another
        // tab's would let the page controls and typing reach its decoder
        // (finding F-66).
        let active = self.active_channel();
        let key = key.filter(|key| Some(key.channel) == active);
        #[cfg(feature = "interaction")]
        {
            self.opened = key;
            let sends = self.typing.opened(self.active_channel(), key, self.now());
            self.deliver_typing(sends)?;
        }
        #[cfg(feature = "links")]
        self.links.opened(key);
        let sends = self.hearing.open(self.active_channel(), key);
        self.deliver(sends);
        self.page_opened(key)
    }

    /// Reload: the active tab's channel opens afresh (requirement 11.2).
    pub(super) fn reload(&mut self) -> Result<(), String> {
        if self.session.reload() { self.send_all() } else { Ok(()) }
    }

    /// After Back, Forward or a step from their lists: another channel is
    /// sent afresh; another page of the same channel stays open.
    pub(super) fn went(&mut self, went: Went) -> Result<(), String> {
        match went {
            Went::Nowhere => Ok(()),
            Went::Channel => self.send_all(),
            #[cfg_attr(not(feature = "links"), allow(unused_variables, reason = "only a link opens a page"))]
            Went::Page(page) => {
                self.send(&ToInterface::Strip(self.strip()))?;
                #[cfg(feature = "links")]
                if let (Some(page), Some(channel)) = (page, self.active_channel()) {
                    self.link(LinkMessage::Show(PageKey { channel, page }))?;
                }
                Ok(())
            }
        }
    }

    /// A bookmark page of the start page was opened: its bookmark opens.
    /// A start-page bookmark's page opened in the page area: shown, and its
    /// bookmark offered through the site-change screen, as a link to
    /// another channel is, never opened by the intent alone (requirement
    /// 4.5, finding F-67).
    fn open_start_page_bookmark(&mut self, key: PageKey) -> Result<(), String> {
        let bookmark = self.session.start_page_bookmark(key.channel, key.page);
        self.opened(Some(key))?;
        #[cfg(feature = "links")]
        if let Some(address) = bookmark.and_then(|bookmark| neoteletext::address::parse(&bookmark).ok()) {
            let followed = self.links.ask(neoteletext::links::Destination::Channel(address), self.now());
            self.follow(followed, "a start-page bookmark")?;
        }
        #[cfg(not(feature = "links"))]
        let _ = bookmark;
        Ok(())
    }

    /// The window gained or lost the keyboard focus: links hand off only
    /// with it, and the channel being typed into is told.
    #[cfg(any(feature = "links", feature = "interaction"))]
    #[cfg_attr(not(feature = "interaction"), allow(clippy::unnecessary_wraps, reason = "only typing can fail here"))]
    fn focused(&mut self, focused: bool) -> Result<(), String> {
        #[cfg(feature = "links")]
        self.links.focused(focused);
        #[cfg(feature = "interaction")]
        {
            let sends = self.typing.focused(focused, self.now());
            self.deliver_typing(sends)?;
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "links"))]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::super::test_support::Test;
    use interface_core::canvas::PageKey;
    use protocol::ToCore;
    use protocol::links::PromptKind;

    /// Finding F-67 (requirement 4.5): a start-page bookmark's page opened
    /// in the page area is shown, not acted on: its bookmark opens only
    /// after the site-change screen, as a link to another channel does.
    #[test]
    fn a_start_page_bookmark_opens_after_the_site_change_screen() {
        let mut test = Test::new("start-page-intent");
        test.core.session.bookmarks.add(neoteletext::bookmarks::BAR, "News", "https://news.invalid/").unwrap();
        test.core.session.open(neoteletext::address::START_PAGE).unwrap();
        let channel = test.core.session.tabs[0].channel.id;
        test.core.intent(ToCore::Opened(Some(PageKey { channel, page: 0x201 }))).unwrap();
        assert!(test.core.session.active_tab().and_then(|tab| tab.address()).is_some_and(neoteletext::address::Address::is_start_page), "still the start page");
        assert_eq!(test.core.links.prompt().map(|prompt| prompt.kind), Some(PromptKind::Leave), "the site-change screen asks");
    }
}
