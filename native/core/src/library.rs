//! History and bookmarks in the trusted core (`history-and-bookmarks.md`,
//! requirements 20.1 to 20.20): recording visits as their pages draw, the
//! panels' views and their requests, suggestions, the Back and Forward
//! lists, the star, the bar, and the files. Every request is strip input:
//! a row is acted on only by the number this core gave it in the view it
//! last sent (requirement 20.5).

use std::collections::BTreeMap;

use confine::LocalTime;
use neoteletext::bookmarks::{BAR, Bookmarks, OTHER, Removed};
use neoteletext::calendar::{day_text, day_words, local_day, time_words};
use neoteletext::history::{DAY, FIFTEEN_MINUTES, FOUR_WEEKS, HOUR, History, Range, Visit, WEEK};
use protocol::library::{
    BarRequest, BarView, BookmarkRequest, BookmarksView, ClearRange, ClosedTab, FolderRow, HistoryRequest, HistoryRow, HistoryView, ItemRow, MAX_CLOSED,
    MAX_LABEL, MAX_NAME, MAX_NOTICE, MAX_SHOWN_ADDRESS, MAX_SUGGESTIONS, MAX_VIEW_ROWS, StarRequest, StarView, Suggestion, Suggestions, TabEntry, TabList,
};
use protocol::{MAX_TITLE, clean};
use protocol::{ToCore, ToInterface};

use crate::Core;
use protocol::console::Level;
use protocol::decoder::BookmarkEntry;

/// What separates a folder's names in a search result's place.
const PLACE_SEPARATOR: &str = " / ";
/// What ends an address cut to fit a view.
const ELLIPSIS: char = '\u{2026}';

/// The numbers of one view: each names one thing of the view this core
/// last sent, and a number from an earlier view names nothing (requirement
/// 20.5). Numbers count on across views, so none is reused.
#[derive(Debug)]
pub struct Numbers<T> {
    next: u32,
    map: BTreeMap<u32, T>,
}

impl<T> Default for Numbers<T> {
    fn default() -> Self {
        Self { next: 1, map: BTreeMap::new() }
    }
}

impl<T> Numbers<T> {
    /// A new view: the last view's numbers name nothing now.
    pub(crate) fn fresh(&mut self) {
        self.map.clear();
    }

    pub(crate) fn number(&mut self, value: T) -> u32 {
        let number = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        self.map.insert(number, value);
        number
    }

    pub(crate) fn get(&self, number: u32) -> Option<&T> {
        self.map.get(&number)
    }
}

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
        let utc = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|since| i64::try_from(since.as_secs()).ok()).unwrap_or(0);
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

    // ---- History ----

    /// What the history panel asks for (requirements 20.4 to 20.9).
    pub(crate) fn history_request(&mut self, request: HistoryRequest) -> Result<(), String> {
        match request {
            HistoryRequest::Panel(true) => {
                self.library.history_panel = Some((String::new(), 0));
                self.library.history_notice.clear();
                let time = self.local_time();
                self.expire_history(local_day(time.utc, time.offset));
            }
            HistoryRequest::Panel(false) => {
                self.library.history_panel = None;
                self.library.history_numbers.fresh();
                self.library.closed_numbers.fresh();
                return Ok(());
            }
            HistoryRequest::Query { text, first } => {
                self.library.history_panel = Some((clean(&text, protocol::library::MAX_QUERY), usize::try_from(first).unwrap_or(usize::MAX)));
            }
            // Chrome's "Remove bookmark": every bookmark of the row's address.
            HistoryRequest::RemoveBookmark(number) => {
                let Some((_, address)) = self.library.history_numbers.get(number).cloned() else { return self.send_history() };
                let mut removed = 0_usize;
                while let Some(id) = self.session.bookmarks.find(&address) {
                    if self.session.bookmarks.remove(id).is_none() {
                        break;
                    }
                    removed = removed.saturating_add(1);
                }
                self.library.history_notice = notice(if removed == 1 { "Bookmark removed" } else { "Bookmarks removed" });
                self.bookmarks_changed()?;
            }
            // Chrome's "More from this site": the panel searches the row's host.
            HistoryRequest::MoreFromSite(number) => {
                let host =
                    self.library.history_numbers.get(number).and_then(|(_, address)| neoteletext::address::parse(address).ok()).map(|address| address.host);
                if let Some(host) = host {
                    let text = clean(&format!("{}{host}", neoteletext::history::HOST_SEARCH), protocol::library::MAX_QUERY);
                    self.library.history_panel = Some((text, 0));
                }
            }
            HistoryRequest::Open { number, new_tab } => {
                let Some((_, address)) = self.library.history_numbers.get(number).cloned() else { return self.send_history() };
                return self.open_address(&address, new_tab);
            }
            HistoryRequest::Remove(numbers) => {
                let keys: Vec<(i64, String)> = numbers.iter().filter_map(|number| self.library.history_numbers.get(*number).cloned()).collect();
                let removed = self.library.history.remove(&keys);
                self.remove_entries_of(&removed)?;
                self.save_history();
                self.library.history_notice = notice(&format!("{} removed from history", removed.len()));
            }
            HistoryRequest::Clear(range) => {
                let now = self.local_time().utc;
                let range = match range {
                    ClearRange::FifteenMinutes => History::since(now, FIFTEEN_MINUTES),
                    ClearRange::Hour => History::since(now, HOUR),
                    ClearRange::Day => History::since(now, DAY),
                    ClearRange::Week => History::since(now, WEEK),
                    ClearRange::FourWeeks => History::since(now, FOUR_WEEKS),
                    ClearRange::All => Range::All,
                };
                let removed = self.library.history.clear(range);
                // The tabs' and closed tabs' entries of that time go too.
                let changed = match range {
                    Range::All => self.session.remove_entries(|_| true),
                    Range::Since(since) => self.session.remove_entries(|visit| visit.at >= since),
                };
                if changed {
                    self.send(&ToInterface::Strip(self.strip()))?;
                }
                self.save_history();
                self.library.history_notice = notice(&format!("History cleared: {removed} removed"));
            }
            HistoryRequest::ClearCookies(range) => {
                let now = self.local_time().utc;
                let since = match range {
                    ClearRange::FifteenMinutes => Some(now.saturating_sub(FIFTEEN_MINUTES)),
                    ClearRange::Hour => Some(now.saturating_sub(HOUR)),
                    ClearRange::Day => Some(now.saturating_sub(DAY)),
                    ClearRange::Week => Some(now.saturating_sub(WEEK)),
                    ClearRange::FourWeeks => Some(now.saturating_sub(FOUR_WEEKS)),
                    ClearRange::All => None,
                };
                let removed = self.clear_cookies_since(since);
                self.library.history_notice = notice(&format!("Cookies cleared: {removed} removed"));
                self.send_cookies()?;
            }
            HistoryRequest::Reopen(number) => {
                let id = match number {
                    Some(number) => match self.library.closed_numbers.get(number) {
                        Some(&id) => Some(id),
                        None => return self.send_history(),
                    },
                    None => None,
                };
                if self.session.reopen(id).is_ok() {
                    self.send_all()?;
                }
            }
        }
        self.send_history()
    }

    /// The history panel's view, while it is open.
    pub(crate) fn send_history(&mut self) -> Result<(), String> {
        let Some((query, first)) = self.library.history_panel.clone() else { return Ok(()) };
        let time = self.local_time();
        let today = local_day(time.utc, time.offset);
        self.library.history_numbers.fresh();
        self.library.closed_numbers.fresh();
        let matching = self.library.history.query(&query);
        let total = matching.len();
        let first = first.min(total);
        let mut rows = Vec::new();
        let mut last_day = None;
        for row in matching.into_iter().skip(first).take(MAX_VIEW_ROWS) {
            let heading = (last_day != Some(row.day)).then(|| {
                let words = day_words(row.day);
                let heading = if row.day == today {
                    format!("Today: {words}")
                } else if row.day == today.saturating_sub(1) {
                    format!("Yesterday: {words}")
                } else {
                    words
                };
                clean(&heading, MAX_LABEL)
            });
            last_day = Some(row.day);
            let page = neoteletext::address::parse(&row.address).ok().and_then(|address| address.page);
            rows.push(HistoryRow {
                number: 0,
                heading,
                time: clean(&time_words(row.last, row.offset), MAX_LABEL),
                channel: clean(&row.channel, MAX_NAME),
                page,
                title: clean(&row.title, MAX_NAME.max(protocol::MAX_PAGE_TITLE)),
                address: shown_address(&row.address),
                bookmarked: self.session.bookmarks.find(&row.address).is_some(),
            });
            if let Some(shown) = rows.last_mut() {
                shown.number = self.library.history_numbers.number((row.day, row.address.clone()));
            }
        }
        let closed: Vec<(u32, String)> = self.session.closed.iter().take(MAX_CLOSED).map(|closed| (closed.id, clean(&closed.title, MAX_NAME))).collect();
        let closed = closed.into_iter().map(|(id, title)| ClosedTab { number: self.library.closed_numbers.number(id), title }).collect();
        let notice_text = self.library.history_notice.clone();
        let view = HistoryView {
            query,
            first: u32::try_from(first).unwrap_or(u32::MAX),
            total: u32::try_from(total).unwrap_or(u32::MAX),
            rows,
            closed,
            notice: notice_text,
        };
        self.send(&ToInterface::History(view))
    }

    /// Suggestions for what is typed in the address field (requirement 20.11).
    pub(crate) fn suggest(&mut self, text: &str) -> Result<(), String> {
        self.library.suggestion_numbers.fresh();
        let typed = text.trim();
        let mut rows = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        if !typed.is_empty() {
            for id in self.session.bookmarks.search(typed) {
                let Some(item) = self.session.bookmarks.item(id) else { continue };
                let Some(address) = item.address.clone() else { continue };
                if rows.len() >= MAX_SUGGESTIONS || seen.contains(&address) {
                    continue;
                }
                let title = clean(&item.name, MAX_NAME);
                seen.push(address.clone());
                rows.push(Suggestion {
                    number: self.library.suggestion_numbers.number(address.clone()),
                    address: shown_address(&address),
                    title,
                    bookmarked: true,
                });
            }
            for row in self.library.history.suggest(typed, MAX_SUGGESTIONS) {
                if rows.len() >= MAX_SUGGESTIONS || seen.contains(&row.address) {
                    continue;
                }
                seen.push(row.address.clone());
                let title = clean(&row.title, MAX_NAME);
                rows.push(Suggestion {
                    number: self.library.suggestion_numbers.number(row.address.clone()),
                    address: shown_address(&row.address),
                    title,
                    bookmarked: false,
                });
            }
        }
        let inline = self.library.history.completion(typed).map(|inline| clean(&inline, protocol::MAX_ADDRESS)).unwrap_or_default();
        self.send(&ToInterface::Suggestions(Suggestions { text: clean(text, protocol::MAX_ADDRESS), inline, rows }))
    }

    /// A suggestion chosen, by its number.
    pub(crate) fn open_suggestion(&mut self, number: u32, new_tab: bool) -> Result<(), String> {
        let Some(address) = self.library.suggestion_numbers.get(number).cloned() else { return Ok(()) };
        self.library.suggestion_numbers.fresh();
        self.open_address(&address, new_tab)
    }

    /// The active tab's Back or Forward list (requirement 20.10).
    pub(crate) fn send_tab_list(&mut self, forward: bool) -> Result<(), String> {
        let entry = |(steps, visit): (usize, &crate::session::Visit)| {
            let title = if visit.title.is_empty() { visit.address.host.clone() } else { visit.title.clone() };
            Some(TabEntry {
                steps: u8::try_from(steps).ok()?,
                page: visit.address.page,
                title: clean(&title, MAX_NAME),
                address: shown_address(&visit.address.text),
            })
        };
        let entries = self.session.tab_list(forward).into_iter().filter_map(entry).collect();
        let chapters = self.session.chapter_stops(forward).into_iter().filter_map(entry).collect();
        self.send(&ToInterface::TabList(TabList { forward, entries, chapters }))
    }

    /// After rows were removed: the addresses no day has a row of any more
    /// leave the tabs' and closed tabs' lists, as Chrome removes the
    /// entries of the addresses whose history is gone.
    fn remove_entries_of(&mut self, removed: &[String]) -> Result<(), String> {
        let gone: Vec<&String> = removed.iter().filter(|address| !self.library.history.has_address(address)).collect();
        if !gone.is_empty() && self.session.remove_entries(|visit| gone.contains(&&visit.address.text)) {
            self.send(&ToInterface::Strip(self.strip()))?;
        }
        Ok(())
    }

    /// Back or Forward several steps at once, from a list.
    pub(crate) fn go_to(&mut self, forward: bool, steps: u8) -> Result<(), String> {
        let went = self.session.go_to(forward, usize::from(steps));
        self.went(went)
    }

    // ---- Bookmarks ----

    /// After any change of the bookmarks: kept, and every view of them
    /// sent afresh.
    fn bookmarks_changed(&mut self) -> Result<(), String> {
        self.save_bookmarks();
        self.others_stale();
        self.send(&ToInterface::Strip(self.strip()))?;
        self.send_bar()?;
        self.send_star()?;
        self.send_bookmarks()
    }

    /// What the bookmarks manager asks for (requirement 20.17).
    pub(crate) fn bookmark_request(&mut self, request: BookmarkRequest) -> Result<(), String> {
        let item_of = |core: &Self, number: u32| core.library.manager_numbers.get(number).copied().filter(|id| core.session.bookmarks.item(*id).is_some());
        match request {
            BookmarkRequest::Panel(true) => {
                self.library.manager = Some(Manager { folder: BAR, query: String::new(), folders_first: 0, items_first: 0 });
                self.library.manager_notice.clear();
            }
            BookmarkRequest::Panel(false) => {
                self.library.manager = None;
                self.library.manager_numbers.fresh();
                return Ok(());
            }
            BookmarkRequest::Folder(number) => {
                if let (Some(id), Some(manager)) = (item_of(self, number), self.library.manager.as_mut())
                    && self.session.bookmarks.item(id).is_some_and(neoteletext::bookmarks::Item::is_folder)
                {
                    *manager = Manager { folder: id, query: String::new(), folders_first: manager.folders_first, items_first: 0 };
                }
            }
            BookmarkRequest::Scroll { folders_first, items_first } => {
                if let Some(manager) = self.library.manager.as_mut() {
                    manager.folders_first = usize::try_from(folders_first).unwrap_or(usize::MAX);
                    manager.items_first = usize::try_from(items_first).unwrap_or(usize::MAX);
                }
            }
            BookmarkRequest::Query(text) => {
                if let Some(manager) = self.library.manager.as_mut() {
                    manager.query = clean(&text, protocol::library::MAX_QUERY);
                    manager.items_first = 0;
                }
            }
            BookmarkRequest::Open { number, new_tab } => {
                let address = item_of(self, number).and_then(|id| self.session.bookmarks.item(id)).and_then(|item| item.address.clone());
                if let Some(address) = address {
                    return self.open_address(&address, new_tab);
                }
            }
            BookmarkRequest::Edit { number, name, address } => {
                if let Some(id) = item_of(self, number)
                    && let Err(reason) = self.session.bookmarks.edit(id, &name, &address)
                {
                    self.library.manager_notice = notice(&reason);
                }
            }
            BookmarkRequest::Move { number, folder, index } => {
                if let (Some(id), Some(folder)) = (item_of(self, number), item_of(self, folder))
                    && let Err(reason) = self.session.bookmarks.move_to(id, folder, usize::try_from(index).unwrap_or(usize::MAX))
                {
                    self.library.manager_notice = notice(&reason);
                }
            }
            BookmarkRequest::NewFolder(name) => {
                let folder = self.library.manager.as_ref().map_or(OTHER, |manager| manager.folder);
                if let Err(reason) = self.session.bookmarks.add_folder(folder, &name) {
                    self.library.manager_notice = notice(&reason);
                }
            }
            BookmarkRequest::Add { name, address } => {
                let folder = self.library.manager.as_ref().map_or(OTHER, |manager| manager.folder);
                if let Err(reason) = self.session.bookmarks.add(folder, &name, &address) {
                    self.library.manager_notice = notice(&reason);
                }
            }
            BookmarkRequest::Remove(number) => {
                if let Some(removed) = item_of(self, number).and_then(|id| self.session.bookmarks.remove(id)) {
                    self.library.manager_notice.clear();
                    self.library.undo = Some(removed);
                }
            }
            BookmarkRequest::Undo => {
                if let Some(removed) = self.library.undo.take()
                    && !self.session.bookmarks.restore(removed)
                {
                    self.library.manager_notice = notice("It cannot be put back: its folder is gone");
                }
            }
            BookmarkRequest::Sort => {
                let folder = self.library.manager.as_ref().map_or(OTHER, |manager| manager.folder);
                self.session.bookmarks.sort(folder);
            }
            BookmarkRequest::Import(path) => self.import_bookmarks(&path),
            BookmarkRequest::Export => self.export_bookmarks(),
        }
        self.bookmarks_changed()
    }

    /// Reads bookmarks in this browser's format from a file the user named,
    /// through the broker as a file channel is read (decision 56), into
    /// "Imported".
    fn import_bookmarks(&mut self, path: &str) {
        let read = match self.broker() {
            Ok(broker) => broker.read_source(std::path::Path::new(path.trim())),
            Err(reason) => Err(reason),
        };
        self.library.manager_notice = match read {
            // Another browser's file: read by a decoder, never here
            // (requirement 3.2, decision 71).
            Ok(bytes) if bytes.len() <= neoteletext::bookmarks::MAX_FILE && protocol::decoder::is_netscape_bookmarks(&bytes) => {
                match self.broker().and_then(|broker| crate::start_bookmark_reader(broker, bytes, self.events.clone())) {
                    Ok(()) => notice("Reading another browser's bookmarks…"),
                    Err(reason) => notice(&format!("Not imported: {reason}")),
                }
            }
            Ok(bytes) if bytes.len() <= neoteletext::bookmarks::MAX_FILE => {
                let (read, dropped) = Bookmarks::read(&String::from_utf8_lossy(&bytes));
                if read.is_empty() {
                    notice("No bookmarks in that file: it is not one this browser wrote")
                } else {
                    match self.session.bookmarks.import(&read) {
                        Ok(count) if dropped.is_empty() => notice(&format!("{count} imported into Imported")),
                        Ok(count) => notice(&format!("{count} imported into Imported; {} lines of the file were not bookmarks", dropped.len())),
                        Err(reason) => notice(&reason),
                    }
                }
            }
            Ok(_) => notice("That file is too large to be bookmarks"),
            Err(reason) => notice(&format!("Not imported: {reason}")),
        };
    }

    /// What a decoder read from another browser's bookmark file, into
    /// "Imported" (decision 71).
    pub(crate) fn bookmarks_read(&mut self, read: Result<(Vec<BookmarkEntry>, u32), String>) -> Result<(), String> {
        self.library.manager_notice = match read {
            Ok((entries, skipped)) => {
                let (read, refused) = Bookmarks::from_entries(&entries);
                let left_out = usize::try_from(skipped).unwrap_or(usize::MAX).saturating_add(refused);
                if read.is_empty() {
                    notice("No bookmarks in that file that this browser can open")
                } else {
                    match self.session.bookmarks.import(&read) {
                        Ok(count) if left_out == 0 => notice(&format!("{count} imported into Imported")),
                        Ok(count) => notice(&format!("{count} imported into Imported; {left_out} left out, not channel addresses or too deep")),
                        Err(reason) => notice(&reason),
                    }
                }
            }
            Err(reason) => notice(&format!("Not imported: {reason}")),
        };
        self.bookmarks_changed()
    }

    /// Writes the bookmarks to a new file beside the browser's own.
    fn export_bookmarks(&mut self) {
        let Some(folder) = self.settings_folder.clone() else {
            self.library.manager_notice = notice("There is no folder for the settings to export into");
            return;
        };
        let time = self.local_time();
        self.library.manager_notice = match self.session.bookmarks.export(&folder, &day_text(local_day(time.utc, time.offset))) {
            Ok(path) => notice(&format!("Exported to {}", path.display())),
            Err(error) => notice(&format!("Not exported: {error}")),
        };
    }

    fn item_row(&mut self, id: u32, with_place: bool) -> Option<ItemRow> {
        let item = self.session.bookmarks.item(id)?.clone();
        let place = if with_place {
            let names: Vec<String> = self
                .session
                .bookmarks
                .path(item.parent)
                .iter()
                .filter_map(|folder| self.session.bookmarks.item(*folder).map(|folder| folder.name.clone()))
                .collect();
            clean(&names.join(PLACE_SEPARATOR), MAX_LABEL)
        } else {
            String::new()
        };
        Some(ItemRow {
            number: self.library.manager_numbers.number(id),
            folder: item.is_folder(),
            name: clean(&item.name, MAX_NAME),
            address: item.address.as_deref().map(shown_address).unwrap_or_default(),
            place,
        })
    }

    /// The bookmarks manager's view, while it is open.
    pub(crate) fn send_bookmarks(&mut self) -> Result<(), String> {
        let Some(manager) = &self.library.manager else { return Ok(()) };
        let (folder, query, folders_first, items_first) = (manager.folder, manager.query.clone(), manager.folders_first, manager.items_first);
        // A folder removed while open: the manager goes back to the bar.
        let folder = if self.session.bookmarks.item(folder).is_some() { folder } else { BAR };
        if let Some(manager) = self.library.manager.as_mut() {
            manager.folder = folder;
        }
        self.library.manager_numbers.fresh();
        let path: Vec<FolderRow> = self
            .session
            .bookmarks
            .path(folder)
            .into_iter()
            .enumerate()
            .filter_map(|(depth, id)| {
                let name = clean(&self.session.bookmarks.item(id)?.name, MAX_NAME);
                Some(FolderRow { number: self.library.manager_numbers.number(id), name, depth: depth_of(depth) })
            })
            .collect();
        let all_folders = self.session.bookmarks.folders();
        let folders_total = all_folders.len();
        let folders_first = folders_first.min(folders_total);
        let folders = all_folders
            .into_iter()
            .skip(folders_first)
            .take(MAX_VIEW_ROWS)
            .filter_map(|(id, depth)| {
                let name = clean(&self.session.bookmarks.item(id)?.name, MAX_NAME);
                Some(FolderRow { number: self.library.manager_numbers.number(id), name, depth: depth_of(depth) })
            })
            .collect();
        let ids: Vec<u32> = if query.is_empty() { self.session.bookmarks.children(folder).to_vec() } else { self.session.bookmarks.search(&query) };
        let items_total = ids.len();
        let items_first = items_first.min(items_total);
        let items = ids.into_iter().skip(items_first).take(MAX_VIEW_ROWS).filter_map(|id| self.item_row(id, !query.is_empty())).collect();
        let undo = self.library.undo.as_ref().map(|removed| notice(&format!("Removed {}", removed.name))).unwrap_or_default();
        let view = BookmarksView {
            path,
            folders,
            folders_first: u32::try_from(folders_first).unwrap_or(u32::MAX),
            folders_total: u32::try_from(folders_total).unwrap_or(u32::MAX),
            query,
            items,
            items_first: u32::try_from(items_first).unwrap_or(u32::MAX),
            items_total: u32::try_from(items_total).unwrap_or(u32::MAX),
            undo,
            notice: self.library.manager_notice.clone(),
        };
        self.send(&ToInterface::Bookmarks(view))
    }

    /// The star, or its shortcut (requirement 20.16): the address the strip
    /// shows is bookmarked, or its bookmark shown, in the popover.
    pub(crate) fn star(&mut self) -> Result<(), String> {
        let Some(tab) = self.session.active_tab() else { return Ok(()) };
        let Some(address) = tab.address().cloned() else { return Ok(()) };
        if let Some(id) = self.session.bookmarks.find(&address.text) {
            self.library.star = Some(id);
            return self.send_star();
        }
        let page = address.page.and_then(|page| tab.channel.page_title(page).map(|title| format!("{page:03X} {title}")));
        let name = match page {
            Some(page) => format!("{} {page}", tab.channel.title),
            None => tab.channel.title.clone(),
        };
        let folder = self.library.last_folder.filter(|folder| self.session.bookmarks.item(*folder).is_some()).unwrap_or(OTHER);
        match self.session.bookmarks.add(folder, &clean(&name, MAX_TITLE), &address.text) {
            Ok(id) => {
                self.library.star = Some(id);
                self.bookmarks_changed()
            }
            Err(reason) => self.say(reason),
        }
    }

    /// What the star's popover asks for.
    pub(crate) fn star_request(&mut self, request: StarRequest) -> Result<(), String> {
        let Some(id) = self.library.star.filter(|id| self.session.bookmarks.item(*id).is_some()) else {
            self.library.star = None;
            return self.send_star();
        };
        match request {
            StarRequest::Edit { name, folder } => {
                let _ = self.session.bookmarks.edit(id, &name, "");
                if let Some(&folder) = self.library.star_numbers.get(folder)
                    && self.session.bookmarks.item(id).is_some_and(|item| item.parent != folder)
                    && self.session.bookmarks.move_to(id, folder, usize::MAX).is_ok()
                {
                    self.library.last_folder = Some(folder);
                }
            }
            StarRequest::WholeChannel => {
                let channel = self
                    .session
                    .bookmarks
                    .item(id)
                    .and_then(|item| item.address.as_deref())
                    .and_then(|address| neoteletext::address::parse(address).ok())
                    .map(|address| address.channel_text().to_owned());
                let name = self.session.bookmarks.item(id).map(|item| item.name.clone()).unwrap_or_default();
                if let Some(channel) = channel {
                    let _ = self.session.bookmarks.edit(id, &name, &channel);
                }
            }
            StarRequest::Remove => {
                let _ = self.session.bookmarks.remove(id);
                self.library.star = None;
            }
            StarRequest::Done => self.library.star = None,
        }
        self.bookmarks_changed()
    }

    /// The star's popover.
    pub(crate) fn send_star(&mut self) -> Result<(), String> {
        self.library.star_numbers.fresh();
        let item = self.library.star.and_then(|id| self.session.bookmarks.item(id)).cloned();
        let Some(item) = item else {
            self.library.star = None;
            return self.send(&ToInterface::Star(StarView::default()));
        };
        let folders: Vec<FolderRow> = self
            .session
            .bookmarks
            .folders()
            .into_iter()
            .take(MAX_VIEW_ROWS)
            .filter_map(|(id, depth)| {
                let name = clean(&self.session.bookmarks.item(id)?.name, MAX_NAME);
                Some(FolderRow { number: self.library.star_numbers.number(id), name, depth: depth_of(depth) })
            })
            .collect();
        let folder = folders.iter().find(|row| self.library.star_numbers.get(row.number) == Some(&item.parent)).map_or(0, |row| row.number);
        let page = item.address.as_deref().and_then(|address| neoteletext::address::parse(address).ok()).is_some_and(|address| address.page.is_some());
        self.send(&ToInterface::Star(StarView { open: true, name: clean(&item.name, MAX_NAME), folder, folders, page }))
    }

    /// What the bookmarks bar asks for (requirement 20.18).
    pub(crate) fn bar_request(&mut self, request: &BarRequest) -> Result<(), String> {
        match *request {
            BarRequest::Show(shown) => {
                self.settings.bar = shown;
                self.save_settings();
            }
            BarRequest::Folder(number) => {
                self.library.bar_open = number.and_then(|number| self.library.bar_numbers.get(number).copied());
            }
            BarRequest::Open { number, new_tab } => {
                let address = self.library.bar_numbers.get(number).and_then(|id| self.session.bookmarks.item(*id)).and_then(|item| item.address.clone());
                self.library.bar_open = None;
                if let Some(address) = address {
                    self.send_bar()?;
                    return self.open_address(&address, new_tab);
                }
            }
        }
        self.send_bar()
    }

    fn bar_row(&mut self, id: u32) -> Option<ItemRow> {
        let item = self.session.bookmarks.item(id)?.clone();
        Some(ItemRow {
            number: self.library.bar_numbers.number(id),
            folder: item.is_folder(),
            name: clean(&item.name, MAX_NAME),
            address: item.address.as_deref().map(shown_address).unwrap_or_default(),
            place: String::new(),
        })
    }

    /// The bookmarks bar.
    pub(crate) fn send_bar(&mut self) -> Result<(), String> {
        self.library.bar_numbers.fresh();
        let shown = self.settings.bar;
        let ids: Vec<u32> = if shown { self.session.bookmarks.children(BAR).iter().take(MAX_VIEW_ROWS).copied().collect() } else { Vec::new() };
        let items = ids.into_iter().filter_map(|id| self.bar_row(id)).collect();
        let open_folder = self.library.bar_open.filter(|id| shown && self.session.bookmarks.item(*id).is_some_and(neoteletext::bookmarks::Item::is_folder));
        let open = open_folder.map(|folder| {
            let ids: Vec<u32> = self.session.bookmarks.children(folder).iter().take(MAX_VIEW_ROWS).copied().collect();
            let number = self.library.bar_numbers.number(folder);
            (number, ids.into_iter().filter_map(|id| self.bar_row(id)).collect())
        });
        self.send(&ToInterface::Bar(BarView { shown, items, open }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;
    use crate::WINDOW_STILL;
    use crate::session::Session;
    use interface_core::library::WindowGeometry;
    use neoteletext::outbox::Outbox;
    use neoteletext::settings::Settings;
    use protocol::read_from_core;
    use std::io::Write;
    use std::sync::mpsc;

    /// What the core writes to the interface, a chunk at a time.
    struct Sink(mpsc::Sender<Vec<u8>>);

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.send(bytes.to_vec());
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Test {
        core: Core,
        written: mpsc::Receiver<Vec<u8>>,
        folder: std::path::PathBuf,
    }

    impl Test {
        fn new(name: &str) -> Self {
            let folder = std::env::temp_dir().join(format!("neoteletext-library-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&folder);
            std::fs::create_dir_all(&folder).unwrap();
            let (sender, written) = mpsc::channel();
            let (events, _incoming) = mpsc::channel();
            let core =
                Core::new(Session::new(Bookmarks::default()), Vec::new(), (Settings::default(), Some(folder.clone())), events, Outbox::new(Sink(sender)), None);
            Self { core, written, folder }
        }

        /// Every message sent since the last call.
        fn sent(&mut self) -> Vec<ToInterface> {
            self.core.to_interface.flush().unwrap();
            let mut bytes = Vec::new();
            while let Ok(chunk) = self.written.recv_timeout(std::time::Duration::from_millis(200)) {
                bytes.extend(chunk);
            }
            let mut input = bytes.as_slice();
            let mut messages = Vec::new();
            while let Some(message) = read_from_core(&mut input).unwrap() {
                messages.push(message);
            }
            messages
        }

        fn last_history(&mut self) -> HistoryView {
            self.sent().into_iter().rev().find_map(|message| if let ToInterface::History(view) = message { Some(view) } else { None }).unwrap()
        }

        fn last_bookmarks(&mut self) -> BookmarksView {
            self.sent().into_iter().rev().find_map(|message| if let ToInterface::Bookmarks(view) = message { Some(view) } else { None }).unwrap()
        }

        /// Opens `address` in a new tab and draws its first frame.
        fn visit(&mut self, address: &str) {
            self.core.session.open(address).unwrap();
            let channel = self.core.session.tabs.last().unwrap().channel.id;
            self.core.record_pending(channel).unwrap();
        }
    }

    impl Drop for Test {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
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

    /// Requirements 20.21 and 20.22: the session is written after every
    /// change of the tabs, with a visit's title once its page draws, and
    /// the window's place once the window is still.
    #[test]
    fn the_session_is_kept_after_every_change() {
        let mut test = Test::new("session");
        let kept = |test: &Test| neoteletext::restore::Saved::load(&test.folder).unwrap().0.unwrap_or_default();
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

    /// Requirement 20.1: a visit is recorded when its page draws; not in a
    /// private tab, not the browser's own page, not while history is not
    /// remembered.
    #[test]
    fn a_visit_is_recorded_once_its_page_draws() {
        let mut test = Test::new("record");
        test.core.session.open("news.invalid").unwrap();
        assert!(test.core.library.history.is_empty(), "not before it draws");
        let channel = test.core.session.tabs[0].channel.id;
        test.core.record_pending(channel).unwrap();
        assert_eq!(test.core.library.history.len(), 1);
        test.core.record_pending(channel).unwrap();
        assert_eq!(test.core.library.history.query("")[0].visits, 1, "one draw is one visit");
        assert!(std::fs::read_to_string(test.folder.join("history.txt")).unwrap().contains("https://news.invalid/"), "appended");
        test.core.session.open_private("secret.invalid").unwrap();
        let private = test.core.session.tabs[1].channel.id;
        test.core.record_pending(private).unwrap();
        test.visit("neoteletext:home");
        assert_eq!(test.core.library.history.len(), 1, "{:?}", test.core.library.history.query(""));
        assert_eq!(test.core.session.tabs[0].history[0].title, "news.invalid", "the tab's entry has its title");
    }

    /// Requirements 20.4 to 20.6: rows by their view's numbers only; a
    /// removal is in the file before the view says so.
    #[test]
    fn rows_are_removed_by_their_numbers_and_from_the_file() {
        let mut test = Test::new("remove");
        test.visit("news.invalid");
        test.visit("sport.invalid");
        test.core.history_request(HistoryRequest::Panel(true)).unwrap();
        let first = test.last_history();
        assert_eq!((first.total, first.rows.len()), (2, 2));
        assert!(first.rows[0].heading.as_deref().unwrap().starts_with("Today: "), "{:?}", first.rows[0].heading);
        assert_eq!(first.rows[1].heading, None, "one heading a day");
        test.core.history_request(HistoryRequest::Query { text: "SPORT".into(), first: 0 }).unwrap();
        let second = test.last_history();
        assert_eq!(second.rows.len(), 1);
        // A number from the earlier view names nothing now.
        test.core.history_request(HistoryRequest::Remove(vec![first.rows[0].number, first.rows[1].number])).unwrap();
        assert_eq!(test.core.library.history.len(), 2);
        // Each answer is a new view, with new numbers.
        test.core.history_request(HistoryRequest::Remove(vec![second.rows[0].number])).unwrap();
        assert_eq!(test.core.library.history.len(), 2, "the second view's number is stale too");
        let latest = test.last_history();
        assert_eq!(latest.rows[0].address, second.rows[0].address);
        test.core.history_request(HistoryRequest::Remove(vec![latest.rows[0].number])).unwrap();
        assert_eq!(test.core.library.history.len(), 1);
        let file = std::fs::read_to_string(test.folder.join("history.txt")).unwrap();
        assert!(!file.contains("sport.invalid"), "gone from the file: {file}");
        test.core.session.close(0);
        test.core.history_request(HistoryRequest::Clear(ClearRange::All)).unwrap();
        let cleared = test.last_history();
        assert!(cleared.rows.is_empty() && cleared.closed.is_empty(), "everything, and the closed tabs");
    }

    /// Decision 64: a bookmarked row says so, and Chrome's "Remove bookmark"
    /// on it removes every bookmark of its address.
    #[test]
    fn a_rows_bookmarks_are_removed_from_its_menu() {
        let mut test = Test::new("unbookmark");
        test.visit("news.invalid");
        let address = test.core.library.history.query("")[0].address.clone();
        let other = test.core.session.bookmarks.folders()[0].0;
        test.core.session.bookmarks.add(other, "News", &address).unwrap();
        test.core.session.bookmarks.add(other, "News again", &address).unwrap();
        test.core.history_request(HistoryRequest::Panel(true)).unwrap();
        let view = test.last_history();
        assert!(view.rows[0].bookmarked);
        test.core.history_request(HistoryRequest::RemoveBookmark(view.rows[0].number)).unwrap();
        assert_eq!(test.core.session.bookmarks.find(&address), None, "both gone");
        assert!(!test.last_history().rows[0].bookmarked);
    }

    /// Requirement 20.11: bookmarks first, then history; inline only from typed.
    #[test]
    fn suggestions_come_from_bookmarks_then_history() {
        let mut test = Test::new("suggest");
        test.core.session.bookmarks.add(BAR, "News bookmark", "news.invalid/#302").unwrap();
        test.visit("news.invalid");
        test.sent();
        test.core.suggest("news").unwrap();
        let suggestions = test.sent().into_iter().find_map(|message| if let ToInterface::Suggestions(view) = message { Some(view) } else { None }).unwrap();
        let addresses: Vec<&str> = suggestions.rows.iter().map(|row| row.address.as_str()).collect();
        assert_eq!(addresses, ["https://news.invalid/#302", "https://news.invalid/"]);
        assert!(suggestions.rows[0].bookmarked && !suggestions.rows[1].bookmarked);
        assert_eq!(suggestions.inline, "", "a visit not typed never completes the field");
        // A test core has no broker to start decoders: the navigation is
        // made, and starting its decoder then fails.
        let _ = test.core.open_suggestion(suggestions.rows[1].number, false);
        assert_eq!(test.core.session.strip().address, "https://news.invalid/");
        let _ = test.core.open_suggestion(suggestions.rows[0].number, false);
        assert_eq!(test.core.session.strip().address, "https://news.invalid/", "a used view's number names nothing");
    }

    /// Requirement 20.16: the star bookmarks the page shown; the popover
    /// keeps the whole channel or removes it.
    #[test]
    fn the_star_bookmarks_the_page_shown() {
        let mut test = Test::new("star");
        test.core.session.open("news.invalid/#302").unwrap();
        test.core.star().unwrap();
        let id = test.core.session.bookmarks.find("https://news.invalid/#302").unwrap();
        assert_eq!(test.core.session.bookmarks.item(id).unwrap().parent, OTHER);
        assert!(test.core.session.strip().bookmarked);
        let star = test.sent().into_iter().rev().find_map(|message| if let ToInterface::Star(view) = message { Some(view) } else { None }).unwrap();
        assert!(star.open && star.page);
        let bar = star.folders.iter().find(|folder| folder.name == "Bookmarks bar").unwrap().number;
        test.core.star_request(StarRequest::Edit { name: "Weather".into(), folder: bar }).unwrap();
        assert_eq!(test.core.session.bookmarks.item(id).unwrap().parent, BAR);
        test.core.star_request(StarRequest::WholeChannel).unwrap();
        assert_eq!(test.core.session.bookmarks.item(id).unwrap().address.as_deref(), Some("https://news.invalid/"));
        assert!(!test.core.session.strip().bookmarked, "the page is no longer the bookmark's");
        test.core.star_request(StarRequest::Remove).unwrap();
        assert!(test.core.session.bookmarks.is_empty());
        assert!(std::fs::read_to_string(test.folder.join("bookmarks.txt")).unwrap().lines().count() == 3, "the two folders and the version");
    }

    /// Decision 71: what a decoder read from another browser's file comes
    /// in under Imported, its folders kept, its names cleaned, and only
    /// what the address rules accept; what is left out is counted.
    #[test]
    fn another_browsers_bookmarks_come_in_as_channels_only() {
        let mut test = Test::new("netscape");
        let link = |name: &str, address: &str| BookmarkEntry::Link { name: name.into(), address: address.into() };
        let entries = vec![
            BookmarkEntry::Folder("Bookmarks bar".into()),
            link("Teefax\u{202e}", "https://teastop.co.uk/teefax/"),
            link("Most Visited", "place:sort=8"),
            link("A script", "javascript:alert(1)"),
            BookmarkEntry::FolderEnd,
            link("News", "news.invalid/#302"),
        ];
        test.core.bookmarks_read(Ok((entries, 2))).unwrap();
        let bookmarks = &test.core.session.bookmarks;
        let imported = bookmarks.children(OTHER).iter().copied().find(|&id| bookmarks.item(id).unwrap().name == "Imported").unwrap();
        let names = |folder: u32| bookmarks.children(folder).iter().map(|&id| bookmarks.item(id).unwrap().name.clone()).collect::<Vec<_>>();
        assert_eq!(names(imported), ["Bookmarks bar", "News"]);
        let bar = bookmarks.children(imported)[0];
        assert_eq!(names(bar), ["Teefax"], "the name cleaned; place: and javascript: refused");
        assert_eq!(test.core.library.manager_notice, notice("3 imported into Imported; 4 left out, not channel addresses or too deep"));
        test.core.bookmarks_read(Err("it is not a bookmark file the reader can read".into())).unwrap();
        assert_eq!(test.core.library.manager_notice, notice("Not imported: it is not a bookmark file the reader can read"));
        test.core.bookmarks_read(Ok((vec![link("Only a script", "javascript:void(0)")], 0))).unwrap();
        assert_eq!(test.core.library.manager_notice, notice("No bookmarks in that file that this browser can open"));
    }

    /// Requirement 20.17: the manager's folders, a new folder, removal and
    /// undo, a move refused, an import without its file, an export.
    #[test]
    fn the_manager_edits_by_number() {
        let mut test = Test::new("manager");
        test.core.session.bookmarks.add(BAR, "News", "news.invalid").unwrap();
        test.core.bookmark_request(BookmarkRequest::Panel(true)).unwrap();
        let view = test.last_bookmarks();
        assert_eq!(view.path.iter().map(|folder| folder.name.as_str()).collect::<Vec<_>>(), ["Bookmarks bar"]);
        assert_eq!(view.items.len(), 1);
        test.core.bookmark_request(BookmarkRequest::NewFolder("Weather".into())).unwrap();
        let view = test.last_bookmarks();
        let (news, weather) = (view.items[0].number, view.items[1].number);
        test.core.bookmark_request(BookmarkRequest::Move { number: news, folder: weather, index: 0 }).unwrap();
        let view = test.last_bookmarks();
        let weather = view.items.iter().find(|item| item.folder).unwrap().number;
        test.core.bookmark_request(BookmarkRequest::Remove(weather)).unwrap();
        let view = test.last_bookmarks();
        assert_eq!(view.undo, "Removed Weather");
        assert_eq!(view.items, Vec::new());
        test.core.bookmark_request(BookmarkRequest::Undo).unwrap();
        assert_eq!(test.last_bookmarks().items.len(), 1);
        test.core.bookmark_request(BookmarkRequest::Import("/nowhere/bookmarks.txt".into())).unwrap();
        assert!(test.last_bookmarks().notice.starts_with("Not imported"));
        test.core.bookmark_request(BookmarkRequest::Export).unwrap();
        let notice = test.last_bookmarks().notice;
        assert!(notice.starts_with("Exported to") && notice.contains("bookmarks-export-"), "{notice}");
        test.core.bookmark_request(BookmarkRequest::Query("news".into())).unwrap();
        let found = test.last_bookmarks();
        assert_eq!((found.items.len(), found.items[0].place.as_str()), (1, "Bookmarks bar / Weather"));
    }

    /// Requirement 20.18: the bar shows the Bookmarks bar folder when on,
    /// and the setting is kept.
    #[test]
    fn the_bar_shows_its_folder() {
        let mut test = Test::new("bar");
        test.core.session.bookmarks.add(BAR, "News", "news.invalid").unwrap();
        test.core.send_bar().unwrap();
        let hidden = test.sent().into_iter().find_map(|message| if let ToInterface::Bar(view) = message { Some(view) } else { None }).unwrap();
        assert!(!hidden.shown && hidden.items.is_empty());
        test.core.bar_request(&BarRequest::Show(true)).unwrap();
        let shown = test.sent().into_iter().find_map(|message| if let ToInterface::Bar(view) = message { Some(view) } else { None }).unwrap();
        assert_eq!(shown.items.len(), 1);
        assert!(std::fs::read_to_string(test.folder.join("settings.txt")).unwrap().contains("bar on"));
        let _ = test.core.bar_request(&BarRequest::Open { number: shown.items[0].number, new_tab: true });
        assert_eq!(test.core.session.tabs.len(), 1);
        assert_eq!(test.core.session.strip().address, "https://news.invalid/");
    }

    /// Requirement 20.10: the list and a step of several.
    #[test]
    fn a_list_entry_goes_there() {
        let mut test = Test::new("list");
        test.core.session.open("a.invalid").unwrap();
        test.core.session.navigate("b.invalid", true).unwrap();
        test.core.session.navigate("c.invalid", true).unwrap();
        test.core.send_tab_list(false).unwrap();
        let list = test.sent().into_iter().find_map(|message| if let ToInterface::TabList(list) = message { Some(list) } else { None }).unwrap();
        assert_eq!(list.entries.iter().map(|entry| entry.title.as_str()).collect::<Vec<_>>(), ["b.invalid", "a.invalid"]);
        let _ = test.core.go_to(false, list.entries[1].steps);
        assert_eq!(test.core.session.strip().identity, "a.invalid");
    }
}
