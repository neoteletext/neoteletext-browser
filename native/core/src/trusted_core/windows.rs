//! Windows (`multiple-windows.md`): what each window set aside shows, the
//! window at hand, windows opened, closed and given a tab, and the session
//! kept after every change (requirement 20.21).

use neoteletext::frames;
use neoteletext::session_file::Saved;
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
    /// The open page's controls and the choice on its way, which are the
    /// window's own: one window's pages no longer change another's
    /// (DEC-6358, fault 8).
    controls: super::page_controls::State,
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
        // The strip's notice was the other window's, perhaps a private
        // one's, and is not said in this one.
        self.notice = None;
        let next = self.shown_elsewhere.remove(&id).unwrap_or_default();
        self.frames = next.frames;
        self.showing = next.showing;
        self.shown_pages = next.pages;
        self.controls.restore(next.controls);
        true
    }

    /// Sets aside what the window at hand, `id`, shows.
    fn park_shown(&mut self, id: u32) {
        let shown = Shown {
            frames: std::mem::take(&mut self.frames),
            showing: self.showing.take(),
            pages: std::mem::take(&mut self.shown_pages),
            controls: self.controls.park(),
        };
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
        self.open_window_at_or_home(private, &[])
    }

    /// A new normal window at `addresses`, for what opens from outside when
    /// every window is private.
    pub(super) fn open_window_at(&mut self, addresses: &[String]) -> Result<(), String> {
        self.open_window_at_or_home(false, addresses)
    }

    fn open_window_at_or_home(&mut self, private: bool, addresses: &[String]) -> Result<(), String> {
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
        for reason in self.session.open_at_end(addresses) {
            self.log(Level::Warning, None, &reason);
        }
        if self.session.tabs.is_empty() {
            let home = self.settings.home.clone();
            open_home(&mut self.session, &home)?;
        }
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
        // A tab keeps to its kind of window: a private tab's cookies,
        // history and session are kept apart from an ordinary one's
        // (finding F-83).
        if !into_new && self.session.others.iter().find(|other| other.id == window).is_some_and(|other| other.private != self.session.private_window) {
            return self.say(String::from("A tab cannot move between a private window and another"));
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

    /// DEC-6358, fault 8: the open page's controls are each window's own.
    /// A page open in one window is still that window's when another
    /// window has been at hand and has opened its own.
    #[test]
    fn each_window_keeps_its_own_open_page() {
        use interface_core::canvas::PageKey;
        let mut test = Test::new("windows controls");
        test.core.session.open("news.invalid").unwrap();
        let first = test.core.session.window;
        let news = PageKey { channel: test.core.active_channel().unwrap(), page: 0x100 };
        test.core.page_opened(Some(news)).unwrap();
        // A second window, made as the session makes one, then come to
        // as the core comes to a window.
        let second = test.core.session.new_window(false).unwrap();
        assert!(test.core.session.switch_to(first));
        assert!(test.core.switch_window(second));
        test.core.session.open("sport.invalid").unwrap();
        assert_eq!(test.core.page_open(), None, "the new window has no page open");
        let sport = PageKey { channel: test.core.active_channel().unwrap(), page: 0x300 };
        test.core.page_opened(Some(sport)).unwrap();
        assert!(test.core.switch_window(first));
        assert_eq!(test.core.page_open(), Some(news), "the first window's page, not the second's");
        assert!(test.core.switch_window(second));
        assert_eq!(test.core.page_open(), Some(sport));
    }

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
        use protocol::links::{Candidate, Fastext, Spot};
        let mut test = Test::new("question-window");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let first = test.core.session.window;
        let channel = test.core.session.tabs[0].channel.id;
        let key = PageKey { channel, page: 0x100 };
        let address = test.core.session.tabs[0].address().unwrap().clone();
        let web = Candidate { row: 5, column: 0, length: 7, double: false, light: false, target: "web:https://www.example.invalid/".into() };
        test.core.links.opened(Some(key));
        test.core.links.found(key, vec![web], Vec::new(), Vec::new(), Fastext::default());
        test.core.links.image(key, 1, &Channel { id: channel, address: &address }, 0);
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

    /// Finding F-83: a tab does not move between a private window and an
    /// ordinary one, either way: its cookies, history and kept session
    /// are its window's kind's.
    #[test]
    fn a_tab_stays_in_its_kind_of_window() {
        let mut test = Test::new("tab-kind");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        test.core.session.open("https://sport.invalid/teletext/").unwrap();
        let ordinary = test.core.session.window;
        let private = test.core.session.new_window(true).unwrap();
        test.core.session.open("https://weather.invalid/teletext/").unwrap();
        test.core.switch_window(ordinary);
        test.core.intent(ToCore::From(ordinary)).unwrap();
        // A move made would start the moved tab's helpers, which this
        // core, with no broker, cannot: only where the tabs are is looked at.
        let _ = test.core.intent(ToCore::TabToWindow { tab: 0, window: private, index: 0 });
        test.core.switch_window(ordinary);
        assert_eq!(test.core.session.tabs.len(), 2, "the ordinary tab stayed");
        test.core.intent(ToCore::From(private)).unwrap();
        let _ = test.core.intent(ToCore::TabToWindow { tab: 0, window: ordinary, index: 0 });
        test.core.switch_window(private);
        assert_eq!(test.core.session.tabs.len(), 1, "the private tab stayed");
    }

    /// Finding F-84: what a private tab does is logged in its own console,
    /// never in the browser-wide lines every window's console shows.
    #[test]
    fn a_private_tabs_log_lines_stay_in_its_window() {
        let mut test = Test::new("private-log");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let ordinary = test.core.session.window;
        test.core.session.new_window(true).unwrap();
        test.core.session.open_private("https://secret.invalid/teletext/").unwrap();
        let _ = test.core.intent(ToCore::Navigate("ftp://secret-typed.invalid/".into()));
        test.core.switch_window(ordinary);
        test.core.console.set_open(true);
        let shown: Vec<String> =
            test.core.console.updates(test.core.active_channel()).into_iter().flat_map(|update| update.entries).map(|entry| entry.text).collect();
        assert!(!shown.iter().any(|text| text.contains("ftp")), "{shown:?}");
        assert_eq!(test.core.strip().notice, "", "nor does its refusal's notice in the strip");
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

/// The whole window flow, from the interface's intents, against the
/// stand-in broker that starts and ends the windows' helpers.
#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod flow_tests {
    use super::super::test_support::{core_watched, sent_to_interface};
    use super::*;
    use confine::broker::stand_in::{StandIn, Started};
    use confine::broker::{Program, Request};

    /// What the stand-in broker reads as the first window's file channel.
    const FILE: &str = "/tmp/news.tti";
    /// A line of that file as the test's stand-in reads it
    /// (`test_support::core_writing_to`).
    const FILE_LINE: &[u8] = b"OL,1,A";
    /// Ceiling on the stand-in's account once the core has gone.
    const FINISH_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

    /// The helpers started since `seen` of them were, of `program`.
    fn started_since(stand_in: &StandIn, seen: &mut usize, program: Program) -> Vec<Started> {
        let started = stand_in.started();
        let new = started[*seen..].iter().filter(|helper| helper.program == program).cloned().collect();
        *seen = started.len();
        new
    }

    fn killed(requests: &[Request]) -> Vec<u32> {
        requests.iter().filter_map(|request| if let Request::Kill(id) = request { Some(*id) } else { None }).collect()
    }

    // REQ-760, DEC-965, TODO-1257: a window opened by the interface's intent
    // is told to the interface and starts a decoder for its tab; a tab moved
    // into it keeps its decoder, and the window it left, empty, closes; a
    // private window's decoder is never given another window's channel file;
    // and a window closed ends its tabs' decoders and fetchers.
    #[test]
    fn a_window_opened_moved_into_and_closed_starts_and_ends_its_helpers() {
        let (mut core, stand_in, written) = core_watched(FILE);
        let first = core.session.window;
        let file_channel = core.session.tabs[0].channel.id;
        let mut seen = 0;

        core.intent(ToCore::NewWindow { private: false }).unwrap();
        let second = core.session.window;
        assert_ne!(second, first);
        let sent = sent_to_interface(&mut core, &written);
        assert_eq!(sent.first(), Some(&ToInterface::OpenWindow { window: second, geometry: None }), "the interface opens it");
        assert!(sent.iter().any(|message| matches!(message, ToInterface::Tabs { tabs, .. } if tabs.len() == 1)), "and is sent its strip");
        let home_channel = core.session.tabs[0].channel.id;
        assert!(core.decoders.contains_key(&home_channel), "its tab's decoder");
        let decoders = started_since(&stand_in, &mut seen, Program::Decoder);
        assert_eq!(decoders.len(), 2, "the new window's tab's decoder, and the first window's, which the flush wakes: {decoders:?}");
        let _ = stand_in.take_requests();

        core.intent(ToCore::From(first)).unwrap();
        core.intent(ToCore::TabToWindow { tab: 0, window: second, index: 1 }).unwrap();
        let sent = sent_to_interface(&mut core, &written);
        assert!(sent.contains(&ToInterface::CloseWindow(first)), "the window left empty closes");
        assert_eq!(core.session.window_ids(), vec![second]);
        core.switch_window(second);
        let channels: Vec<u32> = core.session.tabs.iter().map(|tab| tab.channel.id).collect();
        assert_eq!(channels, [home_channel, file_channel], "the tab moved, its channel with it");
        assert!(core.decoders.contains_key(&file_channel), "its decoder kept");
        assert_eq!(started_since(&stand_in, &mut seen, Program::Decoder), [], "none started again");
        assert_eq!(killed(&stand_in.take_requests()), [] as [u32; 0], "none ended");

        core.intent(ToCore::NewWindow { private: true }).unwrap();
        let private = core.session.window;
        let private_channel = core.session.tabs[0].channel.id;
        let private_decoder = started_since(&stand_in, &mut seen, Program::Decoder);
        assert_eq!(private_decoder.len(), 1, "the private window's tab's decoder");
        assert!(core.fetchers.contains_key(&private_channel), "and its fetcher");
        let _ = stand_in.take_requests();

        core.intent(ToCore::From(private)).unwrap();
        core.intent(ToCore::CloseWindow).unwrap();
        assert!(!core.session.window_ids().contains(&private));
        assert!(!core.decoders.contains_key(&private_channel) && !core.fetchers.contains_key(&private_channel), "its helpers are ended");
        assert!(killed(&stand_in.take_requests()).contains(&private_decoder[0].id), "its decoder killed at once");
        assert!(core.decoders.contains_key(&file_channel) && core.decoders.contains_key(&home_channel), "the other window's are not");

        drop(core);
        let written = stand_in.finish(FINISH_WITHIN).unwrap();
        let given_the_file = |id: u32| {
            written.iter().find(|written| written.helper.id == id).is_some_and(|written| written.bytes.windows(FILE_LINE.len()).any(|bytes| bytes == FILE_LINE))
        };
        assert!(!given_the_file(private_decoder[0].id), "the private window's decoder was never given the file");
        assert!(decoders.iter().any(|decoder| given_the_file(decoder.id)), "the file's own decoder was");
    }
}
