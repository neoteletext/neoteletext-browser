//! The bookmarks manager, the star, the bar and the bookmarks' files
//! (`history-and-bookmarks.md` section 3, requirements 20.15 to 20.20).

use neoteletext::bookmarks::{BAR, Bookmarks, OTHER};
use neoteletext::calendar::{day_text, local_day};
use protocol::ToInterface;
use protocol::decoder::BookmarkEntry;
use protocol::library::{BarRequest, BarView, BookmarkRequest, BookmarksView, FolderRow, ItemRow, MAX_LABEL, MAX_NAME, MAX_VIEW_ROWS, StarRequest, StarView};
use protocol::{MAX_TITLE, clean};

use super::super::Core;
use super::super::processes::start_bookmark_reader;
use super::{Manager, depth_of, notice, shown_address};

/// What separates a folder's names in a search result's place.
const PLACE_SEPARATOR: &str = " / ";

impl Core {
    /// After any change of the bookmarks: kept, and every view of them
    /// sent afresh.
    pub(super) fn bookmarks_changed(&mut self) -> Result<(), String> {
        self.save_bookmarks();
        self.others_stale();
        self.send(&ToInterface::Strip(self.strip()))?;
        self.send_bar()?;
        self.send_star()?;
        self.send_bookmarks()
    }

    /// What the bookmarks manager asks for (requirement 20.17).
    pub(crate) fn bookmark_request(&mut self, request: BookmarkRequest) -> Result<(), String> {
        let item_of = |core: &Self, number: u32| core.library.manager_numbers.get(number).copied().filter(|id| core.session.bookmarks.item(*id).is_some());
        match request {
            BookmarkRequest::Panel(true) => {
                self.library.manager = Some(Manager { folder: BAR, query: String::new(), folders_first: 0, items_first: 0 });
                self.library.manager_notice.clear();
            }
            BookmarkRequest::Panel(false) => {
                self.library.manager = None;
                self.library.manager_numbers.fresh();
                return Ok(());
            }
            BookmarkRequest::Folder(number) => {
                if let (Some(id), Some(manager)) = (item_of(self, number), self.library.manager.as_mut())
                    && self.session.bookmarks.item(id).is_some_and(neoteletext::bookmarks::Item::is_folder)
                {
                    *manager = Manager { folder: id, query: String::new(), folders_first: manager.folders_first, items_first: 0 };
                }
            }
            BookmarkRequest::Scroll { folders_first, items_first } => {
                if let Some(manager) = self.library.manager.as_mut() {
                    manager.folders_first = usize::try_from(folders_first).unwrap_or(usize::MAX);
                    manager.items_first = usize::try_from(items_first).unwrap_or(usize::MAX);
                }
            }
            BookmarkRequest::Query(text) => {
                if let Some(manager) = self.library.manager.as_mut() {
                    manager.query = clean(&text, protocol::library::MAX_QUERY);
                    manager.items_first = 0;
                }
            }
            BookmarkRequest::Open { number, new_tab } => {
                let address = item_of(self, number).and_then(|id| self.session.bookmarks.item(id)).and_then(|item| item.address.clone());
                if let Some(address) = address {
                    return self.open_address(&address, new_tab);
                }
            }
            BookmarkRequest::Edit { number, name, address } => {
                if let Some(id) = item_of(self, number)
                    && let Err(reason) = self.session.bookmarks.edit(id, &name, &address)
                {
                    self.library.manager_notice = notice(&reason);
                }
            }
            BookmarkRequest::Move { number, folder, index } => {
                if let (Some(id), Some(folder)) = (item_of(self, number), item_of(self, folder))
                    && let Err(reason) = self.session.bookmarks.move_to(id, folder, usize::try_from(index).unwrap_or(usize::MAX))
                {
                    self.library.manager_notice = notice(&reason);
                }
            }
            BookmarkRequest::NewFolder(name) => {
                let folder = self.library.manager.as_ref().map_or(OTHER, |manager| manager.folder);
                if let Err(reason) = self.session.bookmarks.add_folder(folder, &name) {
                    self.library.manager_notice = notice(&reason);
                }
            }
            BookmarkRequest::Add { name, address } => {
                let folder = self.library.manager.as_ref().map_or(OTHER, |manager| manager.folder);
                if let Err(reason) = self.session.bookmarks.add(folder, &name, &address) {
                    self.library.manager_notice = notice(&reason);
                }
            }
            BookmarkRequest::Remove(number) => {
                if let Some(removed) = item_of(self, number).and_then(|id| self.session.bookmarks.remove(id)) {
                    self.library.manager_notice.clear();
                    self.library.undo = Some(removed);
                }
            }
            BookmarkRequest::Undo => {
                if let Some(removed) = self.library.undo.take()
                    && !self.session.bookmarks.restore(removed)
                {
                    self.library.manager_notice = notice("It cannot be put back: its folder is gone");
                }
            }
            BookmarkRequest::Sort => {
                let folder = self.library.manager.as_ref().map_or(OTHER, |manager| manager.folder);
                self.session.bookmarks.sort(folder);
            }
            BookmarkRequest::Import(path) => self.import_bookmarks(&path),
            BookmarkRequest::Export => self.export_bookmarks(),
        }
        self.bookmarks_changed()
    }

    /// Reads bookmarks in this browser's format from a file the user named,
    /// through the broker as a file channel is read (decision 56), into
    /// "Imported".
    fn import_bookmarks(&mut self, path: &str) {
        let read = match self.broker() {
            Ok(broker) => broker.read_source(std::path::Path::new(path.trim())),
            Err(reason) => Err(reason),
        };
        self.library.manager_notice = match read {
            // Another browser's file: read by a decoder, never here
            // (requirement 3.2, decision 71).
            Ok(bytes) if bytes.len() <= neoteletext::bookmarks::MAX_FILE && protocol::decoder::is_netscape_bookmarks(&bytes) => {
                match self.broker().and_then(|broker| start_bookmark_reader(broker, bytes, self.events.clone())) {
                    Ok(()) => notice("Reading another browser's bookmarks…"),
                    Err(reason) => notice(&format!("Not imported: {reason}")),
                }
            }
            Ok(bytes) if bytes.len() <= neoteletext::bookmarks::MAX_FILE => {
                let (read, dropped) = Bookmarks::read(&String::from_utf8_lossy(&bytes));
                if read.is_empty() {
                    notice("No bookmarks in that file: it is not one this browser wrote")
                } else {
                    match self.session.bookmarks.import(&read) {
                        Ok(count) if dropped.is_empty() => notice(&format!("{count} imported into Imported")),
                        Ok(count) => notice(&format!("{count} imported into Imported; {} lines of the file were not bookmarks", dropped.len())),
                        Err(reason) => notice(&reason),
                    }
                }
            }
            Ok(_) => notice("That file is too large to be bookmarks"),
            Err(reason) => notice(&format!("Not imported: {reason}")),
        };
    }

    /// What a decoder read from another browser's bookmark file, into
    /// "Imported" (decision 71).
    pub(crate) fn bookmarks_read(&mut self, read: Result<(Vec<BookmarkEntry>, u32), String>) -> Result<(), String> {
        self.library.manager_notice = match read {
            Ok((entries, skipped)) => {
                let (read, refused) = Bookmarks::from_entries(&entries);
                let left_out = usize::try_from(skipped).unwrap_or(usize::MAX).saturating_add(refused);
                if read.is_empty() {
                    notice("No bookmarks in that file that this browser can open")
                } else {
                    match self.session.bookmarks.import(&read) {
                        Ok(count) if left_out == 0 => notice(&format!("{count} imported into Imported")),
                        Ok(count) => notice(&format!("{count} imported into Imported; {left_out} left out, not channel addresses or too deep")),
                        Err(reason) => notice(&reason),
                    }
                }
            }
            Err(reason) => notice(&format!("Not imported: {reason}")),
        };
        self.bookmarks_changed()
    }

    /// Writes the bookmarks to a new file beside the browser's own.
    fn export_bookmarks(&mut self) {
        let Some(folder) = self.settings_folder.clone() else {
            self.library.manager_notice = notice("There is no folder for the settings to export into");
            return;
        };
        let time = self.local_time();
        self.library.manager_notice = match self.session.bookmarks.export(&folder, &day_text(local_day(time.utc, time.offset))) {
            Ok(path) => notice(&format!("Exported to {}", path.display())),
            Err(error) => notice(&format!("Not exported: {error}")),
        };
    }

    fn item_row(&mut self, id: u32, with_place: bool) -> Option<ItemRow> {
        let item = self.session.bookmarks.item(id)?.clone();
        let place = if with_place {
            let names: Vec<String> = self
                .session
                .bookmarks
                .path(item.parent)
                .iter()
                .filter_map(|folder| self.session.bookmarks.item(*folder).map(|folder| folder.name.clone()))
                .collect();
            clean(&names.join(PLACE_SEPARATOR), MAX_LABEL)
        } else {
            String::new()
        };
        Some(ItemRow {
            number: self.library.manager_numbers.number(id),
            folder: item.is_folder(),
            name: clean(&item.name, MAX_NAME),
            address: item.address.as_deref().map(shown_address).unwrap_or_default(),
            place,
        })
    }

    /// The bookmarks manager's view, while it is open.
    pub(crate) fn send_bookmarks(&mut self) -> Result<(), String> {
        let Some(manager) = &self.library.manager else { return Ok(()) };
        let (folder, query, folders_first, items_first) = (manager.folder, manager.query.clone(), manager.folders_first, manager.items_first);
        // A folder removed while open: the manager goes back to the bar.
        let folder = if self.session.bookmarks.item(folder).is_some() { folder } else { BAR };
        if let Some(manager) = self.library.manager.as_mut() {
            manager.folder = folder;
        }
        self.library.manager_numbers.fresh();
        let path: Vec<FolderRow> = self
            .session
            .bookmarks
            .path(folder)
            .into_iter()
            .enumerate()
            .filter_map(|(depth, id)| {
                let name = clean(&self.session.bookmarks.item(id)?.name, MAX_NAME);
                Some(FolderRow { number: self.library.manager_numbers.number(id), name, depth: depth_of(depth) })
            })
            .collect();
        let all_folders = self.session.bookmarks.folders();
        let folders_total = all_folders.len();
        let folders_first = folders_first.min(folders_total);
        let folders = all_folders
            .into_iter()
            .skip(folders_first)
            .take(MAX_VIEW_ROWS)
            .filter_map(|(id, depth)| {
                let name = clean(&self.session.bookmarks.item(id)?.name, MAX_NAME);
                Some(FolderRow { number: self.library.manager_numbers.number(id), name, depth: depth_of(depth) })
            })
            .collect();
        let ids: Vec<u32> = if query.is_empty() { self.session.bookmarks.children(folder).to_vec() } else { self.session.bookmarks.search(&query) };
        let items_total = ids.len();
        let items_first = items_first.min(items_total);
        let items = ids.into_iter().skip(items_first).take(MAX_VIEW_ROWS).filter_map(|id| self.item_row(id, !query.is_empty())).collect();
        let undo = self.library.undo.as_ref().map(|removed| notice(&format!("Removed {}", removed.name))).unwrap_or_default();
        let view = BookmarksView {
            path,
            folders,
            folders_first: u32::try_from(folders_first).unwrap_or(u32::MAX),
            folders_total: u32::try_from(folders_total).unwrap_or(u32::MAX),
            query,
            items,
            items_first: u32::try_from(items_first).unwrap_or(u32::MAX),
            items_total: u32::try_from(items_total).unwrap_or(u32::MAX),
            undo,
            notice: self.library.manager_notice.clone(),
        };
        self.send(&ToInterface::Bookmarks(view))
    }

    /// The star, or its shortcut (requirement 20.16): the address the strip
    /// shows is bookmarked, or its bookmark shown, in the popover.
    pub(crate) fn star(&mut self) -> Result<(), String> {
        let Some(tab) = self.session.active_tab() else { return Ok(()) };
        let Some(address) = tab.address().cloned() else { return Ok(()) };
        if let Some(id) = self.session.bookmarks.find(&address.text) {
            self.library.star = Some(id);
            return self.send_star();
        }
        let page = address.page.and_then(|page| tab.channel.page_title(page).map(|title| format!("{page:03X} {title}")));
        let name = match page {
            Some(page) => format!("{} {page}", tab.channel.title),
            None => tab.channel.title.clone(),
        };
        let folder = self.library.last_folder.filter(|folder| self.session.bookmarks.item(*folder).is_some()).unwrap_or(OTHER);
        match self.session.bookmarks.add(folder, &clean(&name, MAX_TITLE), &address.text) {
            Ok(id) => {
                self.library.star = Some(id);
                self.bookmarks_changed()
            }
            Err(reason) => self.say(reason),
        }
    }

    /// What the star's popover asks for.
    pub(crate) fn star_request(&mut self, request: StarRequest) -> Result<(), String> {
        let Some(id) = self.library.star.filter(|id| self.session.bookmarks.item(*id).is_some()) else {
            self.library.star = None;
            return self.send_star();
        };
        match request {
            StarRequest::Edit { name, folder } => {
                let _ = self.session.bookmarks.edit(id, &name, "");
                if let Some(&folder) = self.library.star_numbers.get(folder)
                    && self.session.bookmarks.item(id).is_some_and(|item| item.parent != folder)
                    && self.session.bookmarks.move_to(id, folder, usize::MAX).is_ok()
                {
                    self.library.last_folder = Some(folder);
                }
            }
            StarRequest::WholeChannel => {
                let channel = self
                    .session
                    .bookmarks
                    .item(id)
                    .and_then(|item| item.address.as_deref())
                    .and_then(|address| neoteletext::address::parse(address).ok())
                    .map(|address| address.channel_text().to_owned());
                let name = self.session.bookmarks.item(id).map(|item| item.name.clone()).unwrap_or_default();
                if let Some(channel) = channel {
                    let _ = self.session.bookmarks.edit(id, &name, &channel);
                }
            }
            StarRequest::Remove => {
                let _ = self.session.bookmarks.remove(id);
                self.library.star = None;
            }
            StarRequest::Done => self.library.star = None,
        }
        self.bookmarks_changed()
    }

    /// The star's popover.
    pub(crate) fn send_star(&mut self) -> Result<(), String> {
        self.library.star_numbers.fresh();
        let item = self.library.star.and_then(|id| self.session.bookmarks.item(id)).cloned();
        let Some(item) = item else {
            self.library.star = None;
            return self.send(&ToInterface::Star(StarView::default()));
        };
        let folders: Vec<FolderRow> = self
            .session
            .bookmarks
            .folders()
            .into_iter()
            .take(MAX_VIEW_ROWS)
            .filter_map(|(id, depth)| {
                let name = clean(&self.session.bookmarks.item(id)?.name, MAX_NAME);
                Some(FolderRow { number: self.library.star_numbers.number(id), name, depth: depth_of(depth) })
            })
            .collect();
        let folder = folders.iter().find(|row| self.library.star_numbers.get(row.number) == Some(&item.parent)).map_or(0, |row| row.number);
        let page = item.address.as_deref().and_then(|address| neoteletext::address::parse(address).ok()).is_some_and(|address| address.page.is_some());
        self.send(&ToInterface::Star(StarView { open: true, name: clean(&item.name, MAX_NAME), folder, folders, page }))
    }

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
