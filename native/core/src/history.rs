//! History (`history-and-bookmarks.md` section 2, requirements 20.1 to
//! 20.3, 20.6 and 20.11): one row for each address on each local day,
//! kept 90 days in `history.txt`, appended at each visit and written whole
//! when compacted or after a removal; read strictly, since another program
//! can write the file.

use crate::dropped::Dropped;
use std::collections::BTreeMap;
use std::path::Path;

use crate::address;
use crate::calendar::{SECONDS_PER_DAY, day_text, local_day, read_day};
use crate::private_file::{Read, private_options, read_text, write_whole};

/// The first line of the file: its format and version.
pub(crate) const VERSION_LINE: &str = "neoteletext-history 1";
/// The file's name in the settings folder, and the one written before the
/// rename.
const FILE_NAME: &str = "history.txt";
const TEMPORARY_NAME: &str = "history.txt.new";
/// Days of history kept, today among them (section 2.2): Chrome's 90.
pub const KEEP_DAYS: i64 = 90;
/// The most rows kept (section 2.2): 90 days of more than 550 pages a day.
pub const MAX_ROWS: usize = 50_000;
/// The largest file read or kept (section 2.2).
pub const MAX_FILE: usize = 16 * 1024 * 1024;
/// The longest channel title kept (`protocol::MAX_TITLE`).
pub const MAX_CHANNEL_TITLE: usize = protocol::MAX_TITLE;
/// The longest page title kept (`protocol::MAX_PAGE_TITLE`).
pub const MAX_PAGE_TITLE: usize = protocol::MAX_PAGE_TITLE;
/// A line's fields: day, last visit, offset, visits, typed, address,
/// channel title, page title.
const FIELDS: usize = 8;
/// The field separator: never in an address, which the address rules keep
/// free of hidden characters, nor in a cleaned title.
const SEPARATOR: char = '\t';
/// The most a zone is ahead of or behind UTC.
const MAX_OFFSET: i32 = confine::MAX_OFFSET;

/// One row: one address on one local day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The local day, in days since 1970.
    pub day: i64,
    /// The last visit, in seconds since 1970, UTC.
    pub last: i64,
    /// The zone's offset at the last visit, in seconds, for its time of day.
    pub offset: i32,
    /// Visits that day.
    pub visits: u32,
    /// Of them, typed in the address field.
    pub typed: u32,
    /// The address, as the address rules print it.
    pub address: String,
    /// The channel's title.
    pub channel: String,
    /// The page's title, or the channel's.
    pub title: String,
}

/// A visit to record (section 2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    /// The address, as the address rules print it.
    pub address: String,
    /// The channel's title, cleaned.
    pub channel: String,
    /// The page's title, or the channel's, cleaned.
    pub title: String,
    /// Typed in the address field.
    pub typed: bool,
    /// A reload: it moves the row's last visit, or makes the day's row, and
    /// counts no visit, as Chrome and Firefox count none (Chrome's
    /// `HistoryBackend::AddPageVisit`, Firefox's `VISIT_COUNT_INC`).
    pub reload: bool,
}

/// What a clearing removes (section 2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    /// Rows whose last visit is at or after this many seconds since 1970.
    Since(i64),
    /// Every row.
    All,
}

/// The history.
#[derive(Debug, Default)]
pub struct History {
    /// In no order; [`History::index`] finds a row by its day and address.
    rows: Vec<Row>,
    index: BTreeMap<(i64, String), usize>,
    /// Lines the file holds beyond its version line: rows written whole,
    /// and lines appended since.
    file_lines: usize,
}

impl Row {
    fn line(&self) -> String {
        let fields = [
            day_text(self.day),
            self.last.to_string(),
            self.offset.to_string(),
            self.visits.to_string(),
            self.typed.to_string(),
            self.address.clone(),
            self.channel.clone(),
            self.title.clone(),
        ];
        fields.join(&SEPARATOR.to_string())
    }

    /// A row from a line, if every field keeps its rule.
    fn read(line: &str) -> Option<Self> {
        let fields: Vec<&str> = line.split(SEPARATOR).collect();
        let [day, last, offset, visits, typed, address, channel, title] = <[&str; FIELDS]>::try_from(fields).ok()?;
        let row = Self {
            day: read_day(day)?,
            last: last.parse().ok().filter(|last: &i64| *last >= 0)?,
            offset: offset.parse().ok().filter(|offset: &i32| (-MAX_OFFSET..=MAX_OFFSET).contains(offset))?,
            visits: visits.parse().ok().filter(|visits: &u32| *visits > 0)?,
            typed: typed.parse().ok()?,
            address: recordable(address)?,
            channel: clean_exactly(channel, MAX_CHANNEL_TITLE)?,
            title: clean_exactly(title, MAX_PAGE_TITLE.max(MAX_CHANNEL_TITLE))?,
        };
        (row.typed <= row.visits && local_day(row.last, row.offset) == row.day).then_some(row)
    }
}

/// Text already clean and within `max`, as the file must hold it.
fn clean_exactly(text: &str, max: usize) -> Option<String> {
    (protocol::clean(text, max) == text).then(|| text.to_owned())
}

/// An address history keeps: one the address rules accept, in their form,
/// that is not the browser's own and holds no separator.
#[must_use]
pub fn recordable(text: &str) -> Option<String> {
    let parsed = address::parse(text).ok()?;
    (parsed.text == text && !parsed.is_start_page() && !text.contains(SEPARATOR) && !text.contains('\n')).then(|| text.to_owned())
}

impl History {
    /// Reads the file's text (section 2.3): lines that break a rule are
    /// dropped and described in the second value, for the log.
    #[must_use]
    pub fn read(text: &str) -> (Self, Dropped) {
        let mut history = Self::default();
        let mut dropped = Dropped::default();
        if text.len() > MAX_FILE {
            dropped.push(|| format!("the history file is longer than {MAX_FILE} bytes; none read"));
            return (history, dropped);
        }
        let mut lines = text.lines();
        if lines.next() != Some(VERSION_LINE) {
            if !text.is_empty() {
                dropped.push(|| "the history file has no version line; none read".to_owned());
            }
            return (history, dropped);
        }
        for (index, line) in lines.enumerate() {
            if line.is_empty() {
                continue;
            }
            history.file_lines = history.file_lines.saturating_add(1);
            match Row::read(line) {
                // A later line for the same day and address is that row's
                // newer state.
                Some(row) => history.put(row),
                None => dropped.push(|| format!("line {} of the history file dropped", index.saturating_add(2))),
            }
        }
        (history, dropped)
    }

    fn put(&mut self, row: Row) {
        let key = (row.day, row.address.clone());
        if let Some(existing) = self.index.get(&key).and_then(|&at| self.rows.get_mut(at)) {
            *existing = row;
        } else {
            self.index.insert(key, self.rows.len());
            self.rows.push(row);
        }
    }

    fn reindex(&mut self) {
        self.index = self.rows.iter().enumerate().map(|(at, row)| ((row.day, row.address.clone()), at)).collect();
    }

    /// How many rows there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Records a visit at `utc` in a zone `offset` ahead; returns the row's
    /// line to append, or nothing when the address is not kept.
    pub fn record(&mut self, visit: &Visit, utc: i64, offset: i32) -> Option<String> {
        let address = recordable(&visit.address)?;
        let day = local_day(utc, offset);
        let key = (day, address);
        let typed = u32::from(visit.typed && !visit.reload);
        let counted = u32::from(!visit.reload);
        let row = if let Some(row) = self.index.get(&key).and_then(|&at| self.rows.get_mut(at)) {
            row.visits = row.visits.saturating_add(counted);
            row.typed = row.typed.saturating_add(typed);
            row.last = utc;
            row.offset = offset;
            row.channel = protocol::clean(&visit.channel, MAX_CHANNEL_TITLE);
            row.title = protocol::clean(&visit.title, MAX_PAGE_TITLE.max(MAX_CHANNEL_TITLE));
            row.clone()
        } else {
            let row = Row {
                day,
                last: utc,
                offset,
                visits: counted,
                typed,
                address: key.1.clone(),
                channel: protocol::clean(&visit.channel, MAX_CHANNEL_TITLE),
                title: protocol::clean(&visit.title, MAX_PAGE_TITLE.max(MAX_CHANNEL_TITLE)),
            };
            self.put(row.clone());
            row
        };
        self.file_lines = self.file_lines.saturating_add(1);
        Some(row.line())
    }

    /// Removes rows older than [`KEEP_DAYS`] before `today`, then the
    /// oldest beyond [`MAX_ROWS`] and beyond a file of [`MAX_FILE`] bytes;
    /// returns whether any went.
    pub fn expire(&mut self, today: i64) -> bool {
        let before = self.rows.len();
        let first_kept = today.saturating_sub(KEEP_DAYS.saturating_sub(1));
        self.rows.retain(|row| row.day >= first_kept);
        self.rows.sort_by_key(|row| std::cmp::Reverse(row.last));
        self.rows.truncate(MAX_ROWS);
        let mut bytes = VERSION_LINE.len().saturating_add(1);
        let keep = self
            .rows
            .iter()
            .take_while(|row| {
                bytes = bytes.saturating_add(row.line().len()).saturating_add(1);
                bytes <= MAX_FILE
            })
            .count();
        self.rows.truncate(keep);
        self.reindex();
        self.rows.len() != before
    }

    /// The file's text: every row, oldest first.
    #[must_use]
    pub fn text(&self) -> String {
        let mut rows: Vec<&Row> = self.rows.iter().collect();
        rows.sort_by_key(|row| row.last);
        let mut text = String::from(VERSION_LINE);
        text.push('\n');
        for row in rows {
            text.push_str(&row.line());
            text.push('\n');
        }
        text
    }

    /// Whether the file holds twice the lines its rows need, and is worth
    /// writing whole (section 2.3).
    #[must_use]
    pub fn wants_compacting(&self) -> bool {
        self.file_lines > self.rows.len().saturating_mul(2)
    }

    /// Removes the rows of these days and addresses; returns the addresses
    /// of the rows that went.
    pub fn remove(&mut self, keys: &[(i64, String)]) -> Vec<String> {
        self.remove_where(|row| keys.iter().any(|(day, address)| *day == row.day && *address == row.address))
    }

    fn remove_where(&mut self, gone: impl Fn(&Row) -> bool) -> Vec<String> {
        let mut addresses = Vec::new();
        self.rows.retain(|row| {
            let goes = gone(row);
            if goes {
                addresses.push(row.address.clone());
            }
            !goes
        });
        self.reindex();
        addresses
    }

    /// Removes the rows `range` covers; returns how many went.
    pub fn clear(&mut self, range: Range) -> usize {
        let before = self.rows.len();
        match range {
            Range::All => self.rows.clear(),
            Range::Since(since) => self.rows.retain(|row| row.last < since),
        }
        self.reindex();
        before.saturating_sub(self.rows.len())
    }

    /// The row of a day and address.
    #[must_use]
    pub fn row(&self, day: i64, address: &str) -> Option<&Row> {
        self.index.get(&(day, address.to_owned())).and_then(|&at| self.rows.get(at))
    }

    /// Reads `history.txt` in `folder`; none when there is no file.
    ///
    /// # Errors
    /// When the file exists but cannot be read.
    pub fn load(folder: &Path) -> std::io::Result<(Self, Dropped)> {
        let path = folder.join(FILE_NAME);
        match read_text(&path, MAX_FILE)? {
            Read::Missing => Ok((Self::default(), Dropped::default())),
            Read::TooLong => Ok((Self::default(), Dropped::one(format!("the history file is longer than {MAX_FILE} bytes; none read")))),
            Read::Text(text) => Ok(Self::read(&text)),
        }
    }

    /// Writes `history.txt` in `folder` whole: to a temporary file, then
    /// renamed over the old one, readable by the user alone.
    ///
    /// # Errors
    /// When the folder cannot be made or the file written or renamed.
    pub fn save(&mut self, folder: &Path) -> std::io::Result<()> {
        write_whole(folder, FILE_NAME, TEMPORARY_NAME, self.text())?;
        self.file_lines = self.rows.len();
        Ok(())
    }

    /// Appends one row's line to `history.txt` in `folder`, starting the
    /// file when there is none.
    ///
    /// # Errors
    /// When the file cannot be opened or written.
    pub fn append(folder: &Path, line: &str) -> std::io::Result<()> {
        use std::io::Write as _;
        let path = folder.join(FILE_NAME);
        let new = !path.exists();
        let mut file = private_options().append(true).create(true).open(&path)?;
        if new {
            writeln!(file, "{VERSION_LINE}")?;
        }
        writeln!(file, "{line}")
    }

    /// The seconds since 1970 a clearing of the last `seconds` starts at.
    #[must_use]
    pub fn since(now: i64, seconds: i64) -> Range {
        Range::Since(now.saturating_sub(seconds))
    }
}

/// "Clear history"'s last 15 minutes, in seconds (section 2.5).
pub const FIFTEEN_MINUTES: i64 = 15 * 60;
/// Its last hour.
pub const HOUR: i64 = 3_600;
/// Its last 24 hours.
pub const DAY: i64 = SECONDS_PER_DAY;
/// Its last 7 days.
pub const WEEK: i64 = SECONDS_PER_DAY * 7;
/// Its last 4 weeks.
pub const FOUR_WEEKS: i64 = SECONDS_PER_DAY * 28;

mod search;
pub use search::HOST_SEARCH;

#[cfg(test)]
mod tests;
