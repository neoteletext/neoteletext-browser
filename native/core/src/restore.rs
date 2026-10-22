//! The session (`history-and-bookmarks.md` section 15, requirements 20.21
//! to 20.23): the window's place and size and every tab but private ones,
//! each with its back-and-forward list, kept in `session.txt` after every
//! change so that it can always be restored; read strictly, since another
//! program can write the file.

use std::path::Path;

use interface_core::library::WindowGeometry;

use crate::history::write_whole;

const VERSION_LINE: &str = "neoteletext-session 1";
const FILE_NAME: &str = "session.txt";
const TEMPORARY_NAME: &str = "session.txt.new";
const SEPARATOR: char = '\t';
/// The most tabs kept: as many as the window has (`protocol::MAX_TABS`).
pub const MAX_TABS: usize = protocol::MAX_TABS;
/// The most entries kept of a tab's back-and-forward list, those nearest
/// its place (section 15.1).
pub const MAX_ENTRIES: usize = 50;
/// The longest title kept of an entry.
pub const MAX_TITLE: usize = protocol::MAX_TITLE;
/// The longest line: `entry`, a flag, an address and a title.
const MAX_LINE: usize = "entry".len() + 1 + 1 + 1 + protocol::MAX_ADDRESS + 1 + MAX_TITLE;
/// The largest file read: every tab with every entry at the longest line,
/// with room for the window, active and tab lines, rounded up to 8 MiB.
pub const MAX_FILE: usize = 8 * 1024 * 1024;
const _: () = assert!(MAX_TABS * (MAX_ENTRIES + 1) * (MAX_LINE + 1) + 1024 < MAX_FILE, "the file bound holds every tab at the longest lines");

/// One entry of a kept tab's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// A link on a page started it.
    pub by_page: bool,
    /// Its address, as the address rules print it.
    pub address: String,
    /// Its title.
    pub title: String,
}

/// A kept tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    /// Its sound is off.
    pub muted: bool,
    /// Its place in its list.
    pub position: usize,
    /// Its back-and-forward list, oldest first.
    pub entries: Vec<Entry>,
}

/// The kept session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    /// The window's place and size, when the interface has reported them.
    pub window: Option<WindowGeometry>,
    /// The active tab's place.
    pub active: usize,
    /// The kept tabs, in the strip's order.
    pub tabs: Vec<Tab>,
}

impl Tab {
    /// A tab's list cut to the [`MAX_ENTRIES`] nearest its place.
    #[must_use]
    pub fn trimmed(muted: bool, position: usize, entries: Vec<Entry>) -> Self {
        let first = position.saturating_sub(MAX_ENTRIES / 2).min(entries.len().saturating_sub(MAX_ENTRIES));
        let entries: Vec<Entry> = entries.into_iter().skip(first).take(MAX_ENTRIES).collect();
        let position = position.saturating_sub(first).min(entries.len().saturating_sub(1));
        Self { muted, position, entries }
    }
}

fn flag(text: &str) -> Option<bool> {
    match text {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

fn digit(flag: bool) -> &'static str {
    if flag { "1" } else { "0" }
}

/// An address a kept entry may hold: one the address rules accept, the
/// start page among them, in their form, with no separator.
fn kept_address(text: &str) -> Option<String> {
    let parsed = crate::address::parse(text).ok()?;
    (parsed.text == text && !text.contains(SEPARATOR)).then(|| text.to_owned())
}

impl Saved {
    /// Reads the file's text (section 15.4): what breaks a rule is dropped,
    /// a tab with its entries, and described in the second value.
    #[must_use]
    pub fn read(text: &str) -> (Self, Vec<String>) {
        let mut saved = Self::default();
        let mut dropped = Vec::new();
        if text.len() > MAX_FILE {
            dropped.push(format!("the session file is longer than {MAX_FILE} bytes; none read"));
            return (saved, dropped);
        }
        let mut lines = text.lines();
        if lines.next() != Some(VERSION_LINE) {
            if !text.is_empty() {
                dropped.push("the session file has no version line; none read".to_owned());
            }
            return (saved, dropped);
        }
        // A tab being read, dropped whole when any line of it breaks a rule.
        let mut tab: Option<(Tab, bool)> = None;
        let finish = |saved: &mut Self, tab: Option<(Tab, bool)>, dropped: &mut Vec<String>| {
            let Some((tab, broken)) = tab else { return };
            if broken || tab.entries.is_empty() || tab.position >= tab.entries.len() || saved.tabs.len() >= MAX_TABS {
                dropped.push("a tab of the session file dropped".to_owned());
            } else {
                saved.tabs.push(tab);
            }
        };
        let mut active = 0;
        for (index, line) in lines.enumerate() {
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split(SEPARATOR).collect();
            let fine = line.len() <= MAX_LINE
                && match fields.as_slice() {
                    ["window", x, y, width, height, maximised] => {
                        let window = (|| {
                            Some(WindowGeometry {
                                x: x.parse().ok()?,
                                y: y.parse().ok()?,
                                width: width.parse().ok()?,
                                height: height.parse().ok()?,
                                maximised: flag(maximised)?,
                            })
                        })();
                        match window.filter(WindowGeometry::is_plausible) {
                            Some(window) if saved.window.is_none() => {
                                saved.window = Some(window);
                                true
                            }
                            _ => false,
                        }
                    }
                    ["active", place] => place.parse().map(|place| active = place).is_ok(),
                    ["tab", muted, position] => {
                        finish(&mut saved, tab.take(), &mut dropped);
                        if let (Some(muted), Ok(position)) = (flag(muted), position.parse::<usize>()) {
                            tab = Some((Tab { muted, position, entries: Vec::new() }, false));
                            true
                        } else {
                            tab = Some((Tab { muted: false, position: 0, entries: Vec::new() }, true));
                            false
                        }
                    }
                    ["entry", by_page, address, title] => {
                        let entry = (|| {
                            let title = (protocol::clean(title, MAX_TITLE) == *title).then(|| (*title).to_owned())?;
                            Some(Entry { by_page: flag(by_page)?, address: kept_address(address)?, title })
                        })();
                        match (&mut tab, entry) {
                            (Some((tab, _)), Some(entry)) if tab.entries.len() < MAX_ENTRIES => {
                                tab.entries.push(entry);
                                true
                            }
                            (Some((_, broken)), _) => {
                                *broken = true;
                                false
                            }
                            (None, _) => false,
                        }
                    }
                    _ => false,
                };
            if !fine {
                dropped.push(format!("line {} of the session file dropped", index.saturating_add(2)));
            }
        }
        finish(&mut saved, tab.take(), &mut dropped);
        saved.active = if active < saved.tabs.len() { active } else { 0 };
        (saved, dropped)
    }

    /// The file's text.
    #[must_use]
    pub fn text(&self) -> String {
        let mut lines = vec![VERSION_LINE.to_owned()];
        if let Some(window) = self.window {
            lines.push(format!("window\t{}\t{}\t{}\t{}\t{}", window.x, window.y, window.width, window.height, digit(window.maximised)));
        }
        lines.push(format!("active\t{}", self.active));
        for tab in self.tabs.iter().take(MAX_TABS) {
            lines.push(format!("tab\t{}\t{}", digit(tab.muted), tab.position));
            for entry in tab.entries.iter().take(MAX_ENTRIES) {
                lines.push(format!("entry\t{}\t{}\t{}", digit(entry.by_page), entry.address, entry.title));
            }
        }
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    /// Reads `session.txt` in `folder`; none when there is no file.
    ///
    /// # Errors
    /// When the file exists but cannot be read.
    pub fn load(folder: &Path) -> std::io::Result<(Option<Self>, Vec<String>)> {
        let path = folder.join(FILE_NAME);
        match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((None, Vec::new())),
            Err(error) => return Err(error),
            Ok(metadata) if metadata.len() > u64::try_from(MAX_FILE).unwrap_or(u64::MAX) => {
                return Ok((None, vec![format!("the session file is longer than {MAX_FILE} bytes; none read")]));
            }
            Ok(_) => {}
        }
        let bytes = std::fs::read(&path)?;
        let (saved, dropped) = Self::read(&String::from_utf8_lossy(&bytes));
        Ok((Some(saved), dropped))
    }

    /// Writes `text` as `session.txt` in `folder` whole, readable by the
    /// user alone.
    ///
    /// # Errors
    /// When the folder cannot be made or the file written or renamed.
    pub fn save(folder: &Path, text: &str) -> std::io::Result<()> {
        write_whole(folder, FILE_NAME, TEMPORARY_NAME, text)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    fn entry(address: &str) -> Entry {
        Entry { by_page: false, address: address.into(), title: "News".into() }
    }

    fn saved() -> Saved {
        Saved {
            window: Some(WindowGeometry { x: -120, y: 80, width: 1440, height: 900, maximised: false }),
            active: 1,
            tabs: vec![
                Tab {
                    muted: false,
                    position: 1,
                    entries: vec![entry("https://news.example/teletext/"), Entry { by_page: true, ..entry("https://news.example/teletext/#302") }],
                },
                Tab { muted: true, position: 0, entries: vec![entry(crate::address::START_PAGE)] },
            ],
        }
    }

    /// Requirements 20.21 and 20.23: written, read back the same; written
    /// whole, readable by the user alone.
    #[test]
    fn the_session_round_trips_through_its_file() {
        let saved = saved();
        assert_eq!(Saved::read(&saved.text()), (saved.clone(), Vec::new()));
        let folder = std::env::temp_dir().join(format!("neoteletext-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(Saved::load(&folder).unwrap(), (None, Vec::new()), "no file: none");
        Saved::save(&folder, &saved.text()).unwrap();
        assert_eq!(Saved::load(&folder).unwrap().0, Some(saved.clone()));
        // A temporary file left by a crash before the rename is ignored.
        std::fs::write(folder.join(TEMPORARY_NAME), "neoteletext-session 1\nactive\t0").unwrap();
        assert_eq!(Saved::load(&folder).unwrap().0, Some(saved));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(folder.join(FILE_NAME)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Requirement 20.23: each rule; a broken tab goes with its entries; an
    /// active place beyond the tabs is the first.
    #[test]
    fn the_file_is_read_strictly() {
        let text = [
            VERSION_LINE,
            "window\t10\t20\t0\t900\t0",
            "window\t10\t20\t1440\t900\t0",
            "window\t11\t21\t1440\t900\t0",
            "active\t7",
            "tab\t0\t0",
            "entry\t0\thttps://a.example/\tA",
            "tab\t2\t0",
            "entry\t0\thttps://b.example/\tB",
            "tab\t0\t5",
            "entry\t0\thttps://c.example/\tC",
            "tab\t0\t0",
            "entry\t0\tnot an address\tD",
            "tab\t0\t0",
            "entry\t0\thttps://e.example/\tE\u{202E}",
            "tab\t0\t0",
            "entry\t0\thttps://f.example/\tF",
            "elephant",
        ]
        .join("\n");
        let (saved, dropped) = Saved::read(&text);
        let addresses: Vec<&str> = saved.tabs.iter().map(|tab| tab.entries[0].address.as_str()).collect();
        assert_eq!(addresses, ["https://a.example/", "https://f.example/"]);
        assert_eq!(saved.window, Some(WindowGeometry { x: 10, y: 20, width: 1440, height: 900, maximised: false }));
        assert_eq!(saved.active, 0, "seven is beyond the tabs");
        let line = |n: usize| format!("line {n} of the session file dropped");
        let tab = "a tab of the session file dropped".to_owned();
        assert_eq!(dropped, [line(2), line(4), line(8), tab.clone(), tab.clone(), line(13), tab.clone(), line(15), tab, line(18)]);
        assert_eq!(Saved::read("no version\ntab\t0\t0").0, Saved::default());
    }

    /// Section 15.1: a long list is kept around its place.
    #[test]
    fn a_long_list_is_kept_around_its_place() {
        let entries: Vec<Entry> = (0..200).map(|n| entry(&format!("https://p{n}.example/"))).collect();
        let tab = Tab::trimmed(false, 150, entries.clone());
        assert_eq!(tab.entries.len(), MAX_ENTRIES);
        assert_eq!(tab.entries[tab.position].address, "https://p150.example/");
        let tab = Tab::trimmed(false, 199, entries.clone());
        assert_eq!(tab.entries[tab.position].address, "https://p199.example/");
        let tab = Tab::trimmed(false, 0, entries);
        assert_eq!((tab.position, tab.entries.len()), (0, MAX_ENTRIES));
    }
}
