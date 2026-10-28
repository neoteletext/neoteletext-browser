//! The star's popover (`history-and-bookmarks.md` section 3.2,
//! requirement 20.16): it opens under the star once the trusted core has
//! bookmarked the address the strip shows, and asks for every change, a
//! name, a folder, the whole channel, a removal, by number.

use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::bookmarks_bar::CHIP_RADIUS;
use crate::browser::widgets::{TRIANGLE, button, field, panel, triangle};
use crate::browser::{Browser, Key, Output};
use crate::canvas::Canvas;
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::library::{MAX_NAME, StarRequest, StarView};
use crate::text::{HAIRLINE, border, left_centred};
use crate::theme::{BLACK, BUTTON, HEADING, ROW_TITLE, RULE, RULE_SOFT, SELECTED, WHITE};

const POPOVER_WIDTH: f64 = 380.0;
const PADDING: f64 = 16.0;
const LINE: f64 = 32.0;
pub(super) const GAP: f64 = 8.0;
const LABEL_WIDTH: f64 = 72.0;
const BUTTON_WIDTH: f64 = 90.0;
pub(super) const LIST_ROW: f64 = 28.0;
const INDENT: f64 = 14.0;
/// The most folders the popover's list shows at once.
const LIST_SHOWN: usize = 10;

/// Where the popover's controls were last drawn.
#[derive(Debug, Default)]
struct PopoverControls {
    area: Rect,
    name: Rect,
    folder: Rect,
    folders: Vec<(u32, Rect)>,
    whole: Rect,
    remove: Rect,
    done: Rect,
}

/// The popover's state in the browser.
pub(super) struct StarUi {
    view: StarView,
    name: Editor,
    focused: bool,
    /// The folder chosen, sent with the name when done.
    folder: u32,
    listing: bool,
    controls: PopoverControls,
}

impl Default for StarUi {
    fn default() -> Self {
        Self { view: StarView::default(), name: Editor::new(MAX_NAME), focused: false, folder: 0, listing: false, controls: PopoverControls::default() }
    }
}

impl Browser {
    /// The star's popover as the trusted core sent it.
    pub fn set_star(&mut self, view: StarView) {
        let opening = view.open && !self.star.view.open;
        if opening || !self.star.focused {
            self.star.name.set(&view.name);
        }
        self.star.folder = view.folder;
        if !view.open {
            self.unfocus_star();
            self.star.listing = false;
        }
        self.star.view = view;
    }

    /// Whether the popover is open.
    #[must_use]
    pub fn star_open(&self) -> bool {
        self.star.view.open
    }

    fn unfocus_star(&mut self) {
        if self.star.focused {
            self.star.focused = false;
            self.outputs.push(Output::TextInput(false));
        }
    }

    /// Done: the name and folder go with it, and the popover closes.
    pub(super) fn star_done(&mut self) {
        let name = String::from(self.star.name.text().trim());
        self.unfocus_star();
        self.outputs.push(Output::Star(StarRequest::Edit { name, folder: self.star.folder }));
        self.outputs.push(Output::Star(StarRequest::Done));
        self.star.view.open = false;
        self.star.listing = false;
    }

    /// Whether the popover's name takes the keys.
    pub(super) fn star_editing(&self) -> bool {
        self.star.view.open && self.star.focused
    }

    /// A key while the popover is open; `None` when it is not the popover's.
    pub(super) fn star_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
        if !self.star.view.open {
            return None;
        }
        if !self.star.focused {
            if matches!(key, Key::Escape | Key::Enter) {
                self.star_done();
                return Some(true);
            }
            return None;
        }
        let editor = &mut self.star.name;
        match key {
            Key::Text(text) => editor.insert(text),
            Key::Backspace => editor.backspace(),
            Key::Delete => editor.delete(),
            Key::Left => editor.left(shift),
            Key::Right => editor.right(shift),
            Key::Home | Key::Up => editor.home(shift),
            Key::End | Key::Down => editor.end(shift),
            Key::Escape | Key::Enter => self.star_done(),
            Key::Tab => self.unfocus_star(),
        }
        Some(true)
    }

    /// The popover's name, while it has the keys.
    pub(super) fn star_field(&mut self) -> Option<&mut Editor> {
        self.star_editing().then_some(&mut self.star.name)
    }

    /// A press while the popover is open; a press outside it is Done, as a
    /// browser's bookmark bubble closes, keeping what it shows.
    /// Whether `point` is on the star's open popover.
    pub(super) fn over_star(&self, point: Point) -> bool {
        self.star.view.open && self.star.controls.area.contains(point)
    }

    pub(super) fn star_press(&mut self, point: Point) -> bool {
        if !self.star.view.open {
            return false;
        }
        let controls = &self.star.controls;
        if !controls.area.contains(point) {
            self.star_done();
            // The star itself pressed again only closes the popover.
            return self.regions.star.contains(point);
        }
        if controls.name.contains(point) {
            if !self.star.focused {
                self.star.focused = true;
                self.outputs.push(Output::TextInput(true));
            }
            return true;
        }
        self.unfocus_star();
        let controls = &self.star.controls;
        if let Some(&(number, _)) = controls.folders.iter().find(|(_, rect)| rect.contains(point)) {
            self.star.folder = number;
            self.star.listing = false;
        } else if controls.folder.contains(point) {
            self.star.listing = !self.star.listing;
        } else if controls.whole.contains(point) {
            let name = String::from(self.star.name.text().trim());
            self.outputs.push(Output::Star(StarRequest::Edit { name, folder: self.star.folder }));
            self.outputs.push(Output::Star(StarRequest::WholeChannel));
        } else if controls.remove.contains(point) {
            self.outputs.push(Output::Star(StarRequest::Remove));
            self.star.view.open = false;
        } else if controls.done.contains(point) {
            self.star_done();
        }
        true
    }

    /// The popover, under the star.
    pub(super) fn draw_star_popover<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let mut controls = PopoverControls::default();
        if !self.star.view.open {
            self.star.controls = controls;
            return;
        }
        let star = self.regions.star;
        let lines = if self.star.view.page { 5.0 } else { 4.0 };
        let height = PADDING * 2.0 + LINE * lines + GAP * (lines - 1.0);
        let x1 = star.x1.min(self.body.x1);
        let area = Rect::new((x1 - POPOVER_WIDTH).max(self.body.x0), star.y1 + GAP, x1, star.y1 + GAP + height);
        panel(canvas, area);
        controls.area = area;
        let (left, right) = (area.x0 + PADDING, area.x1 - PADDING);
        let mut y = area.y0 + PADDING;
        left_centred(canvas, Rect::new(left, y, right, y + LINE), 0.0, "Bookmark added", HEADING, WHITE);
        y += LINE + GAP;
        left_centred(canvas, Rect::new(left, y, left + LABEL_WIDTH, y + LINE), 0.0, "Name", ROW_TITLE, WHITE);
        controls.name = Rect::new(left + LABEL_WIDTH, y, right, y + LINE);
        field(canvas, controls.name, &self.star.name, self.star.focused);
        y += LINE + GAP;
        left_centred(canvas, Rect::new(left, y, left + LABEL_WIDTH, y + LINE), 0.0, "Folder", ROW_TITLE, WHITE);
        controls.folder = Rect::new(left + LABEL_WIDTH, y, right, y + LINE);
        let chosen = self.star.view.folders.iter().find(|folder| folder.number == self.star.folder).map_or_else(String::new, |folder| folder.name.clone());
        canvas.fill(controls.folder, CHIP_RADIUS, BUTTON);
        border(canvas, controls.folder, HAIRLINE, RULE);
        canvas.push_clip(Rect::new(controls.folder.x0, controls.folder.y0, controls.folder.x1 - TRIANGLE - GAP * 2.0, controls.folder.y1));
        left_centred(canvas, controls.folder, GAP, &chosen, ROW_TITLE, WHITE);
        canvas.pop_clip();
        triangle(canvas, Point::new(controls.folder.x1 - GAP - TRIANGLE / 2.0, f64::midpoint(controls.folder.y0, controls.folder.y1)), true, WHITE);
        let list_top = controls.folder.y1;
        y += LINE + GAP;
        if self.star.view.page {
            controls.whole = Rect::new(left, y, right, y + LINE);
            button(canvas, controls.whole, "The whole channel", false);
            y += LINE + GAP;
        }
        controls.done = Rect::new(right - BUTTON_WIDTH, y, right, y + LINE);
        controls.remove = Rect::new(controls.done.x0 - GAP - BUTTON_WIDTH, y, controls.done.x0 - GAP, y + LINE);
        button(canvas, controls.remove, "Remove", false);
        button(canvas, controls.done, "Done", true);
        // The folders to choose from, over the rest.
        if self.star.listing {
            let shown = self.star.view.folders.len().min(LIST_SHOWN);
            #[allow(clippy::cast_precision_loss, reason = "at most ten rows")]
            let list = Rect::new(left + LABEL_WIDTH, list_top, right, list_top + LIST_ROW * shown as f64);
            canvas.fill(list, 0.0, BLACK);
            border(canvas, list, HAIRLINE, RULE);
            let mut row_y = list.y0;
            for folder in self.star.view.folders.iter().take(LIST_SHOWN) {
                let rect = Rect::new(list.x0, row_y, list.x1, row_y + LIST_ROW);
                if folder.number == self.star.folder {
                    canvas.fill(rect, 0.0, SELECTED);
                } else if rect.contains(self.pointer) {
                    canvas.fill(rect, 0.0, RULE_SOFT);
                }
                left_centred(canvas, rect, GAP + INDENT * f64::from(folder.depth), &folder.name, ROW_TITLE, WHITE);
                controls.folders.push((folder.number, rect));
                row_y += LIST_ROW;
            }
        }
        self.star.controls = controls;
    }
}

#[cfg(test)]
mod tests;
