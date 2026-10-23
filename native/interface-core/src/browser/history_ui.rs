//! The history panel of the trusted strip (`history-and-bookmarks.md`
//! section 2.4, requirement 20.4): opened by Command-Y or Control-H and from
//! the menu; it shows the rows the trusted core sent and asks for each
//! search, opening and removal by the rows' numbers, as strip input
//! (requirement 20.5). It keeps no history of its own.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::widgets::{button, field, panel, panel_rect};
use super::{Browser, Key, Output};
use crate::canvas::Canvas;
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::library::{ClearRange, HistoryRequest, HistoryView, MAX_QUERY, MAX_VIEW_ROWS};
use crate::theme::{ADDRESS, BUTTON, CAPTION, DIM, HEADING, MUTED, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SELECTED, WHITE};
use crate::ui::{border, centred, left_centred, right_centred};

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
    /// The panel's symbol buttons.
    pub(super) fn history_tips(&self, tips: &mut Vec<(Rect, String)>) {
        let controls = &self.history.controls;
        tips.push((controls.close, format!("Close history ({})", self.keys_named("Cmd+Y", "Ctrl+H"))));
        for &(_, _, menu, _) in &controls.rows {
            tips.push((menu, "More from this site, or remove from history".into()));
        }
    }

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
    pub(super) fn toggle_history(&mut self) {
        self.set_history_open(!self.history.open);
    }

    pub(super) fn set_history_open(&mut self, open: bool) {
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
    pub(super) fn clear_history_choices(&mut self) {
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
    pub(super) fn history_editing(&self) -> bool {
        self.history.open && self.history.focused
    }

    /// A key while the panel is open; `None` when it is not the panel's.
    pub(super) fn history_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
        if !self.history.open {
            return None;
        }
        if !self.history.focused {
            if key == Key::Escape {
                if self.history.clearing {
                    self.history.clearing = false;
                } else {
                    self.set_history_open(false);
                }
                return Some(true);
            }
            return None;
        }
        let editor = &mut self.history.query;
        let before = String::from(editor.text());
        match key {
            Key::Text(text) => editor.insert(text),
            Key::Backspace => editor.backspace(),
            Key::Delete => editor.delete(),
            Key::Left => editor.left(shift),
            Key::Right => editor.right(shift),
            Key::Home | Key::Up => editor.home(shift),
            Key::End | Key::Down => editor.end(shift),
            Key::Escape | Key::Tab | Key::Enter => self.unfocus_history(),
        }
        // The search follows each change as it is typed.
        if self.history.query.text() != before {
            self.history_query(0);
        }
        Some(true)
    }

    /// Command-A in the search field.
    pub(super) fn history_select_all(&mut self) -> bool {
        if !self.history_editing() {
            return false;
        }
        self.history.query.select_all();
        true
    }

    /// The wheel over the panel: further rows are asked for.
    pub(super) fn history_scroll(&mut self, distance: f64) -> bool {
        if !self.history.open || !self.history.controls.panel.contains(self.pointer) {
            return false;
        }
        let Some(view) = &self.history.view else { return true };
        let first = usize::try_from(view.first).unwrap_or(0);
        let total = usize::try_from(view.total).unwrap_or(0);
        let shown = self.history.controls.rows.len().max(1);
        let next = if distance > 0.0 { first.saturating_add(WHEEL_ROWS).min(total.saturating_sub(shown)) } else { first.saturating_sub(WHEEL_ROWS) };
        if next != first {
            self.history_query(next);
        }
        true
    }

    /// The row whose title is at `point`, not its check box or menu.
    pub(super) fn history_row_at(&self, point: Point) -> Option<u32> {
        if !self.history.open || self.history.menu.is_some() {
            return None;
        }
        let (number, check, menu, _) = self.history.controls.rows.iter().find(|(_, _, _, row)| row.contains(point))?;
        (!check.contains(point) && !menu.contains(point)).then_some(*number)
    }

    /// A press on the panel; returns whether it was the panel's. With
    /// `new_tab`, a row opens in a new tab.
    pub(super) fn history_press(&mut self, point: Point, new_tab: bool) -> bool {
        if !self.history.open {
            return false;
        }
        let controls = &self.history.controls;
        if !controls.panel.contains(point) {
            return false;
        }
        // A row's open menu takes the press; anywhere else closes it.
        if let Some(number) = self.history.menu.take() {
            match controls.menu.iter().find(|(_, rect)| rect.contains(point)).map(|(action, _)| *action) {
                Some(RowAction::MoreFromSite) => self.outputs.push(Output::History(HistoryRequest::MoreFromSite(number))),
                Some(RowAction::Remove) => self.outputs.push(Output::History(HistoryRequest::Remove(alloc::vec![number]))),
                Some(RowAction::RemoveBookmark) => self.outputs.push(Output::History(HistoryRequest::RemoveBookmark(number))),
                None => {}
            }
            return true;
        }
        if controls.close.contains(point) {
            self.set_history_open(false);
            return true;
        }
        if controls.search.contains(point) {
            if !self.history.focused {
                self.history.focused = true;
                self.outputs.push(Output::TextInput(true));
            }
            return true;
        }
        self.unfocus_history();
        let controls = &self.history.controls;
        if controls.clear.contains(point) {
            self.history.clearing = !self.history.clearing;
        } else if controls.cancel.contains(point) {
            self.history.clearing = false;
        } else if controls.with_history.contains(point) {
            self.history.clears.history = !self.history.clears.history;
        } else if controls.with_cookies.contains(point) {
            self.history.clears.cookies = !self.history.clears.cookies;
        } else if let Some(&(range, _)) = controls.ranges.iter().find(|(_, rect)| rect.contains(point)) {
            self.history.clearing = false;
            self.history.selected.clear();
            if self.history.clears.history {
                self.outputs.push(Output::History(HistoryRequest::Clear(range)));
            }
            if self.history.clears.cookies {
                self.outputs.push(Output::History(HistoryRequest::ClearCookies(range)));
            }
        } else if controls.remove_selected.contains(point) {
            let numbers: Vec<u32> = self.history.selected.iter().copied().take(MAX_VIEW_ROWS).collect();
            self.history.selected.clear();
            self.outputs.push(Output::History(HistoryRequest::Remove(numbers)));
        } else if let Some(&(number, _)) = controls.closed.iter().find(|(_, rect)| rect.contains(point)) {
            self.outputs.push(Output::History(HistoryRequest::Reopen(Some(number))));
        } else if let Some(&(number, check, menu, _)) = controls.rows.iter().find(|(_, _, _, row)| row.contains(point)) {
            if menu.contains(point) {
                self.history.menu = Some(number);
            } else if check.contains(point) {
                if !self.history.selected.remove(&number) {
                    self.history.selected.insert(number);
                }
            } else {
                self.outputs.push(Output::History(HistoryRequest::Open { number, new_tab }));
                if !new_tab {
                    self.set_history_open(false);
                }
            }
        }
        true
    }

    /// The panel, over the window below the strip, while it is open.
    #[allow(clippy::too_many_lines, reason = "one part of the panel after another, in drawing order")]
    pub(super) fn draw_history_panel<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let mut controls = Controls::default();
        if !self.history.open {
            self.history.controls = controls;
            return;
        }
        let area = panel_rect(self.body, PANEL_WIDTH, PANEL_HEIGHT);
        panel(canvas, area);
        controls.panel = area;
        let (left, right) = (area.x0 + PADDING, area.x1 - PADDING);
        let heading = Rect::new(left, area.y0 + PADDING, right, area.y0 + PADDING + HEADING_HEIGHT);
        left_centred(canvas, heading, 0.0, "History", HEADING, WHITE);
        controls.close = Rect::new(right - SMALL_BUTTON, heading.y0, right, heading.y0 + SMALL_BUTTON);
        button(canvas, controls.close, "\u{00D7}", false);
        let view = self.history.view.clone().unwrap_or_default();
        let mut y = heading.y1 + GAP;

        // The search, and "Clear history…".
        controls.clear = Rect::new(right - WIDE_BUTTON, y, right, y + CONTROL_HEIGHT);
        button(canvas, controls.clear, "Clear history\u{2026}", self.history.clearing);
        controls.search = Rect::new(left, y, controls.clear.x0 - GAP, y + CONTROL_HEIGHT);
        field(canvas, controls.search, &self.history.query, self.history.focused);
        if self.history.query.text().is_empty() && !self.history.focused {
            left_centred(canvas, controls.search, GAP, "Search history", ROW_STATUS, DIM);
        }
        y += CONTROL_HEIGHT + GAP;

        // The clearing choices, or what to do with the rows selected.
        if self.history.clearing {
            left_centred(canvas, Rect::new(left, y, left + RANGE_BUTTON, y + CONTROL_HEIGHT), 0.0, "Clear", CAPTION, MUTED);
            controls.with_history = Rect::new(left + RANGE_BUTTON, y, left + RANGE_BUTTON * 2.0, y + CONTROL_HEIGHT);
            button(canvas, controls.with_history, "History", self.history.clears.history);
            controls.with_cookies = Rect::new(controls.with_history.x1 + GAP, y, controls.with_history.x1 + GAP + RANGE_BUTTON, y + CONTROL_HEIGHT);
            button(canvas, controls.with_cookies, "Cookies", self.history.clears.cookies);
            y += CONTROL_HEIGHT + GAP;
            let mut x = left;
            for (range, word) in RANGES {
                let rect = Rect::new(x, y, x + RANGE_BUTTON, y + CONTROL_HEIGHT);
                button(canvas, rect, word, false);
                controls.ranges.push((range, rect));
                x = rect.x1 + GAP;
            }
            controls.cancel = Rect::new(x, y, (x + RANGE_BUTTON).min(right), y + CONTROL_HEIGHT);
            button(canvas, controls.cancel, "Cancel", false);
            y += CONTROL_HEIGHT + GAP;
        } else if !self.history.selected.is_empty() {
            controls.remove_selected = Rect::new(left, y, left + WIDE_BUTTON * 1.2, y + CONTROL_HEIGHT);
            button(canvas, controls.remove_selected, &format!("Remove selected ({})", self.history.selected.len()), false);
            y += CONTROL_HEIGHT + GAP;
        }

        // Recently closed tabs.
        if !view.closed.is_empty() {
            let line = Rect::new(left, y, right, y + CONTROL_HEIGHT);
            left_centred(canvas, line, 0.0, "Recently closed", CAPTION, MUTED);
            let mut x = left + WIDE_BUTTON;
            for closed in view.closed.iter().take(CLOSED_SHOWN) {
                let rect = Rect::new(x, y, (x + CLOSED_WIDTH).min(right), y + CONTROL_HEIGHT);
                canvas.push_clip(rect);
                button(canvas, rect, &closed.title, false);
                canvas.pop_clip();
                controls.closed.push((closed.number, rect));
                x = rect.x1 + GAP;
                if x >= right {
                    break;
                }
            }
            y += CONTROL_HEIGHT + GAP;
        }

        // The rows, under their days' headings.
        let footer = area.y1 - PADDING - CONTROL_HEIGHT;
        controls.list = Rect::new(left, y, right, footer - GAP);
        canvas.push_clip(controls.list);
        if view.rows.is_empty() {
            let empty = if view.query.is_empty() { "No history" } else { "Nothing in history matches" };
            centred(canvas, Rect::new(left, y, right, y + ROW_HEIGHT * 2.0), empty, ROW_TITLE, MUTED);
        }
        for row in &view.rows {
            if y + ROW_HEIGHT > controls.list.y1 {
                break;
            }
            if let Some(heading) = &row.heading {
                left_centred(canvas, Rect::new(left, y, right, y + DAY_HEIGHT), 0.0, heading, CAPTION, WHITE);
                y += DAY_HEIGHT;
                if y + ROW_HEIGHT > controls.list.y1 {
                    break;
                }
            }
            let rect = Rect::new(left, y, right, y + ROW_HEIGHT);
            let hovered = rect.contains(self.pointer);
            if hovered {
                canvas.fill(rect, 0.0, RULE_SOFT);
            }
            let check_top = y + (ROW_HEIGHT - CHECK_SIZE) / 2.0;
            let check = Rect::new(left, check_top, left + CHECK_SIZE, check_top + CHECK_SIZE);
            let checked = self.history.selected.contains(&row.number);
            if checked {
                canvas.fill(check, 0.0, SELECTED);
            }
            border(canvas, check, 1.0, if checked { SELECTED } else { MUTED });
            let time = Rect::new(check.x1 + GAP, y, check.x1 + GAP + TIME_WIDTH, y + ROW_HEIGHT);
            left_centred(canvas, time, 0.0, &row.time, ROW_STATUS, MUTED);
            // The row's menu button, as Chrome's: shown while hovered or open.
            let menu = Rect::new(right - SMALL_BUTTON, y, right, y + ROW_HEIGHT);
            if hovered || self.history.menu == Some(row.number) {
                centred(canvas, menu, "\u{22EE}", ROW_TITLE, WHITE);
            }
            // The title and the address it leads to, always both (requirement 20.13).
            let text = Rect::new(time.x1, y, menu.x0 - GAP, y + ROW_HEIGHT);
            let middle = f64::midpoint(text.x0, text.x1);
            let title = match row.page {
                Some(page) => format!("{} {page:03X} {}", row.channel, row.title),
                None => row.title.clone(),
            };
            canvas.push_clip(Rect::new(text.x0, y, middle - GAP, y + ROW_HEIGHT));
            left_centred(canvas, text, 0.0, &title, ROW_TITLE, WHITE);
            canvas.pop_clip();
            canvas.push_clip(Rect::new(middle, y, text.x1, y + ROW_HEIGHT));
            left_centred(canvas, Rect::new(middle, y, text.x1, y + ROW_HEIGHT), 0.0, &row.address, ADDRESS, DIM);
            canvas.pop_clip();
            controls.rows.push((row.number, check, menu, rect));
            y += ROW_HEIGHT;
        }
        canvas.pop_clip();

        // The open row's menu, below its button and inside the panel.
        if let Some(&(number, _, button_rect, _)) = controls.rows.iter().find(|(number, ..)| Some(*number) == self.history.menu) {
            let bookmarked = view.rows.iter().any(|row| row.number == number && row.bookmarked);
            let actions = RowAction::of(bookmarked);
            #[allow(clippy::cast_precision_loss, reason = "three items at most")]
            let height = ROW_HEIGHT * actions.len() as f64;
            let top = if button_rect.y1 + height > area.y1 { button_rect.y0 - height } else { button_rect.y1 };
            let menu_area = Rect::new(button_rect.x1 - MENU_WIDTH, top, button_rect.x1, top + height);
            canvas.fill(menu_area, 0.0, BUTTON);
            border(canvas, menu_area, 1.0, RULE);
            for (index, &action) in actions.iter().enumerate() {
                #[allow(clippy::cast_precision_loss, reason = "three items at most")]
                let item_top = menu_area.y0 + ROW_HEIGHT * index as f64;
                let item = Rect::new(menu_area.x0, item_top, menu_area.x1, item_top + ROW_HEIGHT);
                if item.contains(self.pointer) {
                    canvas.fill(item, 0.0, SELECTED);
                }
                left_centred(canvas, item, GAP, action.words(), ROW_TITLE, WHITE);
                controls.menu.push((action, item));
            }
            controls.menu_area = menu_area;
        } else {
            self.history.menu = None;
        }

        // Where the rows are among all, and the trusted core's word.
        let footer = Rect::new(left, footer, right, footer + CONTROL_HEIGHT);
        if view.total > 0 {
            let last = (usize::try_from(view.first).unwrap_or(0) + controls.rows.len()).min(usize::try_from(view.total).unwrap_or(0));
            right_centred(canvas, footer, 0.0, &format!("{} to {last} of {}", view.first + 1, view.total), ROW_STATUS, MUTED);
        }
        if !view.notice.is_empty() {
            canvas.push_clip(Rect::new(left, footer.y0, f64::midpoint(left, right) + WIDE_BUTTON, footer.y1));
            left_centred(canvas, footer, 0.0, &view.notice, ROW_STATUS, MUTED);
            canvas.pop_clip();
        }
        self.history.controls = controls;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, Modifiers};
    use crate::library::{ClosedTab, HistoryRow};
    use crate::test_canvas::Recorder;

    fn row(number: u32, heading: Option<&str>, address: &str) -> HistoryRow {
        HistoryRow {
            number,
            heading: heading.map(Into::into),
            time: "14:05".into(),
            channel: "News Example".into(),
            page: Some(0x302),
            title: "WEATHER".into(),
            address: address.into(),
            // The first row's address is bookmarked.
            bookmarked: number == 7,
        }
    }

    fn view() -> HistoryView {
        HistoryView {
            query: String::new(),
            first: 0,
            total: 2,
            rows: alloc::vec![row(7, Some("Today: Thursday 22 October 2026"), "https://news.example/#302"), row(8, None, "https://sport.example/")],
            closed: alloc::vec![ClosedTab { number: 3, title: "weather.example".into() }],
            notice: String::new(),
        }
    }

    fn open() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.key(Key::Text("y"), Modifiers { command: true, ..Modifiers::default() }, 0);
        browser.set_history(view());
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.take_outputs();
        browser
    }

    fn middle(rect: Rect) -> Point {
        Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
    }

    /// Requirement 20.4: the panel opens by its key, shows the day's heading,
    /// each row's title and address, and opens a row by its number.
    #[test]
    fn the_panel_shows_rows_and_opens_them_by_number() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.key(Key::Text("y"), Modifiers { command: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Panel(true))]);
        browser.set_history(view());
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
        for expected in ["History", "Today: Thursday 22 October 2026", "14:05", "News Example 302 WEATHER", "https://news.example/#302", "weather.example"] {
            assert!(texts.contains(&expected), "{expected} in {texts:?}");
        }
        let row = browser.history.controls.rows[1].3;
        assert!(browser.history_press(middle(row), false));
        assert_eq!(
            browser.take_outputs(),
            [Output::History(HistoryRequest::Open { number: 8, new_tab: false }), Output::History(HistoryRequest::Panel(false))]
        );
    }

    /// The middle button and Command-click open a row in a new tab and leave
    /// the panel open; on its check box, its menu or a button, nothing.
    #[test]
    fn the_middle_button_opens_a_row_in_a_new_tab() {
        let mut browser = open();
        let (_, check, menu, row) = browser.history.controls.rows[1];
        let clear = browser.history.controls.clear;
        for (pressed, expected) in [(row, true), (check, false), (menu, false), (clear, false)] {
            let at = if pressed == row { Point::new(row.x0 + 200.0, middle(row).y) } else { middle(pressed) };
            browser.pointer_moved(at.x, at.y, 0);
            assert_eq!(browser.middle_pressed(), expected);
        }
        assert!(!browser.new_tab_pressed(), "the pointer is on Clear history");
        assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Open { number: 8, new_tab: true })]);
        assert!(browser.history.open);
    }

    /// Requirement 20.6: remove a row, selected rows, a channel, a range;
    /// reopen a closed tab.
    #[test]
    fn rows_are_removed_by_number() {
        let mut browser = open();
        let (number, _, menu, _) = browser.history.controls.rows[0];
        // A row not bookmarked has no third item.
        let other = browser.history.controls.rows[1];
        browser.history_press(middle(other.2), false);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        assert_eq!(browser.history.controls.menu.len(), 2);
        // A press inside the panel but on no item closes the menu.
        browser.history_press(middle(other.2), false);
        assert!(browser.take_outputs().is_empty() && browser.history.menu.is_none());
        browser.pointer_moved(middle(menu).x, middle(menu).y, 0);
        // The row's menu, as Chrome's: more from this site, remove, and,
        // its address being bookmarked, remove the bookmark.
        assert_eq!(number, 7, "the bookmarked row");
        for (index, expected) in
            [(0, HistoryRequest::MoreFromSite(number)), (1, HistoryRequest::Remove(alloc::vec![number])), (2, HistoryRequest::RemoveBookmark(number))]
        {
            browser.history_press(middle(menu), false);
            browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
            let item = browser.history.controls.menu[index].1;
            browser.history_press(middle(item), false);
            assert_eq!(browser.take_outputs(), [Output::History(expected)]);
        }
        // The rows move as the toolbar comes and goes: measured afresh.
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let check = browser.history.controls.rows[0].1;
        browser.history_press(middle(check), false);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let remove_selected = browser.history.controls.remove_selected;
        browser.history_press(middle(remove_selected), false);
        assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Remove(alloc::vec![number]))]);
        let clear = browser.history.controls.clear;
        browser.history_press(middle(clear), false);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let all = browser.history.controls.ranges.iter().find(|(range, _)| *range == ClearRange::All).unwrap().1;
        browser.history_press(middle(all), false);
        assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Clear(ClearRange::All))]);
        // Requirement 21.8: cookies too, or cookies alone.
        browser.history_press(middle(clear), false);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let (with_history, with_cookies) = (browser.history.controls.with_history, browser.history.controls.with_cookies);
        browser.history_press(middle(with_cookies), false);
        let hour = browser.history.controls.ranges.iter().find(|(range, _)| *range == ClearRange::Hour).unwrap().1;
        browser.history_press(middle(hour), false);
        assert_eq!(
            browser.take_outputs(),
            [Output::History(HistoryRequest::Clear(ClearRange::Hour)), Output::History(HistoryRequest::ClearCookies(ClearRange::Hour))]
        );
        browser.history_press(middle(clear), false);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.history_press(middle(with_history), false);
        let day = browser.history.controls.ranges.iter().find(|(range, _)| *range == ClearRange::Day).unwrap().1;
        browser.history_press(middle(day), false);
        assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::ClearCookies(ClearRange::Day))]);
        let closed = browser.history.controls.closed[0].1;
        browser.history_press(middle(closed), false);
        assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Reopen(Some(3)))]);
    }

    /// The search follows what is typed; the wheel asks for further rows.
    #[test]
    fn the_search_and_the_wheel_ask_for_rows() {
        let mut browser = open();
        let search = browser.history.controls.search;
        browser.history_press(middle(search), false);
        browser.key(Key::Text("n"), Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(true), Output::History(HistoryRequest::Query { text: "n".into(), first: 0 })]);
        let mut many = view();
        many.total = 500;
        browser.set_history(many);
        let list = browser.history.controls.list;
        browser.pointer_moved(middle(list).x, middle(list).y, 0);
        assert!(browser.history_scroll(1.0));
        assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Query { text: "n".into(), first: 3 })]);
    }
}
