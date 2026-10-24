//! The star and its popover (`history-and-bookmarks.md` section 3,
//! requirement 20.16): the address the strip shows bookmarked, or its
//! bookmark shown, renamed, moved or removed.

use neoteletext::bookmarks::OTHER;
use protocol::library::{FolderRow, MAX_NAME, MAX_VIEW_ROWS, StarRequest, StarView};
use protocol::{MAX_TITLE, ToInterface, clean};

use super::super::Core;
use super::depth_of;

impl Core {
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
}
