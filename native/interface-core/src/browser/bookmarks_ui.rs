//! The bookmarks manager of the trusted strip (`history-and-bookmarks.md`
//! section 3.3, requirement 20.17): opened by Command-Option-B or
//! Control-Shift-O and from the menu. It shows the folder tree and a
//! folder's contents as the trusted core sent them, and asks for every
//! change by the items' numbers, as strip input.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::widgets::{TRIANGLE, button, field, panel, panel_rect, triangle};
use super::{Browser, Key, Output};
use crate::canvas::Canvas;
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::library::{BookmarkRequest, BookmarksView, MAX_NAME, MAX_PATH, MAX_QUERY};
use crate::theme::{ADDRESS, BUTTON, CAPTION, DIM, HEADING, MUTED, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SELECTED, WHITE};
use crate::ui::{border, centred, left_centred};

const PANEL_WIDTH: f64 = 900.0;
const PANEL_HEIGHT: f64 = 680.0;
const PADDING: f64 = 20.0;
const HEADING_HEIGHT: f64 = 34.0;
const CONTROL_HEIGHT: f64 = 32.0;
const GAP: f64 = 8.0;
const SMALL_BUTTON: f64 = 32.0;
const BUTTON_WIDTH: f64 = 120.0;
const TREE_WIDTH: f64 = 240.0;
const ROW_HEIGHT: f64 = 30.0;
const INDENT: f64 = 14.0;
const EDIT_BUTTON: f64 = 56.0;
/// The "Move…" button of a row, and the folder chooser it opens.
const MOVE_BUTTON: f64 = 64.0;
const CHOOSER_WIDTH: f64 = 260.0;
/// How long "Removed …" offers Undo, as long as the strip's notices last.
const UNDO_MILLISECONDS: u64 = 10_000;
/// How far a press moves before it is a drag.
const DRAG_DISTANCE: f64 = 4.0;
/// How many rows one turn of the wheel moves a list.
const WHEEL_ROWS: u32 = 3;

/// A text field of the manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Query,
    Name,
    Address,
}

/// What the form at the top of the contents is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    /// A new bookmark: name and address.
    Add,
    /// A new folder: its name.
    Folder,
    /// An import: the file's path, in the address field.
    Import,
    /// Editing the item of this number.
    Edit(u32),
}

/// Where the manager's controls were last drawn.
#[derive(Debug, Default)]
struct Controls {
    panel: Rect,
    close: Rect,
    search: Rect,
    new_folder: Rect,
    add: Rect,
    sort: Rect,
    import: Rect,
    export: Rect,
    undo: Rect,
    name: Rect,
    address: Rect,
    save: Rect,
    cancel: Rect,
    tree: Rect,
    contents: Rect,
    /// Folders of the tree and of the path: number and row.
    folders: Vec<(u32, Rect)>,
    /// Items of the open folder, as drawn.
    items: Vec<ItemControls>,
    /// The folder chooser's folders, and where it lies, while it is open.
    chooser: Vec<(u32, Rect)>,
    chooser_area: Rect,
}

/// An item's row and buttons, where last drawn.
#[derive(Clone, Copy, Debug, Default)]
struct ItemControls {
    number: u32,
    folder: bool,
    row: Rect,
    edit: Rect,
    /// "Move…": a chooser of the folder to move it into.
    moving: Rect,
    remove: Rect,
}

/// The manager's state in the browser.
pub(super) struct BookmarksUi {
    view: Option<BookmarksView>,
    open: bool,
    focus: Option<Field>,
    query: Editor,
    name: Editor,
    address: Editor,
    form: Option<Form>,
    /// An item pressed, and where, which a move of the pointer makes a drag.
    pressed: Option<(u32, Point)>,
    /// The item whose folder chooser is open.
    moving: Option<u32>,
    /// When the view's "Removed …" arrived, and what it said.
    undo_since: Option<(u64, String)>,
    controls: Controls,
}

impl Default for BookmarksUi {
    fn default() -> Self {
        Self {
            view: None,
            open: false,
            focus: None,
            query: Editor::new(MAX_QUERY),
            name: Editor::new(MAX_NAME),
            address: Editor::new(MAX_PATH),
            form: None,
            pressed: None,
            moving: None,
            undo_since: None,
            controls: Controls::default(),
        }
    }
}

impl BookmarksUi {
    fn editor(&mut self, field: Field) -> &mut Editor {
        match field {
            Field::Query => &mut self.query,
            Field::Name => &mut self.name,
            Field::Address => &mut self.address,
        }
    }
}

impl Browser {
    /// The manager's symbol buttons.
    pub(super) fn bookmarks_tips(&self, tips: &mut Vec<(Rect, String)>) {
        let controls = &self.bookmarks.controls;
        tips.push((controls.close, format!("Close bookmarks ({})", self.keys_named("Option+Cmd+B", "Ctrl+Shift+O"))));
        for item in &controls.items {
            tips.push((item.remove, if item.folder { "Delete this folder and what it holds" } else { "Delete this bookmark" }.into()));
        }
    }

    /// The manager's view as the trusted core sent it, at `now`.
    pub fn set_bookmarks(&mut self, view: BookmarksView, now: u64) {
        let undo = (!view.undo.is_empty()).then(|| view.undo.clone());
        if undo != self.bookmarks.undo_since.as_ref().map(|(_, text)| text.clone()) {
            self.bookmarks.undo_since = undo.map(|text| (now, text));
        }
        self.bookmarks.view = Some(view);
    }

    /// Whether the manager is open.
    #[must_use]
    pub fn bookmarks_open(&self) -> bool {
        self.bookmarks.open
    }

    /// Opens the manager, or closes it.
    pub(super) fn toggle_bookmarks(&mut self) {
        let open = !self.bookmarks.open;
        if open {
            self.close_panels();
        }
        self.bookmarks.open = open;
        self.bookmarks.form = None;
        self.bookmarks.pressed = None;
        self.unfocus_bookmarks();
        if open {
            self.bookmarks.query.set("");
        }
        self.outputs.push(Output::Bookmarks(BookmarkRequest::Panel(open)));
    }

    fn focus_bookmarks(&mut self, field: Field) {
        if self.bookmarks.focus.is_none() {
            self.outputs.push(Output::TextInput(true));
        }
        self.bookmarks.focus = Some(field);
        let editor = self.bookmarks.editor(field);
        let end = editor.text().len();
        editor.place(end, false);
    }

    fn unfocus_bookmarks(&mut self) {
        if self.bookmarks.focus.take().is_some() {
            self.outputs.push(Output::TextInput(false));
        }
    }

    /// Whether a field of the manager takes the keys.
    pub(super) fn bookmarks_editing(&self) -> bool {
        self.bookmarks.open && self.bookmarks.focus.is_some()
    }

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
    pub(super) fn bookmarks_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
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

    /// Command-A in a field of the manager.
    pub(super) fn bookmarks_select_all(&mut self) -> bool {
        let Some(field) = self.bookmarks.focus.filter(|_| self.bookmarks.open) else { return false };
        self.bookmarks.editor(field).select_all();
        true
    }

    /// The wheel over the manager: its lists move.
    pub(super) fn bookmarks_scroll(&mut self, distance: f64) -> bool {
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
    pub(super) fn bookmarks_press(&mut self, point: Point) -> bool {
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
    pub(super) fn bookmark_at(&self, point: Point) -> Option<u32> {
        if !self.bookmarks.open || self.bookmarks.moving.is_some() {
            return None;
        }
        let item = self.bookmarks.controls.items.iter().find(|item| item.row.contains(point))?;
        let on_button = [item.edit, item.moving, item.remove].iter().any(|button| button.contains(point));
        (!item.folder && !on_button).then_some(item.number)
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

    /// The main button released over the manager: an item pressed and not
    /// moved opens; one dragged onto a folder or another item moves there.
    pub(super) fn bookmarks_released(&mut self, new_tab: bool) -> bool {
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

    /// Whether Undo is still offered at `now`.
    pub(super) fn bookmarks_undo_shown(&self, now: u64) -> bool {
        self.bookmarks.undo_since.as_ref().is_some_and(|(since, _)| now.saturating_sub(*since) < UNDO_MILLISECONDS)
    }

    /// When the manager next changes by itself: Undo's end.
    pub(super) fn bookmarks_deadline(&self, now: u64) -> Option<u64> {
        let (since, _) = self.bookmarks.undo_since.as_ref().filter(|_| self.bookmarks.open && self.bookmarks_undo_shown(now))?;
        Some(since.saturating_add(UNDO_MILLISECONDS).saturating_sub(now))
    }

    /// The manager, over the window below the strip, while it is open.
    #[allow(clippy::too_many_lines, reason = "one part of the manager after another, in drawing order")]
    pub(super) fn draw_bookmarks_panel<C: Canvas + ?Sized>(&mut self, canvas: &mut C, now: u64) {
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

        // The search, and the toolbar.
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
        y += CONTROL_HEIGHT + GAP;
        let footer = area.y1 - PADDING - CONTROL_HEIGHT;

        // The folder tree.
        controls.tree = Rect::new(left, y, left + TREE_WIDTH, footer - GAP);
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

        // The open folder's path, or the search's word.
        let contents_left = controls.tree.x1 + GAP;
        let path_line = Rect::new(contents_left, y, right, y + CONTROL_HEIGHT);
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
        let mut row_y = path_line.y1;

        // The form: a new bookmark, a new folder, an import, an edit.
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

        // The open folder's contents, or the search's results.
        controls.contents = Rect::new(contents_left, row_y, right, footer - GAP);
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
        let within = Rect::new(left, controls.contents.y0, right, footer);
        self.draw_chooser(canvas, &view, &mut controls, within);

        // Undo, for a while, and the trusted core's word.
        let footer = Rect::new(left, footer, right, footer + CONTROL_HEIGHT);
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
        self.bookmarks.controls = controls;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, Modifiers};
    use crate::library::{FolderRow, ItemRow};
    use crate::test_canvas::Recorder;

    fn view() -> BookmarksView {
        BookmarksView {
            path: alloc::vec![FolderRow { number: 1, name: "Bookmarks bar".into(), depth: 0 }],
            folders: alloc::vec![
                FolderRow { number: 2, name: "Bookmarks bar".into(), depth: 0 },
                FolderRow { number: 3, name: "Other bookmarks".into(), depth: 0 },
                FolderRow { number: 4, name: "Weather".into(), depth: 1 },
            ],
            folders_first: 0,
            folders_total: 3,
            query: String::new(),
            items: alloc::vec![
                ItemRow { number: 5, folder: false, name: "News Example".into(), address: "https://news.example/".into(), place: String::new() },
                ItemRow { number: 6, folder: true, name: "Sport".into(), address: String::new(), place: String::new() },
            ],
            items_first: 0,
            items_total: 2,
            undo: String::new(),
            notice: String::new(),
        }
    }

    fn open() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.key(Key::Text("b"), Modifiers { command: true, alt: true, ..Modifiers::default() }, 0);
        browser.set_bookmarks(view(), 0);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.take_outputs();
        browser
    }

    fn middle(rect: Rect) -> Point {
        Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
    }

    fn click(browser: &mut Browser, rect: Rect) -> Vec<Output> {
        let point = middle(rect);
        browser.pointer_moved(point.x, point.y, 0);
        browser.bookmarks_press(point);
        browser.bookmarks_released(false);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.take_outputs()
    }

    /// Requirement 20.17: open a bookmark or a folder by number; each
    /// shows its address beside its name.
    #[test]
    fn items_open_by_number() {
        let mut browser = open();
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
        for expected in ["Bookmarks", "News Example", "https://news.example/", "Weather"] {
            assert!(texts.contains(&expected), "{expected} in {texts:?}");
        }
        let folder = browser.bookmarks.controls.items[1].row;
        assert_eq!(click(&mut browser, folder), [Output::Bookmarks(BookmarkRequest::Folder(6))]);
        let bookmark = browser.bookmarks.controls.items[0].row;
        assert_eq!(
            click(&mut browser, bookmark),
            [Output::Bookmarks(BookmarkRequest::Open { number: 5, new_tab: false }), Output::Bookmarks(BookmarkRequest::Panel(false))]
        );
    }

    /// "Move…" opens a chooser of the folders, the item's own left out; a
    /// folder chosen asks for the move, to the folder's end; Escape or a
    /// press elsewhere closes it and asks for nothing.
    #[test]
    fn move_to_chooses_a_folder() {
        let mut browser = open();
        let sport = browser.bookmarks.controls.items[1];
        let moving = sport.moving;
        browser.pointer_moved(middle(sport.row).x, middle(sport.row).y, 0);
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        assert!(canvas.texts.iter().any(|(_, text)| text == "Move\u{2026}"), "shown on the row under the pointer");
        browser.bookmarks_press(middle(moving));
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let chosen: Vec<u32> = browser.bookmarks.controls.chooser.iter().map(|&(number, _)| number).collect();
        assert_eq!(chosen, [2, 3, 4]);
        let weather = browser.bookmarks.controls.chooser[2].1;
        assert!(browser.bookmarks_press(middle(weather)));
        assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Move { number: 6, folder: 4, index: u32::MAX })]);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        assert!(browser.bookmarks.controls.chooser.is_empty(), "closed once chosen");
        // Escape closes it; the manager stays open.
        browser.bookmarks_press(middle(moving));
        browser.key(Key::Escape, Modifiers::default(), 0);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        assert!(browser.bookmarks.controls.chooser.is_empty() && browser.bookmarks.open);
        // A press elsewhere closes it and does nothing else.
        browser.bookmarks_press(middle(moving));
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let bookmark = browser.bookmarks.controls.items[0].row;
        assert!(browser.bookmarks_press(middle(bookmark)));
        browser.bookmarks_released(false);
        assert_eq!(browser.take_outputs(), []);
        // A folder moved never lists itself.
        let news = browser.bookmarks.controls.items[0].moving;
        browser.set_bookmarks(
            BookmarksView {
                items: alloc::vec![ItemRow { number: 4, folder: true, name: "Weather".into(), address: String::new(), place: String::new() }],
                items_total: 1,
                ..view()
            },
            0,
        );
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.bookmarks_press(middle(news));
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        assert!(browser.bookmarks.controls.chooser.iter().all(|&(number, _)| number != 4));
    }

    /// The middle button and Command-click open a bookmark in a new tab,
    /// and leave the manager open; on a folder, a button or the chooser
    /// they open nothing.
    #[test]
    fn the_middle_button_and_command_click_open_in_a_new_tab() {
        let mut browser = open();
        let [news, sport] = [browser.bookmarks.controls.items[0], browser.bookmarks.controls.items[1]];
        for (pressed, expected) in [(news.row, true), (sport.row, false), (news.remove, false), (news.edit, false)] {
            browser.pointer_moved(middle(pressed).x, middle(pressed).y, 0);
            assert_eq!(browser.middle_pressed(), expected);
        }
        assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Open { number: 5, new_tab: true })]);
        browser.pointer_moved(middle(news.row).x, middle(news.row).y, 0);
        assert!(browser.new_tab_pressed());
        assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Open { number: 5, new_tab: true })]);
        assert!(browser.bookmarks.open);
    }

    /// Requirement 20.17: a new folder, an edit, a removal and its undo, a
    /// drag onto a folder.
    #[test]
    fn the_manager_asks_for_each_change() {
        let mut browser = open();
        let new_folder = browser.bookmarks.controls.new_folder;
        click(&mut browser, new_folder);
        browser.key(Key::Text("Weather"), Modifiers::default(), 0);
        browser.key(Key::Enter, Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(false), Output::Bookmarks(BookmarkRequest::NewFolder("Weather".into()))]);
        let edit = browser.bookmarks.controls.items[0].edit;
        browser.pointer_moved(middle(edit).x, middle(edit).y, 0);
        browser.bookmarks_press(middle(edit));
        browser.take_outputs();
        browser.key(Key::Text("a"), Modifiers { command: true, ..Modifiers::default() }, 0);
        browser.key(Key::Text("News"), Modifiers::default(), 0);
        browser.key(Key::Enter, Modifiers::default(), 0);
        assert_eq!(
            browser.take_outputs(),
            [Output::TextInput(false), Output::Bookmarks(BookmarkRequest::Edit { number: 5, name: "News".into(), address: "https://news.example/".into() })]
        );
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let remove = browser.bookmarks.controls.items[0].remove;
        browser.pointer_moved(middle(remove).x, middle(remove).y, 0);
        browser.bookmarks_press(middle(remove));
        assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Remove(5))]);
        browser.set_bookmarks(BookmarksView { undo: "Removed News".into(), ..view() }, 100);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 200);
        let undo = browser.bookmarks.controls.undo;
        assert!(browser.bookmarks_press(middle(undo)));
        assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Undo)]);
        assert!(!browser.bookmarks_undo_shown(100 + UNDO_MILLISECONDS), "ten seconds");
        // Dragged onto Weather in the tree.
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let (bookmark, weather) = (browser.bookmarks.controls.items[0].row, browser.bookmarks.controls.folders[2].1);
        browser.pointer_moved(middle(bookmark).x, middle(bookmark).y, 0);
        browser.bookmarks_press(middle(bookmark));
        browser.pointer_moved(middle(weather).x, middle(weather).y, 0);
        browser.bookmarks_released(false);
        assert_eq!(browser.take_outputs(), [Output::Bookmarks(BookmarkRequest::Move { number: 5, folder: 4, index: u32::MAX })]);
    }
}
