//! The history panel, suggestions and the Back and Forward lists
//! (`history-and-bookmarks.md` section 2, requirements 20.4 to 20.11).

use neoteletext::calendar::{day_words, local_day, time_words};
use neoteletext::history::{DAY, FIFTEEN_MINUTES, FOUR_WEEKS, HOUR, History, Range, WEEK};
use protocol::ToInterface;
use protocol::clean;
use protocol::library::{
    ClearRange, ClosedTab, HistoryRequest, HistoryRow, HistoryView, MAX_CLOSED, MAX_LABEL, MAX_NAME, MAX_SUGGESTIONS, MAX_VIEW_ROWS, Suggestion, Suggestions,
    TabEntry, TabList,
};

use super::super::Core;
use super::{notice, shown_address};

impl Core {
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
}
