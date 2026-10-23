//! Windows (`multiple-windows.md`): what each window set aside shows, the
//! window at hand, windows opened, closed and given a tab, and the session
//! kept after every change (requirement 20.21).

use neoteletext::frames;
use neoteletext::restore::Saved;
use protocol::console::Level;
use protocol::decoder::ToDecoder;
use protocol::{ToCore, ToInterface};

use super::Core;
use super::startup::open_home;

/// What a window shows of its active channel, kept while it is set aside.
#[derive(Default)]
pub(super) struct Shown {
    frames: frames::Frames,
    showing: Option<u32>,
    pages: Vec<u16>,
}

impl Core {
    /// When a frame held in any window falls due (finding F-11).
    pub(super) fn frames_due_at(&self) -> Option<std::time::Instant> {
        std::iter::once(&self.frames).chain(self.shown_elsewhere.values().map(|shown| &shown.frames)).filter_map(frames::Frames::next_due).min()
    }

    /// Sends every window's held frames whose interval is up, each in its
    /// own window.
    pub(super) fn send_due_frames(&mut self, now: std::time::Instant) -> Result<(), String> {
        let here = self.session.window;
        let windows: Vec<u32> = std::iter::once(here)
            .chain(self.shown_elsewhere.iter().filter(|(_, shown)| shown.frames.next_due().is_some_and(|due| due <= now)).map(|(id, _)| *id))
            .collect();
        let mut result = Ok(());
        for window in windows {
            self.switch_window(window);
            for (key, version, image) in self.frames.due(now) {
                result = result.and_then(|()| self.send_frame(key, version, image));
            }
        }
        self.switch_window(here);
        result.and_then(|()| self.send_controls())
    }
}

impl Core {
    /// Makes window `id` the one at hand, with what it shows; false when
    /// there is no such window.
    pub(super) fn switch_window(&mut self, id: u32) -> bool {
        let from = self.session.window;
        if id == from {
            return true;
        }
        if !self.session.switch_to(id) {
            return false;
        }
        self.park_shown(from);
        let next = self.shown_elsewhere.remove(&id).unwrap_or_default();
        self.frames = next.frames;
        self.showing = next.showing;
        self.shown_pages = next.pages;
        true
    }

    /// Sets aside what the window at hand, `id`, shows.
    fn park_shown(&mut self, id: u32) {
        let shown = Shown { frames: std::mem::take(&mut self.frames), showing: self.showing.take(), pages: std::mem::take(&mut self.shown_pages) };
        self.shown_elsewhere.insert(id, shown);
    }

    /// Does `work` in the window holding `channel`'s tab, then returns to
    /// the window at hand: what a channel says is for its own window.
    pub(super) fn in_window_of<T>(&mut self, channel: u32, work: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let here = self.session.window;
        let there = self.session.window_of(channel).unwrap_or(here);
        self.switch_window(there);
        let result = work(self);
        self.switch_window(here);
        result
    }

    /// Every other window's shared views are out of date.
    pub(super) fn others_stale(&mut self) {
        let here = self.session.window;
        self.stale.extend(self.session.window_ids().into_iter().filter(|id| *id != here));
    }

    /// The window at hand's shared views, if they went out of date while it
    /// was set aside.
    fn refresh_if_stale(&mut self) -> Result<(), String> {
        if !self.stale.remove(&self.session.window) {
            return Ok(());
        }
        self.send(&ToInterface::Strip(self.strip()))?;
        self.send_bar()?;
        self.send_settings()?;
        self.send_update()?;
        self.send(&ToInterface::Remote(self.settings.remote))
    }

    /// Opens a new window, private or not, with the home address in it,
    /// and makes it the one at hand.
    fn open_window(&mut self, private: bool) -> Result<(), String> {
        if let Err(reason) = self.session.room_for_a_tab() {
            return self.say(reason);
        }
        let from = self.session.window;
        let window = match self.session.new_window(private) {
            Ok(window) => window,
            Err(reason) => return self.say(reason),
        };
        self.park_shown(from);
        self.send_untargeted(&ToInterface::OpenWindow { window, geometry: None })?;
        let home = self.settings.home.clone();
        open_home(&mut self.session, &home)?;
        self.send_window_whole()
    }

    /// Everything a window shows, for a window just opened.
    pub(super) fn send_window_whole(&mut self) -> Result<(), String> {
        self.stale.remove(&self.session.window);
        self.send_settings()?;
        self.send_update()?;
        self.send_all()?;
        self.send_bar()?;
        self.send(&ToInterface::Remote(self.settings.remote))
    }

    /// Closes the window at hand and its tabs; false when it was the last,
    /// and the browser ends.
    pub(super) fn close_window(&mut self) -> Result<bool, String> {
        if self.session.window_count() <= 1 {
            return Ok(false);
        }
        let closing = self.session.window;
        if let Some(previous) = self.showing.take() {
            self.tell(previous, ToDecoder::Showing(Vec::new()));
        }
        self.session.close_window();
        let next = self.shown_elsewhere.remove(&self.session.window).unwrap_or_default();
        self.frames = next.frames;
        self.showing = next.showing;
        self.shown_pages = next.pages;
        self.shown_elsewhere.remove(&closing);
        self.stale.remove(&closing);
        self.send_untargeted(&ToInterface::CloseWindow(closing))?;
        if self.targeted == closing {
            // The next message names its window again.
            self.targeted = protocol::NEW_WINDOW;
        }
        self.match_decoders()?;
        self.send_history()?;
        Ok(true)
    }

    /// Moves the tab at `tab` of the window at hand into window `window`, or
    /// a new one, before its tab at `index` (`multiple-windows.md`); the
    /// window it leaves closes when it has no tab left.
    fn tab_to_window(&mut self, tab: usize, window: u32, index: usize) -> Result<(), String> {
        let from = self.session.window;
        if window == from {
            return Ok(());
        }
        let into_new = window == protocol::NEW_WINDOW;
        if !into_new && !self.session.window_ids().contains(&window) {
            return Ok(());
        }
        if into_new && self.session.window_count() >= interface_core::library::MAX_WINDOWS {
            return self.say(format!("At most {} windows are open at once", interface_core::library::MAX_WINDOWS));
        }
        if let Some(previous) = self.showing.take() {
            self.tell(previous, ToDecoder::Showing(Vec::new()));
        }
        let Some(moved) = self.session.take_tab(tab) else { return Ok(()) };
        let left_empty = self.session.tabs.is_empty();
        if into_new {
            let private = self.session.private_window;
            let new = self.session.new_window(private)?;
            self.park_shown(from);
            self.send_untargeted(&ToInterface::OpenWindow { window: new, geometry: None })?;
            self.session.put_tab(moved, 0);
            self.send_window_whole()?;
        } else {
            self.switch_window(window);
            self.session.put_tab(moved, index);
            self.send_all()?;
        }
        let there = self.session.window;
        self.switch_window(from);
        if left_empty {
            self.close_window()?;
        } else {
            self.send_all()?;
        }
        self.switch_window(there);
        Ok(())
    }

    /// The window intents (`multiple-windows.md`); false when the browser
    /// should end.
    pub(super) fn window_intent(&mut self, intent: &ToCore) -> Result<bool, String> {
        match *intent {
            ToCore::From(window) => {
                // The strip's question is the window's that asked it: once
                // intents come from another, it is forgotten, so no answer
                // from there follows it (finding F-65).
                #[cfg(feature = "links")]
                if window != self.session.window && self.links.forget_prompt() {
                    self.link(protocol::links::Message::Prompt(None))?;
                }
                self.from_closed = !self.switch_window(window);
                if !self.from_closed {
                    self.refresh_if_stale()?;
                }
                return Ok(true);
            }
            _ if self.from_closed => return Ok(true),
            ToCore::NewWindow { private } => self.open_window(private)?,
            ToCore::TabToWindow { tab, window, index } => self.tab_to_window(usize::from(tab), window, usize::from(index))?,
            ToCore::CloseWindow => {
                let open = self.close_window()?;
                self.keep_session();
                return Ok(open);
            }
            _ => {}
        }
        self.keep_session();
        Ok(true)
    }

    /// Keeps the place the interface last reported of a window.
    pub(super) fn keep_window(&mut self) {
        if let Some((id, geometry, _)) = self.window_moved.take() {
            let here = self.session.window;
            if self.switch_window(id) {
                self.session.geometry = Some(geometry);
                self.switch_window(here);
            }
            self.keep_session();
        }
    }

    /// Writes the session if it changed, so that it can always be restored
    /// (requirement 20.21); a failure is logged.
    pub(super) fn keep_session(&mut self) {
        let Some(folder) = &self.settings_folder else { return };
        let text = self.session.saved().text();
        if text == self.kept_session {
            return;
        }
        match Saved::save(folder, &text) {
            Ok(()) => self.kept_session = text,
            Err(error) => self.log(Level::Error, None, &format!("the session could not be kept in {}: {error}", folder.display())),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::super::WINDOW_STILL;
    use super::super::test_support::Test;
    use super::*;
    use interface_core::library::WindowGeometry;

    /// `multiple-windows.md`: a message for another window than the last
    /// is preceded by its window's number; intents act on the window they
    /// say they are from, and are not acted on from a window not open.
    #[test]
    fn messages_and_intents_name_their_window() {
        let mut test = Test::new("windows");
        test.core.session.open("news.invalid").unwrap();
        let first = test.core.session.window;
        let second = test.core.session.new_window(false).unwrap();
        test.core.session.open("sport.invalid").unwrap();
        test.sent();
        test.core.send(&ToInterface::Strip(test.core.strip())).unwrap();
        test.core.send(&ToInterface::Strip(test.core.strip())).unwrap();
        let sent = test.sent();
        assert_eq!(sent.iter().filter(|message| matches!(message, ToInterface::Target(_))).count(), 1, "once, while the window stays");
        assert_eq!(sent[0], ToInterface::Target(second));
        test.core.intent(ToCore::From(first)).unwrap();
        test.core.intent(ToCore::MuteTab(0)).unwrap();
        assert!(test.core.session.tabs[0].muted && test.core.session.window == first);
        let sent = test.sent();
        assert_eq!(sent.first(), Some(&ToInterface::Target(first)));
        assert!(sent.iter().any(|message| matches!(message, ToInterface::Tabs { tabs, .. } if tabs.len() == 1 && tabs[0].muted)));
        test.core.intent(ToCore::From(99)).unwrap();
        test.core.intent(ToCore::MuteTab(0)).unwrap();
        assert!(test.core.session.tabs[0].muted, "from no open window: not acted on");
        test.core.intent(ToCore::From(second)).unwrap();
        test.core.intent(ToCore::MuteTab(0)).unwrap();
        assert!(test.core.session.window == second && test.core.session.tabs[0].muted);
        // A window's place is kept with that window.
        let place = WindowGeometry { x: 700, y: 10, width: 640, height: 480, maximised: false };
        test.core.intent(ToCore::Window(place)).unwrap();
        test.core.intent(ToCore::From(first)).unwrap();
        test.core.keep_window();
        assert_eq!(test.core.session.geometry, None);
        test.core.switch_window(second);
        assert_eq!(test.core.session.geometry, Some(place));
    }

    /// Finding F-65: the strip's question belongs to the window that asked
    /// it: once intents come from another window, a confirmation follows
    /// nothing there. Before, window A's question confirmed from window B
    /// handed B's tab on.
    #[cfg(feature = "links")]
    #[test]
    fn a_question_is_answered_only_in_its_own_window() {
        use interface_core::canvas::PageKey;
        use neoteletext::links::{Channel, DWELL_MILLISECONDS, Follow};
        use protocol::links::{Candidate, Fastext, Found, Spot};
        let mut test = Test::new("question-window");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let first = test.core.session.window;
        let channel = test.core.session.tabs[0].channel.id;
        let key = PageKey { channel, page: 0x100 };
        let address = test.core.session.tabs[0].address().unwrap().clone();
        let web = Candidate { row: 5, column: 0, length: 7, double: false, light: false, target: Found::Declared("web:https://www.example.invalid/".into()) };
        test.core.links.opened(Some(key));
        test.core.links.found(key, vec![web], Vec::new(), Fastext::default());
        test.core.links.image(key, 1, &Channel { id: channel, address: &address, exists: &|_| true }, 0);
        let spot = Spot { key, version: 1, column: 1, row: 5 };
        test.core.links.point(Some(spot));
        let Ok(Follow::Ask(prompt)) = test.core.links.click(spot, Some(channel), DWELL_MILLISECONDS) else { panic!("asked") };
        let second = test.core.session.new_window(false).unwrap();
        test.core.session.open("https://sport.invalid/teletext/").unwrap();
        test.core.switch_window(first);
        test.core.intent(ToCore::From(second)).unwrap();
        // Past the question's half second.
        std::thread::sleep(std::time::Duration::from_millis(neoteletext::links::PROMPT_DELAY_MILLISECONDS + DWELL_MILLISECONDS + 100));
        assert_eq!(test.core.intent(ToCore::Confirm(prompt.id)), Ok(true), "nothing handed on: with no broker, a hand-off fails");
        assert_eq!(test.core.links.prompt(), None, "the question went with its window");
    }

    /// Requirements 20.21 and 20.22: the session is written after every
    /// change of the tabs, with a visit's title once its page draws, and
    /// the window's place once the window is still.
    #[test]
    fn the_session_is_kept_after_every_change() {
        let mut test = Test::new("session");
        let kept = |test: &Test| Saved::load(&test.folder).unwrap().0.unwrap_or_default();
        test.core.session.open("news.invalid").unwrap();
        test.core.intent(ToCore::MuteTab(0)).unwrap();
        let saved = kept(&test);
        let tabs = &saved.windows[0].tabs;
        assert_eq!((tabs.len(), tabs[0].muted, tabs[0].entries[0].title.as_str()), (1, true, ""));
        let channel = test.core.session.tabs[0].channel.id;
        test.core.record_pending(channel).unwrap();
        assert_eq!(kept(&test).windows[0].tabs[0].entries[0].title, "news.invalid", "the title once the page drew");
        let window = WindowGeometry { x: 40, y: 50, width: 1200, height: 800, maximised: false };
        test.core.intent(ToCore::Window(window)).unwrap();
        assert_eq!(kept(&test).windows[0].geometry, None, "not while the window may still be moving");
        assert!(test.core.next_wake().is_some_and(|at| at <= std::time::Instant::now() + WINDOW_STILL));
        test.core.keep_window();
        assert_eq!(kept(&test).windows[0].geometry, Some(window));
        test.core.session.open_private("secret.invalid").unwrap();
        // Choosing a tab starts its decoder, which needs the broker.
        test.core.session.active = 1;
        test.core.keep_session();
        assert_eq!(kept(&test).tab_count(), 1, "a private tab is never kept");
    }
}
