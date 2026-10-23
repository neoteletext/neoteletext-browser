//! The history panel of the trusted strip (`history-and-bookmarks.md`
//! section 2.4, requirement 20.4): opened by Command-Y or Control-H and from
//! the menu; it shows the rows the trusted core sent and asks for each
//! search, opening and removal by the rows' numbers, as strip input
//! (requirement 20.5). It keeps no history of its own.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::{Browser, Output};
use crate::editor::Editor;
use crate::geometry::Rect;
use crate::library::{ClearRange, HistoryRequest, HistoryView, MAX_QUERY};

mod draw;
mod input;

const PANEL_WIDTH: f64 = 820.0;
const PANEL_HEIGHT: f64 = 680.0;
const PADDING: f64 = 20.0;
const HEADING_HEIGHT: f64 = 34.0;
const CONTROL_HEIGHT: f64 = 32.0;
const GAP: f64 = 8.0;
const SMALL_BUTTON: f64 = 32.0;
const WIDE_BUTTON: f64 = 150.0;
/// A row's menu: as wide as its longest item.
const MENU_WIDTH: f64 = 200.0;
const RANGE_BUTTON: f64 = 112.0;
const ROW_HEIGHT: f64 = 30.0;
const DAY_HEIGHT: f64 = 30.0;
const CHECK_SIZE: f64 = 16.0;
const TIME_WIDTH: f64 = 56.0;
const CLOSED_WIDTH: f64 = 150.0;
/// The most recently closed tabs the panel lists across its top.
const CLOSED_SHOWN: usize = 4;
/// How many rows one turn of the wheel moves the view.
const WHEEL_ROWS: usize = 3;
/// The ranges "Clear history" offers, as the design names them (section 2.5).
const RANGES: [(ClearRange, &str); 6] = [
    (ClearRange::FifteenMinutes, "Last 15 minutes"),
    (ClearRange::Hour, "Last hour"),
    (ClearRange::Day, "Last 24 hours"),
    (ClearRange::Week, "Last 7 days"),
    (ClearRange::FourWeeks, "Last 4 weeks"),
    (ClearRange::All, "All time"),
];

/// Where the panel's controls were last drawn.
#[derive(Debug, Default)]
struct Controls {
    panel: Rect,
    close: Rect,
    search: Rect,
    clear: Rect,
    ranges: Vec<(ClearRange, Rect)>,
    /// What clearing clears: history, cookies (requirement 21.8).
    with_history: Rect,
    with_cookies: Rect,
    cancel: Rect,
    remove_selected: Rect,
    closed: Vec<(u32, Rect)>,
    /// Each row: its number, its check box, its menu button, the row.
    rows: Vec<(u32, Rect, Rect, Rect)>,
    list: Rect,
    /// A row's open menu: its items and where they were drawn.
    menu: Vec<(RowAction, Rect)>,
    menu_area: Rect,
}

/// What a row's menu offers, as Chrome's history page does: "More from
/// this site", "Remove from history", and "Remove bookmark" when the
/// address is bookmarked (its `history_list.html.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAction {
    MoreFromSite,
    Remove,
    RemoveBookmark,
}

impl RowAction {
    /// The menu of a row, bookmarked or not.
    const fn of(bookmarked: bool) -> &'static [Self] {
        if bookmarked { &[Self::MoreFromSite, Self::Remove, Self::RemoveBookmark] } else { &[Self::MoreFromSite, Self::Remove] }
    }

    const fn words(self) -> &'static str {
        match self {
            Self::MoreFromSite => "More from this site",
            Self::Remove => "Remove from history",
            Self::RemoveBookmark => "Remove bookmark",
        }
    }
}

/// The panel's state in the browser.
pub(super) struct HistoryUi {
    view: Option<HistoryView>,
    open: bool,
    query: Editor,
    focused: bool,
    selected: BTreeSet<u32>,
    clearing: bool,
    clears: Clears,
    controls: Controls,
    /// The row whose menu is open.
    menu: Option<u32>,
}

/// What clearing clears: history, as it did; cookies, when chosen
/// (requirement 21.8).
#[derive(Clone, Copy, Debug)]
struct Clears {
    history: bool,
    cookies: bool,
}

impl Default for HistoryUi {
    fn default() -> Self {
        Self {
            view: None,
            open: false,
            query: Editor::new(MAX_QUERY),
            focused: false,
            selected: BTreeSet::new(),
            clearing: false,
            clears: Clears { history: true, cookies: false },
            controls: Controls::default(),
            menu: None,
        }
    }
}

impl Browser {
    /// The history panel's rows as the trusted core sent them.
    pub fn set_history(&mut self, view: HistoryView) {
        // Numbers of an earlier view name nothing now.
        let numbers: BTreeSet<u32> = view.rows.iter().map(|row| row.number).collect();
        let addresses: BTreeSet<&str> = self
            .history
            .view
            .iter()
            .flat_map(|old| old.rows.iter().filter(|row| self.history.selected.contains(&row.number)).map(|row| row.address.as_str()))
            .collect();
        self.history.selected = view.rows.iter().filter(|row| addresses.contains(row.address.as_str())).map(|row| row.number).collect();
        self.history.selected.retain(|number| numbers.contains(number));
        // A search the trusted core made, "More from this site", shows in
        // the field.
        if !self.history.focused && self.history.query.text() != view.query {
            self.history.query.set(&view.query);
        }
        self.history.view = Some(view);
    }

    /// Whether the history panel is open.
    #[must_use]
    pub fn history_open(&self) -> bool {
        self.history.open
    }

    /// Opens the history panel, or closes it.
    pub(in crate::browser) fn toggle_history(&mut self) {
        self.set_history_open(!self.history.open);
    }

    pub(in crate::browser) fn set_history_open(&mut self, open: bool) {
        if self.history.open == open {
            return;
        }
        if open {
            self.close_panels();
        }
        self.history.open = open;
        self.history.clearing = false;
        self.history.selected.clear();
        self.unfocus_history();
        if open {
            self.history.query.set("");
        }
        self.outputs.push(Output::History(HistoryRequest::Panel(open)));
    }

    /// Opens the panel at its clearing choices (Command-Shift-Delete).
    pub(in crate::browser) fn clear_history_choices(&mut self) {
        self.set_history_open(true);
        self.history.clearing = true;
    }

    fn unfocus_history(&mut self) {
        if self.history.focused {
            self.history.focused = false;
            self.outputs.push(Output::TextInput(false));
        }
    }

    fn history_query(&mut self, first: usize) {
        let text = String::from(self.history.query.text().trim());
        self.outputs.push(Output::History(HistoryRequest::Query { text, first: u32::try_from(first).unwrap_or(u32::MAX) }));
    }

    /// Whether the search field takes the keys.
    pub(in crate::browser) fn history_editing(&self) -> bool {
        self.history.open && self.history.focused
    }

    /// The search field, while it has the keys.
    pub(in crate::browser) fn history_field(&mut self) -> Option<&mut Editor> {
        self.history_editing().then_some(&mut self.history.query)
    }
}

#[cfg(test)]
mod tests;
