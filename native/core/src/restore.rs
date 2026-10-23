//! The session (`history-and-bookmarks.md` section 15, requirements 20.21
//! to 20.23): each window's place and size and its tabs but private ones
//! (`multiple-windows.md`), and the recently closed tabs, each with its back-and-forward list and each
//! entry's time, kept in `session.txt` after every change so that it can
//! always be restored, as Chrome keeps its session and its closed tabs
//! (`TabRestoreServiceImpl`); read strictly, since another program can
//! write the file.

use std::path::Path;

use interface_core::library::{MAX_CLOSED, MAX_TAB_ENTRIES, WindowGeometry};

use crate::private_file::write_whole;

const VERSION_LINE: &str = "neoteletext-session 3";
const FILE_NAME: &str = "session.txt";
const TEMPORARY_NAME: &str = "session.txt.new";
const SEPARATOR: char = '\t';
/// The most tabs kept, in all windows: as many as the session holds
/// (`protocol::MAX_TABS`).
pub const MAX_TABS: usize = protocol::MAX_TABS;
/// The most windows kept.
pub const MAX_WINDOWS: usize = interface_core::library::MAX_WINDOWS;
/// The most entries kept of a tab's back-and-forward list, those nearest
/// its place (section 15.1): all a tab holds.
pub const MAX_ENTRIES: usize = MAX_TAB_ENTRIES;
/// The longest title kept of an entry.
pub const MAX_TITLE: usize = protocol::MAX_TITLE;
/// The longest digits of an entry's time, `i64::MAX`.
const MAX_TIME_DIGITS: usize = 19;
/// The longest digits of a closed tab's window, `u32::MAX`.
const MAX_WINDOW_DIGITS: usize = 10;
/// The longest line: `entry`, a flag, a time, an address and a title.
const MAX_LINE: usize = "entry".len() + 1 + 1 + 1 + MAX_TIME_DIGITS + 1 + protocol::MAX_ADDRESS + 1 + MAX_TITLE;
/// The largest file read: every tab and closed tab with every entry at the
/// longest line, with room for the windows' and the tabs' lines, rounded
/// up to 16 MiB, as the history file's bound.
pub const MAX_FILE: usize = 16 * 1024 * 1024;
const _: () = assert!(
    (MAX_TABS + MAX_CLOSED) * (MAX_ENTRIES + 1) * (MAX_LINE + 1) + MAX_WINDOWS * 2 * 64 < MAX_FILE,
    "the file bound holds every tab and closed tab at the longest lines, and every window's lines"
);
const _: () = assert!(MAX_WINDOW_DIGITS + "closed".len() + 6 < MAX_LINE, "a closed tab's line is within the longest");

/// One entry of a kept tab's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// A link on a page started it.
    pub by_page: bool,
    /// When the tab last came to it, in seconds since 1970 (UTC).
    pub at: i64,
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
    /// A closed tab's window: the closed tabs of one window share a number,
    /// and reopen together; 0 for a tab closed alone, and for an open tab.
    pub window: u32,
}

/// A kept window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    /// Its place and size, when the interface has reported them.
    pub geometry: Option<WindowGeometry>,
    /// The active tab's place among its tabs.
    pub active: usize,
    /// Its tabs, in the strip's order.
    pub tabs: Vec<Tab>,
}

/// The kept session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    /// The windows, each with at least one tab, at most [`MAX_WINDOWS`],
    /// with at most [`MAX_TABS`] tabs among them.
    pub windows: Vec<Window>,
    /// The recently closed tabs, the most recent first, at most
    /// [`MAX_CLOSED`].
    pub closed: Vec<Tab>,
}

impl Tab {
    /// A tab's list cut to the [`MAX_ENTRIES`] nearest its place.
    #[must_use]
    pub fn trimmed(muted: bool, position: usize, entries: Vec<Entry>) -> Self {
        let first = position.saturating_sub(MAX_ENTRIES / 2).min(entries.len().saturating_sub(MAX_ENTRIES));
        let entries: Vec<Entry> = entries.into_iter().skip(first).take(MAX_ENTRIES).collect();
        let position = position.saturating_sub(first).min(entries.len().saturating_sub(1));
        Self { muted, position, entries, window: 0 }
    }
}

impl Saved {
    /// How many tabs the windows hold.
    #[must_use]
    pub fn tab_count(&self) -> usize {
        self.windows.iter().map(|window| window.tabs.len()).sum()
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

/// A `window` line's fields after its name: none, for a window whose place
/// was never reported, or its place and size.
fn window_line(fields: &[&str]) -> Result<Option<WindowGeometry>, ()> {
    match fields {
        [] => Ok(None),
        [x, y, width, height, maximised] => {
            let geometry = (|| {
                Some(WindowGeometry {
                    x: x.parse().ok()?,
                    y: y.parse().ok()?,
                    width: width.parse().ok()?,
                    height: height.parse().ok()?,
                    maximised: flag(maximised)?,
                })
            })();
            geometry.filter(WindowGeometry::is_plausible).map(Some).ok_or(())
        }
        _ => Err(()),
    }
}

/// A `tab` or `closed` line's fields after its name: muted, place, and for
/// a closed tab of a window, the window's number.
fn tab_line(closed: bool, fields: &[&str]) -> Option<Tab> {
    let (muted, position, window) = match fields {
        [muted, position] => (muted, position, 0),
        [muted, position, window] if closed && window.len() <= MAX_WINDOW_DIGITS => (muted, position, window.parse().ok()?),
        _ => return None,
    };
    Some(Tab { muted: flag(muted)?, position: position.parse().ok()?, entries: Vec::new(), window })
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
                dropped.push(format!("the session file's first line is not \"{VERSION_LINE}\"; none read"));
            }
            return (saved, dropped);
        }
        // A tab or closed tab being read, dropped whole when any line of it
        // breaks a rule: the tab, whether broken, whether closed.
        let mut tab: Option<(Tab, bool, bool)> = None;
        let finish = |saved: &mut Self, tab: Option<(Tab, bool, bool)>, dropped: &mut Vec<String>| {
            let Some((tab, broken, closed)) = tab else { return };
            let full = if closed { saved.closed.len() >= MAX_CLOSED } else { saved.tab_count() >= MAX_TABS };
            let list = if closed { Some(&mut saved.closed) } else { saved.windows.last_mut().map(|window| &mut window.tabs) };
            match list {
                Some(list) if !broken && !tab.entries.is_empty() && tab.position < tab.entries.len() && !full => list.push(tab),
                _ => dropped.push("a tab of the session file dropped".to_owned()),
            }
        };
        for (index, line) in lines.enumerate() {
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split(SEPARATOR).collect();
            let fine = line.len() <= MAX_LINE
                && match fields.as_slice() {
                    // A window begins, with its place and size when known.
                    ["window", rest @ ..] => {
                        finish(&mut saved, tab.take(), &mut dropped);
                        match window_line(rest) {
                            Ok(geometry) if saved.windows.len() < MAX_WINDOWS => {
                                saved.windows.push(Window { geometry, active: 0, tabs: Vec::new() });
                                true
                            }
                            // Its tabs join the window before, if any.
                            _ => false,
                        }
                    }
                    ["active", place] => match (place.parse::<usize>(), saved.windows.last_mut()) {
                        (Ok(place), Some(window)) => {
                            window.active = place;
                            true
                        }
                        _ => false,
                    },
                    [kind @ ("tab" | "closed"), rest @ ..] => {
                        finish(&mut saved, tab.take(), &mut dropped);
                        let closed = *kind == "closed";
                        let read = tab_line(closed, rest);
                        let fine = read.is_some();
                        tab = Some((read.unwrap_or(Tab { muted: false, position: 0, entries: Vec::new(), window: 0 }), !fine, closed));
                        fine
                    }
                    ["entry", by_page, at, address, title] => {
                        let entry = (|| {
                            let title = (protocol::clean(title, MAX_TITLE) == *title).then(|| (*title).to_owned())?;
                            let at = at.parse::<i64>().ok().filter(|at| *at >= 0)?;
                            Some(Entry { by_page: flag(by_page)?, at, address: kept_address(address)?, title })
                        })();
                        match (&mut tab, entry) {
                            (Some((tab, _, _)), Some(entry)) if tab.entries.len() < MAX_ENTRIES => {
                                tab.entries.push(entry);
                                true
                            }
                            (Some((_, broken, _)), _) => {
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
        // A window with no tab left is not one; an active place beyond its
        // tabs is the first.
        saved.windows.retain(|window| !window.tabs.is_empty());
        for window in &mut saved.windows {
            if window.active >= window.tabs.len() {
                window.active = 0;
            }
        }
        (saved, dropped)
    }

    /// The file's text.
    #[must_use]
    pub fn text(&self) -> String {
        let mut lines = vec![VERSION_LINE.to_owned()];
        let push_tab = |lines: &mut Vec<String>, kind: &str, tab: &Tab| {
            if kind == "closed" && tab.window != 0 {
                lines.push(format!("{kind}\t{}\t{}\t{}", digit(tab.muted), tab.position, tab.window));
            } else {
                lines.push(format!("{kind}\t{}\t{}", digit(tab.muted), tab.position));
            }
            for entry in tab.entries.iter().take(MAX_ENTRIES) {
                lines.push(format!("entry\t{}\t{}\t{}\t{}", digit(entry.by_page), entry.at, entry.address, entry.title));
            }
        };
        let mut kept = 0_usize;
        for window in self.windows.iter().filter(|window| !window.tabs.is_empty()).take(MAX_WINDOWS) {
            if kept >= MAX_TABS {
                break;
            }
            lines.push(match window.geometry {
                Some(geometry) => format!("window\t{}\t{}\t{}\t{}\t{}", geometry.x, geometry.y, geometry.width, geometry.height, digit(geometry.maximised)),
                None => "window".to_owned(),
            });
            lines.push(format!("active\t{}", window.active));
            for tab in window.tabs.iter().take(MAX_TABS.saturating_sub(kept)) {
                push_tab(&mut lines, "tab", tab);
                kept = kept.saturating_add(1);
            }
        }
        for tab in self.closed.iter().take(MAX_CLOSED) {
            push_tab(&mut lines, "closed", tab);
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
mod tests;
