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
/// The row of a list Page Up and Page Down keep in view, so that the viewer
/// keeps their place (REQ-1758 item 1).
const ROWS_OVERLAPPED: usize = 1;

/// The rows Page Up and Page Down move a list `height` high: one
/// list-height, less the overlap, and always at least one, so that a key
/// moves even a list too short to hold two rows.
pub(super) fn rows_per_key(height: f64) -> usize {
    // Whole rows, and a negative height saturates to none.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "the cast is the rounding down wanted")]
    let rows = (height / ROW_HEIGHT) as usize;
    rows.saturating_sub(ROWS_OVERLAPPED).max(1)
}

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
        // Only now can a row be put in view: the regions are cleared at the
        // start of each draw, so the list has no height before this.
        self.follow_single();
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
            if let Some(count) = self.subpage_count(index) {
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
            self.draw_subpage_list(canvas, Rect::new(whole.x0, area.y1, whole.x1, whole.y1), colour, now);
        }
        border(canvas, panel, LIST_BORDER, colour);
    }
}

/// The mark of a page with sub-pages, ending at `right` in `row`: two
/// offset outlines, a stack of pages, and how many it has. Returns its width.
fn subpage_mark<C: Canvas + ?Sized>(canvas: &mut C, row: Rect, right: f64, count: u16) -> f64 {
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
    use super::rows_per_key;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, middle, press_at};
    use crate::browser::list_focus::ListFocus;
    use crate::browser::pointer::Cursor;
    use crate::browser::{Key, Modifiers};
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

    /// The bank's page at `position` in its list.
    fn at(browser: &crate::browser::Browser, position: usize) -> Option<usize> {
        browser.bank_pages().get(position).copied()
    }

    /// The first row chosen, with the keys in the list.
    fn on_first_row() -> (crate::browser::Browser, Recorder) {
        let (mut browser, mut canvas) = browser();
        let row = browser.regions.rows[0].0;
        press_at(&mut browser, &mut canvas, row);
        (browser, canvas)
    }

    /// REQ-1758 items 1 and 2: Page Down chooses the page one list-height
    /// less one row on, scrolled into view, and Page Up returns.
    #[test]
    fn page_down_moves_the_pages_a_list_height_and_page_up_returns() {
        let (mut browser, mut canvas) = on_first_row();
        let rows = rows_per_key(browser.regions.list.height());
        assert!(rows < browser.bank_pages().len(), "the list is shorter than its rows: {rows}");
        browser.key(Key::PageDown, Modifiers::default(), 0);
        assert_eq!(browser.selected, at(&browser, rows));
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(browser.regions.rows.iter().any(|&(row, index)| Some(index) == browser.selected && row.y1 <= browser.regions.list.y1), "scrolled into view");
        browser.key(Key::PageUp, Modifiers::default(), 0);
        assert_eq!(browser.selected, at(&browser, 0));
    }

    /// REQ-1758 item 3: neither key wraps round.
    #[test]
    fn the_keys_stay_on_the_end_rows_of_the_pages() {
        let (mut browser, _) = on_first_row();
        browser.key(Key::PageUp, Modifiers::default(), 0);
        assert_eq!(browser.selected, at(&browser, 0), "stays on the first row");
        let last = browser.bank_pages().len() - 1;
        for _ in 0..=last {
            browser.key(Key::PageDown, Modifiers::default(), 0);
        }
        assert_eq!(browser.selected, at(&browser, last), "stops on the last row");
        browser.key(Key::PageDown, Modifiers::default(), 0);
        assert_eq!(browser.selected, at(&browser, last), "and stays there");
    }

    /// REQ-1758 item 4: the distance follows the height the divider left.
    #[test]
    fn the_distance_follows_the_height_of_the_pages() {
        let (mut browser, mut canvas) = on_first_row();
        browser.set_subpage_count(7, 0x100, 3);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let before = rows_per_key(browser.regions.list.height());
        let divider = middle(browser.regions.subpage_divider);
        browser.pointer_moved(divider.x, divider.y, 0);
        browser.pointer_pressed(&mut canvas, false, 0);
        browser.pointer_moved(divider.x, divider.y - 135.0, 0);
        browser.pointer_released();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let after = rows_per_key(browser.regions.list.height());
        assert!(after < before, "the list is shorter: {after} rows from {before}");
        browser.key(Key::PageDown, Modifiers::default(), 0);
        assert_eq!(browser.selected, at(&browser, after));
    }

    /// With the page alone no list has the keys, and the arrows move from
    /// page to page: these keys do too, further, and the page follows.
    #[test]
    fn with_the_page_alone_the_keys_move_it_a_list_height() {
        let (mut browser, _) = on_first_row();
        browser.key(Key::Enter, Modifiers::default(), 0);
        let first = browser.wall.zoomed().expect("the page open").page;
        let rows = rows_per_key(browser.regions.list.height());
        browser.key(Key::PageDown, Modifiers::default(), 0);
        let page = crate::channel::page_number(at(&browser, rows).unwrap());
        assert_ne!(page, Some(first));
        assert_eq!(browser.wall.zoomed().map(|key| key.page), page);
    }

    /// The page the wall opens by itself is chosen during a draw, before the
    /// list has its height, and its row must still land inside the list.
    #[test]
    fn with_the_page_alone_the_chosen_row_starts_in_the_list() {
        let (mut browser, mut canvas) = browser();
        browser.toggle_wall(0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let chosen = browser.selected.expect("the first page chosen");
        let row = browser.regions.rows.iter().find(|&&(_, index)| index == chosen).expect("its row drawn").0;
        let list = browser.regions.list;
        assert!(row.y0 >= list.y0 && row.y1 <= list.y1, "{row:?} in {list:?}");
    }

    /// REQ-1758 item 7: with the page alone a page clicked in the list
    /// opens, and the list keeps the keys, so the arrows and the Page keys
    /// move it on as they do on the wall, and the page follows.
    #[test]
    fn with_the_page_alone_a_row_clicked_keeps_the_keys_in_the_list() {
        let (mut browser, mut canvas) = browser();
        browser.toggle_wall(0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(browser.wall.single(), "the page alone");
        let row = browser.regions.rows[0].0;
        press_at(&mut browser, &mut canvas, row);
        assert_eq!(browser.list_focus, Some(ListFocus::Pages), "the list has the keys");
        browser.key(Key::Down, Modifiers::default(), 0);
        assert_eq!(browser.selected, at(&browser, 1), "Down moves a row");
        assert_eq!(browser.wall.zoomed().map(|key| key.page), crate::channel::page_number(at(&browser, 1).unwrap()));
        browser.key(Key::Up, Modifiers::default(), 0);
        assert_eq!(browser.selected, at(&browser, 0), "Up moves back");
        let rows = rows_per_key(browser.regions.list.height());
        browser.key(Key::PageDown, Modifiers::default(), 0);
        assert_eq!(browser.selected, at(&browser, rows), "Page Down moves a list-height less a row");
        assert_eq!(browser.list_focus, Some(ListFocus::Pages), "and the list keeps them");
    }
}
