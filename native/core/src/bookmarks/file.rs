//! The bookmarks file, `bookmarks.txt` (`history-and-bookmarks.md` section
//! 3.6, requirements 20.19 and 20.20): its lines read strictly, written
//! whole, and exported beside it.

use std::collections::BTreeMap;
use std::path::Path;

use crate::private_file::{Read, read_text, write_whole};

use super::{BAR, Bookmarks, Item, MAX_DEPTH, MAX_ITEMS, MAX_NAME, OTHER, ROOT, SEPARATOR, bookmark_address};

/// The first line of the file: its format and version.
pub(super) const VERSION_LINE: &str = "neoteletext-bookmarks 1";
pub(super) const FILE_NAME: &str = "bookmarks.txt";
const TEMPORARY_NAME: &str = "bookmarks.txt.new";
/// An export's name before its date: `bookmarks-export-2026-10-22.txt`.
const EXPORT_PREFIX: &str = "bookmarks-export-";
const EXPORT_SUFFIX: &str = ".txt";
/// The most exports of one day: `-2` to `-99` after the date.
const MAX_EXPORTS_A_DAY: u32 = 99;
/// The longest line: a kind, two numbers, a name and an address.
const MAX_LINE: usize = "bookmark".len() + 2 * 10 + MAX_NAME + protocol::MAX_ADDRESS + 4;
/// The largest file read: [`MAX_ITEMS`] of the longest line, and the
/// version line, rounded up to 24 MiB (section 3.6).
pub const MAX_FILE: usize = 24 * 1024 * 1024;
const _: () = assert!(MAX_FILE == protocol::decoder::MAX_BOOKMARK_FILE, "a decoder reads a file of the same bound");
const _: () = assert!(MAX_ITEMS * (MAX_LINE + 1) + VERSION_LINE.len() < MAX_FILE, "the file bound holds every item at the longest line");

impl Bookmarks {
    /// Reads the file's text (section 3.6): lines that break a rule are
    /// dropped with anything inside them, and described in the second
    /// value, for the log.
    #[must_use]
    pub fn read(text: &str) -> (Self, Vec<String>) {
        let mut bookmarks = Self::default();
        let mut dropped = Vec::new();
        if text.len() > MAX_FILE {
            dropped.push(format!("the bookmarks file is longer than {MAX_FILE} bytes; none read"));
            return (bookmarks, dropped);
        }
        let mut lines = text.lines();
        if lines.next() != Some(VERSION_LINE) {
            if !text.is_empty() {
                dropped.push("the bookmarks file has no version line; none read".to_owned());
            }
            return (bookmarks, dropped);
        }
        // Numbers in the file map to numbers here: each item is renumbered
        // as it is read, so the file's numbers need only be unique.
        let mut numbers: BTreeMap<u32, u32> = BTreeMap::from([(BAR, BAR), (OTHER, OTHER)]);
        for (index, line) in lines.enumerate() {
            if line.is_empty() {
                continue;
            }
            if !bookmarks.take(line, &mut numbers) {
                dropped.push(format!("line {} of the bookmarks file dropped", index.saturating_add(2)));
            }
        }
        (bookmarks, dropped)
    }

    /// Takes one line; false when it breaks a rule.
    fn take(&mut self, line: &str, numbers: &mut BTreeMap<u32, u32>) -> bool {
        // The two folders always there are not counted (as `add` counts).
        if line.len() > MAX_LINE || self.len() >= MAX_ITEMS {
            return false;
        }
        let fields: Vec<&str> = line.split(SEPARATOR).collect();
        let (kind, number, parent, name, address) = match fields.as_slice() {
            ["folder", number, parent, name] => ("folder", *number, *parent, *name, None),
            ["bookmark", number, parent, name, address] => ("bookmark", *number, *parent, *name, Some(*address)),
            _ => return false,
        };
        let (Ok(number), Ok(parent)) = (number.parse::<u32>(), parent.parse::<u32>()) else { return false };
        // The two always there are read by their names' lines, if present,
        // which may rename them; any other number is new.
        if kind == "folder" && parent == ROOT && (number == BAR || number == OTHER) {
            return protocol::clean(name, MAX_NAME) == name && !name.is_empty() && {
                if let Some(item) = self.items.get_mut(&number) {
                    name.clone_into(&mut item.name);
                }
                true
            };
        }
        if numbers.contains_key(&number) || number == ROOT {
            return false;
        }
        // A folder's line comes before anything inside it: so no cycle.
        let Some(&parent) = numbers.get(&parent) else { return false };
        if !self.items.get(&parent).is_some_and(Item::is_folder) || self.depth(parent) >= MAX_DEPTH {
            return false;
        }
        if protocol::clean(name, MAX_NAME) != name || name.is_empty() {
            return false;
        }
        let address = match address {
            Some(text) => match bookmark_address(text) {
                Some(address) if address == text => Some(address),
                _ => return false,
            },
            None => None,
        };
        let id = self.new_id();
        numbers.insert(number, id);
        self.insert(Item { id, parent, name: name.to_owned(), address }, None);
        true
    }

    /// The file's text: every item in order, a folder before its contents.
    #[must_use]
    pub fn text(&self) -> String {
        let mut text = String::from(VERSION_LINE);
        text.push('\n');
        for id in self.walk() {
            let Some(item) = self.items.get(&id) else { continue };
            let line = match &item.address {
                Some(address) => format!("bookmark{SEPARATOR}{id}{SEPARATOR}{}{SEPARATOR}{}{SEPARATOR}{address}", item.parent, item.name),
                None => format!("folder{SEPARATOR}{id}{SEPARATOR}{}{SEPARATOR}{}", item.parent, item.name),
            };
            text.push_str(&line);
            text.push('\n');
        }
        text
    }

    /// Reads `bookmarks.txt` in `folder`; the two folders alone when there
    /// is no file.
    ///
    /// # Errors
    /// When the file exists but cannot be read.
    pub fn load(folder: &Path) -> std::io::Result<(Self, Vec<String>)> {
        let path = folder.join(FILE_NAME);
        match read_text(&path, MAX_FILE)? {
            Read::Missing => Ok((Self::default(), Vec::new())),
            Read::TooLong => Ok((Self::default(), vec![format!("the bookmarks file is longer than {MAX_FILE} bytes; none read")])),
            Read::Text(text) => Ok(Self::read(&text)),
        }
    }

    /// Writes `bookmarks.txt` in `folder` whole, readable by the user alone.
    ///
    /// # Errors
    /// When the folder cannot be made or the file written or renamed.
    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        write_whole(folder, FILE_NAME, TEMPORARY_NAME, self.text())
    }

    /// Writes the bookmarks to a new file beside `bookmarks.txt`, named by
    /// `day`, never over another; returns its path.
    ///
    /// # Errors
    /// When the file cannot be written, or a day's exports are many.
    pub fn export(&self, folder: &Path, day: &str) -> std::io::Result<std::path::PathBuf> {
        for count in 1..=MAX_EXPORTS_A_DAY {
            let name = if count == 1 { format!("{EXPORT_PREFIX}{day}{EXPORT_SUFFIX}") } else { format!("{EXPORT_PREFIX}{day}-{count}{EXPORT_SUFFIX}") };
            let path = folder.join(&name);
            if path.exists() {
                continue;
            }
            write_whole(folder, &name, &format!("{name}.new"), self.text())?;
            return Ok(path);
        }
        Err(std::io::Error::other("too many exports today"))
    }
}
