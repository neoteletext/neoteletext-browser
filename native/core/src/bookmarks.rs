//! Bookmarks (`history-and-bookmarks.md` section 3, requirements 20.15,
//! 20.19 and 20.20): bookmarks and folders in a tree, under two folders
//! that are always there, kept in `bookmarks.txt` written whole at each
//! change and read strictly, since another program can write it.

use protocol::decoder::BookmarkEntry;
use std::collections::BTreeMap;

use crate::settings::checked_address;

/// The most bookmarks and folders together (section 3.1).
pub const MAX_ITEMS: usize = 10_000;
const _: () = assert!(MAX_ITEMS == protocol::decoder::MAX_BOOKMARK_ENTRIES, "a decoder reports no more than are kept");
/// The deepest folder: the two always there are at depth 0.
pub const MAX_DEPTH: usize = protocol::library::MAX_FOLDER_DEPTH;
/// The longest name.
pub const MAX_NAME: usize = protocol::library::MAX_NAME;
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

    /// Bookmarks from what a decoder read from another browser's file
    /// (decision 71), under Other bookmarks: each name cleaned and each
    /// address held to the address rules, as typed ones are. The second
    /// value counts the folders and bookmarks refused.
    #[must_use]
    pub fn from_entries(entries: &[BookmarkEntry]) -> (Self, usize) {
        let mut bookmarks = Self::default();
        // The folder each entry goes into; a folder refused takes its
        // contents into the folder around it.
        let mut folders = vec![OTHER];
        let mut refused = 0_usize;
        for entry in entries {
            let here = folders.last().copied().unwrap_or(OTHER);
            match entry {
                BookmarkEntry::Folder(name) => {
                    let opened = bookmarks.add_folder(here, name).unwrap_or_else(|_| {
                        refused = refused.saturating_add(1);
                        here
                    });
                    folders.push(opened);
                }
                BookmarkEntry::FolderEnd => {
                    if folders.len() > 1 {
                        folders.pop();
                    }
                }
                BookmarkEntry::Link { name, address } => {
                    if bookmarks.add(here, name, address).is_err() {
                        refused = refused.saturating_add(1);
                    }
                }
            }
        }
        (bookmarks, refused)
    }
}

mod file;
#[cfg(test)]
pub(crate) use file::VERSION_LINE;
pub use file::{MAX_FILE, is_bookmarks_file};

#[cfg(test)]
mod tests;
