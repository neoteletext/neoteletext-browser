//! The chosen bank's page list in the middle: a row per page with its
//! number, its title and, for a minute after an update, its age.

use alloc::format;
use alloc::string::String;

use crate::browser::Browser;
use crate::browser::overview::ROW_FLASH;
use crate::browser::window::{HEADING_HEIGHT, PADDING};
use crate::canvas::Canvas;
use crate::channel::page_number;
use crate::geometry::Rect;
use crate::text::{HAIRLINE, left_centred, outline, right_centred};
use crate::theme::{COLUMN_HEAD, HEADING, MUTED, PANEL, ROW_NUMBER, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SELECTED, UPDATED, WHITE, mix};
use crate::wall::{glow, since};

pub(super) const ROW_HEIGHT: f64 = 27.0;
const COLUMN_HEAD_HEIGHT: f64 = 26.0;
/// Where the title column starts: after a three-digit page number.
const TITLE_COLUMN: f64 = 56.0;
const SCROLL_BAR_WIDTH: f64 = 4.0;

impl Browser {
    pub(super) fn draw_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect, now: u64) {
        canvas.fill(panel, 0.0, PANEL);
        let heading = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + HEADING_HEIGHT);
        canvas.fill(heading, 0.0, SELECTED);
        let pages = self.bank_pages();
        let first = self.bank.saturating_add(1);
        let name = self.channel.banks.get(self.bank).map_or("", String::as_str);
        let title = if name.is_empty() {
            format!("Pages {first}00-{first}99 ({})", pages.len())
        } else {
            format!("{first}00 {name} · pages {first}00-{first}99 ({})", pages.len())
        };
        canvas.push_clip(heading);
        left_centred(canvas, heading, PADDING, &title, HEADING, WHITE);
        canvas.pop_clip();

        let head = Rect::new(panel.x0, heading.y1, panel.x1, heading.y1 + COLUMN_HEAD_HEIGHT);
        let title_x = panel.x0 + TITLE_COLUMN;
        for (x, text) in [(panel.x0 + PADDING, "Page"), (title_x + PADDING, "Title")] {
            left_centred(canvas, Rect::new(x, head.y0, panel.x1, head.y1), 0.0, text, COLUMN_HEAD, MUTED);
        }
        canvas.fill(Rect::new(panel.x0, head.y1 - HAIRLINE, panel.x1, head.y1), 0.0, RULE);

        let area = Rect::new(panel.x0, head.y1, panel.x1, panel.y1);
        self.regions.list = area;
        #[allow(clippy::cast_precision_loss, reason = "at most 100 rows")]
        let content = pages.len() as f64 * ROW_HEIGHT;
        let max_scroll = (content - area.height()).max(0.0);
        self.list_scroll = self.list_scroll.min(max_scroll);
        canvas.push_clip(area);
        for (position, &index) in pages.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "at most 100 rows")]
            let top = area.y0 + position as f64 * ROW_HEIGHT - self.list_scroll;
            if top > area.y1 || top + ROW_HEIGHT < area.y0 {
                continue;
            }
            let row = Rect::new(area.x0, top, area.x1, top + ROW_HEIGHT);
            self.regions.rows.push((row, index));
            let Some(Some(info)) = self.channel.pages.get(index) else {
                continue;
            };
            let chosen = self.selected == Some(index);
            if chosen {
                canvas.fill(row, 0.0, SELECTED);
            }
            canvas.fill(Rect::new(row.x0, row.y1 - HAIRLINE, row.x1, row.y1), 0.0, RULE_SOFT);
            // A row just updated flashes amber and fades back.
            let updated = self.updated_at(index);
            let glowing = glow(updated, now);
            if glowing > 0.0 {
                canvas.fill(row, 0.0, mix(if chosen { SELECTED } else { PANEL }, UPDATED, glowing * ROW_FLASH));
            }
            let number = page_number(index).map_or_else(String::new, |page| format!("{page:03X}"));
            left_centred(canvas, row, PADDING, &number, ROW_NUMBER, WHITE);
            let mut title_end = row.x1 - PADDING;
            if let Some(age) = since(updated, now) {
                title_end -= canvas.text_width(&age, ROW_STATUS) + PADDING;
                right_centred(canvas, row, PADDING, &age, ROW_STATUS, UPDATED);
            }
            canvas.push_clip(Rect::new(title_x, row.y0, title_end, row.y1));
            left_centred(canvas, Rect::new(title_x, row.y0, row.x1, row.y1), PADDING, &info.title, ROW_TITLE, WHITE);
            canvas.pop_clip();
        }
        canvas.pop_clip();
        if max_scroll > 0.0 {
            let bar = area.height() * area.height() / content;
            let top = area.y0 + (area.height() - bar) * self.list_scroll / max_scroll;
            canvas.fill(Rect::new(area.x1 - SCROLL_BAR_WIDTH * 2.0, top, area.x1 - SCROLL_BAR_WIDTH, top + bar), SCROLL_BAR_WIDTH / 2.0, RULE);
        }
        outline(canvas, panel, RULE);
    }
}
