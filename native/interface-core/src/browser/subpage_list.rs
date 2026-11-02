//! The sub-pages of the chosen page, under the page list (decision 111):
//! when any page of the magazine has sub-pages, the list is two, the pages
//! above and the chosen page's sub-pages below, with a divider between
//! them that can be pulled; a click on a sub-page opens its page there,
//! and gives the sub-pages the keys, Up and Down stepping through them as
//! they step through the pages of the page list, and Page Up and Page Down
//! moving by a list-height, which land by place because a place far off may
//! not be held yet.
//!
//! The lower list is virtual (decision 112 item 4): it knows each page's
//! count, draws a row for every place to the standard's 8,190, and holds
//! the titles of only the rows it last asked the trusted core for, at most
//! [`SUBPAGE_ROWS`], asking for more as it scrolls (decision 112 item 3).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::list_focus::ListFocus;
use crate::browser::page_list::{ROW_HEIGHT, rows_per_key};
use crate::browser::window::PADDING;
use crate::browser::{Browser, Output};
use crate::canvas::Canvas;
use crate::channel::page_index;
use crate::controls::{PageControl, SUBPAGE_ROWS};
use crate::geometry::{Point, Rect};
use crate::surface::Surface;
use crate::text::{HAIRLINE, left_centred, left_fitted};
use crate::theme::{COLUMN_HEAD, RAISED, ROW_NUMBER, ROW_TITLE, SELECTED};

/// The share of the list the pages take until the divider is pulled.
const PAGES_SHARE: f64 = 0.6;
/// The fewest rows either list keeps.
const FEWEST_ROWS: f64 = 3.0;
/// The divider: a bar the height of a hairline's grip, and how far either
/// side it can be gripped.
const DIVIDER_HEIGHT: f64 = 6.0;
/// The sub-pages' heading row.
const HEADING_HEIGHT: f64 = 26.0;
/// Where a sub-page's title starts: after its place, "8190/8190".
const TITLE_COLUMN: f64 = 84.0;
/// The rows asked for above the first in view, so a short scroll up needs
/// no new request.
const ROWS_ABOVE: u16 = 8;
// A run asked for holds the rows in view and those above them.
const _: () = assert!((ROWS_ABOVE as usize) < SUBPAGE_ROWS);
/// How far Up or Down moves the list.
const ARROW_ROWS: u16 = 1;

/// The rows of one page's sub-pages the list holds: from place `first`.
#[derive(Debug)]
struct Held {
    index: usize,
    first: u16,
    rows: Vec<(u16, String)>,
}

impl Held {
    /// The sub-code and title at `place`, when held.
    fn at(&self, index: usize, place: u16) -> Option<&(u16, String)> {
        if self.index != index {
            return None;
        }
        self.rows.get(usize::from(place.checked_sub(self.first)?))
    }
}

/// Where Page Up or Page Down moved the list (REQ-1758): a place, because a
/// row not held has no sub-code to ask for yet.
#[derive(Clone, Copy, Debug)]
pub(super) struct Landing {
    index: usize,
    place: u16,
    /// Its sub-page was asked for. A key held down moves on from the
    /// landing, not from the sub-page last shown.
    asked: bool,
}

/// The page list's sub-pages: each page's count, the rows held, the
/// divider and the scroll.
#[derive(Debug)]
pub(super) struct SubPages {
    /// Each page's count of sub-pages, two or more, by page index, for the
    /// channel `channel`.
    pub(super) counts: BTreeMap<usize, u16>,
    channel: u32,
    /// The rows held, of one page.
    held: Option<Held>,
    /// The rows last asked for, by page index and first place, until they
    /// are answered.
    asked: Option<(usize, u16)>,
    /// The pages' share of the list, as the divider was pulled.
    share: f64,
    /// Where the divider was gripped, from its top, while it is pulled.
    grip: Option<f64>,
    /// The sub-pages' scroll, in pixels.
    pub(super) scroll: f64,
    /// The sub-page row under the pointer, by its sub-code.
    pub(super) hovered: Option<u16>,
    /// The chosen place the list last scrolled to while it had the keys,
    /// so a step brings the new one into view and the wheel stays free.
    revealed: Option<u16>,
    /// Where Page Up or Page Down last moved the list, until that sub-page
    /// is the one chosen.
    pub(super) landing: Option<Landing>,
}

impl Default for SubPages {
    fn default() -> Self {
        Self {
            counts: BTreeMap::new(),
            channel: 0,
            held: None,
            asked: None,
            share: PAGES_SHARE,
            grip: None,
            scroll: 0.0,
            hovered: None,
            revealed: None,
            landing: None,
        }
    }
}

impl SubPages {
    /// Starts afresh for `channel` when it is another.
    fn for_channel(&mut self, channel: u32) {
        if channel != self.channel {
            self.counts.clear();
            self.held = None;
            self.asked = None;
            self.landing = None;
            self.channel = channel;
        }
    }
}

impl Browser {
    /// How many sub-pages a page has, as the trusted core says; 0 or 1
    /// when it has none (decision 111).
    pub fn set_subpage_count(&mut self, channel: u32, page: u16, count: u16) {
        self.subpages.for_channel(channel);
        let Some(index) = page_index(page) else { return };
        if count > 1 {
            self.subpages.counts.insert(index, count);
        } else {
            self.subpages.counts.remove(&index);
        }
        // The rows held may be of another numbering now: asked afresh.
        if self.subpages.held.as_ref().is_some_and(|held| held.index == index) {
            self.subpages.held = None;
        }
        if self.subpages.asked.is_some_and(|(asked, _)| asked == index) {
            self.subpages.asked = None;
        }
    }

    /// Rows of a page's sub-pages the list asked for, from place `first`,
    /// with the page's count; at most [`SUBPAGE_ROWS`] are kept.
    pub fn set_subpage_rows(&mut self, channel: u32, page: u16, count: u16, first: u16, mut rows: Vec<(u16, String)>) {
        self.subpages.for_channel(channel);
        let Some(index) = page_index(page) else { return };
        if self.subpages.asked.is_some_and(|(asked, _)| asked == index) {
            self.subpages.asked = None;
        }
        if count > 1 {
            self.subpages.counts.insert(index, count);
        } else {
            self.subpages.counts.remove(&index);
        }
        rows.truncate(SUBPAGE_ROWS);
        self.subpages.held = Some(Held { index, first, rows });
    }

    /// How many sub-pages page `index` has, when it has more than one.
    pub(super) fn subpage_count(&self, index: usize) -> Option<u16> {
        (self.subpages.channel == self.channel.channel).then(|| self.subpages.counts.get(&index).copied()).flatten()
    }

    /// Whether any page of the chosen magazine has sub-pages: the list is
    /// then two.
    pub(super) fn magazine_has_subpages(&self) -> bool {
        self.bank_pages().iter().any(|&index| self.subpage_count(index).is_some())
    }

    /// Where the divider lies in `area`, the list under its heading: the
    /// pages above it, the sub-pages below.
    pub(super) fn split_at(&self, area: Rect) -> f64 {
        let least = FEWEST_ROWS * ROW_HEIGHT;
        let most = (area.height() - least - HEADING_HEIGHT - DIVIDER_HEIGHT).max(least);
        area.y0 + (area.height() * self.subpages.share).clamp(least, most)
    }

    /// Draws the chosen page's sub-pages in `area`, under the divider: a
    /// row for each place in view, with its title once held, and asks for
    /// the rows in view that are not.
    pub(super) fn draw_subpage_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect, colour: crate::theme::Colour, surface: Surface, now: u64) {
        let divider = Rect::new(area.x0, area.y0, area.x1, area.y0 + DIVIDER_HEIGHT);
        canvas.fill(divider, 0.0, colour);
        self.regions.subpage_divider = Rect::new(divider.x0, divider.y0 - DIVIDER_HEIGHT, divider.x1, divider.y1 + DIVIDER_HEIGHT);
        let heading = Rect::new(area.x0, divider.y1, area.x1, divider.y1 + HEADING_HEIGHT);
        canvas.fill(heading, 0.0, surface.back);
        let selected = self.selected.and_then(|index| Some((index, self.subpage_count(index)?, crate::channel::page_number(index)?)));
        let words = match selected {
            Some((_, count, page)) => format!("Sub-pages of {page:03X} ({count})"),
            None => String::from("Choose a page marked with sub-pages"),
        };
        left_fitted(canvas, heading, PADDING, &words, COLUMN_HEAD, surface.muted);
        canvas.fill(Rect::new(heading.x0, heading.y1 - HAIRLINE, heading.x1, heading.y1), 0.0, surface.rule);
        let rows = Rect::new(area.x0, heading.y1, area.x1, area.y1);
        self.regions.subpage_list = rows;
        let Some((index, count, page)) = selected else { return };
        let chosen = self.chosen_place(page);
        if self.subpages.landing.is_some_and(|landing| landing.index != index || chosen == Some(landing.place)) {
            self.subpages.landing = None;
        }
        self.open_landing(now);
        // Stepped with the keys, the list follows the sub-page chosen, as
        // the page list follows the page, once what a key moved to is shown.
        if self.list_focus == Some(ListFocus::SubPages) && self.subpages.landing.is_none() && chosen != self.subpages.revealed {
            self.subpages.revealed = chosen;
            if let Some(place) = chosen {
                self.reveal_subpage(place, rows.height());
            }
        }
        let content = f64::from(count) * ROW_HEIGHT;
        self.subpages.scroll = self.subpages.scroll.min((content - rows.height()).max(0.0));
        let in_view = self.places_in_view(rows.height(), count);
        canvas.push_clip(rows);
        let mut missing = None;
        for place in in_view.clone() {
            let top = rows.y0 + f64::from(place) * ROW_HEIGHT - self.subpages.scroll;
            let row = Rect::new(rows.x0, top, rows.x1, top + ROW_HEIGHT);
            let held = self.subpages.held.as_ref().and_then(|held| held.at(index, place));
            // A row with a fill of its own keeps white text.
            let fill = if chosen == Some(place) {
                Some(SELECTED)
            } else if held.is_some_and(|(subcode, _)| self.subpages.hovered == Some(*subcode)) {
                Some(RAISED)
            } else {
                None
            };
            let on = if fill.is_some() { Surface::BROWSER } else { surface };
            if let Some(fill) = fill {
                canvas.fill(row, 0.0, fill);
            }
            canvas.fill(Rect::new(row.x0, row.y1 - HAIRLINE, row.x1, row.y1), 0.0, on.rule_soft);
            if let Some(fill) = fill {
                surface.edge(canvas, row, fill);
            }
            left_centred(canvas, row, PADDING, &format!("{}/{count}", u32::from(place).saturating_add(1)), ROW_NUMBER, on.ink);
            match held {
                Some((subcode, title)) => {
                    let (subcode, title) = (*subcode, title.clone());
                    self.regions.subrows.push((row, subcode));
                    left_fitted(canvas, Rect::new(row.x0 + TITLE_COLUMN, row.y0, row.x1 - PADDING, row.y1), PADDING, &title, ROW_TITLE, on.ink);
                }
                None => missing = missing.or(Some(place)),
            }
        }
        canvas.pop_clip();
        if let Some(place) = missing {
            self.ask_subpage_rows(index, page, place);
        }
    }

    /// The sub-page drawn of `page`, when it is the page open and one is
    /// chosen.
    fn chosen_place(&self, page: u16) -> Option<u16> {
        let controls = &self.page_controls.controls;
        (controls.page == Some(page)).then_some(controls.chosen).flatten()
    }

    /// Scrolls the list, `height` high, so that `place` is in view.
    fn reveal_subpage(&mut self, place: u16, height: f64) {
        let top = f64::from(place) * ROW_HEIGHT;
        if top < self.subpages.scroll {
            self.subpages.scroll = top;
        } else if top + ROW_HEIGHT > self.subpages.scroll + height {
            self.subpages.scroll = top + ROW_HEIGHT - height;
        }
    }

    /// Page Up or Page Down: moves the list one list-height less one row
    /// from where it is, staying on the first and the last row, and shows
    /// the sub-page there, as the arrows show the one they land on. A row
    /// not held is fetched as a scroll fetches it, and shown when it arrives
    /// (REQ-1758).
    pub(super) fn page_through_subpages(&mut self, forward: bool, now: u64) {
        let rows = u16::try_from(rows_per_key(self.regions.subpage_list.height())).unwrap_or(u16::MAX);
        self.move_through_subpages(forward, rows, now);
    }

    /// Up or Down with the keys in the list: moves it one row as
    /// [`Browser::page_through_subpages`] moves it further, from the list's
    /// own rows, so that it does not wait on what the open page's controls
    /// count and stays on the end rows as the page list does (REQ-1758).
    pub(super) fn step_subpages(&mut self, forward: bool, now: u64) {
        self.move_through_subpages(forward, ARROW_ROWS, now);
    }

    /// Moves the list `rows` rows on or back and shows the sub-page there.
    fn move_through_subpages(&mut self, forward: bool, rows: u16, now: u64) {
        let Some(index) = self.selected else { return };
        let (Some(count), Some(page)) = (self.subpage_count(index), crate::channel::page_number(index)) else { return };
        let height = self.regions.subpage_list.height();
        let landed = self.subpages.landing.filter(|landing| landing.index == index).map(|landing| landing.place);
        let top = self.places_in_view(height, count).start;
        let from = landed.or_else(|| self.chosen_place(page)).unwrap_or(top);
        let place = if forward { from.saturating_add(rows).min(count.saturating_sub(1)) } else { from.saturating_sub(rows) };
        self.reveal_subpage(place, height);
        self.subpages.landing = Some(Landing { index, place, asked: false });
        self.open_landing(now);
    }

    /// Shows the sub-page the list moved to, once its row is held.
    fn open_landing(&mut self, now: u64) {
        let Some(landing) = self.subpages.landing.filter(|landing| !landing.asked) else { return };
        let Some(subcode) = self.subpages.held.as_ref().and_then(|held| held.at(landing.index, landing.place)).map(|&(subcode, _)| subcode) else { return };
        self.subpages.landing = Some(Landing { asked: true, ..landing });
        self.open_subpage(landing.index, subcode, now);
    }

    /// Opens sub-code `subcode` of page `index`, opening the page first so
    /// that the trusted core shows it at the sub-page asked.
    fn open_subpage(&mut self, index: usize, subcode: u16, now: u64) {
        let (Some(page), Some(key)) = (crate::channel::page_number(index), self.page_key(index)) else { return };
        if self.wall.zoomed() != Some(key) {
            self.wall.zoom(key, now);
        }
        self.ask_control(PageControl::SubPageOf { page, subcode });
    }

    /// The places of the rows in view in a list `height` high of `count`
    /// sub-pages, at the list's scroll.
    fn places_in_view(&self, height: f64, count: u16) -> core::ops::Range<u16> {
        // Places are whole rows from the top, within the count, so the
        // conversions cannot lose anything that matters.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to the count first")]
        let place = |pixels: f64| (pixels / ROW_HEIGHT).clamp(0.0, f64::from(count)) as u16;
        place(self.subpages.scroll)..place(self.subpages.scroll + height + ROW_HEIGHT)
    }

    /// Asks for the rows of page `index`, numbered `page`, from a little
    /// above `place`, the first in view not held: once, until answered.
    fn ask_subpage_rows(&mut self, index: usize, page: u16, place: u16) {
        let first = place.saturating_sub(ROWS_ABOVE);
        if self.subpages.asked == Some((index, first)) {
            return;
        }
        self.subpages.asked = Some((index, first));
        let count = u16::try_from(SUBPAGE_ROWS).unwrap_or(u16::MAX);
        self.outputs.push(Output::SubPageRows { page, first, count });
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
        // The sub-pages then have the keys: Up and Down step through them.
        self.list_focus = Some(ListFocus::SubPages);
        self.subpages.revealed = None;
        let Some(&(_, subcode)) = self.regions.subrows.iter().find(|(rect, _)| rect.contains(point)) else { return true };
        let Some(index) = self.selected else { return true };
        self.subpages.landing = None;
        self.open_subpage(index, subcode, now);
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
    #![allow(clippy::arithmetic_side_effects, reason = "a test")]
    use crate::browser::fixture::{HEIGHT, WIDTH, browser, middle, press_at};
    use crate::browser::page_list::rows_per_key;
    use crate::browser::pointer::Cursor;
    use crate::browser::{Key, Modifiers, Output};
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
        browser.set_subpage_count(7, 0x100, 3);
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
        browser.take_outputs();
        browser.draw(&mut canvas3, WIDTH, HEIGHT, 0);
        assert!(canvas3.texts.iter().any(|(_, text)| text == "Sub-pages of 100 (3)"));
        assert_eq!(browser.take_outputs(), [Output::SubPageRows { page: 0x100, first: 0, count: 64 }], "the rows in view asked for");
        browser.set_subpage_rows(7, 0x100, 3, 0, titles());
        browser.draw(&mut canvas3, WIDTH, HEIGHT, 0);
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
        browser.set_subpage_count(7, 0x100, 0);
        browser.draw(&mut canvas3, WIDTH, HEIGHT, 0);
        assert_eq!(browser.regions.list.y1, whole.y1, "one list again");
    }

    /// Decision 112 item 4: a page of the standard's 8,190 sub-pages is
    /// listed as a virtual list: a row for every place, the titles of only
    /// the rows in view, asked for once as they scroll into view.
    #[test]
    fn the_sub_page_list_reaches_every_one_of_8190_rows_by_range() {
        let (mut browser, mut canvas) = browser();
        browser.set_subpage_count(7, 0x100, 8_190);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let row = browser.regions.rows[0].0;
        press_at(&mut browser, &mut canvas, row);
        browser.take_outputs();
        browser.subpages.scroll = f64::MAX;
        let mut drawn = Recorder::new();
        browser.draw(&mut drawn, WIDTH, HEIGHT, 0);
        assert!(drawn.texts.iter().any(|(_, text)| text == "Sub-pages of 100 (8190)"));
        assert!(drawn.texts.iter().any(|(_, text)| text == "8190/8190"), "the last row reached");
        let asked = browser.take_outputs();
        let [Output::SubPageRows { page: 0x100, first, count: 64 }] = asked[..] else { panic!("one run asked for: {asked:?}") };
        assert!(first > 8_000 && first < 8_190, "near the end: {first}");
        browser.draw(&mut drawn, WIDTH, HEIGHT, 0);
        assert_eq!(browser.take_outputs(), [], "asked once until answered");
        let rows: alloc::vec::Vec<(u16, alloc::string::String)> = (first..8_190).take(64).map(|place| (place, alloc::format!("Story {place}"))).collect();
        browser.set_subpage_rows(7, 0x100, 8_190, first, rows);
        let mut answered = Recorder::new();
        browser.draw(&mut answered, WIDTH, HEIGHT, 0);
        assert!(answered.texts.iter().any(|(_, text)| text == "Story 8189"));
        assert!(!browser.regions.subrows.is_empty() && browser.regions.subrows.len() <= 64, "only the rows in view: {}", browser.regions.subrows.len());
        assert_eq!(browser.take_outputs(), [], "every row in view held");
    }

    /// A browser whose chosen page 100 has `count` sub-pages, listed from
    /// sub-code 1, with the first run of rows held and the list's keys.
    fn listed(count: u16) -> (crate::browser::Browser, Recorder) {
        let (mut browser, mut canvas) = browser();
        browser.set_subpage_count(7, 0x100, count);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let row = browser.regions.rows[0].0;
        press_at(&mut browser, &mut canvas, row);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        browser.set_subpage_rows(7, 0x100, count, 0, held(0, count.min(64)));
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let list = browser.regions.subpage_list;
        browser.pointer_moved(list.x0 + 1.0, list.y1 - 1.0, 0);
        browser.pointer_pressed(&mut canvas, false, 0);
        browser.take_outputs();
        (browser, canvas)
    }

    /// The rows from place `first`, each named by its sub-code, one more
    /// than its place.
    fn held(first: u16, count: u16) -> alloc::vec::Vec<(u16, alloc::string::String)> {
        (first..first + count).map(|place| (place + 1, alloc::format!("Story {place}"))).collect()
    }

    /// The trusted core shows place `chosen` of the list, as it does when
    /// a sub-page opens.
    fn chosen(browser: &mut crate::browser::Browser, count: u16, chosen: u16) {
        browser.set_page_controls(crate::controls::PageControls { page: Some(0x100), subpages: count, chosen: Some(chosen), ..Default::default() });
    }

    /// The sub-page a key asked to be shown.
    fn asked(browser: &mut crate::browser::Browser) -> Option<u16> {
        browser.take_outputs().iter().find_map(|output| match output {
            Output::PageControl(PageControl::SubPageOf { page: 0x100, subcode }) => Some(*subcode),
            _ => None,
        })
    }

    fn page_key(browser: &mut crate::browser::Browser, key: Key<'_>, canvas: &mut Recorder) {
        browser.key(key, Modifiers::default(), 0);
        browser.draw(canvas, WIDTH, HEIGHT, 0);
    }

    /// REQ-1758 items 1 and 2: Page Down shows the sub-page one list-height
    /// less one row on, scrolled into view, and Page Up returns.
    #[test]
    fn page_down_moves_the_sub_pages_a_list_height_and_page_up_returns() {
        let (mut browser, mut canvas) = listed(60);
        chosen(&mut browser, 60, 0);
        let rows = rows_per_key(browser.regions.subpage_list.height());
        assert!(rows < 60, "the list is shorter than its rows: {rows}");
        let landed = u16::try_from(rows).unwrap();
        browser.key(Key::PageDown, Modifiers::default(), 0);
        assert_eq!(asked(&mut browser), Some(landed + 1), "the row a list-height less one row on, by its sub-code");
        chosen(&mut browser, 60, landed);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert!(browser.regions.subrows.iter().any(|&(_, subcode)| subcode == landed + 1), "scrolled into view");
        browser.key(Key::PageUp, Modifiers::default(), 0);
        assert_eq!(asked(&mut browser), Some(1), "and back");
    }

    /// REQ-1758 item 3: neither key wraps round.
    #[test]
    fn the_keys_stay_on_the_end_rows_of_the_sub_pages() {
        let (mut browser, mut canvas) = listed(40);
        chosen(&mut browser, 40, 0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        page_key(&mut browser, Key::PageUp, &mut canvas);
        assert_eq!(asked(&mut browser), Some(1), "stays on the first row");
        chosen(&mut browser, 40, 38);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        page_key(&mut browser, Key::PageDown, &mut canvas);
        assert_eq!(asked(&mut browser), Some(40), "goes to the last row");
        chosen(&mut browser, 40, 39);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        page_key(&mut browser, Key::PageDown, &mut canvas);
        assert_eq!(asked(&mut browser), Some(40), "and stays there");
    }

    /// REQ-1758 item 6: Up and Down move the list a row, by place, and stay
    /// on the end rows, as Page Up and Page Down do and as the page list does.
    #[test]
    fn up_and_down_move_the_sub_pages_a_row_and_stay_on_the_end_rows() {
        let (mut browser, mut canvas) = listed(40);
        chosen(&mut browser, 40, 0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        page_key(&mut browser, Key::Down, &mut canvas);
        assert_eq!(asked(&mut browser), Some(2), "the next row, by its sub-code");
        chosen(&mut browser, 40, 1);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        page_key(&mut browser, Key::Up, &mut canvas);
        assert_eq!(asked(&mut browser), Some(1), "and back");
        chosen(&mut browser, 40, 0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        page_key(&mut browser, Key::Up, &mut canvas);
        assert_eq!(asked(&mut browser), Some(1), "stays on the first row");
        chosen(&mut browser, 40, 39);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        page_key(&mut browser, Key::Down, &mut canvas);
        assert_eq!(asked(&mut browser), Some(40), "and on the last");
    }

    /// REQ-1758 item 6: the list's own rows decide, not the page's controls,
    /// which may not yet count the sub-pages the list shows.
    #[test]
    fn up_and_down_follow_the_list_whatever_the_controls_count() {
        let (mut browser, mut canvas) = listed(40);
        chosen(&mut browser, 1, 0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        page_key(&mut browser, Key::Down, &mut canvas);
        assert_eq!(asked(&mut browser), Some(2));
    }

    /// REQ-1758 item 4: the distance follows the height the divider left.
    #[test]
    fn the_distance_follows_the_divider() {
        let (mut browser, mut canvas) = listed(60);
        chosen(&mut browser, 60, 0);
        let before = rows_per_key(browser.regions.subpage_list.height());
        let divider = middle(browser.regions.subpage_divider);
        browser.pointer_moved(divider.x, divider.y, 0);
        browser.pointer_pressed(&mut canvas, false, 0);
        browser.pointer_moved(divider.x, divider.y - 135.0, 0);
        browser.pointer_released();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let after = rows_per_key(browser.regions.subpage_list.height());
        assert!(after > before, "the list is taller: {after} rows from {before}");
        let first = browser.regions.subrows[0].0;
        press_at(&mut browser, &mut canvas, first);
        browser.take_outputs();
        browser.key(Key::PageDown, Modifiers::default(), 0);
        assert_eq!(asked(&mut browser), Some(u16::try_from(after).unwrap() + 1), "by the new height");
    }

    /// REQ-1758 item 5: a key that lands past the rows held asks for the
    /// part it landed in, and shows the sub-page when it arrives. A key held
    /// down moves on from the landing, not from the sub-page last shown.
    #[test]
    fn page_down_past_the_fetched_rows_asks_for_the_part_it_landed_in() {
        let (mut browser, mut canvas) = listed(1_000);
        chosen(&mut browser, 1_000, 0);
        let rows = u16::try_from(rows_per_key(browser.regions.subpage_list.height())).unwrap();
        let presses = 64 / rows + 1;
        for _ in 0..presses {
            page_key(&mut browser, Key::PageDown, &mut canvas);
        }
        let place = rows * presses;
        assert!(place >= 64, "past the part held: {place}");
        let outputs = browser.take_outputs();
        let first = outputs.iter().rev().find_map(|output| match output {
            Output::SubPageRows { page: 0x100, first, count: 64 } => Some(*first),
            _ => None,
        });
        let first = first.expect("the part asked for");
        assert!(first <= place && place < first + 64, "the part holds the row landed on: {first} and {place}");
        browser.set_subpage_rows(7, 0x100, 1_000, first, held(first, 64));
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        assert_eq!(asked(&mut browser), Some(place + 1), "shown once it arrived");
    }
}
