//! The tab strip (`tab-strip.md`, decision 69): tabs at a width that fits,
//! the active one always shown; when they overflow, a count, and a panel
//! joined under the strip into which the rest wrap while the pointer is on
//! either; and a tab dragged along to a new place, which the trusted core
//! decides.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{
    BANK_BORDER, Browser, GLYPH, NEW_TAB, Output, PRIVATE_MARK, STRIP_HEIGHT, TAB_CLOSE, TAB_COLOURS, TAB_GAP, TAB_MAX_WIDTH, TAB_MIN_WIDTH, TAB_SPEAKER,
    TAB_TEXT_INSET, TAB_TOP, TAB_TOP_INACTIVE, TabRegion, speaker_glyph,
};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::theme::{BLACK, BUTTON, PRIVATE, ROW_STATUS, RULE, SELECTED, TAB_TITLE, WHITE};
use crate::ui::{HAIRLINE, border, centred, left_centred, outline};

/// How far a tab must move with the button held before it is dragged, not
/// chosen: about a finger's tremble.
const DRAG_THRESHOLD: f64 = 4.0;
/// How far below the strip a dragged tab is let go before it leaves the
/// window rather than staying on the strip: a little slack for a hand that
/// drifts while dragging along it.
const TEAR_DISTANCE: f64 = 24.0;
/// The count of tabs that do not fit: "+12".
const COUNT_WIDTH: f64 = 40.0;
/// A row of the overflow panel: a tab's height and the gap below it.
const PANEL_ROW: f64 = STRIP_HEIGHT - TAB_TOP_INACTIVE + TAB_GAP;
/// The line that shows where a dragged tab will go.
const MARKER_WIDTH: f64 = 3.0;

/// A tab being pressed or dragged.
#[derive(Clone, Copy, Debug)]
struct Drag {
    /// The tab's place.
    index: usize,
    /// Where the press was.
    start: Point,
    /// Moved past the threshold: dragged, not chosen.
    moving: bool,
    /// Where it would go if released now.
    target: usize,
}

/// The strip's state in the browser.
#[derive(Debug, Default)]
pub(super) struct TabsUi {
    /// The pointer is on the strip or the overflow panel, which then shows.
    open: bool,
    /// The count was pressed: the panel stays until the pointer leaves.
    pinned: bool,
    drag: Option<Drag>,
    /// Where the panel and the count were last drawn.
    panel: Rect,
    count: Rect,
    /// Some tabs did not fit on the strip.
    overflowing: bool,
    /// Those tabs, the strip's tab width, and its left and right ends, for
    /// the panel drawn at the end of the frame.
    rest: (Vec<usize>, f64, f64, f64),
}

/// A coordinate in whole pixels, held within `i32`.
fn whole(value: f64) -> i32 {
    let rounded = libm::round(value).clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[expect(clippy::cast_possible_truncation, reason = "rounded and held within i32 above")]
    let whole = rounded as i32;
    whole
}

/// Which tabs the strip shows, in order, and which wrap into the panel: the
/// first that fit, the active one taking the last place when it would not.
fn arrangement(count: usize, active: usize, fit: usize) -> (Vec<usize>, Vec<usize>) {
    if count <= fit {
        return ((0..count).collect(), Vec::new());
    }
    let mut shown: Vec<usize> = (0..fit).collect();
    if active >= fit
        && let Some(last) = shown.last_mut()
    {
        *last = active;
    }
    let rest = (0..count).filter(|index| !shown.contains(index)).collect();
    (shown, rest)
}

impl Browser {
    /// Draws the strip's tabs between `left` and `right`, the new-tab
    /// button and the count; the panel is drawn last, over everything.
    pub(super) fn draw_tabs<C: Canvas + ?Sized>(&mut self, canvas: &mut C, left: f64, right: f64) {
        let room = right - left - NEW_TAB - TAB_GAP * 2.0;
        #[allow(clippy::cast_precision_loss, reason = "a few tabs")]
        let count = self.tabs.len().max(1) as f64;
        let all_fit = count * (TAB_MIN_WIDTH + TAB_GAP) - TAB_GAP <= room;
        let (tab_width, fit) = if all_fit {
            (((room - TAB_GAP * (count - 1.0)) / count).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH), self.tabs.len())
        } else {
            // Each tab and its gap, with the count's room kept.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a count of tabs, at least one")]
            let fit = libm::floor((room - COUNT_WIDTH) / (TAB_MIN_WIDTH + TAB_GAP)).max(1.0) as usize;
            (TAB_MIN_WIDTH, fit)
        };
        let (shown, rest) = arrangement(self.tabs.len(), self.active, fit);
        self.tab_strip.overflowing = !rest.is_empty();
        let mut x = left;
        for &index in &shown {
            let region = self.draw_tab(canvas, index, Rect::new(x, 0.0, x + tab_width, STRIP_HEIGHT));
            self.regions.tabs.push(region);
            x += tab_width + TAB_GAP;
        }
        self.tab_strip.count = Rect::ZERO;
        if !rest.is_empty() {
            let count = Rect::new(x, TAB_TOP_INACTIVE, x + COUNT_WIDTH, STRIP_HEIGHT - TAB_GAP);
            canvas.fill(count, 0.0, if self.tab_strip.open { SELECTED } else { BUTTON });
            outline(canvas, count, RULE);
            centred(canvas, count, &format!("+{}", rest.len()), ROW_STATUS, WHITE);
            self.tab_strip.count = count;
            x = count.x1 + TAB_GAP;
        }
        self.regions.new_tab = Rect::new(x + TAB_GAP, TAB_TOP_INACTIVE, x + TAB_GAP + NEW_TAB, STRIP_HEIGHT - TAB_GAP);
        canvas.fill(self.regions.new_tab, 0.0, BUTTON);
        outline(canvas, self.regions.new_tab, RULE);
        centred(canvas, self.regions.new_tab, "+", GLYPH, WHITE);
        self.tab_strip.rest = (rest, tab_width, left, right);
    }

    /// One tab in its slot: the active one standing taller, with its sound
    /// and close buttons.
    fn draw_tab<C: Canvas + ?Sized>(&self, canvas: &mut C, index: usize, slot: Rect) -> TabRegion {
        let active = index == self.active;
        let rect = Rect::new(slot.x0, slot.y0 + if active { TAB_TOP } else { TAB_TOP_INACTIVE }, slot.x1, slot.y1);
        let Some(tab) = self.tabs.get(index) else { return TabRegion { index, tab: rect, close: Rect::ZERO, speaker: None } };
        let (fill, ink) = TAB_COLOURS[usize::from(tab.colour) % TAB_COLOURS.len()];
        canvas.fill(rect, 0.0, fill);
        if active {
            border(canvas, Rect::new(rect.x0, rect.y0, rect.x1, rect.y1 + BANK_BORDER), BANK_BORDER - 1.0, WHITE);
        }
        let close = Rect::new(rect.x1 - TAB_CLOSE - TAB_GAP * 2.0, rect.y0, rect.x1 - TAB_GAP, rect.y1);
        let speaker = (tab.muted || self.sounding.contains(&tab.channel)).then(|| Rect::new(close.x0 - TAB_SPEAKER, rect.y0, close.x0, rect.y1));
        canvas.push_clip(Rect::new(rect.x0, rect.y0, speaker.map_or(close.x0, |speaker| speaker.x0), rect.y1));
        // A private tab's mark, before its title (requirement 20.8).
        let inset = if tab.private {
            let middle = f64::midpoint(rect.y0, rect.y1);
            let mark = Rect::new(rect.x0 + TAB_TEXT_INSET, middle - PRIVATE_MARK / 2.0, rect.x0 + TAB_TEXT_INSET + PRIVATE_MARK, middle + PRIVATE_MARK / 2.0);
            canvas.fill(mark, PRIVATE_MARK / 2.0, PRIVATE);
            TAB_TEXT_INSET + PRIVATE_MARK + TAB_GAP * 2.0
        } else {
            TAB_TEXT_INSET
        };
        left_centred(canvas, rect, inset, &tab.title, TAB_TITLE, ink);
        canvas.pop_clip();
        if let Some(speaker) = speaker {
            speaker_glyph(canvas, speaker, tab.muted, ink);
        }
        centred(canvas, close, "\u{00D7}", GLYPH, ink);
        TabRegion { index, tab: rect, close, speaker }
    }

    /// The overflow panel, joined under the strip, while the pointer is on
    /// either; and a dragged tab where the pointer has it. Drawn over the
    /// toolbar and the page area, as the menu's list is.
    pub(super) fn draw_tab_overflow<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64) {
        self.tab_strip.panel = Rect::ZERO;
        let (rest, tab_width, left, right) = core::mem::take(&mut self.tab_strip.rest);
        if self.tab_strip.open && !rest.is_empty() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a count of tabs, at least one")]
            let per_row = libm::floor((right - left + TAB_GAP) / (tab_width + TAB_GAP)).max(1.0) as usize;
            let rows = rest.len().div_ceil(per_row);
            #[allow(clippy::cast_precision_loss, reason = "a few rows")]
            let panel = Rect::new(0.0, STRIP_HEIGHT - HAIRLINE, width, STRIP_HEIGHT + rows as f64 * PANEL_ROW + TAB_GAP);
            // The strip's own colour, with no seam: the strip grown down.
            canvas.fill(panel, 0.0, BLACK);
            canvas.fill(Rect::new(0.0, panel.y1 - HAIRLINE, width, panel.y1), 0.0, RULE);
            for (place, &index) in rest.iter().enumerate() {
                #[allow(clippy::cast_precision_loss, reason = "a few tabs")]
                let (row, column) = ((place / per_row) as f64, (place % per_row) as f64);
                let x = left + column * (tab_width + TAB_GAP);
                let top = STRIP_HEIGHT + row * PANEL_ROW - TAB_TOP_INACTIVE + TAB_GAP;
                let region = self.draw_tab(canvas, index, Rect::new(x, top, x + tab_width, top + STRIP_HEIGHT));
                self.regions.tabs.push(region);
            }
            self.tab_strip.panel = panel;
        }
        // A dragged tab follows the pointer; a line shows where it will go.
        if let Some(drag) = self.tab_strip.drag.filter(|drag| drag.moving)
            && let Some(target) = self.regions.tabs.iter().find(|region| region.index == drag.target).map(|region| region.tab)
        {
            let x = if drag.target > drag.index { target.x1 + TAB_GAP / 2.0 } else { target.x0 - TAB_GAP / 2.0 };
            canvas.fill(Rect::new(x - MARKER_WIDTH / 2.0, target.y0, x + MARKER_WIDTH / 2.0, target.y1), 0.0, WHITE);
            if let Some(source) = self.regions.tabs.iter().find(|region| region.index == drag.index).map(|region| region.tab) {
                let dx = self.pointer.x - drag.start.x;
                let dy = self.pointer.y - drag.start.y;
                let ghost = Rect::new(source.x0 + dx, source.y0 + dy, source.x1 + dx, source.y1 + dy);
                let _ = self.draw_tab(canvas, drag.index, Rect::new(ghost.x0, ghost.y0 - TAB_TOP, ghost.x1, ghost.y1));
            }
        }
    }

    /// A press on the strip's tabs, its count or the overflow panel;
    /// returns whether it was there. A tab is chosen at once and may then
    /// be dragged.
    pub(super) fn tabs_press(&mut self, point: Point) -> bool {
        if self.tab_strip.count.contains(point) {
            self.tab_strip.pinned = !self.tab_strip.pinned;
            self.tab_strip.open = self.tab_strip.pinned || self.tab_strip.open;
            return true;
        }
        let Some(region) = self.regions.tabs.iter().rev().find(|region| region.tab.contains(point)).copied() else {
            return self.tab_strip.panel.contains(point);
        };
        if region.close.contains(point) {
            self.outputs.push(Output::CloseTab(region.index));
        } else if region.speaker.is_some_and(|speaker| speaker.contains(point)) {
            self.outputs.push(Output::MuteTab(region.index));
        } else {
            self.outputs.push(Output::SelectTab(region.index));
            self.tab_strip.drag = Some(Drag { index: region.index, start: point, moving: false, target: region.index });
        }
        true
    }

    /// The pointer moved: the panel opens or goes, and a pressed tab is
    /// dragged. Returns whether anything changed.
    pub(super) fn tabs_moved(&mut self, point: Point) -> bool {
        let strip = Rect::new(0.0, 0.0, self.regions.strip.x1, STRIP_HEIGHT);
        let on = strip.contains(point) || self.tab_strip.panel.contains(point);
        let open = self.tab_strip.overflowing && (on || self.tab_strip.drag.is_some_and(|drag| drag.moving));
        let mut changed = open != self.tab_strip.open;
        self.tab_strip.open = open;
        if !on {
            self.tab_strip.pinned = false;
        }
        if let Some(mut drag) = self.tab_strip.drag {
            if !drag.moving && libm::hypot(point.x - drag.start.x, point.y - drag.start.y) > DRAG_THRESHOLD {
                drag.moving = true;
            }
            if drag.moving {
                if let Some(region) = self.regions.tabs.iter().rev().find(|region| region.tab.contains(point)) {
                    drag.target = region.index;
                }
                changed = true;
            }
            self.tab_strip.drag = Some(drag);
        }
        changed
    }

    /// The button was released: a dragged tab goes where it was let go.
    pub(super) fn tabs_released(&mut self) {
        let Some(drag) = self.tab_strip.drag.take() else { return };
        // Let go off the strip and the panel, below them or outside the
        // window: the tab leaves this window (`multiple-windows.md`).
        let point = self.pointer;
        let strip = Rect::new(0.0, 0.0, self.regions.strip.x1, STRIP_HEIGHT + TEAR_DISTANCE);
        if drag.moving && !strip.contains(point) && !self.tab_strip.panel.contains(point) {
            self.outputs.push(Output::TabOut { index: drag.index, x: whole(point.x), y: whole(point.y) });
            return;
        }
        if drag.moving && drag.target != drag.index {
            self.outputs.push(Output::MoveTab { from: drag.index, to: drag.target });
        }
    }

    /// Where a tab let go at `point` of this window, in logical pixels,
    /// would go on its strip: before the tab there, or at the end; none
    /// off the strip.
    #[must_use]
    pub fn tab_place_at(&self, point: Point) -> Option<usize> {
        if !self.regions.strip.contains(point) {
            return None;
        }
        let shown = self.regions.tabs.iter().find(|region| region.tab.contains(point)).map(|region| region.index);
        Some(shown.unwrap_or(self.tabs.len()))
    }

    /// Escape while dragging: the tab stays where it was.
    pub(super) fn tabs_cancel(&mut self) -> bool {
        self.tab_strip.drag.take().is_some_and(|drag| drag.moving)
    }

    /// The count's tip.
    pub(super) fn tabs_tips(&self, tips: &mut Vec<(Rect, String)>) {
        if self.tab_strip.count != Rect::ZERO {
            tips.push((self.tab_strip.count, String::from("More tabs: rest the pointer on the strip to see them all")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// Decision 69: the tabs that fit, the active one always among them,
    /// the rest in order for the panel.
    #[test]
    fn the_active_tab_is_always_shown() {
        assert_eq!(arrangement(3, 0, 5), (vec![0, 1, 2], vec![]));
        assert_eq!(arrangement(8, 1, 4), (vec![0, 1, 2, 3], vec![4, 5, 6, 7]));
        assert_eq!(arrangement(8, 6, 4), (vec![0, 1, 2, 6], vec![3, 4, 5, 7]), "the active tab takes the last place");
    }

    use crate::browser::{Frame, Key, Modifiers, TabInfo};
    use crate::test_canvas::Recorder;

    fn browser_with(count: usize, active: usize) -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        let tabs =
            (0..count).map(|index| TabInfo { title: format!("Tab {index}"), colour: 0, channel: u32::try_from(index).unwrap(), muted: false, private: false });
        browser.set_tabs(tabs.collect(), active);
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser
    }

    fn middle(rect: Rect) -> Point {
        Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))
    }

    /// Decision 69: too many tabs show a count; rolling onto the strip
    /// opens the panel the rest wrap into, every one reachable; leaving
    /// closes it.
    #[test]
    fn overflowing_tabs_wrap_into_a_panel_under_the_strip() {
        let mut browser = browser_with(30, 25);
        let shown = browser.regions.tabs.len();
        assert!(shown < 30 && browser.tab_strip.count != Rect::ZERO, "a count of the rest");
        assert!(browser.regions.tabs.iter().any(|region| region.index == 25), "the active tab is shown");
        let strip_point = Point::new(5.0, 20.0);
        assert!(browser.pointer_moved(strip_point.x, strip_point.y, 0), "the panel opens");
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let mut indices: Vec<usize> = browser.regions.tabs.iter().map(|region| region.index).collect();
        indices.sort_unstable();
        assert_eq!(indices, (0..30).collect::<Vec<_>>(), "every tab, on the strip or in the panel");
        let in_panel = *browser.regions.tabs.last().unwrap();
        assert!(in_panel.tab.y0 >= STRIP_HEIGHT - TAB_TOP_INACTIVE, "below the strip");
        browser.pointer_moved(middle(in_panel.tab).x, middle(in_panel.tab).y, 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        assert_eq!(browser.take_outputs(), [Output::SelectTab(in_panel.index)], "a tab in the panel is chosen");
        browser.pointer_released();
        browser.pointer_moved(700.0, 600.0, 0);
        assert!(!browser.tab_strip.open, "leaving closes the panel");
    }

    /// A tab dragged along the strip asks to move there; one barely moved
    /// is only chosen; Escape cancels a drag.
    #[test]
    fn a_dragged_tab_moves_where_it_is_let_go() {
        let mut browser = browser_with(4, 0);
        let first = browser.regions.tabs[0].tab;
        let third = browser.regions.tabs[2].tab;
        browser.pointer_moved(middle(first).x - 20.0, middle(first).y, 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        browser.pointer_moved(middle(third).x, middle(third).y, 0);
        browser.pointer_released();
        assert_eq!(browser.take_outputs(), [Output::SelectTab(0), Output::MoveTab { from: 0, to: 2 }]);
        browser.pointer_moved(middle(first).x - 20.0, middle(first).y, 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        browser.pointer_moved(middle(first).x - 18.0, middle(first).y, 0);
        browser.pointer_released();
        assert_eq!(browser.take_outputs(), [Output::SelectTab(0)], "barely moved: chosen, not dragged");
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        browser.pointer_moved(middle(third).x, middle(third).y, 0);
        browser.key(Key::Escape, Modifiers::default(), 0);
        browser.pointer_released();
        assert_eq!(browser.take_outputs(), [Output::SelectTab(0)], "Escape: it stays");
    }

    /// `multiple-windows.md`: a tab dragged off the strip, below it or out
    /// of the window, leaves; let go just below the strip, it stays on it.
    /// Another window says where on its strip a tab let go would go.
    #[test]
    fn a_tab_dragged_off_the_strip_leaves_the_window() {
        let mut browser = browser_with(4, 0);
        let first = browser.regions.tabs[0].tab;
        browser.pointer_moved(middle(first).x, middle(first).y, 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        browser.pointer_moved(middle(first).x + 30.0, 300.0, 0);
        browser.pointer_released();
        assert_eq!(browser.take_outputs(), [Output::SelectTab(0), Output::TabOut { index: 0, x: whole(middle(first).x + 30.0), y: 300 }]);
        browser.pointer_moved(middle(first).x, middle(first).y, 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        browser.pointer_moved(-200.0, -50.0, 0);
        browser.pointer_released();
        assert_eq!(browser.take_outputs(), [Output::SelectTab(0), Output::TabOut { index: 0, x: -200, y: -50 }], "out of the window");
        browser.pointer_moved(middle(first).x, middle(first).y, 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        browser.pointer_moved(middle(first).x + 30.0, STRIP_HEIGHT + TEAR_DISTANCE / 2.0, 0);
        browser.pointer_released();
        assert_eq!(browser.take_outputs(), [Output::SelectTab(0)], "within the slack: still the strip's");
        let third = browser.regions.tabs[2].tab;
        assert_eq!(browser.tab_place_at(middle(third)), Some(2));
        assert_eq!(browser.tab_place_at(Point::new(third.x1 + 600.0, middle(third).y)), Some(4), "past the last tab: the end");
        assert_eq!(browser.tab_place_at(Point::new(middle(third).x, 400.0)), None);
        browser.key(Key::Text("n"), Modifiers { command: true, ..Modifiers::default() }, 0);
        assert_eq!(browser.take_outputs(), [Output::NewWindow { private: false }]);
    }

    /// The tab keys of decision 69.
    #[test]
    fn tab_keys_choose_tabs() {
        let mut browser = browser_with(12, 3);
        let command = Modifiers { command: true, ..Modifiers::default() };
        browser.key(Key::Text("2"), command, 0);
        browser.key(Key::Text("9"), command, 0);
        browser.key(Key::Right, Modifiers { alt: true, ..command }, 0);
        browser.key(Key::Left, Modifiers { alt: true, ..command }, 0);
        assert_eq!(browser.take_outputs(), [Output::SelectTab(1), Output::SelectTab(11), Output::SelectTab(4), Output::SelectTab(2)]);
    }
}
