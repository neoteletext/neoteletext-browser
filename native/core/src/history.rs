//! History (`history-and-bookmarks.md` section 2, requirements 20.1 to
//! 20.3, 20.6 and 20.11): one row for each address on each local day,
//! kept 90 days in `history.txt`, appended at each visit and written whole
//! when compacted or after a removal; read strictly, since another program
//! can write the file.

use std::collections::BTreeMap;
use std::path::Path;

use crate::address;
use crate::calendar::{SECONDS_PER_DAY, day_text, local_day, read_day};

/// The first line of the file: its format and version.
const VERSION_LINE: &str = "neoteletext-history 1";
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
/// The schemes an inline completion leaves out when the text typed has none.
const SCHEMES: [&str; 2] = ["https://", "http://"];

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

    /// The address without its scheme, for completion and channel matching.
    fn bare(&self) -> &str {
        SCHEMES.iter().find_map(|scheme| self.address.strip_prefix(scheme)).unwrap_or(&self.address)
    }

    /// The channel the row's address is of.
    fn channel_text(&self) -> String {
        address::parse(&self.address).map(|parsed| parsed.channel_text().to_owned()).unwrap_or_default()
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
    pub fn read(text: &str) -> (Self, Vec<String>) {
        let mut history = Self::default();
        let mut dropped = Vec::new();
        if text.len() > MAX_FILE {
            dropped.push(format!("the history file is longer than {MAX_FILE} bytes; none read"));
            return (history, dropped);
        }
        let mut lines = text.lines();
        if lines.next() != Some(VERSION_LINE) {
            if !text.is_empty() {
                dropped.push("the history file has no version line; none read".to_owned());
            }
            return (history, dropped);
        }
        for (index, line) in lines.enumerate() {
            history.file_lines = history.file_lines.saturating_add(1);
            match Row::read(line) {
                // A later line for the same day and address is that row's
                // newer state.
                Some(row) => history.put(row),
                None => dropped.push(format!("line {} of the history file dropped", index.saturating_add(2))),
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
        let typed = u32::from(visit.typed);
        let row = if let Some(row) = self.index.get(&key).and_then(|&at| self.rows.get_mut(at)) {
            row.visits = row.visits.saturating_add(1);
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
                visits: 1,
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

    /// The rows that answer `query`, newest first: those whose address,
    /// channel title or page title holds it, ignoring case.
    #[must_use]
    pub fn query(&self, query: &str) -> Vec<&Row> {
        let query = query.to_lowercase();
        let mut rows: Vec<&Row> = self
            .rows
            .iter()
            .filter(|row| query.is_empty() || [&row.address, &row.channel, &row.title].iter().any(|text| text.to_lowercase().contains(&query)))
            .collect();
        rows.sort_by(|a, b| b.last.cmp(&a.last).then_with(|| a.address.cmp(&b.address)));
        rows
    }

    /// Removes the rows of these days and addresses; returns how many went.
    pub fn remove(&mut self, keys: &[(i64, String)]) -> usize {
        let before = self.rows.len();
        self.rows.retain(|row| !keys.iter().any(|(day, address)| *day == row.day && *address == row.address));
        self.reindex();
        before.saturating_sub(self.rows.len())
    }

    /// Removes every row of the channel `address` is of; returns how many went.
    pub fn forget_channel(&mut self, address: &str) -> usize {
        let Ok(channel) = address::parse(address).map(|parsed| parsed.channel_text().to_owned()) else { return 0 };
        let before = self.rows.len();
        self.rows.retain(|row| row.channel_text() != channel);
        self.reindex();
        before.saturating_sub(self.rows.len())
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

    /// The address to complete the field with as `typed` is typed: the most
    /// recent address the user typed before that starts with it, ignoring
    /// case, and without its scheme when `typed` has none (section 2.10).
    #[must_use]
    pub fn completion(&self, typed: &str) -> Option<String> {
        if typed.is_empty() {
            return None;
        }
        let lower = typed.to_lowercase();
        let with_scheme = SCHEMES.iter().any(|scheme| lower.starts_with(scheme) || scheme.starts_with(lower.as_str()));
        self.rows
            .iter()
            .filter(|row| row.typed > 0)
            .filter_map(|row| {
                let candidate = if with_scheme { row.address.as_str() } else { row.bare() };
                candidate.to_lowercase().starts_with(&lower).then_some((row.last, candidate))
            })
            .max_by_key(|(last, _)| *last)
            .map(|(_, candidate)| format!("{typed}{}", candidate.get(typed.len()..).unwrap_or_default()))
    }

    /// The rows that answer `typed` for suggestions: one per address, the
    /// most visited first, then the most recent (section 2.10).
    #[must_use]
    pub fn suggest(&self, typed: &str, max: usize) -> Vec<Row> {
        if typed.is_empty() {
            return Vec::new();
        }
        let mut by_address: BTreeMap<&str, Row> = BTreeMap::new();
        for row in self.query(typed) {
            let entry = by_address.entry(row.address.as_str()).or_insert_with(|| Row { visits: 0, ..row.clone() });
            entry.visits = entry.visits.saturating_add(row.visits);
            if row.last > entry.last {
                entry.last = row.last;
            }
        }
        let mut rows: Vec<Row> = by_address.into_values().collect();
        rows.sort_by(|a, b| b.visits.cmp(&a.visits).then_with(|| b.last.cmp(&a.last)));
        rows.truncate(max);
        rows
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
    pub fn load(folder: &Path) -> std::io::Result<(Self, Vec<String>)> {
        let path = folder.join(FILE_NAME);
        match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Self::default(), Vec::new())),
            Err(error) => return Err(error),
            Ok(metadata) if metadata.len() > u64::try_from(MAX_FILE).unwrap_or(u64::MAX) => {
                return Ok((Self::default(), vec![format!("the history file is longer than {MAX_FILE} bytes; none read")]));
            }
            Ok(_) => {}
        }
        let bytes = std::fs::read(&path)?;
        Ok(Self::read(&String::from_utf8_lossy(&bytes)))
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

/// "Clear history"'s last hour, in seconds (section 2.5).
pub const HOUR: i64 = 3_600;
/// Its last 24 hours.
pub const DAY: i64 = SECONDS_PER_DAY;
/// Its last 7 days.
pub const WEEK: i64 = SECONDS_PER_DAY * 7;
/// Its last 4 weeks.
pub const FOUR_WEEKS: i64 = SECONDS_PER_DAY * 28;

/// Options for a file the user alone may read and write (section 2.3).
pub(crate) fn private_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

/// Writes `bytes` to `name` in `folder` whole, through `temporary`.
pub(crate) fn write_whole(folder: &Path, name: &str, temporary: &str, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write as _;
    std::fs::create_dir_all(folder)?;
    let temporary = folder.join(temporary);
    let mut file = private_options().create(true).truncate(true).open(&temporary)?;
    file.write_all(bytes.as_ref())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, folder.join(name))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    /// 2026-10-22 03:05 UTC: 14:05 in Sydney.
    const NOW: i64 = 20_748 * SECONDS_PER_DAY + 3 * 3_600 + 5 * 60;
    const SYDNEY: i32 = 11 * 3_600;

    fn visit(address: &str, typed: bool) -> Visit {
        Visit { address: address.into(), channel: "News Example".into(), title: "WEATHER".into(), typed }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("neoteletext-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// Requirement 20.1: one row per address per day, its visits counted,
    /// its typed visits apart; the browser's own page is not kept.
    #[test]
    fn a_page_is_one_row_a_day() {
        let mut history = History::default();
        assert!(history.record(&visit("https://news.example/teletext/#302", true), NOW, SYDNEY).is_some());
        history.record(&visit("https://news.example/teletext/#302", false), NOW + 60, SYDNEY);
        history.record(&visit("https://news.example/teletext/#302", false), NOW + SECONDS_PER_DAY, SYDNEY);
        assert_eq!(history.record(&visit("neoteletext:home", true), NOW, SYDNEY), None);
        assert_eq!(history.record(&visit("not an address", true), NOW, SYDNEY), None);
        assert_eq!(history.len(), 2);
        let today = history.row(20_748, "https://news.example/teletext/#302").unwrap();
        assert_eq!((today.visits, today.typed, today.last), (2, 1, NOW + 60));
    }

    /// Requirement 20.2: written, appended, read back; the last line of a
    /// row stands; a file of many lines wants compacting.
    #[test]
    fn history_round_trips_through_its_file() {
        let folder = scratch("history-round-trip");
        let mut history = History::default();
        for minute in 0..3 {
            let line = history.record(&visit("https://news.example/teletext/", minute == 0), NOW + minute * 60, SYDNEY).unwrap();
            History::append(&folder, &line).unwrap();
        }
        let (read, dropped) = History::load(&folder).unwrap();
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(read.query(""), history.query(""));
        assert!(read.wants_compacting(), "three lines for one row");
        let mut read = read;
        read.save(&folder).unwrap();
        let text = std::fs::read_to_string(folder.join(FILE_NAME)).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(folder.join(FILE_NAME)).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    /// Requirement 20.3: each rule; a broken line is dropped and the rest
    /// stand.
    #[test]
    fn the_file_is_read_strictly() {
        let good = Row {
            day: 20_748,
            last: NOW,
            offset: SYDNEY,
            visits: 2,
            typed: 1,
            address: "https://news.example/teletext/#302".into(),
            channel: "News".into(),
            title: "WEATHER".into(),
        };
        let line = good.line();
        let broken = [
            line.replace("2026-10-22", "2026-02-30"),
            line.replace(&NOW.to_string(), "-5"),
            line.replace(&SYDNEY.to_string(), "90000"),
            line.replacen("\t2\t", "\t0\t", 1),
            line.replacen("\t2\t1\t", "\t1\t2\t", 1),
            line.replace("https://news.example/teletext/#302", "neoteletext:home"),
            line.replace("https://news.example/teletext/#302", "https://news.example/a b"),
            line.replace("WEATHER", "WEA\u{202E}THER"),
            format!("{line}\textra"),
            line.replace("2026-10-22", "2026-10-21"),
            String::new(),
        ];
        let mut text = format!("{VERSION_LINE}\n{line}\n");
        for line in &broken {
            text.push_str(line);
            text.push('\n');
        }
        let (history, dropped) = History::read(&text);
        assert_eq!(history.query(""), vec![&good]);
        assert_eq!(dropped.len(), broken.len(), "{dropped:?}");
        assert!(History::read("no version\n").0.is_empty());
        assert!(History::read(&"x".repeat(MAX_FILE + 1)).0.is_empty());
    }

    /// Requirement 20.2: 90 days kept, today among them; at most
    /// `MAX_ROWS`, the oldest going first.
    #[test]
    fn history_is_kept_ninety_days_and_bounded() {
        let mut history = History::default();
        history.record(&visit("https://old.example/", false), NOW - 90 * SECONDS_PER_DAY, 0);
        history.record(&visit("https://kept.example/", false), NOW - 89 * SECONDS_PER_DAY, 0);
        assert!(history.expire(local_day(NOW, 0)));
        let addresses: Vec<&str> = history.query("").iter().map(|row| row.address.as_str()).collect();
        assert_eq!(addresses, ["https://kept.example/"]);
        let mut history = History::default();
        for n in 0..=MAX_ROWS {
            history.record(&visit(&format!("https://p{n}.example/"), false), NOW + i64::try_from(n).unwrap(), 0);
        }
        history.expire(local_day(NOW, 0));
        assert_eq!(history.len(), MAX_ROWS);
        assert!(history.row(local_day(NOW, 0), "https://p0.example/").is_none(), "the oldest went");
    }

    /// Requirement 20.6: a row, a channel, a range, everything.
    #[test]
    fn history_is_removed_as_asked() {
        let mut history = History::default();
        history.record(&visit("https://news.example/teletext/#100", false), NOW - 2 * HOUR, 0);
        history.record(&visit("https://news.example/teletext/#302", false), NOW - 30 * 60, 0);
        history.record(&visit("https://sport.example/", false), NOW - 10, 0);
        assert_eq!(history.clear(History::since(NOW, HOUR)), 2);
        assert_eq!(history.forget_channel("https://news.example/teletext/#999"), 1);
        assert!(history.is_empty());
        history.record(&visit("https://sport.example/", false), NOW, 0);
        assert_eq!(history.remove(&[(local_day(NOW, 0), "https://sport.example/".into())]), 1);
        history.record(&visit("https://sport.example/", false), NOW, 0);
        assert_eq!(history.clear(Range::All), 1);
    }

    /// Requirement 20.11: inline completion only from addresses typed;
    /// suggestions from every visit, the most visited first.
    #[test]
    fn completion_comes_only_from_what_was_typed() {
        let mut history = History::default();
        history.record(&visit("https://news.example/teletext/", false), NOW, 0);
        assert_eq!(history.completion("new"), None, "a link followed never completes the field");
        history.record(&visit("https://news.example/teletext/", true), NOW, 0);
        assert_eq!(history.completion("New").as_deref(), Some("News.example/teletext/"));
        assert_eq!(history.completion("https://n").as_deref(), Some("https://news.example/teletext/"));
        assert_eq!(history.completion("sport"), None);
        history.record(&visit("https://newer.example/", false), NOW + 5, 0);
        let suggested: Vec<String> = history.suggest("new", 8).into_iter().map(|row| row.address).collect();
        assert_eq!(suggested, ["https://news.example/teletext/", "https://newer.example/"]);
        assert_eq!(history.suggest("", 8), Vec::new());
    }
}
