//! The bookmarks manager of the trusted strip (`history-and-bookmarks.md`
//! section 3.3, requirement 20.17): opened by Command-Option-B or
//! Control-Shift-O and from the menu. It shows the folder tree and a
//! folder's contents as the trusted core sent them, and asks for every
//! change by the items' numbers, as strip input.

use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::{Browser, Output};
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::library::{BookmarkRequest, BookmarksView, MAX_NAME, MAX_PATH, MAX_QUERY};

mod draw;
mod input;

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
    pub(in crate::browser) fn toggle_bookmarks(&mut self) {
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
    pub(in crate::browser) fn bookmarks_editing(&self) -> bool {
        self.bookmarks.open && self.bookmarks.focus.is_some()
    }

    /// The manager's field with the keys.
    pub(in crate::browser) fn bookmarks_field(&mut self) -> Option<&mut Editor> {
        let field = self.bookmarks.focus.filter(|_| self.bookmarks.open)?;
        Some(self.bookmarks.editor(field))
    }

    /// Whether Undo is still offered at `now`.
    pub(in crate::browser) fn bookmarks_undo_shown(&self, now: u64) -> bool {
        self.bookmarks.undo_since.as_ref().is_some_and(|(since, _)| now.saturating_sub(*since) < UNDO_MILLISECONDS)
    }

    /// When the manager next changes by itself: Undo's end.
    pub(in crate::browser) fn bookmarks_deadline(&self, now: u64) -> Option<u64> {
        let (since, _) = self.bookmarks.undo_since.as_ref().filter(|_| self.bookmarks.open && self.bookmarks_undo_shown(now))?;
        Some(since.saturating_add(UNDO_MILLISECONDS).saturating_sub(now))
    }
}

#[cfg(test)]
mod tests;
