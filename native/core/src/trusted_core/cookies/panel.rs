//! The cookies panel (`cookies.md` section 8, requirement 21.6): its
//! requests, strip input, and its view of each site and channel.

use neoteletext::cookies::Channel;
use protocol::cookies::{CookieRow, CookiesRequest, CookiesView, MAX_LABEL, MAX_NOTICE, MAX_USED, MAX_VIEW_ROWS};
use protocol::{ToInterface, clean};

use super::super::Core;
use super::Cookies;

/// A row of the panel's view, by what it names.
#[derive(Clone, Debug)]
pub(super) enum Row {
    /// An origin, with all its channels.
    Site(String),
    /// A channel.
    Channel(String),
}

/// A channel listed in the panel: its origin, its key, its cookies' count
/// and bytes, when one was last used, and whether it is blocked.
type Listed = (String, String, usize, usize, u64, bool);

impl Core {
    /// A request of the panel, strip input (requirement 21.6).
    pub(crate) fn cookies_request(&mut self, request: CookiesRequest) -> Result<(), String> {
        let row = |number: u32, cookies: &Cookies| cookies.numbers.get(number).cloned();
        match request {
            CookiesRequest::Panel(open) => {
                self.cookies.panel_open = open;
                self.cookies.notice.clear();
                if !open {
                    return Ok(());
                }
            }
            CookiesRequest::Query { text, first } => {
                self.cookies.query = clean(&text, protocol::cookies::MAX_QUERY);
                self.cookies.first = first;
            }
            CookiesRequest::Delete(number) => {
                let removed = match row(number, &self.cookies) {
                    Some(Row::Site(origin)) => self.cookies.store.delete_origin(&origin),
                    Some(Row::Channel(key)) => self.cookies.store.delete_channel(&key),
                    None => return self.send_cookies(),
                };
                self.cookies.notice = format!("{removed} cookies deleted");
                self.cookies_changed();
            }
            CookiesRequest::Block { number, blocked } => {
                let Some(Row::Channel(key)) = row(number, &self.cookies) else { return self.send_cookies() };
                self.cookies.store.block(&key, blocked);
                self.settings.cookie_block = self.cookies.store.blocked.iter().take(neoteletext::settings::MAX_BLOCKED).cloned().collect();
                self.save_settings();
                self.cookies.said_blocked.remove(&key);
                self.cookies.notice = if blocked { String::from("Blocked, and its cookies deleted") } else { String::from("Unblocked") };
                self.cookies_changed();
            }
            CookiesRequest::DeleteAll => {
                self.cookies.store.delete_all();
                self.cookies.notice = String::from("Every cookie deleted");
                self.cookies_changed();
            }
        }
        self.send_cookies()
    }

    /// The panel's view, while it is open: each site with cookies, and its
    /// channels, matching the search; blocked channels too, to unblock.
    pub(crate) fn send_cookies(&mut self) -> Result<(), String> {
        if !self.cookies.panel_open {
            return Ok(());
        }
        let offset = self.local_time().offset;
        let listed = self.cookies.store.listed();
        let (count, bytes) = listed.iter().fold((0_usize, 0_usize), |(count, bytes), row| (count.saturating_add(row.count), bytes.saturating_add(row.bytes)));
        let query = self.cookies.query.to_lowercase();
        let mut channels: Vec<Listed> = listed
            .into_iter()
            .map(|row| {
                let blocked = self.cookies.store.blocked.contains(&row.key);
                (row.origin, row.key, row.count, row.bytes, row.used_at, blocked)
            })
            .collect();
        for key in &self.cookies.store.blocked {
            if !channels.iter().any(|(_, listed, ..)| listed == key)
                && let Some(channel) = Channel::of(key)
            {
                channels.push((channel.origin().to_owned(), key.clone(), 0, 0, 0, true));
            }
        }
        channels.retain(|(_, key, ..)| query.is_empty() || key.to_lowercase().contains(&query));
        channels.sort();
        let rows = site_rows(&channels, offset);
        let total = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        let first = self.cookies.first.min(total);
        self.cookies.numbers.fresh();
        let shown: Vec<CookieRow> = rows
            .into_iter()
            .skip(usize::try_from(first).unwrap_or(usize::MAX))
            .take(MAX_VIEW_ROWS)
            .map(|(row, mut shown)| {
                shown.number = self.cookies.numbers.number(row);
                shown
            })
            .collect();
        let notice = self.cookies.for_this_run.clone().filter(|_| self.cookies.notice.is_empty()).unwrap_or_else(|| self.cookies.notice.clone());
        let view = CookiesView {
            query: self.cookies.query.clone(),
            first,
            total,
            rows: shown,
            count: u32::try_from(count).unwrap_or(u32::MAX),
            bytes: u64::try_from(bytes).unwrap_or(u64::MAX),
            notice: clean(&notice, MAX_NOTICE),
        };
        self.send(&ToInterface::Cookies(view))
    }
}

/// The panel's rows of the sorted `channels`: each origin, then its
/// channels; when each was last used, as a day at `offset` from UTC.
fn site_rows(channels: &[Listed], offset: i32) -> Vec<(Row, CookieRow)> {
    let used = |at: u64| {
        if at == 0 {
            String::new()
        } else {
            let day = neoteletext::calendar::local_day(i64::try_from(at).unwrap_or(i64::MAX), offset);
            clean(&neoteletext::calendar::day_words(day), MAX_USED)
        }
    };
    let mut rows: Vec<(Row, CookieRow)> = Vec::new();
    let mut index = 0;
    while let Some((origin, ..)) = channels.get(index) {
        let origin = origin.clone();
        let members: Vec<&Listed> = channels.iter().skip(index).take_while(|(other, ..)| *other == origin).collect();
        let site_row = CookieRow {
            number: 0,
            site: true,
            label: clean(&origin, MAX_LABEL),
            count: u32::try_from(members.iter().map(|member| member.2).sum::<usize>()).unwrap_or(u32::MAX),
            bytes: u64::try_from(members.iter().map(|member| member.3).sum::<usize>()).unwrap_or(u64::MAX),
            used: used(members.iter().map(|member| member.4).max().unwrap_or(0)),
            blocked: false,
        };
        rows.push((Row::Site(origin.clone()), site_row));
        for (_, key, count, size, at, blocked) in &members {
            let folder = key.strip_prefix(origin.as_str()).unwrap_or(key);
            let row = CookieRow {
                number: 0,
                site: false,
                label: clean(folder, MAX_LABEL),
                count: u32::try_from(*count).unwrap_or(u32::MAX),
                bytes: u64::try_from(*size).unwrap_or(u64::MAX),
                used: used(*at),
                blocked: *blocked,
            };
            rows.push((Row::Channel(key.clone()), row));
        }
        index = index.saturating_add(members.len());
    }
    rows
}
