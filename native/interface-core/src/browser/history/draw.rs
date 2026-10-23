//! The history panel drawn: the search and Clear history, the clearing
//! choices or Remove selected, the recently closed tabs, the rows under
//! their days' headings, a row's menu and the footer; and its tips.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{
    CHECK_SIZE, CLOSED_SHOWN, CLOSED_WIDTH, CONTROL_HEIGHT, Controls, DAY_HEIGHT, GAP, HEADING_HEIGHT, MENU_WIDTH, PADDING, PANEL_HEIGHT, PANEL_WIDTH,
    RANGE_BUTTON, RANGES, ROW_HEIGHT, RowAction, SMALL_BUTTON, TIME_WIDTH, WIDE_BUTTON,
};
use crate::browser::Browser;
use crate::browser::widgets::{button, field, panel, panel_rect};
use crate::canvas::Canvas;
use crate::geometry::Rect;
use crate::theme::{ADDRESS, BUTTON, CAPTION, DIM, HEADING, MUTED, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SELECTED, WHITE};
use crate::ui::{border, centred, left_centred, right_centred};

impl Browser {
    /// The panel's symbol buttons.
    pub(in crate::browser) fn history_tips(&self, tips: &mut Vec<(Rect, String)>) {
        let controls = &self.history.controls;
        tips.push((controls.close, format!("Close history ({})", self.keys_named("Cmd+Y", "Ctrl+H"))));
        for &(_, _, menu, _) in &controls.rows {
            tips.push((menu, "More from this site, or remove from history".into()));
        }
    }

    /// The panel, over the window below the strip, while it is open.
    #[allow(clippy::too_many_lines, reason = "one part of the panel after another, in drawing order")]
    pub(in crate::browser) fn draw_history_panel<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
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
