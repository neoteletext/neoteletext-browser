//! The bookmarks manager drawn: the toolbar and search, the folder tree,
//! the open folder's path, the form, the contents with each row's buttons,
//! the folder chooser, Undo and the trusted core's word; and its tips.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{
    BUTTON_WIDTH, CHOOSER_WIDTH, CONTROL_HEIGHT, Controls, DRAG_DISTANCE, EDIT_BUTTON, Field, Form, GAP, HEADING_HEIGHT, INDENT, ItemControls, MOVE_BUTTON,
    PADDING, PANEL_HEIGHT, PANEL_WIDTH, ROW_HEIGHT, SMALL_BUTTON, TREE_WIDTH,
};
use crate::browser::Browser;
use crate::browser::widgets::{TRIANGLE, button, field, panel, panel_rect, triangle};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::library::BookmarksView;
use crate::text::{border, centred, left_centred};
use crate::theme::{ADDRESS, BUTTON, CAPTION, DIM, HEADING, MUTED, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SELECTED, WHITE};

impl Browser {
    /// The manager's symbol buttons.
    pub(in crate::browser) fn bookmarks_tips(&self, tips: &mut Vec<(Rect, String)>) {
        let controls = &self.bookmarks.controls;
        tips.push((controls.close, format!("Close bookmarks ({})", self.keys_named("Option+Cmd+B", "Ctrl+Shift+O"))));
        for item in &controls.items {
            tips.push((item.remove, if item.folder { "Delete this folder and what it holds" } else { "Delete this bookmark" }.into()));
        }
    }

    /// The folder chooser of the item being moved, below its "Move…"
    /// button, or above it near the foot of `within`: the folders the tree
    /// shows, as far as they fit, but the item itself.
    fn draw_chooser<C: Canvas + ?Sized>(&mut self, canvas: &mut C, view: &BookmarksView, controls: &mut Controls, within: Rect) {
        let Some(number) = self.bookmarks.moving else { return };
        let Some(item) = controls.items.iter().find(|item| item.number == number).copied() else {
            // Its row is gone: scrolled away, or the item removed.
            self.bookmarks.moving = None;
            return;
        };
        let folders: Vec<_> = view.folders.iter().filter(|folder| folder.number != number).collect();
        let fit = libm::floor((within.y1 - within.y0) / ROW_HEIGHT);
        #[allow(clippy::cast_precision_loss, reason = "at most a view's rows")]
        let rows = (folders.len() as f64).min(fit).max(1.0);
        let height = rows * ROW_HEIGHT;
        let top = if item.moving.y1 + height > within.y1 { (item.moving.y0 - height).max(within.y0) } else { item.moving.y1 };
        let area = Rect::new((item.moving.x1 - CHOOSER_WIDTH).max(within.x0), top, item.moving.x1, top + height);
        canvas.fill(area, 0.0, BUTTON);
        border(canvas, area, 1.0, RULE);
        if folders.is_empty() {
            left_centred(canvas, area, GAP, "No other folder to move it into", ROW_STATUS, DIM);
        }
        canvas.push_clip(area);
        let mut y = area.y0;
        for folder in folders {
            if y + ROW_HEIGHT > area.y1 + f64::EPSILON {
                break;
            }
            let row = Rect::new(area.x0, y, area.x1, y + ROW_HEIGHT);
            if row.contains(self.pointer) {
                canvas.fill(row, 0.0, SELECTED);
            }
            left_centred(canvas, row, GAP + INDENT * f64::from(folder.depth), &folder.name, ROW_TITLE, WHITE);
            controls.chooser.push((folder.number, row));
            y += ROW_HEIGHT;
        }
        canvas.pop_clip();
        controls.chooser_area = area;
    }

    /// The manager, over the window below the strip, while it is open.
    pub(in crate::browser) fn draw_bookmarks_panel<C: Canvas + ?Sized>(&mut self, canvas: &mut C, now: u64) {
        let mut controls = Controls::default();
        if !self.bookmarks.open {
            self.bookmarks.controls = controls;
            return;
        }
        let area = panel_rect(self.body, PANEL_WIDTH, PANEL_HEIGHT);
        panel(canvas, area);
        controls.panel = area;
        let (left, right) = (area.x0 + PADDING, area.x1 - PADDING);
        let heading = Rect::new(left, area.y0 + PADDING, right, area.y0 + PADDING + HEADING_HEIGHT);
        left_centred(canvas, heading, 0.0, "Bookmarks", HEADING, WHITE);
        controls.close = Rect::new(right - SMALL_BUTTON, heading.y0, right, heading.y0 + SMALL_BUTTON);
        button(canvas, controls.close, "\u{00D7}", false);
        let view = self.bookmarks.view.clone().unwrap_or_default();
        let mut y = heading.y1 + GAP;

        self.draw_bookmarks_toolbar(canvas, &mut controls, Rect::new(left, y, right, y + CONTROL_HEIGHT));
        y += CONTROL_HEIGHT + GAP;
        let footer = area.y1 - PADDING - CONTROL_HEIGHT;

        controls.tree = Rect::new(left, y, left + TREE_WIDTH, footer - GAP);
        self.draw_bookmarks_tree(canvas, &view, &mut controls);

        let contents_left = controls.tree.x1 + GAP;
        let path_line = Rect::new(contents_left, y, right, y + CONTROL_HEIGHT);
        draw_path(canvas, &view, &mut controls, path_line);
        let row_y = self.draw_bookmarks_form(canvas, &mut controls, Rect::new(contents_left, path_line.y1, right, footer));

        controls.contents = Rect::new(contents_left, row_y, right, footer - GAP);
        self.draw_bookmarks_contents(canvas, &view, &mut controls);
        let within = Rect::new(left, controls.contents.y0, right, footer);
        self.draw_chooser(canvas, &view, &mut controls, within);

        self.draw_bookmarks_footer(canvas, &view, &mut controls, Rect::new(left, footer, right, footer + CONTROL_HEIGHT), now);
        self.bookmarks.controls = controls;
    }

    /// The search, and the toolbar, along `line`.
    fn draw_bookmarks_toolbar<C: Canvas + ?Sized>(&self, canvas: &mut C, controls: &mut Controls, line: Rect) {
        let (left, right, y) = (line.x0, line.x1, line.y0);
        let mut x = right;
        for (rect, word) in [
            (&mut controls.export, "Export"),
            (&mut controls.import, "Import\u{2026}"),
            (&mut controls.sort, "Sort by name"),
            (&mut controls.add, "Add bookmark"),
            (&mut controls.new_folder, "New folder"),
        ] {
            *rect = Rect::new(x - BUTTON_WIDTH, y, x, y + CONTROL_HEIGHT);
            button(canvas, *rect, word, false);
            x = rect.x0 - GAP;
        }
        controls.search = Rect::new(left, y, x, y + CONTROL_HEIGHT);
        field(canvas, controls.search, &self.bookmarks.query, self.bookmarks.focus == Some(Field::Query));
        if self.bookmarks.query.text().is_empty() && self.bookmarks.focus != Some(Field::Query) {
            left_centred(canvas, controls.search, GAP, "Search bookmarks", ROW_STATUS, DIM);
        }
    }

    /// The folder tree, in `controls.tree`.
    fn draw_bookmarks_tree<C: Canvas + ?Sized>(&self, canvas: &mut C, view: &BookmarksView, controls: &mut Controls) {
        let (left, y) = (controls.tree.x0, controls.tree.y0);
        canvas.fill(Rect::new(controls.tree.x1 + GAP / 2.0 - 0.5, y, controls.tree.x1 + GAP / 2.0 + 0.5, controls.tree.y1), 0.0, RULE);
        let open_folder = view.path.last().map(|folder| folder.name.clone());
        canvas.push_clip(controls.tree);
        let mut row_y = y;
        for folder in &view.folders {
            if row_y + ROW_HEIGHT > controls.tree.y1 {
                break;
            }
            let rect = Rect::new(left, row_y, controls.tree.x1, row_y + ROW_HEIGHT);
            let current = view.query.is_empty() && open_folder.as_deref() == Some(folder.name.as_str()) && view.path.len() == usize::from(folder.depth) + 1;
            if current {
                canvas.fill(rect, 0.0, SELECTED);
            } else if rect.contains(self.pointer) {
                canvas.fill(rect, 0.0, RULE_SOFT);
            }
            let inset = INDENT * f64::from(folder.depth) + GAP;
            triangle(canvas, Point::new(rect.x0 + inset + TRIANGLE / 2.0, f64::midpoint(rect.y0, rect.y1)), false, WHITE);
            left_centred(canvas, rect, inset + TRIANGLE + GAP, &folder.name, ROW_TITLE, WHITE);
            controls.folders.push((folder.number, rect));
            row_y += ROW_HEIGHT;
        }
        canvas.pop_clip();
    }

    /// The form at the top of `room`: a new bookmark, a new folder, an
    /// import, an edit. Returns where the contents start below it.
    fn draw_bookmarks_form<C: Canvas + ?Sized>(&self, canvas: &mut C, controls: &mut Controls, room: Rect) -> f64 {
        let (contents_left, right, mut row_y) = (room.x0, room.x1, room.y0);
        if let Some(form) = self.bookmarks.form {
            let two = matches!(form, Form::Add | Form::Edit(_));
            let (name_label, address_label) = match form {
                Form::Import => ("", "The file's path"),
                Form::Folder => ("The folder's name", ""),
                _ => ("Name", "Address"),
            };
            controls.cancel = Rect::new(right - BUTTON_WIDTH * 0.7, row_y, right, row_y + CONTROL_HEIGHT);
            controls.save = Rect::new(controls.cancel.x0 - GAP - BUTTON_WIDTH * 0.7, row_y, controls.cancel.x0 - GAP, row_y + CONTROL_HEIGHT);
            button(canvas, controls.save, if form == Form::Import { "Import" } else { "Save" }, true);
            button(canvas, controls.cancel, "Cancel", false);
            let room = controls.save.x0 - GAP - contents_left;
            let (name_width, address_width) = if two {
                (room * 0.4, room * 0.6 - GAP)
            } else if form == Form::Import {
                (0.0, room)
            } else {
                (room, 0.0)
            };
            if name_width > 0.0 {
                controls.name = Rect::new(contents_left, row_y, contents_left + name_width, row_y + CONTROL_HEIGHT);
                field(canvas, controls.name, &self.bookmarks.name, self.bookmarks.focus == Some(Field::Name));
                if self.bookmarks.name.text().is_empty() && self.bookmarks.focus != Some(Field::Name) {
                    left_centred(canvas, controls.name, GAP, name_label, ROW_STATUS, DIM);
                }
            }
            if address_width > 0.0 {
                let x0 = if name_width > 0.0 { controls.name.x1 + GAP } else { contents_left };
                controls.address = Rect::new(x0, row_y, x0 + address_width, row_y + CONTROL_HEIGHT);
                field(canvas, controls.address, &self.bookmarks.address, self.bookmarks.focus == Some(Field::Address));
                if self.bookmarks.address.text().is_empty() && self.bookmarks.focus != Some(Field::Address) {
                    left_centred(canvas, controls.address, GAP, address_label, ROW_STATUS, DIM);
                }
            }
            row_y += CONTROL_HEIGHT + GAP;
        }
        row_y
    }

    /// The open folder's contents, or the search's results, in
    /// `controls.contents`, each row with its buttons.
    fn draw_bookmarks_contents<C: Canvas + ?Sized>(&self, canvas: &mut C, view: &BookmarksView, controls: &mut Controls) {
        let (contents_left, right, mut row_y) = (controls.contents.x0, controls.contents.x1, controls.contents.y0);
        canvas.push_clip(controls.contents);
        if view.items.is_empty() {
            let empty = if view.query.is_empty() { "This folder is empty" } else { "No bookmark matches" };
            centred(canvas, Rect::new(contents_left, row_y, right, row_y + ROW_HEIGHT * 2.0), empty, ROW_TITLE, MUTED);
        }
        let dragging =
            self.bookmarks.pressed.filter(|(_, from)| (self.pointer.x - from.x).abs() > DRAG_DISTANCE || (self.pointer.y - from.y).abs() > DRAG_DISTANCE);
        for item in &view.items {
            if row_y + ROW_HEIGHT > controls.contents.y1 {
                break;
            }
            let rect = Rect::new(contents_left, row_y, right, row_y + ROW_HEIGHT);
            let hovered = rect.contains(self.pointer);
            if dragging.is_some_and(|(number, _)| number == item.number) {
                canvas.fill(rect, 0.0, SELECTED);
            } else if hovered {
                canvas.fill(rect, 0.0, RULE_SOFT);
            }
            let remove = Rect::new(right - SMALL_BUTTON, row_y, right, row_y + ROW_HEIGHT);
            let edit = Rect::new(remove.x0 - EDIT_BUTTON, row_y, remove.x0, row_y + ROW_HEIGHT);
            let moving = Rect::new(edit.x0 - MOVE_BUTTON, row_y, edit.x0, row_y + ROW_HEIGHT);
            if hovered || self.bookmarks.moving == Some(item.number) {
                centred(canvas, moving, "Move\u{2026}", ROW_STATUS, WHITE);
                centred(canvas, edit, "Edit", ROW_STATUS, WHITE);
                centred(canvas, remove, "\u{00D7}", ROW_TITLE, WHITE);
            }
            let text = Rect::new(contents_left + GAP, row_y, moving.x0 - GAP, row_y + ROW_HEIGHT);
            let middle = f64::midpoint(text.x0, text.x1);
            canvas.push_clip(Rect::new(text.x0, row_y, if item.folder { text.x1 } else { middle - GAP }, row_y + ROW_HEIGHT));
            let inset = if item.folder {
                triangle(canvas, Point::new(text.x0 + TRIANGLE / 2.0, f64::midpoint(rect.y0, rect.y1)), false, WHITE);
                TRIANGLE + GAP
            } else {
                0.0
            };
            left_centred(canvas, text, inset, &item.name, ROW_TITLE, WHITE);
            canvas.pop_clip();
            // A bookmark's address, always beside its name (requirement 20.13).
            if !item.folder {
                let place = if item.place.is_empty() { item.address.clone() } else { format!("{}  \u{00B7}  {}", item.address, item.place) };
                canvas.push_clip(Rect::new(middle, row_y, text.x1, row_y + ROW_HEIGHT));
                left_centred(canvas, Rect::new(middle, row_y, text.x1, row_y + ROW_HEIGHT), 0.0, &place, ADDRESS, DIM);
                canvas.pop_clip();
            }
            controls.items.push(ItemControls { number: item.number, folder: item.folder, row: rect, edit, moving, remove });
            row_y += ROW_HEIGHT;
        }
        canvas.pop_clip();
    }

    /// Undo, for a while, and the trusted core's word, along `footer`.
    fn draw_bookmarks_footer<C: Canvas + ?Sized>(&self, canvas: &mut C, view: &BookmarksView, controls: &mut Controls, footer: Rect, now: u64) {
        let (left, right) = (footer.x0, footer.x1);
        if self.bookmarks_undo_shown(now) {
            let text = self.bookmarks.undo_since.as_ref().map(|(_, text)| text.clone()).unwrap_or_default();
            controls.undo = Rect::new(right - BUTTON_WIDTH * 0.7, footer.y0, right, footer.y1);
            button(canvas, controls.undo, "Undo", false);
            canvas.push_clip(Rect::new(left, footer.y0, controls.undo.x0 - GAP, footer.y1));
            left_centred(canvas, footer, 0.0, &text, ROW_STATUS, WHITE);
            canvas.pop_clip();
        } else if !view.notice.is_empty() {
            canvas.push_clip(footer);
            left_centred(canvas, footer, 0.0, &view.notice, ROW_STATUS, MUTED);
            canvas.pop_clip();
        }
    }
}

/// The open folder's path, or the search's word, along `path_line`.
fn draw_path<C: Canvas + ?Sized>(canvas: &mut C, view: &BookmarksView, controls: &mut Controls, path_line: Rect) {
    let (contents_left, y) = (path_line.x0, path_line.y0);
    if view.query.is_empty() {
        let mut x = contents_left;
        for (index, folder) in view.path.iter().enumerate() {
            let word = if index == 0 { folder.name.clone() } else { format!("/ {}", folder.name) };
            let width = canvas.text_width(&word, CAPTION) + GAP;
            let rect = Rect::new(x, y, x + width, y + CONTROL_HEIGHT);
            left_centred(canvas, rect, 0.0, &word, CAPTION, WHITE);
            controls.folders.push((folder.number, rect));
            x = rect.x1;
        }
    } else {
        left_centred(canvas, path_line, 0.0, &format!("Bookmarks that match \u{201C}{}\u{201D}", view.query), CAPTION, WHITE);
    }
}
