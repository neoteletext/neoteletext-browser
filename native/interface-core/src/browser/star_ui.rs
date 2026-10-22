//! The star's popover and the bookmarks bar (`history-and-bookmarks.md`
//! sections 3.2 and 3.4, requirements 20.16 and 20.18). The popover opens
//! under the star once the trusted core has bookmarked the address the
//! strip shows; the bar is a row of the trusted strip, below the address,
//! out of every page's reach. Both show what the trusted core sent and ask
//! for every change by number.

use alloc::string::String;
use alloc::vec::Vec;

use super::widgets::{TRIANGLE, button, field, panel, triangle};
use super::{Browser, Key, Output};
use crate::canvas::Canvas;
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::library::{BarRequest, BarView, ItemRow, MAX_NAME, StarRequest, StarView};
use crate::theme::{BLACK, BUTTON, DIM, HEADING, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SELECTED, WHITE};
use crate::ui::{HAIRLINE, border, left_centred};

const POPOVER_WIDTH: f64 = 380.0;
const PADDING: f64 = 16.0;
const LINE: f64 = 32.0;
const GAP: f64 = 8.0;
const LABEL_WIDTH: f64 = 72.0;
const BUTTON_WIDTH: f64 = 90.0;
const LIST_ROW: f64 = 28.0;
const INDENT: f64 = 14.0;
/// The most folders the popover's list shows at once.
const LIST_SHOWN: usize = 10;
/// The bar's height, below the address row.
pub(super) const BAR_HEIGHT: f64 = 34.0;
const CHIP_HEIGHT: f64 = 26.0;
const CHIP_MAX: f64 = 180.0;
const CHIP_PADDING: f64 = 10.0;
const CHIP_RADIUS: f64 = 4.0;
const MENU_WIDTH: f64 = 280.0;

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

/// The bar's state in the browser.
#[derive(Debug, Default)]
pub(super) struct BarUi {
    view: BarView,
    /// Each item drawn: its number, whether a folder, its rect.
    chips: Vec<(u32, bool, Rect)>,
    /// An open folder's list: its items' numbers and rects.
    list: Vec<(u32, Rect)>,
    list_area: Rect,
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

    /// Command-A in the popover's name.
    pub(super) fn star_select_all(&mut self) -> bool {
        if !self.star_editing() {
            return false;
        }
        self.star.name.select_all();
        true
    }

    /// A press while the popover is open; a press outside it is Done, as a
    /// browser's bookmark bubble closes, keeping what it shows.
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

    // ---- The bar ----

    /// The bar as the trusted core sent it.
    pub fn set_bar(&mut self, view: BarView) {
        self.bar.view = view;
    }

    /// Whether the bar is shown: the strip is a row taller.
    #[must_use]
    pub fn bar_shown(&self) -> bool {
        self.bar.view.shown
    }

    /// Shows the bar, or hides it (Command-Shift-B).
    pub(super) fn toggle_bar(&mut self) {
        self.outputs.push(Output::Bar(BarRequest::Show(!self.bar.view.shown)));
    }

    /// A press on the bar or its open list; with `new_tab`, a bookmark
    /// opens in a new tab.
    pub(super) fn bar_press(&mut self, point: Point, new_tab: bool) -> bool {
        if !self.bar.view.shown {
            return false;
        }
        if let Some(&(number, _)) = self.bar.list.iter().find(|(_, rect)| rect.contains(point)) {
            self.outputs.push(Output::Bar(BarRequest::Open { number, new_tab }));
            return true;
        }
        let list_open = self.bar.view.open.is_some();
        if let Some(&(number, folder, _)) = self.bar.chips.iter().find(|(_, _, rect)| rect.contains(point)) {
            let open_now = self.bar.view.open.as_ref().map(|(open, _)| *open);
            self.outputs.push(Output::Bar(if folder {
                BarRequest::Folder((open_now != Some(number)).then_some(number))
            } else {
                BarRequest::Open { number, new_tab }
            }));
            return true;
        }
        // A press elsewhere closes an open list.
        if list_open {
            self.outputs.push(Output::Bar(BarRequest::Folder(None)));
            return self.bar.list_area.contains(point);
        }
        false
    }

    /// The bar, below the address row, from `top`.
    pub(super) fn draw_bar<C: Canvas + ?Sized>(&mut self, canvas: &mut C, top: f64, width: f64) {
        self.bar.chips.clear();
        self.bar.list.clear();
        self.bar.list_area = Rect::ZERO;
        if !self.bar.view.shown {
            return;
        }
        let row = Rect::new(0.0, top, width, top + BAR_HEIGHT);
        canvas.fill(row, 0.0, BLACK);
        canvas.fill(Rect::new(0.0, row.y1 - HAIRLINE, width, row.y1), 0.0, RULE);
        let chip_top = top + (BAR_HEIGHT - CHIP_HEIGHT) / 2.0;
        let mut x = GAP;
        if self.bar.view.items.is_empty() {
            left_centred(canvas, row, GAP * 2.0, "Bookmarks bar: the star puts bookmarks here when you choose its folder", ROW_STATUS, DIM);
        }
        let items = self.bar.view.items.clone();
        for item in &items {
            // A folder's chip ends in a triangle pointing down, as a list's does.
            let mark = if item.folder { TRIANGLE + GAP } else { 0.0 };
            let width_needed = (canvas.text_width(&item.name, ROW_STATUS) + CHIP_PADDING * 2.0 + mark).min(CHIP_MAX);
            if x + width_needed > width - GAP {
                break;
            }
            let chip = Rect::new(x, chip_top, x + width_needed, chip_top + CHIP_HEIGHT);
            let open = self.bar.view.open.as_ref().is_some_and(|(number, _)| *number == item.number);
            canvas.fill(
                chip,
                CHIP_RADIUS,
                if open {
                    SELECTED
                } else if chip.contains(self.pointer) {
                    RULE_SOFT
                } else {
                    BUTTON
                },
            );
            canvas.push_clip(Rect::new(chip.x0, chip.y0, chip.x1 - mark, chip.y1));
            left_centred(canvas, chip, CHIP_PADDING, &item.name, ROW_STATUS, WHITE);
            canvas.pop_clip();
            if item.folder {
                triangle(canvas, Point::new(chip.x1 - CHIP_PADDING - TRIANGLE / 2.0, f64::midpoint(chip.y0, chip.y1)), true, WHITE);
            }
            self.bar.chips.push((item.number, item.folder, chip));
            x = chip.x1 + GAP;
        }
        // An open folder's list, under its chip.
        if let Some((number, contents)) = self.bar.view.open.clone() {
            let below = self.bar.chips.iter().find(|(each, _, _)| *each == number).map(|&(_, _, chip)| chip);
            if let Some(chip) = below {
                self.draw_bar_list(canvas, chip, &contents);
            }
        }
    }

    fn draw_bar_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C, chip: Rect, contents: &[ItemRow]) {
        #[allow(clippy::cast_precision_loss, reason = "at most a hundred rows")]
        let height = (LIST_ROW * contents.len().max(1) as f64).min(self.body.y1 - chip.y1);
        let area = Rect::new(chip.x0, chip.y1 + HAIRLINE, chip.x0 + MENU_WIDTH, chip.y1 + HAIRLINE + height);
        canvas.fill(area, 0.0, BLACK);
        border(canvas, area, HAIRLINE, RULE);
        self.bar.list_area = area;
        if contents.is_empty() {
            left_centred(canvas, area, GAP, "This folder is empty", ROW_STATUS, DIM);
        }
        canvas.push_clip(area);
        let mut y = area.y0;
        for item in contents {
            let rect = Rect::new(area.x0, y, area.x1, y + LIST_ROW);
            if rect.contains(self.pointer) {
                canvas.fill(rect, 0.0, RULE_SOFT);
            }
            left_centred(canvas, rect, GAP, &item.name, ROW_TITLE, if item.folder { DIM } else { WHITE });
            if !item.folder {
                self.bar.list.push((item.number, rect));
            }
            y += LIST_ROW;
        }
        canvas.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, Modifiers};
    use crate::library::FolderRow;
    use crate::test_canvas::Recorder;

    fn middle(rect: Rect) -> Point {
        Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
    }

    fn browser() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser
    }

    /// Requirement 20.16: the star asks; the popover renames, moves, keeps
    /// the whole channel, removes, and closes as Done when pressed outside.
    #[test]
    fn the_popover_asks_for_each_change() {
        let mut browser = browser();
        browser.key(Key::Text("d"), Modifiers { command: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Bookmark]);
        let folders =
            alloc::vec![FolderRow { number: 11, name: "Bookmarks bar".into(), depth: 0 }, FolderRow { number: 12, name: "Other bookmarks".into(), depth: 0 }];
        browser.set_star(StarView { open: true, name: "News 302 WEATHER".into(), folder: 12, folders, page: true });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let (name, folder) = (browser.star.controls.name, browser.star.controls.folder);
        browser.star_press(middle(name));
        browser.key(Key::Text("a"), Modifiers { command: true, ..Modifiers::default() }, 0);
        browser.key(Key::Text("Weather"), Modifiers::default(), 0);
        browser.star_press(middle(folder));
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let bar = browser.star.controls.folders[0].1;
        browser.star_press(middle(bar));
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let whole = browser.star.controls.whole;
        browser.star_press(middle(whole));
        assert_eq!(
            browser.take_outputs(),
            [
                Output::TextInput(true),
                Output::TextInput(false),
                Output::Star(StarRequest::Edit { name: "Weather".into(), folder: 11 }),
                Output::Star(StarRequest::WholeChannel)
            ]
        );
        // A press elsewhere is Done, with the name and folder.
        browser.star_press(Point::new(10.0, 600.0));
        assert_eq!(browser.take_outputs(), [Output::Star(StarRequest::Edit { name: "Weather".into(), folder: 11 }), Output::Star(StarRequest::Done)]);
        assert!(!browser.star_open());
    }

    /// Requirement 20.18: the bar's chips open bookmarks and folders, and
    /// Command-Shift-B asks to show or hide it.
    #[test]
    fn the_bar_opens_its_bookmarks() {
        let mut browser = browser();
        browser.key(Key::Text("b"), Modifiers { command: true, shift: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::Bar(BarRequest::Show(true))]);
        let items = alloc::vec![
            ItemRow { number: 1, folder: false, name: "News".into(), address: "https://news.example/".into(), place: String::new() },
            ItemRow { number: 2, folder: true, name: "Weather".into(), address: String::new(), place: String::new() },
        ];
        browser.set_bar(BarView { shown: true, items, open: None });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        assert!(browser.body.y0 > 0.0 && browser.bar.chips[0].2.y1 <= browser.body.y0, "the bar is above the body");
        let (news, weather) = (browser.bar.chips[0].2, browser.bar.chips[1].2);
        assert!(browser.bar_press(middle(news), true));
        assert!(browser.bar_press(middle(weather), false));
        assert_eq!(browser.take_outputs(), [Output::Bar(BarRequest::Open { number: 1, new_tab: true }), Output::Bar(BarRequest::Folder(Some(2)))]);
    }
}
