//! The bookmarks bar (`history-and-bookmarks.md` section 3.4, requirement
//! 20.18): a row of the trusted strip, below the address, out of every
//! page's reach, with a chip for each bookmark and folder and a folder's
//! list under its chip, each opened by number.

use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::star::{GAP, LIST_ROW};
use crate::browser::widgets::{TRIANGLE, triangle};
use crate::browser::{Browser, Output};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::library::{BarRequest, BarView, ItemRow};
use crate::text::{HAIRLINE, border, left_centred};
use crate::theme::{BLACK, BUTTON, DIM, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SELECTED, WHITE};

/// The bar's height, below the address row.
pub(in crate::browser) const BAR_HEIGHT: f64 = 34.0;
const CHIP_HEIGHT: f64 = 26.0;
const CHIP_MAX: f64 = 180.0;
const CHIP_PADDING: f64 = 10.0;
pub(super) const CHIP_RADIUS: f64 = 4.0;
const MENU_WIDTH: f64 = 280.0;

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
    /// The bar's chips: each bookmark's whole name, which a chip may cut short.
    pub(super) fn bar_tips(&self, tips: &mut Vec<(Rect, String)>) {
        for &(number, folder, rect) in &self.bar.chips {
            if let Some(item) = self.bar.view.items.iter().find(|item| item.number == number) {
                tips.push((rect, if folder { alloc::format!("{} (folder)", item.name) } else { item.name.clone() }));
            }
        }
    }

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

    /// The bookmark of the bar, or of its open list, at `point`.
    pub(super) fn bar_bookmark_at(&self, point: Point) -> Option<u32> {
        if !self.bar.view.shown {
            return None;
        }
        if let Some(&(number, _)) = self.bar.list.iter().find(|(_, rect)| rect.contains(point)) {
            return Some(number);
        }
        self.bar.chips.iter().find(|(_, folder, rect)| !folder && rect.contains(point)).map(|&(number, _, _)| number)
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
    use crate::browser::fixture::{blank_browser, middle};
    use crate::browser::{Key, Modifiers};
    use crate::test_canvas::Recorder;

    /// The middle button opens a bar bookmark in a new tab; on a folder's
    /// chip it opens nothing.
    #[test]
    fn the_middle_button_opens_a_bar_bookmark_in_a_new_tab() {
        let mut browser = blank_browser();
        let items = alloc::vec![
            ItemRow { number: 1, folder: false, name: "News".into(), address: "https://news.example/".into(), place: String::new() },
            ItemRow { number: 2, folder: true, name: "Weather".into(), address: String::new(), place: String::new() },
        ];
        browser.set_bar(BarView { shown: true, items, open: None });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let (news, weather) = (browser.bar.chips[0].2, browser.bar.chips[1].2);
        browser.pointer_moved(middle(weather).x, middle(weather).y, 0);
        assert!(!browser.middle_pressed());
        browser.pointer_moved(middle(news).x, middle(news).y, 0);
        assert!(browser.middle_pressed());
        assert_eq!(browser.take_outputs(), [Output::Bar(BarRequest::Open { number: 1, new_tab: true })]);
    }

    /// Requirement 20.18: the bar's chips open bookmarks and folders, and
    /// Command-Shift-B asks to show or hide it.
    #[test]
    fn the_bar_opens_its_bookmarks() {
        let mut browser = blank_browser();
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
