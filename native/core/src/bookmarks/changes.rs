//! Changing the bookmarks (`history-and-bookmarks.md` section 3,
//! requirement 20.17): a bookmark or folder added, renamed, removed and
//! put back, moved and sorted, each held to the tree's bounds.

use super::{BAR, Bookmarks, Item, MAX_DEPTH, MAX_ITEMS, OTHER, Removed, bookmark_address, name_of};

impl Bookmarks {
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

    /// Puts back what a removal took, where it was, if its folder is still
    /// there, there is room, and its folders would be no deeper than
    /// folders may go, its folder having perhaps moved deeper since
    /// (finding F-79).
    ///
    /// # Errors
    /// Why it cannot be put back.
    pub fn restore(&mut self, removed: Removed) -> Result<(), String> {
        let Some(first) = removed.items.first() else { return Err("There is nothing to put back".into()) };
        let parent = first.parent;
        if !self.items.get(&parent).is_some_and(Item::is_folder) {
            return Err("It cannot be put back: its folder is gone".into());
        }
        if self.items.len().saturating_add(removed.items.len()) > MAX_ITEMS.saturating_add(2) {
            return Err(format!("There are at most {MAX_ITEMS} bookmarks"));
        }
        if first.is_folder() && self.depth(parent).saturating_add(1).saturating_add(removed.height(first.id)) > MAX_DEPTH {
            return Err(format!("It cannot be put back: folders are at most {MAX_DEPTH} deep"));
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
        Ok(())
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
}
