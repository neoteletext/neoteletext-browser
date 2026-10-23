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
