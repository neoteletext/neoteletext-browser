//! The bookmarks manager's input: the form's start and submission, the
//! keys, the wheel, presses on its controls, and an item pressed then let
//! go, opened or dragged to a new place.

use alloc::string::String;

use super::{DRAG_DISTANCE, Field, Form, ItemControls, WHEEL_ROWS};
use crate::browser::{Browser, Key, Output};
use crate::geometry::Point;
use crate::library::BookmarkRequest;

impl Browser {
    fn start_form(&mut self, form: Form, name: &str, address: &str) {
        self.bookmarks.form = Some(form);
        self.bookmarks.name.set(name);
        self.bookmarks.address.set(address);
        self.focus_bookmarks(if form == Form::Import { Field::Address } else { Field::Name });
    }

    /// The form's change goes to the trusted core.
    fn submit_form(&mut self) {
        let Some(form) = self.bookmarks.form.take() else { return };
        let name = String::from(self.bookmarks.name.text().trim());
        let address = String::from(self.bookmarks.address.text().trim());
        self.unfocus_bookmarks();
        let request = match form {
            Form::Add => BookmarkRequest::Add { name, address },
            Form::Folder => BookmarkRequest::NewFolder(name),
            Form::Import => BookmarkRequest::Import(address),
            Form::Edit(number) => BookmarkRequest::Edit { number, name, address },
        };
        self.outputs.push(Output::Bookmarks(request));
    }

    /// A key while the manager is open; `None` when it is not the manager's.
    pub(in crate::browser) fn bookmarks_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
        if !self.bookmarks.open {
            return None;
        }
        let Some(field) = self.bookmarks.focus else {
            if key == Key::Escape {
                if self.bookmarks.moving.take().is_some() {
                    return Some(true);
                }
                if self.bookmarks.form.take().is_none() {
                    self.toggle_bookmarks();
                }
                return Some(true);
            }
            return None;
        };
        let editor = self.bookmarks.editor(field);
        let before = String::from(editor.text());
        match key {
            Key::Text(text) => editor.insert(text),
            Key::Backspace => editor.backspace(),
            Key::Delete => editor.delete(),
            Key::Left => editor.left(shift),
            Key::Right => editor.right(shift),
            Key::Home | Key::Up => editor.home(shift),
            Key::End | Key::Down => editor.end(shift),
            Key::Escape => {
                if field != Field::Query {
                    self.bookmarks.form = None;
                }
                self.unfocus_bookmarks();
            }
            Key::Enter => {
                if field == Field::Query {
                    self.unfocus_bookmarks();
                } else {
                    self.submit_form();
                }
            }
            Key::Tab => {
                let next = match (field, self.bookmarks.form) {
                    (Field::Name, Some(Form::Add | Form::Edit(_))) => Field::Address,
                    (Field::Address, Some(Form::Add | Form::Edit(_))) => Field::Name,
                    (other, _) => other,
                };
                self.focus_bookmarks(next);
            }
        }
        // The search follows each change as it is typed.
        if field == Field::Query && self.bookmarks.query.text() != before {
            let text = String::from(self.bookmarks.query.text().trim());
            self.outputs.push(Output::Bookmarks(BookmarkRequest::Query(text)));
        }
        Some(true)
    }

    /// The wheel over the manager: its lists move.
    pub(in crate::browser) fn bookmarks_scroll(&mut self, distance: f64) -> bool {
        if !self.bookmarks.open || !self.bookmarks.controls.panel.contains(self.pointer) {
            return false;
        }
        let Some(view) = &self.bookmarks.view else { return true };
        let step = |first: u32, total: u32| {
            if distance > 0.0 { first.saturating_add(WHEEL_ROWS).min(total.saturating_sub(1)) } else { first.saturating_sub(WHEEL_ROWS) }
        };
        let (mut folders_first, mut items_first) = (view.folders_first, view.items_first);
        if self.bookmarks.controls.tree.contains(self.pointer) {
            folders_first = step(folders_first, view.folders_total);
        } else {
            items_first = step(items_first, view.items_total);
        }
        if (folders_first, items_first) != (view.folders_first, view.items_first) {
            self.outputs.push(Output::Bookmarks(BookmarkRequest::Scroll { folders_first, items_first }));
        }
        true
    }

    /// A press on the manager; returns whether it was the manager's. An
    /// item pressed is opened or moved when the button is released.
    #[allow(clippy::too_many_lines, reason = "one hit-test per control, in drawing order")]
    pub(in crate::browser) fn bookmarks_press(&mut self, point: Point) -> bool {
        if !self.bookmarks.open {
            return false;
        }
        let controls = &self.bookmarks.controls;
        if !controls.panel.contains(point) {
            return false;
        }
        // The folder chooser takes the press; anywhere else closes it.
        if let Some(number) = self.bookmarks.moving.take() {
            if let Some(&(folder, _)) = controls.chooser.iter().find(|(_, rect)| rect.contains(point)) {
                self.outputs.push(Output::Bookmarks(BookmarkRequest::Move { number, folder, index: u32::MAX }));
            }
            return true;
        }
        if controls.close.contains(point) {
            self.toggle_bookmarks();
            return true;
        }
        for (field, rect) in [(Field::Query, controls.search), (Field::Name, controls.name), (Field::Address, controls.address)] {
            if rect.contains(point) {
                self.focus_bookmarks(field);
                return true;
            }
        }
        let controls = &self.bookmarks.controls;
        let (save, cancel, undo) = (controls.save.contains(point), controls.cancel.contains(point), controls.undo.contains(point));
        let form = if controls.new_folder.contains(point) {
            Some(Form::Folder)
        } else if controls.add.contains(point) {
            Some(Form::Add)
        } else if controls.import.contains(point) {
            Some(Form::Import)
        } else {
            None
        };
        let simple = if controls.sort.contains(point) {
            Some(BookmarkRequest::Sort)
        } else if controls.export.contains(point) {
            Some(BookmarkRequest::Export)
        } else {
            None
        };
        let folder = controls.folders.iter().find(|(_, rect)| rect.contains(point)).map(|&(number, _)| number);
        let item = controls.items.iter().find(|item| item.row.contains(point)).copied();
        if save {
            self.submit_form();
            return true;
        }
        self.unfocus_bookmarks();
        if cancel {
            self.bookmarks.form = None;
        } else if undo {
            self.bookmarks.undo_since = None;
            self.outputs.push(Output::Bookmarks(BookmarkRequest::Undo));
        } else if let Some(form) = form {
            self.start_form(form, "", "");
        } else if let Some(request) = simple {
            self.outputs.push(Output::Bookmarks(request));
        } else if let Some(number) = folder {
            self.outputs.push(Output::Bookmarks(BookmarkRequest::Folder(number)));
        } else if let Some(ItemControls { number, edit, moving, remove, .. }) = item {
            if remove.contains(point) {
                self.outputs.push(Output::Bookmarks(BookmarkRequest::Remove(number)));
            } else if moving.contains(point) {
                self.bookmarks.moving = Some(number);
            } else if edit.contains(point) {
                let shown = self.bookmarks.view.as_ref().and_then(|view| view.items.iter().find(|item| item.number == number)).cloned();
                if let Some(shown) = shown {
                    self.start_form(Form::Edit(number), &shown.name, &shown.address);
                }
            } else {
                // Pressed: a release where it was opens it, a move drags it.
                self.bookmarks.pressed = Some((number, point));
            }
        }
        true
    }

    /// The bookmark, not a folder, whose row is at `point`, away from its
    /// buttons.
    pub(in crate::browser) fn bookmark_at(&self, point: Point) -> Option<u32> {
        if !self.bookmarks.open || self.bookmarks.moving.is_some() {
            return None;
        }
        let item = self.bookmarks.controls.items.iter().find(|item| item.row.contains(point))?;
        let on_button = [item.edit, item.moving, item.remove].iter().any(|button| button.contains(point));
        (!item.folder && !on_button).then_some(item.number)
    }

    /// The main button released over the manager: an item pressed and not
    /// moved opens; one dragged onto a folder or another item moves there.
    pub(in crate::browser) fn bookmarks_released(&mut self, new_tab: bool) -> bool {
        let Some((number, from)) = self.bookmarks.pressed.take() else { return false };
        let point = self.pointer;
        let controls = &self.bookmarks.controls;
        let moved = (point.x - from.x).abs() > DRAG_DISTANCE || (point.y - from.y).abs() > DRAG_DISTANCE;
        let is_folder = controls.items.iter().any(|item| item.number == number && item.folder);
        if !moved {
            let request = if is_folder { BookmarkRequest::Folder(number) } else { BookmarkRequest::Open { number, new_tab } };
            self.outputs.push(Output::Bookmarks(request));
            if !is_folder && !new_tab {
                self.toggle_bookmarks();
            }
            return true;
        }
        // Dropped on a folder: into it, at its end; on an item of the open
        // folder: before it.
        let onto_folder = controls.folders.iter().find(|(_, rect)| rect.contains(point)).map(|&(folder, _)| folder);
        let onto_item = controls.items.iter().position(|item| item.row.contains(point));
        let open_folder = self.bookmarks.view.as_ref().and_then(|view| view.path.last()).map(|folder| folder.number);
        let request = match (onto_folder, onto_item, open_folder) {
            (Some(folder), _, _) if folder != number => Some(BookmarkRequest::Move { number, folder, index: u32::MAX }),
            (None, Some(index), Some(folder)) => {
                let first = self.bookmarks.view.as_ref().map_or(0, |view| view.items_first);
                Some(BookmarkRequest::Move { number, folder, index: first.saturating_add(u32::try_from(index).unwrap_or(u32::MAX)) })
            }
            _ => None,
        };
        if let Some(request) = request {
            self.outputs.push(Output::Bookmarks(request));
        }
        true
    }
}
