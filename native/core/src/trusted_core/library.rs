//! History and bookmarks in the trusted core (`history-and-bookmarks.md`,
//! requirements 20.1 to 20.20): recording visits as their pages draw, the
//! panels' views and their requests, suggestions, the Back and Forward
//! lists, the star, the bar, and the files. Every request is strip input:
//! a row is acted on only by the number this core gave it in the view it
//! last sent (requirement 20.5).

use confine::LocalTime;
use neoteletext::bookmarks::Removed;
use neoteletext::calendar::{local_day, utc_now};
use neoteletext::history::{History, Visit};
use protocol::ToCore;
use protocol::clean;
use protocol::console::Level;
use protocol::library::{MAX_NOTICE, MAX_SHOWN_ADDRESS};

use super::Core;
use super::numbers::Numbers;

/// What ends an address cut to fit a view.
const ELLIPSIS: char = '\u{2026}';

/// The bookmarks manager's place.
#[derive(Debug)]
struct Manager {
    folder: u32,
    query: String,
    folders_first: usize,
    items_first: usize,
}

/// History and bookmarks' state in the trusted core.
#[derive(Debug, Default)]
pub struct Library {
    pub history: History,
    /// The history panel, while open: its search and first row.
    history_panel: Option<(String, usize)>,
    history_numbers: Numbers<(i64, String)>,
    closed_numbers: Numbers<u32>,
    history_notice: String,
    suggestion_numbers: Numbers<String>,
    manager: Option<Manager>,
    manager_numbers: Numbers<u32>,
    undo: Option<Removed>,
    manager_notice: String,
    /// The bookmark the star's popover shows, while open.
    star: Option<u32>,
    star_numbers: Numbers<u32>,
    /// The folder the star last put a bookmark in.
    last_folder: Option<u32>,
    /// The bar's folder opened as a list.
    bar_open: Option<u32>,
    bar_numbers: Numbers<u32>,
    /// The local day history was last expired on.
    expired_day: Option<i64>,
}

/// An address cut to fit a view; a row is opened by its number, never by
/// this text.
fn shown_address(address: &str) -> String {
    let cleaned = clean(address, usize::MAX);
    if cleaned.len() <= MAX_SHOWN_ADDRESS {
        return cleaned;
    }
    let mut cut = clean(&cleaned, MAX_SHOWN_ADDRESS.saturating_sub(ELLIPSIS.len_utf8()));
    cut.push(ELLIPSIS);
    cut
}

fn notice(text: &str) -> String {
    clean(text, MAX_NOTICE)
}

fn depth_of(depth: usize) -> u8 {
    u8::try_from(depth).unwrap_or(u8::MAX)
}

impl Core {
    /// The strip input of history and bookmarks.
    pub(crate) fn library_intent(&mut self, intent: ToCore) -> Result<(), String> {
        match intent {
            ToCore::History(request) => self.history_request(request),
            ToCore::Bookmarks(request) => self.bookmark_request(request),
            ToCore::Star(request) => self.star_request(request),
            ToCore::Bar(request) => self.bar_request(&request),
            ToCore::Suggest(text) => self.suggest(&text),
            ToCore::Suggestion { number, new_tab } => self.open_suggestion(number, new_tab),
            ToCore::TabList(forward) => self.send_tab_list(forward),
            ToCore::GoTo { forward, steps } => self.go_to(forward, steps),
            ToCore::NewPrivateTab => {
                let home = self.settings.home.clone();
                if let Err(reason) = self.session.open_private(&home) {
                    self.log(Level::Warning, None, &format!("{home}: {reason}"));
                    self.session.open_private(neoteletext::address::START_PAGE)?;
                }
                self.send_all()
            }
            _ => Ok(()),
        }
    }

    /// The time and the local zone's offset, from the broker; a test's core,
    /// with none, has UTC.
    pub(crate) fn local_time(&self) -> LocalTime {
        let utc = utc_now;
        match self.broker.as_ref().map(|broker| broker.local_time()) {
            Some(Ok(time)) => time,
            Some(Err(reason)) => {
                eprintln!("neoteletext: the local time is not known, so history uses UTC: {reason}");
                LocalTime { utc: utc(), offset: 0 }
            }
            None => LocalTime { utc: utc(), offset: 0 },
        }
    }

    /// A frame of `channel` was drawn: a tab of it waiting to record its
    /// visit records it now (requirement 20.1).
    pub(crate) fn record_pending(&mut self, channel: u32) -> Result<(), String> {
        let index = self.session.tabs.iter().position(|tab| tab.channel.id == channel && tab.pending.is_some());
        let Some(index) = index else { return Ok(()) };
        self.record(index)?;
        // The visit's title is kept with the session.
        self.keep_session();
        Ok(())
    }

    /// Tabs whose channel was already drawing record their visit at once.
    pub(crate) fn record_drawn(&mut self) -> Result<(), String> {
        let drawn: Vec<usize> =
            self.session.tabs.iter().enumerate().filter(|(_, tab)| tab.pending.is_some_and(|pending| pending.drawn)).map(|(index, _)| index).collect();
        for index in drawn {
            self.record(index)?;
        }
        Ok(())
    }

    /// Records tab `index`'s pending visit, unless the tab is private, the
    /// page is the browser's own, or history is not remembered.
    fn record(&mut self, index: usize) -> Result<(), String> {
        let Some(tab) = self.session.tabs.get_mut(index) else { return Ok(()) };
        let Some(pending) = tab.pending.take() else { return Ok(()) };
        let Some(address) = tab.address().cloned() else { return Ok(()) };
        let channel = tab.channel.title.clone();
        let title = address.page.and_then(|page| tab.channel.page_title(page)).filter(|title| !title.is_empty()).map_or_else(|| channel.clone(), str::to_owned);
        if let Some(visit) = tab.history.get_mut(tab.position) {
            visit.title.clone_from(&title);
        }
        if tab.private || address.is_start_page() {
            return Ok(());
        }
        let time = self.local_time();
        self.expire_history(local_day(time.utc, time.offset));
        let visit = Visit { address: address.text.clone(), channel, title, typed: pending.typed, reload: pending.reload };
        if let Some(line) = self.library.history.record(&visit, time.utc, time.offset)
            && let Some(folder) = self.settings_folder.clone()
        {
            if let Err(error) = History::append(&folder, &line) {
                self.log(Level::Error, None, &format!("history could not be written in {}: {error}", folder.display()));
            }
            if self.library.history.wants_compacting() {
                self.save_history();
            }
        }
        self.send_history()
    }

    /// Removes history older than 90 days, once a day.
    fn expire_history(&mut self, today: i64) {
        if self.library.expired_day == Some(today) {
            return;
        }
        self.library.expired_day = Some(today);
        if self.library.history.expire(today) {
            self.save_history();
        }
    }

    /// Writes history whole; a failure is logged.
    fn save_history(&mut self) {
        let Some(folder) = self.settings_folder.clone() else { return };
        if let Err(error) = self.library.history.save(&folder) {
            self.log(Level::Error, None, &format!("history could not be written in {}: {error}", folder.display()));
        }
    }

    /// Writes the bookmarks whole; a failure is logged and said.
    fn save_bookmarks(&mut self) {
        let Some(folder) = self.settings_folder.clone() else { return };
        if let Err(error) = self.session.bookmarks.save(&folder) {
            self.log(Level::Error, None, &format!("the bookmarks could not be written in {}: {error}", folder.display()));
            self.library.manager_notice = notice("The bookmarks could not be saved");
        }
    }

    /// Opens `address` in the active tab or a new one, as the user's own
    /// navigation.
    fn open_address(&mut self, address: &str, new_tab: bool) -> Result<(), String> {
        let opened = if new_tab { self.session.open(address).map(|_| ()) } else { self.session.navigate(address, false) };
        if let Err(reason) = opened {
            self.log(Level::Warning, None, &format!("{address}: {reason}"));
        }
        self.send_all()
    }
}

mod bar;
mod bookmarks;
mod history;
mod star;

#[cfg(test)]
mod tests;
