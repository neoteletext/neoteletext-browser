//! The bookmarks bar (`history-and-bookmarks.md` section 3, requirement
//! 20.18): shown or hidden, its folders opened as lists, its bookmarks
//! opened.

use neoteletext::bookmarks::BAR;
use protocol::library::{BarRequest, BarView, ItemRow, MAX_NAME, MAX_VIEW_ROWS};
use protocol::{ToInterface, clean};

use super::super::Core;
use super::shown_address;

impl Core {
    /// What the bookmarks bar asks for (requirement 20.18).
    pub(crate) fn bar_request(&mut self, request: &BarRequest) -> Result<(), String> {
        match *request {
            BarRequest::Show(shown) => {
                self.settings.bar = shown;
                self.save_settings();
            }
            BarRequest::Folder(number) => {
                self.library.bar_open = number.and_then(|number| self.library.bar_numbers.get(number).copied());
            }
            BarRequest::Open { number, new_tab } => {
                let address = self.library.bar_numbers.get(number).and_then(|id| self.session.bookmarks.item(*id)).and_then(|item| item.address.clone());
                self.library.bar_open = None;
                if let Some(address) = address {
                    self.send_bar()?;
                    return self.open_address(&address, new_tab);
                }
            }
        }
        self.send_bar()
    }

    fn bar_row(&mut self, id: u32) -> Option<ItemRow> {
        let item = self.session.bookmarks.item(id)?.clone();
        Some(ItemRow {
            number: self.library.bar_numbers.number(id),
            folder: item.is_folder(),
            name: clean(&item.name, MAX_NAME),
            address: item.address.as_deref().map(shown_address).unwrap_or_default(),
            place: String::new(),
        })
    }

    /// The bookmarks bar.
    pub(crate) fn send_bar(&mut self) -> Result<(), String> {
        self.library.bar_numbers.fresh();
        let shown = self.settings.bar;
        let ids: Vec<u32> = if shown { self.session.bookmarks.children(BAR).iter().take(MAX_VIEW_ROWS).copied().collect() } else { Vec::new() };
        let items = ids.into_iter().filter_map(|id| self.bar_row(id)).collect();
        let open_folder = self.library.bar_open.filter(|id| shown && self.session.bookmarks.item(*id).is_some_and(neoteletext::bookmarks::Item::is_folder));
        let open = open_folder.map(|folder| {
            let ids: Vec<u32> = self.session.bookmarks.children(folder).iter().take(MAX_VIEW_ROWS).copied().collect();
            let number = self.library.bar_numbers.number(folder);
            (number, ids.into_iter().filter_map(|id| self.bar_row(id)).collect())
        });
        self.send(&ToInterface::Bar(BarView { shown, items, open }))
    }
}
