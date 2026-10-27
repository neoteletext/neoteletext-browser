//! The sub-pages of the chosen page, under the page list (decision 111):
//! when any page of the magazine has sub-pages, the list is two, the pages
//! above and the chosen page's sub-pages below, with a divider between
//! them that can be pulled; a click on a sub-page opens its page there.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::Browser;
use crate::browser::page_list::ROW_HEIGHT;
use crate::browser::window::PADDING;
use crate::canvas::Canvas;
use crate::channel::page_index;
use crate::controls::PageControl;
use crate::geometry::{Point, Rect};
use crate::text::{HAIRLINE, left_centred, left_fitted};
use crate::theme::{COLUMN_HEAD, MUTED, PANEL, RAISED, ROW_NUMBER, ROW_TITLE, RULE, RULE_SOFT, SELECTED, WHITE};

/// The share of the list the pages take until the divider is pulled.
const PAGES_SHARE: f64 = 0.6;
/// The fewest rows either list keeps.
const FEWEST_ROWS: f64 = 3.0;
/// The divider: a bar the height of a hairline's grip, and how far either
/// side it can be gripped.
const DIVIDER_HEIGHT: f64 = 6.0;
/// The sub-pages' heading row.
const HEADING_HEIGHT: f64 = 26.0;
/// Where a sub-page's title starts: after its place, "12/12".
const TITLE_COLUMN: f64 = 56.0;

/// The page list's sub-pages: each page's, the divider and the scroll.
#[derive(Debug)]
pub(super) struct SubPages {
    /// Each page's sub-pages with their titles, by page index, for the
    /// channel `channel`.
    pub(super) pages: BTreeMap<usize, Vec<(u16, String)>>,
    channel: u32,
    /// The pages' share of the list, as the divider was pulled.
    share: f64,
    /// Where the divider was gripped, from its top, while it is pulled.
    grip: Option<f64>,
    /// The sub-pages' scroll, in pixels.
    pub(super) scroll: f64,
    /// The sub-page row under the pointer, by its sub-code.
    pub(super) hovered: Option<u16>,
}

impl Default for SubPages {
    fn default() -> Self {
        Self { pages: BTreeMap::new(), channel: 0, share: PAGES_SHARE, grip: None, scroll: 0.0, hovered: None }
    }
}

impl Browser {
    /// A page's sub-pages and their titles, as the trusted core says; none
    /// when it has one again.
    pub fn set_subpages(&mut self, channel: u32, page: u16, subpages: Vec<(u16, String)>) {
        if channel != self.subpages.channel {
            self.subpages.pages.clear();
            self.subpages.channel = channel;
        }
        let Some(index) = page_index(page) else { return };
        if subpages.len() > 1 {
            self.subpages.pages.insert(index, subpages);
        } else {
            self.subpages.pages.remove(&index);
        }
    }

    /// The sub-pages of page `index`, when it has more than one.
    pub(super) fn subpages_of(&self, index: usize) -> Option<&[(u16, String)]> {
        (self.subpages.channel == self.channel.channel).then(|| self.subpages.pages.get(&index)).flatten().map(Vec::as_slice)
    }

    /// Whether any page of the chosen magazine has sub-pages: the list is
    /// then two.
    pub(super) fn magazine_has_subpages(&self) -> bool {
        self.bank_pages().iter().any(|&index| self.subpages_of(index).is_some())
    }

    /// Where the divider lies in `area`, the list under its heading: the
    /// pages above it, the sub-pages below.
    pub(super) fn split_at(&self, area: Rect) -> f64 {
        let least = FEWEST_ROWS * ROW_HEIGHT;
        let most = (area.height() - least - HEADING_HEIGHT - DIVIDER_HEIGHT).max(least);
        area.y0 + (area.height() * self.subpages.share).clamp(least, most)
    }

    /// Draws the chosen page's sub-pages in `area`, under the divider.
    pub(super) fn draw_subpage_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect, colour: crate::theme::Colour) {
        let divider = Rect::new(area.x0, area.y0, area.x1, area.y0 + DIVIDER_HEIGHT);
        canvas.fill(divider, 0.0, colour);
        self.regions.subpage_divider = Rect::new(divider.x0, divider.y0 - DIVIDER_HEIGHT, divider.x1, divider.y1 + DIVIDER_HEIGHT);
        let heading = Rect::new(area.x0, divider.y1, area.x1, divider.y1 + HEADING_HEIGHT);
        canvas.fill(heading, 0.0, PANEL);
        let selected = self.selected.and_then(|index| Some((index, self.subpages_of(index)?.to_vec())));
        let words = match (&selected, self.selected.and_then(crate::channel::page_number)) {
            (Some((_, list)), Some(page)) => format!("Sub-pages of {page:03X} ({})", list.len()),
            _ => String::from("Choose a page marked with sub-pages"),
        };
        left_fitted(canvas, heading, PADDING, &words, COLUMN_HEAD, MUTED);
        canvas.fill(Rect::new(heading.x0, heading.y1 - HAIRLINE, heading.x1, heading.y1), 0.0, RULE);
        let rows = Rect::new(area.x0, heading.y1, area.x1, area.y1);
        self.regions.subpage_list = rows;
        let Some((index, list)) = selected else { return };
        let Some(page) = crate::channel::page_number(index) else { return };
        // The sub-page drawn, when this is the page open and one is chosen.
        let controls = &self.page_controls.controls;
        let chosen = (controls.page == Some(page)).then_some(controls.chosen).flatten().map(usize::from);
        #[allow(clippy::cast_precision_loss, reason = "at most 64 rows")]
        let content = list.len() as f64 * ROW_HEIGHT;
        self.subpages.scroll = self.subpages.scroll.min((content - rows.height()).max(0.0));
        canvas.push_clip(rows);
        let count = list.len();
        for (place, (subcode, title)) in list.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "at most 64 rows")]
            let top = rows.y0 + place as f64 * ROW_HEIGHT - self.subpages.scroll;
            if top > rows.y1 || top + ROW_HEIGHT < rows.y0 {
                continue;
            }
            let row = Rect::new(rows.x0, top, rows.x1, top + ROW_HEIGHT);
            self.regions.subrows.push((row, *subcode));
            if chosen == Some(place) {
                canvas.fill(row, 0.0, SELECTED);
            } else if self.subpages.hovered == Some(*subcode) {
                canvas.fill(row, 0.0, RAISED);
            }
            canvas.fill(Rect::new(row.x0, row.y1 - HAIRLINE, row.x1, row.y1), 0.0, RULE_SOFT);
            left_centred(canvas, row, PADDING, &format!("{}/{count}", place.saturating_add(1)), ROW_NUMBER, WHITE);
            left_fitted(canvas, Rect::new(row.x0 + TITLE_COLUMN, row.y0, row.x1 - PADDING, row.y1), PADDING, title, ROW_TITLE, WHITE);
        }
        canvas.pop_clip();
    }

    /// A press on the divider grips it; one on a sub-page opens its page
    /// there. Returns whether the press was the sub-pages'.
    pub(super) fn subpages_press(&mut self, point: Point, now: u64) -> bool {
        if self.regions.subpage_divider.contains(point) {
            self.subpages.grip = Some(point.y - self.regions.subpage_divider.y0);
            return true;
        }
        if !self.regions.subpage_list.contains(point) {
            return false;
        }
        let Some(&(_, subcode)) = self.regions.subrows.iter().find(|(rect, _)| rect.contains(point)) else { return true };
        let Some(index) = self.selected else { return true };
        let (Some(page), Some(key)) = (crate::channel::page_number(index), self.page_key(index)) else { return true };
        // Opened first, so the trusted core shows it at the sub-page asked.
        if self.wall.zoomed() != Some(key) {
            self.wall.zoom(key, now);
        }
        self.ask_control(PageControl::SubPageOf { page, subcode });
        true
    }

    /// The pointer moved: the divider follows it while gripped, and the
    /// sub-page under it is noted. Returns whether the window needs drawing.
    pub(super) fn subpages_moved(&mut self, point: Point) -> bool {
        if let Some(grip) = self.subpages.grip {
            let list = self.regions.list;
            let area = Rect::new(list.x0, list.y0, list.x1, self.regions.subpage_list.y1.max(list.y1));
            if area.height() > 0.0 {
                self.subpages.share = ((point.y - grip + DIVIDER_HEIGHT - area.y0) / area.height()).clamp(0.0, 1.0);
            }
            return true;
        }
        let hovered = self.regions.subrows.iter().find(|(rect, _)| rect.contains(point)).map(|&(_, subcode)| subcode);
        let changed = hovered != self.subpages.hovered;
        self.subpages.hovered = hovered;
        changed
    }

    /// The main button let go: the divider stays where it was left.
    pub(super) fn subpages_released(&mut self) {
        self.subpages.grip = None;
    }

    /// Whether the pointer is over the divider or pulling it.
    pub(super) fn over_subpage_divider(&self) -> bool {
        self.subpages.grip.is_some() || self.regions.subpage_divider.contains(self.pointer)
    }
}

#[cfg(test)]
mod tests {
    use crate::browser::Output;
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, middle, press_at};
    use crate::browser::pointer::Cursor;
    use crate::controls::PageControl;
    use crate::test_canvas::Recorder;

    fn titles() -> alloc::vec::Vec<(u16, alloc::string::String)> {
        alloc::vec![(1, "Story one".into()), (2, "Story two".into()), (3, "The comments".into())]
    }

    #[test]
    fn a_magazine_with_sub_pages_lists_the_chosen_pages_below_a_divider() {
        let (mut browser, mut canvas) = browser();
        let whole = browser.regions.list;
        assert_eq!(browser.regions.subpage_list, crate::geometry::Rect::ZERO, "no sub-pages, one list");
        browser.set_subpages(7, 0x100, titles());
        let mut canvas2 = Recorder::new();
        browser.draw(&mut canvas2, WIDTH, HEIGHT, 0);
        assert!(browser.regions.list.y1 < whole.y1, "the pages above");
        assert!(browser.regions.subpage_list.y0 > browser.regions.list.y1, "the sub-pages below");
        assert!(canvas2.texts.iter().any(|(_, text)| text == "Choose a page marked with sub-pages"));
        assert!(canvas2.texts.iter().any(|(_, text)| text == "3"), "page 100 marked with its three sub-pages");
        browser.page_updated(7, 0x100, 0);
        let mut updated = Recorder::new();
        browser.draw(&mut updated, WIDTH, HEIGHT, 0);
        let row = browser.regions.rows[0].0;
        let in_row = |text: &str| {
            updated.texts.iter().find(|(rect, drawn)| drawn.as_str() == text && rect.y0 >= row.y0 && rect.y1 <= row.y1 + 1.0).map(|(rect, _)| *rect)
        };
        let count = in_row("3").expect("the mark's count");
        let age =
            updated.texts.iter().find(|(rect, text)| text.starts_with("Updated") && rect.y0 >= row.y0 && rect.y1 <= row.y1 + 1.0).expect("the row's age").0;
        assert!(age.x1 < count.x0, "the age stands left of the mark, not over it: {age:?} and {count:?}");
        let row = browser.regions.rows[0].0;
        press_at(&mut browser, &mut canvas, row);
        let mut canvas3 = Recorder::new();
        browser.draw(&mut canvas3, WIDTH, HEIGHT, 0);
        assert!(canvas3.texts.iter().any(|(_, text)| text == "Sub-pages of 100 (3)"));
        assert_eq!(browser.regions.subrows.len(), 3);
        assert!(canvas3.texts.iter().any(|(_, text)| text == "The comments"));
        browser.take_outputs();
        let second = browser.regions.subrows[1].0;
        let point = middle(second);
        browser.pointer_moved(point.x, point.y, 0);
        assert_eq!(browser.cursor(), Cursor::Hand);
        browser.pointer_pressed(&mut canvas3, false, 0);
        assert!(browser.take_outputs().contains(&Output::PageControl(PageControl::SubPageOf { page: 0x100, subcode: 2 })));
        assert!(browser.wall.zoomed().is_some(), "its page opened, there");
        let divider = middle(browser.regions.subpage_divider);
        let above = browser.regions.list.y1;
        browser.pointer_moved(divider.x, divider.y, 0);
        assert_eq!(browser.cursor(), Cursor::ResizeRow);
        browser.pointer_pressed(&mut canvas3, false, 0);
        browser.pointer_moved(divider.x, divider.y + 80.0, 0);
        browser.pointer_released();
        browser.draw(&mut canvas3, WIDTH, HEIGHT, 0);
        assert!((browser.regions.list.y1 - (above + 80.0)).abs() < 1.0, "the divider pulled down: {} from {above}", browser.regions.list.y1);
        browser.set_subpages(7, 0x100, alloc::vec::Vec::new());
        browser.draw(&mut canvas3, WIDTH, HEIGHT, 0);
        assert_eq!(browser.regions.list.y1, whole.y1, "one list again");
    }
}
