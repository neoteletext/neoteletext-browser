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
use crate::text::{HAIRLINE, border, left_centred, left_fitted, right_centred};
use crate::theme::{
    BANK_COLOURS, COLUMN_HEAD, HEADING, MUTED, PANEL, RAISED, ROW_NUMBER, ROW_STATUS, ROW_TITLE, RULE, RULE_SOFT, SELECTED, UPDATED, WHITE, ink_on, mix,
};
use crate::wall::{glow, since};

pub(super) const ROW_HEIGHT: f64 = 27.0;
/// The heading's second line, the magazine's range, under its name.
const RANGE_LINE_HEIGHT: f64 = 20.0;
/// The border round the list, in its magazine's colour.
const LIST_BORDER: f64 = 2.0;
const COLUMN_HEAD_HEIGHT: f64 = 26.0;
/// Where the title column starts: after a three-digit page number.
const TITLE_COLUMN: f64 = 56.0;
const SCROLL_BAR_WIDTH: f64 = 4.0;

impl Browser {
    pub(super) fn draw_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect, now: u64) {
        canvas.fill(panel, 0.0, PANEL);
        // The heading in the magazine's colour, as the magazines show it:
        // its name, and under it its range and how many pages it has.
        let colour = BANK_COLOURS.get(self.bank).copied().unwrap_or(SELECTED);
        let ink = ink_on(colour);
        let heading = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + HEADING_HEIGHT + RANGE_LINE_HEIGHT);
        canvas.fill(heading, 0.0, colour);
        let pages = self.bank_pages();
        let first = self.bank.saturating_add(1);
        let name = self.channel.banks.get(self.bank).map_or("", String::as_str);
        let title = if name.is_empty() { format!("Magazine {first}") } else { format!("{first}00 {name}") };
        let range = format!("Pages {first}00-{first}99 ({})", pages.len());
        let name_line = Rect::new(heading.x0, heading.y0, heading.x1, heading.y0 + HEADING_HEIGHT);
        left_fitted(canvas, name_line, PADDING, &title, HEADING, ink);
        let range_line = Rect::new(heading.x0, name_line.y1 - PADDING / 2.0, heading.x1, heading.y1 - PADDING / 2.0);
        left_fitted(canvas, range_line, PADDING, &range, COLUMN_HEAD, ink);

        let head = Rect::new(panel.x0, heading.y1, panel.x1, heading.y1 + COLUMN_HEAD_HEIGHT);
        let title_x = panel.x0 + TITLE_COLUMN;
        for (x, text) in [(panel.x0 + PADDING, "Page"), (title_x + PADDING, "Title")] {
            left_centred(canvas, Rect::new(x, head.y0, panel.x1, head.y1), 0.0, text, COLUMN_HEAD, MUTED);
        }
        canvas.fill(Rect::new(panel.x0, head.y1 - HAIRLINE, panel.x1, head.y1), 0.0, RULE);

        // With sub-pages in the magazine, the list is two: the pages above
        // the divider, the chosen page's sub-pages below (decision 111).
        let whole = Rect::new(panel.x0, head.y1, panel.x1, panel.y1);
        let split = self.magazine_has_subpages();
        let area = if split { Rect::new(whole.x0, whole.y0, whole.x1, self.split_at(whole)) } else { whole };
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
            } else if self.hovered_row == Some(index) {
                canvas.fill(row, 0.0, RAISED);
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
            // A page with sub-pages is marked with how many it has.
            if let Some(count) = self.subpages_of(index).map(<[_]>::len) {
                title_end -= subpage_mark(canvas, row, title_end, count) + PADDING;
            }
            if let Some(age) = since(updated, now) {
                right_centred(canvas, Rect::new(row.x0, row.y0, title_end, row.y1), 0.0, &age, ROW_STATUS, UPDATED);
                title_end -= canvas.text_width(&age, ROW_STATUS) + PADDING;
            }
            left_fitted(canvas, Rect::new(title_x, row.y0, title_end, row.y1), PADDING, &info.title, ROW_TITLE, WHITE);
        }
        canvas.pop_clip();
        if max_scroll > 0.0 {
            let bar = area.height() * area.height() / content;
            let top = area.y0 + (area.height() - bar) * self.list_scroll / max_scroll;
            canvas.fill(Rect::new(area.x1 - SCROLL_BAR_WIDTH * 2.0, top, area.x1 - SCROLL_BAR_WIDTH, top + bar), SCROLL_BAR_WIDTH / 2.0, RULE);
        }
        if split {
            self.draw_subpage_list(canvas, Rect::new(whole.x0, area.y1, whole.x1, whole.y1), colour);
        }
        border(canvas, panel, LIST_BORDER, colour);
    }
}

/// The mark of a page with sub-pages, ending at `right` in `row`: two
/// offset outlines, a stack of pages, and how many it has. Returns its width.
fn subpage_mark<C: Canvas + ?Sized>(canvas: &mut C, row: Rect, right: f64, count: usize) -> f64 {
    /// A card of the stack, and how far the one behind sits up and right.
    const CARD: f64 = 9.0;
    const OFFSET: f64 = 3.0;
    let text = format!("{count}");
    let width = canvas.text_width(&text, ROW_STATUS);
    let middle = f64::midpoint(row.y0, row.y1);
    let left = right - width - CARD - OFFSET - PADDING / 2.0;
    let front = Rect::new(left, middle - CARD / 2.0 + OFFSET / 2.0, left + CARD, middle + CARD / 2.0 + OFFSET / 2.0);
    let back = Rect::new(front.x0 + OFFSET, front.y0 - OFFSET, front.x1 + OFFSET, front.y1 - OFFSET);
    crate::text::outline(canvas, back, MUTED);
    canvas.fill(front, 0.0, PANEL);
    crate::text::outline(canvas, front, WHITE);
    right_centred(canvas, Rect::new(row.x0, row.y0, right, row.y1), 0.0, &text, ROW_STATUS, WHITE);
    right - left
}

#[cfg(test)]
mod tests {
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, middle, press_at};
    use crate::browser::pointer::Cursor;
    use crate::test_canvas::Recorder;
    use crate::theme::{BANK_COLOURS, BLACK, WHITE};

    /// A row and a magazine under the pointer are highlighted and show the
    /// hand, as a click on them chooses.
    #[test]
    fn the_pointer_over_a_row_or_a_magazine_is_a_hand() {
        let (mut browser, mut canvas) = browser();
        let row = browser.regions.rows[1];
        let point = middle(row.0);
        assert!(browser.pointer_moved(point.x, point.y, 0), "drawn again to highlight it");
        assert_eq!(browser.hovered_row, Some(row.1));
        assert_eq!(browser.cursor(), Cursor::Hand);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let bank = middle(browser.regions.banks[3]);
        browser.pointer_moved(bank.x, bank.y, 0);
        assert_eq!((browser.hovered_row, browser.hovered_bank), (None, Some(3)));
        assert_eq!(browser.cursor(), Cursor::Hand);
    }

    /// The list's heading is its magazine's colour, with its range on a
    /// line of its own, in the ink that reads on that colour.
    #[test]
    fn the_heading_is_the_magazines_colour_with_its_range_under_its_name() {
        let (mut browser, mut canvas) = browser();
        let yellow = browser.regions.banks[2];
        press_at(&mut browser, &mut canvas, yellow);
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let range = canvas.texts.iter().find(|(_, text)| text.starts_with("Pages 300-399")).expect("the range");
        let name = canvas.texts.iter().find(|(_, text)| text == "Magazine 3").expect("the name");
        assert!(range.0.y0 > name.0.y0, "the range on the line under the name");
        assert_eq!(canvas.coloured_text(&range.1), Some(BLACK), "black reads on yellow");
        assert!(canvas.fills.iter().any(|&(_, colour)| colour == BANK_COLOURS[2]), "the heading is the magazine's colour");
        let red = browser.regions.banks[0];
        press_at(&mut browser, &mut canvas, red);
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert_eq!(canvas.coloured_text("Magazine 1"), Some(WHITE), "white reads on red");
    }
}
