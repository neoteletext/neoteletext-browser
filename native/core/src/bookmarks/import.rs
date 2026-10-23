//! Bookmarks imported (`history-and-bookmarks.md` section 3, decision
//! 71): another set of this browser's bookmarks into "Imported", and what
//! a decoder read from another browser's file.

use protocol::decoder::BookmarkEntry;
use std::collections::BTreeMap;

use super::{BAR, Bookmarks, IMPORTED_NAME, Item, MAX_DEPTH, MAX_ITEMS, OTHER};

impl Bookmarks {
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
                // A file on this computer another browser kept is not
                // taken: the user never chose it here (finding F-53).
                BookmarkEntry::Link { name, address } => {
                    let file = crate::address::parse(address).is_ok_and(|parsed| parsed.file.is_some());
                    if file || bookmarks.add(here, name, address).is_err() {
                        refused = refused.saturating_add(1);
                    }
                }
            }
        }
        (bookmarks, refused)
    }
}
