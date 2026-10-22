//! The strip's drop-downs (`history-and-bookmarks.md` sections 2.9, 2.10
//! and 6): the menu button's menu; Back's and Forward's lists, opened by
//! holding the button or a secondary click (requirement 20.10); and the
//! address field's suggestions with inline completion (requirement 20.11).
//! Each shows what the trusted core sent and asks by number.

use alloc::string::String;
use alloc::vec::Vec;

use super::{Browser, Output};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::library::{BarRequest, HistoryRequest, Suggestions, TabList};
use crate::theme::{ADDRESS, BLACK, DIM, MUTED, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SELECTED, STAR_ON, WHITE};
use crate::ui::{HAIRLINE, border, left_centred, right_centred};

const MENU_WIDTH: f64 = 300.0;
const LIST_WIDTH: f64 = 460.0;
const ROW: f64 = 30.0;
const GAP: f64 = 8.0;
const SHORTCUT_GAP: f64 = 12.0;
/// How long Back or Forward is held before its list opens.
const HOLD_MILLISECONDS: u64 = 500;

/// What the menu holds (section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    NewTab,
    NewPrivateTab,
    Reopen,
    History,
    Bookmarks,
    Bar,
    BookmarkPage,
    #[cfg(feature = "phone-keyboard")]
    Phone,
    Console,
    Settings,
}

impl Item {
    const ALL: &[Self] = &[
        Self::NewTab,
        Self::NewPrivateTab,
        Self::Reopen,
        Self::History,
        Self::Bookmarks,
        Self::Bar,
        Self::BookmarkPage,
        #[cfg(feature = "phone-keyboard")]
        Self::Phone,
        Self::Console,
        Self::Settings,
    ];

    /// Its words, and its shortcut on macOS and elsewhere, in words: the
    /// browser's fonts have no key symbols.
    fn words(self, bar_shown: bool) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::NewTab => ("New tab", "Cmd+T", "Ctrl+T"),
            Self::NewPrivateTab => ("New private tab", "Shift+Cmd+N", "Ctrl+Shift+N"),
            Self::Reopen => ("Reopen closed tab", "Shift+Cmd+T", "Ctrl+Shift+T"),
            Self::History => ("History", "Cmd+Y", "Ctrl+H"),
            Self::Bookmarks => ("Bookmarks", "Option+Cmd+B", "Ctrl+Shift+O"),
            Self::Bar if bar_shown => ("Hide bookmarks bar", "Shift+Cmd+B", "Ctrl+Shift+B"),
            Self::Bar => ("Show bookmarks bar", "Shift+Cmd+B", "Ctrl+Shift+B"),
            Self::BookmarkPage => ("Bookmark this page", "Cmd+D", "Ctrl+D"),
            #[cfg(feature = "phone-keyboard")]
            Self::Phone => ("Phone keyboard", "", ""),
            Self::Console => ("Developer console", "Option+Cmd+I", "Ctrl+Shift+I"),
            Self::Settings => ("Settings", "Cmd+,", "Ctrl+,"),
        }
    }
}

/// The drop-downs' state in the browser.
#[derive(Debug, Default)]
pub(super) struct PopupUi {
    menu_open: bool,
    menu: Vec<(Item, Rect)>,
    menu_area: Rect,
    tab_list: Option<TabList>,
    /// A list's entries drawn: steps, rect; and "Show all history".
    entries: Vec<(u8, Rect)>,
    show_all: Rect,
    list_area: Rect,
    /// Back or Forward held: which, since when, and whether its list opened.
    held: Option<(bool, u64, bool)>,
    suggestions: Option<Suggestions>,
    chosen: Option<usize>,
    rows: Vec<(u32, Rect)>,
    suggestions_area: Rect,
    /// The last edit deleted text: no inline completion until more is typed.
    deleted: bool,
}

impl Browser {
    // ---- The menu ----

    /// The menu button: its menu opens, or closes.
    pub(super) fn toggle_menu(&mut self) {
        self.popups.menu_open = !self.popups.menu_open;
    }

    /// A press while the menu is open: an item acts; anywhere else closes it.
    pub(super) fn menu_press(&mut self, point: Point) -> bool {
        if !self.popups.menu_open {
            return false;
        }
        self.popups.menu_open = false;
        let Some(&(item, _)) = self.popups.menu.iter().find(|(_, rect)| rect.contains(point)) else {
            // The menu button pressed again only closes it.
            return self.regions.menu.contains(point) || self.popups.menu_area.contains(point);
        };
        match item {
            Item::NewTab => self.outputs.push(Output::NewTab),
            Item::NewPrivateTab => self.outputs.push(Output::NewPrivateTab),
            Item::Reopen => self.outputs.push(Output::History(HistoryRequest::Reopen(None))),
            Item::History => self.set_history_open(true),
            Item::Bookmarks => {
                if !self.bookmarks_open() {
                    self.toggle_bookmarks();
                }
            }
            Item::Bar => self.outputs.push(Output::Bar(BarRequest::Show(!self.bar_shown()))),
            Item::BookmarkPage => self.outputs.push(Output::Bookmark),
            #[cfg(feature = "phone-keyboard")]
            Item::Phone => self.toggle_phone_panel(),
            Item::Console => self.toggle_console(),
            Item::Settings => {
                if !self.settings_open() {
                    self.toggle_settings();
                }
            }
        }
        true
    }

    fn draw_menu<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        self.popups.menu.clear();
        if !self.popups.menu_open {
            self.popups.menu_area = Rect::ZERO;
            return;
        }
        let anchor = self.regions.menu;
        #[allow(clippy::cast_precision_loss, reason = "ten items")]
        let area = Rect::new(anchor.x1 - MENU_WIDTH, anchor.y1 + HAIRLINE, anchor.x1, anchor.y1 + HAIRLINE + ROW * Item::ALL.len() as f64);
        canvas.fill(area, 0.0, BLACK);
        border(canvas, area, HAIRLINE, RULE);
        let bar_shown = self.bar_shown();
        let mut y = area.y0;
        for &item in Item::ALL {
            let rect = Rect::new(area.x0, y, area.x1, y + ROW);
            if rect.contains(self.pointer) {
                canvas.fill(rect, 0.0, SELECTED);
            }
            // macOS's window buttons are its own: the browser draws none there.
            let (words, mac, elsewhere) = item.words(bar_shown);
            let shortcut = if self.frame.window_buttons { elsewhere } else { mac };
            left_centred(canvas, rect, GAP * 2.0, words, ROW_TITLE, WHITE);
            right_centred(canvas, rect, SHORTCUT_GAP, shortcut, ROW_STATUS, MUTED);
            self.popups.menu.push((item, rect));
            y += ROW;
        }
        self.popups.menu_area = area;
    }

    // ---- Back's and Forward's lists ----

    /// Back or Forward pressed: it acts when released, unless held long
    /// enough to open its list.
    pub(super) fn hold_back_or_forward(&mut self, forward: bool, now: u64) {
        self.popups.held = Some((forward, now, false));
    }

    /// The main button released: a Back or Forward pressed and not held
    /// goes back or forward.
    pub(super) fn release_back_or_forward(&mut self) -> bool {
        let Some((forward, _, opened)) = self.popups.held.take() else { return false };
        let over = if forward { self.regions.forward } else { self.regions.back };
        if !opened && over.contains(self.pointer) {
            self.outputs.push(if forward { Output::Forward } else { Output::Back });
        }
        true
    }

    /// Asks for Back's or Forward's list: held long enough, or a secondary click.
    pub(super) fn ask_tab_list(&mut self, forward: bool) {
        self.outputs.push(Output::TabList(forward));
    }

    /// Whether Back or Forward has been held long enough at `now`; its list
    /// is asked for once.
    pub(super) fn settle_hold(&mut self, now: u64) -> bool {
        let Some((forward, since, false)) = self.popups.held else { return false };
        if now.saturating_sub(since) < HOLD_MILLISECONDS {
            return false;
        }
        self.popups.held = Some((forward, since, true));
        self.ask_tab_list(forward);
        true
    }

    /// When the held button's list is due, if one is held.
    pub(super) fn hold_deadline(&self, now: u64) -> Option<u64> {
        let (_, since, opened) = self.popups.held?;
        (!opened).then(|| since.saturating_add(HOLD_MILLISECONDS).saturating_sub(now).max(1))
    }

    /// The active tab's list one way, as the trusted core sent it.
    pub fn set_tab_list(&mut self, list: TabList) {
        self.popups.tab_list = Some(list);
    }

    /// A press while a list is open: an entry goes there; "Show all
    /// history" opens the panel; anywhere else closes it.
    pub(super) fn tab_list_press(&mut self, point: Point) -> bool {
        let Some(list) = self.popups.tab_list.take() else { return false };
        if let Some(&(steps, _)) = self.popups.entries.iter().find(|(_, rect)| rect.contains(point)) {
            self.outputs.push(Output::GoTo { forward: list.forward, steps });
            return true;
        }
        if self.popups.show_all.contains(point) {
            self.set_history_open(true);
            return true;
        }
        self.popups.list_area.contains(point)
    }

    fn draw_tab_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        self.popups.entries.clear();
        self.popups.show_all = Rect::ZERO;
        self.popups.list_area = Rect::ZERO;
        let Some(list) = self.popups.tab_list.clone() else { return };
        let anchor = if list.forward { self.regions.forward } else { self.regions.back };
        #[allow(clippy::cast_precision_loss, reason = "at most fifteen entries")]
        let height = ROW * (list.entries.len() + 1) as f64;
        let area = Rect::new(anchor.x0, anchor.y1 + HAIRLINE, anchor.x0 + LIST_WIDTH, anchor.y1 + HAIRLINE + height);
        canvas.fill(area, 0.0, BLACK);
        border(canvas, area, HAIRLINE, RULE);
        let mut y = area.y0;
        for entry in &list.entries {
            let rect = Rect::new(area.x0, y, area.x1, y + ROW);
            if rect.contains(self.pointer) {
                canvas.fill(rect, 0.0, SELECTED);
            }
            let title = match entry.page {
                Some(page) => alloc::format!("{page:03X} {}", entry.title),
                None => entry.title.clone(),
            };
            canvas.push_clip(rect);
            left_centred(canvas, rect, GAP * 2.0, &title, ROW_TITLE, WHITE);
            canvas.pop_clip();
            self.popups.entries.push((entry.steps, rect));
            y += ROW;
        }
        let show_all = Rect::new(area.x0, y, area.x1, y + ROW);
        canvas.fill(Rect::new(area.x0, y, area.x1, y + HAIRLINE), 0.0, RULE);
        if show_all.contains(self.pointer) {
            canvas.fill(show_all, 0.0, SELECTED);
        }
        left_centred(canvas, show_all, GAP * 2.0, "Show all history", ROW_TITLE, WHITE);
        self.popups.show_all = show_all;
        self.popups.list_area = area;
    }

    // ---- The address field's suggestions ----

    /// The address field's text changed by typing: suggestions are asked
    /// for; `deleted` when the change took text away.
    pub(super) fn edited(&mut self, deleted: bool) {
        self.popups.deleted = deleted;
        self.popups.chosen = None;
        self.outputs.push(Output::Suggest(String::from(self.editor.text())));
    }

    /// Suggestions for what is typed, as the trusted core sent them: shown
    /// if they answer the text as it is, and the field completed inline
    /// unless the last edit deleted (section 2.10).
    pub fn set_suggestions(&mut self, suggestions: Suggestions) {
        if !self.editing || suggestions.text != self.editor.text() {
            return;
        }
        let typed = suggestions.text.len();
        let (start, end) = self.editor.selection();
        let at_end = start == end && end == typed;
        if !self.popups.deleted && at_end && suggestions.inline.len() > typed && suggestions.inline.to_lowercase().starts_with(&suggestions.text.to_lowercase())
        {
            self.editor.set(&suggestions.inline);
            self.editor.place(typed, false);
            self.editor.place(suggestions.inline.len(), true);
        }
        self.popups.chosen = None;
        self.popups.suggestions = (!suggestions.rows.is_empty()).then_some(suggestions);
    }

    /// Up or Down in the address field: the next suggestion is chosen.
    pub(super) fn choose_suggestion(&mut self, down: bool) -> bool {
        let Some(count) = self.popups.suggestions.as_ref().map(|suggestions| suggestions.rows.len()).filter(|count| *count > 0) else { return false };
        self.popups.chosen = match (self.popups.chosen, down) {
            (None, true) => Some(0),
            (None, false) => Some(count - 1),
            (Some(at), true) => (at + 1 < count).then_some(at + 1),
            (Some(at), false) => at.checked_sub(1),
        };
        true
    }

    /// Enter in the address field: the suggestion chosen, if one is.
    pub(super) fn take_chosen_suggestion(&mut self) -> Option<u32> {
        let at = self.popups.chosen.take()?;
        let number = self.popups.suggestions.as_ref()?.rows.get(at)?.number;
        Some(number)
    }

    /// The field is no longer edited: its suggestions go.
    pub(super) fn close_suggestions(&mut self) {
        self.popups.suggestions = None;
        self.popups.chosen = None;
    }

    /// A press on a suggestion: it opens, as the user's own navigation.
    pub(super) fn suggestion_press(&mut self, point: Point, new_tab: bool) -> bool {
        if self.popups.suggestions.is_none() || !self.popups.suggestions_area.contains(point) {
            return false;
        }
        if let Some(&(number, _)) = self.popups.rows.iter().find(|(_, rect)| rect.contains(point)) {
            self.stop_editing();
            self.outputs.push(Output::Suggestion { number, new_tab });
        }
        true
    }

    fn draw_suggestions<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        self.popups.rows.clear();
        self.popups.suggestions_area = Rect::ZERO;
        let Some(suggestions) = self.popups.suggestions.clone().filter(|_| self.editing) else { return };
        let field = self.regions.field;
        #[allow(clippy::cast_precision_loss, reason = "at most eight rows")]
        let area = Rect::new(field.x0, field.y1 + HAIRLINE, field.x1, field.y1 + HAIRLINE + ROW * suggestions.rows.len() as f64);
        canvas.fill(area, 0.0, BLACK);
        border(canvas, area, HAIRLINE, RULE);
        let mut y = area.y0;
        for (at, row) in suggestions.rows.iter().enumerate() {
            let rect = Rect::new(area.x0, y, area.x1, y + ROW);
            if self.popups.chosen == Some(at) {
                canvas.fill(rect, 0.0, SELECTED);
            } else if rect.contains(self.pointer) {
                canvas.fill(rect, 0.0, RULE_SOFT);
            }
            if row.bookmarked {
                super::star(canvas, Point::new(rect.x0 + GAP + super::STAR_RADIUS, f64::midpoint(rect.y0, rect.y1)), STAR_ON);
            }
            // The address as the address rules print it, the title lighter
            // beside it, never in its place (requirement 20.13).
            let middle = f64::midpoint(rect.x0, rect.x1);
            canvas.push_clip(Rect::new(rect.x0, y, middle, y + ROW));
            left_centred(canvas, rect, GAP * 4.0, &row.address, ADDRESS, WHITE);
            canvas.pop_clip();
            canvas.push_clip(Rect::new(middle + GAP, y, rect.x1, y + ROW));
            left_centred(canvas, Rect::new(middle + GAP, y, rect.x1, y + ROW), 0.0, &row.title, ROW_STATUS, DIM);
            canvas.pop_clip();
            self.popups.rows.push((row.number, rect));
            y += ROW;
        }
        self.popups.suggestions_area = area;
    }

    /// Every drop-down, over the rest.
    pub(super) fn draw_popups<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        self.draw_suggestions(canvas);
        self.draw_tab_list(canvas);
        self.draw_menu(canvas);
    }

    /// A press on any drop-down; returns whether it took it.
    pub(super) fn popups_press(&mut self, point: Point, new_tab: bool) -> bool {
        self.menu_press(point) || self.tab_list_press(point) || self.suggestion_press(point, new_tab)
    }

    /// Whether the menu or a Back or Forward list is open.
    #[cfg(feature = "interaction")]
    pub(super) fn popup_open(&self) -> bool {
        self.popups.menu_open || self.popups.tab_list.is_some()
    }

    /// Whether a drop-down is open to close with Escape; closes it.
    pub(super) fn close_popup(&mut self) -> bool {
        if self.popups.menu_open {
            self.popups.menu_open = false;
            return true;
        }
        self.popups.tab_list.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, Key, Modifiers, StripState};
    use crate::library::{Suggestion, TabEntry};
    use crate::test_canvas::Recorder;

    fn middle(rect: Rect) -> Point {
        Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
    }

    fn browser() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.set_strip(StripState { address: "https://news.example/".into(), back: true, forward: true, ..StripState::default() });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser
    }

    fn press(browser: &mut Browser, rect: Rect, now: u64) {
        let point = middle(rect);
        browser.pointer_moved(point.x, point.y, now);
        browser.pointer_pressed(&mut Recorder::new(), false, now);
    }

    /// Requirement 20.10: a press and release goes back; held, the list is
    /// asked for and nothing else; an entry goes that many steps.
    #[test]
    fn back_held_opens_its_list() {
        let mut browser = browser();
        let back = browser.regions.back;
        press(&mut browser, back, 0);
        browser.pointer_released();
        assert_eq!(browser.take_outputs(), [Output::Back]);
        press(&mut browser, back, 1_000);
        assert_eq!(browser.hold_deadline(1_100), Some(400));
        assert!(browser.settle(1_000 + HOLD_MILLISECONDS));
        browser.pointer_released();
        assert_eq!(browser.take_outputs(), [Output::TabList(false)]);
        browser.set_tab_list(TabList {
            forward: false,
            entries: alloc::vec![
                TabEntry { steps: 1, page: None, title: "a".into(), address: "https://a.example/".into() },
                TabEntry { steps: 2, page: Some(0x302), title: "b".into(), address: "https://b.example/#302".into() }
            ],
        });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let second = browser.popups.entries[1].1;
        press(&mut browser, second, 2_000);
        assert_eq!(browser.take_outputs(), [Output::GoTo { forward: false, steps: 2 }]);
        assert!(!browser.secondary_pressed(), "nothing open, and not on Back");
        assert_eq!(browser.take_outputs(), []);
        browser.pointer_moved(middle(back).x, middle(back).y, 3_000);
        assert!(browser.secondary_pressed());
        assert_eq!(browser.take_outputs(), [Output::TabList(false)]);
    }

    /// Requirement 20.11: typing asks; the suggestions answer; the field is
    /// completed inline, not after a deletion; Down and Enter open one.
    #[test]
    fn typing_asks_for_suggestions_and_completes_inline() {
        let mut browser = browser();
        browser.key(Key::Text("l"), Modifiers { command: true, ..Modifiers::default() }, 0);
        browser.key(Key::Text("ne"), Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(true), Output::Suggest("ne".into())]);
        let row = Suggestion { number: 9, address: "https://news.example/#302".into(), title: "WEATHER".into(), bookmarked: true };
        browser.set_suggestions(Suggestions { text: "ne".into(), inline: "news.example/".into(), rows: alloc::vec![row.clone()] });
        assert_eq!((browser.editor.text(), browser.editor.selection()), ("news.example/", (2, 13)), "the rest selected");
        browser.key(Key::Backspace, Modifiers::default(), 0);
        browser.take_outputs();
        browser.set_suggestions(Suggestions { text: "ne".into(), inline: "news.example/".into(), rows: alloc::vec![row] });
        assert_eq!(browser.editor.text(), "ne", "no completion after a deletion");
        browser.key(Key::Down, Modifiers::default(), 0);
        browser.key(Key::Enter, Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(false), Output::Suggestion { number: 9, new_tab: false }]);
    }

    /// Section 6: the menu holds history, bookmarks and private tabs.
    #[test]
    fn the_menu_holds_history_and_bookmarks() {
        let mut browser = browser();
        let menu = browser.regions.menu;
        press(&mut browser, menu, 0);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let private = browser.popups.menu.iter().find(|(item, _)| *item == Item::NewPrivateTab).map(|(_, rect)| *rect).unwrap();
        press(&mut browser, private, 0);
        assert_eq!(browser.take_outputs(), [Output::NewPrivateTab]);
        press(&mut browser, menu, 0);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let history = browser.popups.menu.iter().find(|(item, _)| *item == Item::History).map(|(_, rect)| *rect).unwrap();
        press(&mut browser, history, 0);
        assert_eq!(browser.take_outputs(), [Output::History(HistoryRequest::Panel(true))]);
        assert!(browser.history_open());
    }
}
