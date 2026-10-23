//! The numbers of a panel's view (requirement 20.5): each names one thing
//! of the view the trusted core last sent, and a number from an earlier
//! view names nothing.

use std::collections::BTreeMap;

/// The numbers of one view: each names one thing of the view this core
/// last sent, and a number from an earlier view names nothing (requirement
/// 20.5). Numbers count on across views, so none is reused.
#[derive(Debug)]
pub(super) struct Numbers<T> {
    next: u32,
    map: BTreeMap<u32, T>,
}

impl<T> Default for Numbers<T> {
    fn default() -> Self {
        Self { next: 1, map: BTreeMap::new() }
    }
}

impl<T> Numbers<T> {
    /// A new view: the last view's numbers name nothing now.
    pub(super) fn fresh(&mut self) {
        self.map.clear();
    }

    pub(super) fn number(&mut self, value: T) -> u32 {
        let number = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        self.map.insert(number, value);
        number
    }

    pub(super) fn get(&self, number: u32) -> Option<&T> {
        self.map.get(&number)
    }
}
