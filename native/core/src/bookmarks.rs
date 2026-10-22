//! Bookmarks (`history-and-bookmarks.md` section 3, requirements 20.15,
//! 20.19 and 20.20): bookmarks and folders in a tree, under two folders
//! that are always there, kept in `bookmarks.txt` written whole at each
//! change and read strictly, since another program can write it.

use std::collections::BTreeMap;
use std::path::Path;

use crate::history::write_whole;
use crate::settings::checked_address;

/// The first line of the file: its format and version.
const VERSION_LINE: &str = "neoteletext-bookmarks 1";
const FILE_NAME: &str = "bookmarks.txt";
const TEMPORARY_NAME: &str = "bookmarks.txt.new";
/// An export's name before its date: `bookmarks-export-2026-10-22.txt`.
const EXPORT_PREFIX: &str = "bookmarks-export-";
const EXPORT_SUFFIX: &str = ".txt";
/// The most exports of one day: `-2` to `-99` after the date.
const MAX_EXPORTS_A_DAY: u32 = 99;
/// The most bookmarks and folders together (section 3.1).
pub const MAX_ITEMS: usize = 10_000;
/// The deepest folder: the two always there are at depth 0.
pub const MAX_DEPTH: usize = protocol::library::MAX_FOLDER_DEPTH;
/// The longest name.
pub const MAX_NAME: usize = protocol::library::MAX_NAME;
/// The longest line: a kind, two numbers, a name and an address.
const MAX_LINE: usize = "bookmark".len() + 2 * 10 + MAX_NAME + protocol::MAX_ADDRESS + 4;
/// The largest file read: [`MAX_ITEMS`] of the longest line, and the
/// version line, rounded up to 24 MiB (section 3.6).
pub const MAX_FILE: usize = 24 * 1024 * 1024;
const _: () = assert!(MAX_ITEMS * (MAX_LINE + 1) + VERSION_LINE.len() < MAX_FILE, "the file bound holds every item at the longest line");
/// The folder above the two that are always there.
pub const ROOT: u32 = 0;
/// The Bookmarks bar folder.
pub const BAR: u32 = 1;
/// The Other bookmarks folder.
pub const OTHER: u32 = 2;
/// Their names.
const BAR_NAME: &str = "Bookmarks bar";
const OTHER_NAME: &str = "Other bookmarks";
/// The folder an import goes into, under Other bookmarks.
const IMPORTED_NAME: &str = "Imported";
const SEPARATOR: char = '\t';

/// A bookmark or a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Its number, unique.
    pub id: u32,
    /// The folder it is in.
    pub parent: u32,
    /// Its name.
    pub name: String,
    /// A bookmark's address; none for a folder.
    pub address: Option<String>,
}

impl Item {
    /// Whether it is a folder.
    #[must_use]
    pub fn is_folder(&self) -> bool {
        self.address.is_none()
    }
}

/// What a removal took, to put back with Undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removed {
    /// The item and everything inside it, outermost first.
    items: Vec<Item>,
    /// Each folder's contents, in order, for those removed.
    children: Vec<(u32, Vec<u32>)>,
    /// Where it was.
    index: usize,
    /// Its name, for "Removed …".
    pub name: String,
}

/// One line of the start page's list: a folder's name, or a bookmark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listed {
    /// A folder's name, as a heading.
    Folder(String),
    /// A bookmark's name and address.
    Bookmark {
        /// Its name.
        name: String,
        /// Its address.
        address: String,
    },
}

/// The bookmarks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmarks {
    items: BTreeMap<u32, Item>,
    /// Each folder's contents in order; [`ROOT`] holds the two always there.
    children: BTreeMap<u32, Vec<u32>>,
    next: u32,
}

impl Default for Bookmarks {
    fn default() -> Self {
        let mut bookmarks = Self { items: BTreeMap::new(), children: BTreeMap::new(), next: OTHER.saturating_add(1) };
        bookmarks.children.insert(ROOT, vec![BAR, OTHER]);
        for (id, name) in [(BAR, BAR_NAME), (OTHER, OTHER_NAME)] {
            bookmarks.items.insert(id, Item { id, parent: ROOT, name: name.to_owned(), address: None });
            bookmarks.children.insert(id, Vec::new());
        }
        bookmarks
    }
}

/// A name as kept: cleaned, cut, never empty.
fn name_of(name: &str, fallback: &str) -> String {
    let name = protocol::clean(name.trim(), MAX_NAME);
    if name.is_empty() { protocol::clean(fallback, MAX_NAME) } else { name }
}

/// An address a bookmark may hold, in the address rules' form, with its
/// page: the start page, or one the address rules accept.
#[must_use]
pub fn bookmark_address(text: &str) -> Option<String> {
    let text = text.trim();
    if text.contains(SEPARATOR) || text.contains('\n') {
        return None;
    }
    if checked_address(text).as_deref() == Some(crate::address::START_PAGE) {
        return Some(crate::address::START_PAGE.to_owned());
    }
    crate::address::parse(text).ok().map(|parsed| parsed.text)
}

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

    fn new_id(&mut self) -> u32 {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        id
    }

    fn insert(&mut self, item: Item, index: Option<usize>) {
        let (id, parent, folder) = (item.id, item.parent, item.is_folder());
        self.items.insert(id, item);
        if folder {
            self.children.entry(id).or_default();
        }
        let siblings = self.children.entry(parent).or_default();
        let at = index.unwrap_or(siblings.len()).min(siblings.len());
        siblings.insert(at, id);
    }

    /// The item with this number.
    #[must_use]
    pub fn item(&self, id: u32) -> Option<&Item> {
        self.items.get(&id)
    }

    /// A folder's contents, in order.
    #[must_use]
    pub fn children(&self, folder: u32) -> &[u32] {
        self.children.get(&folder).map_or(&[], Vec::as_slice)
    }

    /// How deep an item is: the two always there are 0.
    #[must_use]
    pub fn depth(&self, id: u32) -> usize {
        let mut depth = 0_usize;
        let mut at = id;
        while let Some(item) = self.items.get(&at) {
            if item.parent == ROOT || depth > MAX_DEPTH {
                break;
            }
            depth = depth.saturating_add(1);
            at = item.parent;
        }
        depth
    }

    /// The folders from the outermost to `id`, which is among them when a
    /// folder.
    #[must_use]
    pub fn path(&self, id: u32) -> Vec<u32> {
        let mut path = Vec::new();
        let mut at = id;
        while let Some(item) = self.items.get(&at) {
            if item.is_folder() {
                path.push(at);
            }
            if item.parent == ROOT || path.len() > MAX_DEPTH {
                break;
            }
            at = item.parent;
        }
        path.reverse();
        path
    }

    /// How many bookmarks and folders there are, the two always there not
    /// counted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len().saturating_sub(2)
    }

    /// Whether there are none but the two always there.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every item in order: each folder, then what is in it.
    #[must_use]
    pub fn walk(&self) -> Vec<u32> {
        let mut order = Vec::new();
        let mut stack: Vec<u32> = self.children(ROOT).iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            order.push(id);
            stack.extend(self.children(id).iter().rev());
        }
        order
    }

    /// Every folder in order, with its depth.
    #[must_use]
    pub fn folders(&self) -> Vec<(u32, usize)> {
        self.walk().into_iter().filter(|id| self.items.get(id).is_some_and(Item::is_folder)).map(|id| (id, self.depth(id))).collect()
    }

    /// The first bookmark of `address`, in order.
    #[must_use]
    pub fn find(&self, address: &str) -> Option<u32> {
        self.walk().into_iter().find(|id| self.items.get(id).and_then(|item| item.address.as_deref()) == Some(address))
    }

    /// The bookmarks and folders whose name or address holds `query`,
    /// ignoring case, in order.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<u32> {
        let query = query.to_lowercase();
        self.walk()
            .into_iter()
            .filter(|id| {
                self.items.get(id).is_some_and(|item| {
                    item.name.to_lowercase().contains(&query) || item.address.as_deref().is_some_and(|address| address.to_lowercase().contains(&query))
                })
            })
            .collect()
    }

    /// What the start page lists (section 3.5): the Bookmarks bar's
    /// bookmarks, then Other bookmarks', each folder's name before its
    /// contents.
    #[must_use]
    pub fn listed(&self) -> Vec<Listed> {
        self.walk()
            .into_iter()
            .filter(|id| *id != BAR && *id != OTHER)
            .filter_map(|id| self.items.get(&id))
            .map(|item| match &item.address {
                Some(address) => Listed::Bookmark { name: item.name.clone(), address: address.clone() },
                None => Listed::Folder(item.name.clone()),
            })
            .collect()
    }

    fn room(&self) -> Result<(), String> {
        if self.len() >= MAX_ITEMS { Err(format!("At most {MAX_ITEMS} bookmarks and folders are kept")) } else { Ok(()) }
    }

    fn folder_for(&self, folder: u32) -> Result<(), String> {
        if !self.items.get(&folder).is_some_and(Item::is_folder) {
            return Err("No such folder".into());
        }
        if self.depth(folder) >= MAX_DEPTH {
            return Err(format!("Folders are at most {MAX_DEPTH} deep"));
        }
        Ok(())
    }

    /// Adds a bookmark at the end of `folder`.
    ///
    /// # Errors
    /// When the address is not one, the folder is not, or the bookmarks are full.
    pub fn add(&mut self, folder: u32, name: &str, address: &str) -> Result<u32, String> {
        let address = bookmark_address(address).ok_or("Not an address that can be opened")?;
        self.room()?;
        self.folder_for(folder)?;
        let id = self.new_id();
        let name = name_of(name, &address);
        self.insert(Item { id, parent: folder, name, address: Some(address) }, None);
        Ok(id)
    }

    /// Adds a folder at the end of `parent`.
    ///
    /// # Errors
    /// When the parent is not a folder, is too deep, or the bookmarks are full.
    pub fn add_folder(&mut self, parent: u32, name: &str) -> Result<u32, String> {
        self.room()?;
        self.folder_for(parent)?;
        let id = self.new_id();
        self.insert(Item { id, parent, name: name_of(name, "New folder"), address: None }, None);
        Ok(id)
    }

    /// Renames an item; for a bookmark, `address` when not empty becomes its address.
    ///
    /// # Errors
    /// When there is no such item or the address is not one.
    pub fn edit(&mut self, id: u32, name: &str, address: &str) -> Result<(), String> {
        let item = self.items.get(&id).ok_or("No such bookmark")?;
        let address = match (&item.address, address.trim().is_empty()) {
            (Some(_), false) => Some(bookmark_address(address).ok_or("Not an address that can be opened")?),
            (Some(old), true) => Some(old.clone()),
            (None, _) => None,
        };
        let item = self.items.get_mut(&id).ok_or("No such bookmark")?;
        let fallback = address.clone().unwrap_or_else(|| item.name.clone());
        item.name = name_of(name, &fallback);
        item.address = address;
        Ok(())
    }

    /// Removes an item and anything inside it; the two always there stay.
    pub fn remove(&mut self, id: u32) -> Option<Removed> {
        if id == BAR || id == OTHER {
            return None;
        }
        let item = self.items.get(&id)?.clone();
        let index = self.children(item.parent).iter().position(|&each| each == id)?;
        let mut inside = vec![id];
        let mut taken = Vec::new();
        let mut children = Vec::new();
        while let Some(at) = inside.pop() {
            if let Some(each) = self.items.remove(&at) {
                taken.push(each);
            }
            if let Some(contents) = self.children.remove(&at) {
                inside.extend(contents.iter().copied());
                children.push((at, contents));
            }
        }
        if let Some(siblings) = self.children.get_mut(&item.parent) {
            siblings.retain(|&each| each != id);
        }
        Some(Removed { items: taken, children, index, name: item.name })
    }

    /// Puts back what a removal took, where it was, if its folder is still there.
    pub fn restore(&mut self, removed: Removed) -> bool {
        let Some(first) = removed.items.first() else { return false };
        let parent = first.parent;
        if !self.items.get(&parent).is_some_and(Item::is_folder) || self.items.len().saturating_add(removed.items.len()) > MAX_ITEMS.saturating_add(2) {
            return false;
        }
        let id = first.id;
        for item in removed.items {
            self.items.insert(item.id, item);
        }
        for (folder, contents) in removed.children {
            self.children.insert(folder, contents);
        }
        let siblings = self.children.entry(parent).or_default();
        siblings.insert(removed.index.min(siblings.len()), id);
        true
    }

    /// Moves an item into `folder`, before its item at `index` (at the end
    /// when beyond).
    ///
    /// # Errors
    /// When either is not there, the folder is inside the item, or it would
    /// be too deep.
    pub fn move_to(&mut self, id: u32, folder: u32, index: usize) -> Result<(), String> {
        if id == BAR || id == OTHER {
            return Err("That folder stays where it is".into());
        }
        let item = self.items.get(&id).ok_or("No such bookmark")?.clone();
        if !self.items.get(&folder).is_some_and(Item::is_folder) {
            return Err("No such folder".into());
        }
        if self.path(folder).contains(&id) {
            return Err("A folder cannot go inside itself".into());
        }
        let height = self.height(id);
        if self.depth(folder).saturating_add(1).saturating_add(height) > MAX_DEPTH {
            return Err(format!("Folders are at most {MAX_DEPTH} deep"));
        }
        let mut index = index;
        if let Some(siblings) = self.children.get_mut(&item.parent)
            && let Some(old) = siblings.iter().position(|&each| each == id)
        {
            siblings.remove(old);
            if item.parent == folder && old < index {
                index = index.saturating_sub(1);
            }
        }
        if let Some(moved) = self.items.get_mut(&id) {
            moved.parent = folder;
        }
        let siblings = self.children.entry(folder).or_default();
        siblings.insert(index.min(siblings.len()), id);
        Ok(())
    }

    /// How many folders deep an item's contents go below it.
    fn height(&self, id: u32) -> usize {
        self.children(id)
            .iter()
            .filter(|child| self.items.get(child).is_some_and(Item::is_folder))
            .map(|&child| self.height(child).saturating_add(1))
            .max()
            .unwrap_or(0)
    }

    /// Sorts a folder's contents by name, ignoring case, folders first.
    pub fn sort(&mut self, folder: u32) {
        let Some(mut contents) = self.children.get(&folder).cloned() else { return };
        contents.sort_by_key(|id| self.items.get(id).map(|item| (!item.is_folder(), item.name.to_lowercase())));
        self.children.insert(folder, contents);
    }

    /// Adds every bookmark and folder of `other` into a new folder
    /// "Imported" under Other bookmarks; returns how many were added.
    ///
    /// # Errors
    /// When they would not all fit.
    pub fn import(&mut self, other: &Self) -> Result<usize, String> {
        let count = other.len();
        if self.items.len().saturating_add(count).saturating_add(1) > MAX_ITEMS.saturating_add(2) {
            return Err(format!("At most {MAX_ITEMS} bookmarks and folders are kept"));
        }
        let into = self.add_folder(OTHER, IMPORTED_NAME)?;
        let mut map: BTreeMap<u32, u32> = BTreeMap::from([(BAR, into), (OTHER, into)]);
        for id in other.walk() {
            if id == BAR || id == OTHER {
                continue;
            }
            let (Some(item), Some(&parent)) = (other.items.get(&id), other.items.get(&id).and_then(|item| map.get(&item.parent))) else { continue };
            if self.depth(parent) >= MAX_DEPTH {
                continue;
            }
            let new = self.new_id();
            map.insert(id, new);
            self.insert(Item { id: new, parent, name: item.name.clone(), address: item.address.clone() }, None);
        }
        Ok(map.len().saturating_sub(2))
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
        match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Self::default(), Vec::new())),
            Err(error) => return Err(error),
            Ok(metadata) if metadata.len() > u64::try_from(MAX_FILE).unwrap_or(u64::MAX) => {
                return Ok((Self::default(), vec![format!("the bookmarks file is longer than {MAX_FILE} bytes; none read")]));
            }
            Ok(_) => {}
        }
        let bytes = std::fs::read(&path)?;
        Ok(Self::read(&String::from_utf8_lossy(&bytes)))
    }

    /// Writes `bookmarks.txt` in `folder` whole, readable by the user alone.
    ///
    /// # Errors
    /// When the folder cannot be made or the file written or renamed.
    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        write_whole(folder, FILE_NAME, TEMPORARY_NAME, &self.text())
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
            write_whole(folder, &name, &format!("{name}.new"), &self.text())?;
            return Ok(path);
        }
        Err(std::io::Error::other("too many exports today"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    fn tree() -> (Bookmarks, u32, u32, u32) {
        let mut bookmarks = Bookmarks::default();
        let news = bookmarks.add(BAR, "News Example", "https://news.example/teletext/").unwrap();
        let weather = bookmarks.add_folder(OTHER, "Weather").unwrap();
        let page = bookmarks.add(weather, "", "https://news.example/teletext/#302").unwrap();
        (bookmarks, news, weather, page)
    }

    /// Requirement 20.15: the two folders, names, folders, the address's page kept.
    #[test]
    fn bookmarks_live_in_folders() {
        let (bookmarks, news, weather, page) = tree();
        assert_eq!(bookmarks.children(ROOT), [BAR, OTHER]);
        assert_eq!(bookmarks.children(BAR), [news]);
        assert_eq!(bookmarks.item(page).unwrap().name, "https://news.example/teletext/#302", "an empty name is the address");
        assert_eq!(bookmarks.path(page), [OTHER, weather]);
        assert_eq!(bookmarks.depth(page), 2);
        assert_eq!(bookmarks.find("https://news.example/teletext/#302"), Some(page));
        assert_eq!(bookmarks.search("WEATHER"), [weather]);
        assert_eq!(
            bookmarks.listed(),
            [
                Listed::Bookmark { name: "News Example".into(), address: "https://news.example/teletext/".into() },
                Listed::Folder("Weather".into()),
                Listed::Bookmark { name: "https://news.example/teletext/#302".into(), address: "https://news.example/teletext/#302".into() },
            ]
        );
    }

    /// Requirement 20.20: a file of exactly the most bookmarks is read
    /// whole, and one more is dropped. Before, the reader counted the two
    /// folders always there and dropped the last two of 10 000.
    #[test]
    fn the_most_bookmarks_are_read_whole() {
        use std::fmt::Write as _;
        let mut text = format!("{VERSION_LINE}\n");
        for n in 0..=MAX_ITEMS {
            let _ = writeln!(text, "bookmark\t{}\t{OTHER}\tName\thttps://b{n}.example/", n + 3);
        }
        let (bookmarks, dropped) = Bookmarks::read(&text);
        assert_eq!((bookmarks.len(), dropped.len()), (MAX_ITEMS, 1));
    }

    /// Requirement 20.15: every bound.
    #[test]
    fn bookmarks_are_bounded() {
        let mut bookmarks = Bookmarks::default();
        assert!(bookmarks.add(BAR, "x", "not an address").is_err());
        assert!(bookmarks.add(999, "x", "https://a.example/").is_err());
        let mut folder = OTHER;
        for _ in 0..MAX_DEPTH {
            folder = bookmarks.add_folder(folder, "deep").unwrap();
        }
        assert!(bookmarks.add_folder(folder, "too deep").is_err());
        let mut bookmarks = Bookmarks::default();
        for n in 0..MAX_ITEMS {
            bookmarks.add(OTHER, "", &format!("https://b{n}.example/")).unwrap();
        }
        assert!(bookmarks.add(OTHER, "", "https://one-more.example/").is_err());
        assert_eq!(
            bookmarks.add(BAR, &"n".repeat(MAX_NAME + 10), "https://x.example/").map(|_| ()),
            Err(format!("At most {MAX_ITEMS} bookmarks and folders are kept"))
        );
        let mut bookmarks = Bookmarks::default();
        let long = bookmarks.add(BAR, &"n".repeat(MAX_NAME + 10), "https://x.example/").unwrap();
        assert_eq!(bookmarks.item(long).unwrap().name.len(), MAX_NAME);
    }

    /// Requirement 20.17: remove and undo, move without cycles, sort, edit.
    #[test]
    fn bookmarks_are_managed() {
        let (mut bookmarks, news, weather, page) = tree();
        let removed = bookmarks.remove(weather).unwrap();
        assert_eq!(removed.name, "Weather");
        assert!(bookmarks.item(page).is_none(), "its contents went with it");
        assert!(bookmarks.restore(removed));
        assert_eq!(bookmarks.path(page), [OTHER, weather]);
        assert!(bookmarks.remove(BAR).is_none());
        assert!(bookmarks.move_to(weather, weather, 0).is_err(), "not into itself");
        bookmarks.move_to(news, weather, 0).unwrap();
        assert_eq!(bookmarks.children(weather), [news, page]);
        bookmarks.move_to(news, weather, 2).unwrap();
        assert_eq!(bookmarks.children(weather), [page, news]);
        bookmarks.sort(weather);
        assert_eq!(bookmarks.children(weather), [page, news], "by name: https… before News");
        bookmarks.edit(page, "Weather page", "").unwrap();
        assert_eq!(bookmarks.item(page).unwrap().address.as_deref(), Some("https://news.example/teletext/#302"));
        assert!(bookmarks.edit(page, "x", "not an address").is_err());
        bookmarks.edit(weather, "Forecasts", "ignored").unwrap();
        assert_eq!(bookmarks.item(weather).unwrap().address, None);
    }

    /// Requirements 20.19 and 20.20: written, read back the same; written
    /// whole; readable by the user alone.
    #[test]
    fn bookmarks_round_trip_through_their_file() {
        let (bookmarks, ..) = tree();
        let (read, dropped) = Bookmarks::read(&bookmarks.text());
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(read.text(), bookmarks.text());
        assert_eq!(read.listed(), bookmarks.listed());
        let folder = std::env::temp_dir().join(format!("neoteletext-bookmarks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        bookmarks.save(&folder).unwrap();
        assert_eq!(Bookmarks::load(&folder).unwrap().0.text(), bookmarks.text());
        let first = bookmarks.export(&folder, "2026-10-22").unwrap();
        let second = bookmarks.export(&folder, "2026-10-22").unwrap();
        assert!(first.ends_with("bookmarks-export-2026-10-22.txt") && second.ends_with("bookmarks-export-2026-10-22-2.txt"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(folder.join(FILE_NAME)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Requirement 20.20: each rule; a broken line is dropped with what is
    /// inside it; no cycle can be read.
    #[test]
    fn the_file_is_read_strictly() {
        let text = [
            VERSION_LINE,
            "folder\t1\t0\tMy bar",
            "folder\t7\t2\tWeather",
            "bookmark\t8\t7\tForecast\thttps://news.example/teletext/#302",
            "folder\t9\t10\tBefore its folder",
            "folder\t10\t9\tA cycle",
            "bookmark\t11\t10\tInside the cycle\thttps://a.example/",
            "bookmark\t7\t2\tA number taken\thttps://a.example/",
            "bookmark\t12\t8\tInside a bookmark\thttps://a.example/",
            "bookmark\t13\t2\tNot an address\tnot an address",
            "bookmark\t14\t2\tHidden\u{202E}\thttps://a.example/",
            "bookmark\t15\t2\t\thttps://a.example/",
            "bookmark\t16\t2\tExtra\thttps://a.example/\textra",
            "elephant\t17\t2\tWhat",
        ]
        .join("\n");
        let (bookmarks, dropped) = Bookmarks::read(&text);
        assert_eq!(bookmarks.item(BAR).unwrap().name, "My bar");
        assert_eq!(bookmarks.len(), 2, "{:?}", bookmarks.listed());
        assert_eq!(dropped.len(), 10, "{dropped:?}");
        assert!(Bookmarks::read("no version\nfolder\t5\t2\tX").0.is_empty());
        let mut deep = vec![VERSION_LINE.to_owned()];
        let mut parent = OTHER;
        for n in 0..=MAX_DEPTH {
            let id = 100 + u32::try_from(n).unwrap();
            deep.push(format!("folder\t{id}\t{parent}\tdeep"));
            parent = id;
        }
        let (bookmarks, dropped) = Bookmarks::read(&deep.join("\n"));
        assert_eq!((bookmarks.len(), dropped.len()), (MAX_DEPTH, 1), "one folder too deep");
    }

    /// Bookmarks read from another file go into "Imported", whole.
    #[test]
    fn an_import_goes_into_its_own_folder() {
        let (mut bookmarks, ..) = tree();
        let (other, ..) = tree();
        assert_eq!(bookmarks.import(&other), Ok(3));
        let imported = bookmarks.children(OTHER).last().copied().unwrap();
        assert_eq!(bookmarks.item(imported).unwrap().name, IMPORTED_NAME);
        assert_eq!(bookmarks.children(imported).len(), 2);
    }
}
