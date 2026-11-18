//! The tab strip drawn: the window's own buttons where it has them, the
//! browser's version before them, the tabs in their sliding row, the count
//! of those not shown, the new-tab button, and a dragged tab over
//! everything.

use alloc::format;

use super::{
    COSTLY_MARK, COUNT_WIDTH, LEAST_TABS_ROOM, MARKER_WIDTH, NEW_TAB, PRIVATE_MARK, STRIP_HEIGHT, STRIP_MARGIN, TAB_CLOSE, TAB_GAP, TAB_SPEAKER,
    TAB_TEXT_INSET, TAB_TOP, TAB_TOP_INACTIVE, TAB_WIDTH, VERSION_GAP, WINDOW_BUTTON_GAP, WINDOW_BUTTON_HEIGHT, WINDOW_BUTTON_WIDTH, strip_slide,
};
use crate::browser::Browser;
use crate::browser::banks::BANK_BORDER;
use crate::browser::glyphs::{WINDOW_GLYPH, WINDOW_GLYPH_THICKNESS, speaker_glyph};
use crate::browser::regions::{Regions, TabRegion};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::text::{border, centred, cross, left_centred, outline};
use crate::theme::{BLACK, BUTTON, COSTLY, GLYPH, GREEN, MUTED, PRIVATE, RED, ROW_STATUS, RULE, SELECTED, TAB_COLOURS, TAB_TITLE, WHITE, YELLOW};
use crate::tween::{STANDARD, Tween};

impl Browser {
    pub(in crate::browser) fn draw_tab_strip<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64, now: u64) {
        canvas.fill(Rect::new(0.0, 0.0, width, STRIP_HEIGHT), 0.0, BLACK);
        let mut right = width - STRIP_MARGIN;
        if self.frame.window_buttons {
            let top = (STRIP_HEIGHT - WINDOW_BUTTON_HEIGHT) / 2.0;
            let button = |right: &mut f64| {
                let rect = Rect::new(*right - WINDOW_BUTTON_WIDTH, top, *right, top + WINDOW_BUTTON_HEIGHT);
                *right = rect.x0 - WINDOW_BUTTON_GAP;
                rect
            };
            self.regions.close = button(&mut right);
            self.regions.maximise = button(&mut right);
            self.regions.minimise = button(&mut right);
            draw_window_buttons(canvas, &self.regions);
        }
        let left = self.frame.left_inset + STRIP_MARGIN;
        let right = self.draw_version(canvas, left, right);
        self.draw_tabs(canvas, left, right, now);
        self.regions.strip = Rect::new(0.0, 0.0, width, STRIP_HEIGHT);
    }

    /// The browser's version and release stage at the strip's right end,
    /// before the window's own buttons (decision 123), as quiet secondary
    /// text with no face or outline, so that it reads as a label and not a
    /// button; returns where the tabs end. Left out when it would leave the
    /// tabs less than [`LEAST_TABS_ROOM`], so that it never covers a tab.
    fn draw_version<C: Canvas + ?Sized>(&self, canvas: &mut C, left: f64, right: f64) -> f64 {
        let text = self.version_label();
        let width = canvas.text_width(&text, ROW_STATUS);
        // In the new-tab button's band, so the label lines up with it.
        let label = Rect::new(right - VERSION_GAP - width, TAB_TOP_INACTIVE, right - VERSION_GAP, STRIP_HEIGHT - TAB_GAP);
        let tabs_end = label.x0 - VERSION_GAP;
        if tabs_end - left < LEAST_TABS_ROOM {
            return right;
        }
        left_centred(canvas, label, 0.0, &text, ROW_STATUS, MUTED);
        tabs_end
    }

    /// Draws the strip's tabs between `left` and `right`, the new-tab
    /// button and the count of those not shown. The tabs are one row, slid
    /// with easing to show the active one whole (REQ-6640 item 8), and cut
    /// at the ends of the room they have.
    fn draw_tabs<C: Canvas + ?Sized>(&mut self, canvas: &mut C, left: f64, right: f64, now: u64) {
        let room = right - left - NEW_TAB - TAB_GAP * 2.0;
        let step = TAB_WIDTH + TAB_GAP;
        let count = self.tabs.len();
        #[allow(clippy::cast_precision_loss, reason = "a few tabs")]
        let fit = if count.max(1) as f64 * step - TAB_GAP <= room {
            count
        } else {
            // Each tab and its gap, with the count's room kept.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a count of tabs, at least one")]
            let fit = libm::floor((room - COUNT_WIDTH) / step).max(1.0) as usize;
            fit
        };
        #[allow(clippy::cast_precision_loss, reason = "a few tabs")]
        let view_width = fit.max(1) as f64 * step - TAB_GAP;
        let target = strip_slide(f64::from(self.filmstrip.strip.target()), self.active, count, view_width, step);
        #[expect(clippy::cast_possible_truncation, reason = "points across a window, far within f32")]
        let target = target as f32;
        // Slid when the active tab changed; placed at once otherwise.
        if self.filmstrip.strip_active.is_some_and(|before| before != self.active) {
            self.filmstrip.strip.set(target, now);
        } else if (self.filmstrip.strip.target() - target).abs() > f32::EPSILON {
            self.filmstrip.strip = Tween::new(target, self.filmstrip_slide_duration(), STANDARD);
        }
        self.filmstrip.strip_active = Some(self.active);
        let slid = f64::from(self.filmstrip.strip.value(now));
        // The active tab's mark stands below the strip's foot.
        let view = Rect::new(left, 0.0, left + view_width, STRIP_HEIGHT + BANK_BORDER);
        canvas.push_clip(view);
        for index in 0..count {
            #[allow(clippy::cast_precision_loss, reason = "a few tabs")]
            let x = left + index as f64 * step - slid;
            if x + TAB_WIDTH <= view.x0 || x >= view.x1 {
                continue;
            }
            let region = self.draw_tab(canvas, index, Rect::new(x, 0.0, x + TAB_WIDTH, STRIP_HEIGHT));
            // What is cut off takes no press.
            let cut = |rect: Rect| {
                let inside = Rect::new(rect.x0.max(view.x0), rect.y0, rect.x1.min(view.x1), rect.y1);
                if inside.x0 < inside.x1 { inside } else { Rect::ZERO }
            };
            self.regions.tabs.push(TabRegion {
                index,
                tab: cut(region.tab),
                close: cut(region.close),
                speaker: region.speaker.map(cut),
                cost: region.cost.map(cut),
            });
        }
        canvas.pop_clip();
        let mut x = view.x1 + TAB_GAP;
        self.tab_strip.count = Rect::ZERO;
        if fit < count {
            let count_rect = Rect::new(x, TAB_TOP_INACTIVE, x + COUNT_WIDTH, STRIP_HEIGHT - TAB_GAP);
            canvas.fill(count_rect, 0.0, if self.filmstrip_open() { SELECTED } else { BUTTON });
            outline(canvas, count_rect, RULE);
            centred(canvas, count_rect, &format!("+{}", count.saturating_sub(fit)), ROW_STATUS, WHITE);
            self.tab_strip.count = count_rect;
            x = count_rect.x1 + TAB_GAP;
        }
        self.tab_strip.tabs_area = Rect::new(left, 0.0, x, STRIP_HEIGHT);
        self.regions.new_tab = Rect::new(x + TAB_GAP, TAB_TOP_INACTIVE, x + TAB_GAP + NEW_TAB, STRIP_HEIGHT - TAB_GAP);
        canvas.fill(self.regions.new_tab, 0.0, BUTTON);
        outline(canvas, self.regions.new_tab, RULE);
        centred(canvas, self.regions.new_tab, "+", GLYPH, WHITE);
    }

    /// One tab in its slot: the active one standing taller, with its sound
    /// and close buttons.
    fn draw_tab<C: Canvas + ?Sized>(&self, canvas: &mut C, index: usize, slot: Rect) -> TabRegion {
        let active = index == self.active;
        let rect = Rect::new(slot.x0, slot.y0 + if active { TAB_TOP } else { TAB_TOP_INACTIVE }, slot.x1, slot.y1);
        let coloured = self.tabs.get(index).and_then(|tab| Some((tab, *TAB_COLOURS.iter().cycle().nth(usize::from(tab.colour))?)));
        let Some((tab, (fill, ink))) = coloured else { return TabRegion { index, tab: rect, close: Rect::ZERO, speaker: None, cost: None } };
        canvas.fill(rect, 0.0, fill);
        if active {
            border(canvas, Rect::new(rect.x0, rect.y0, rect.x1, rect.y1 + BANK_BORDER), BANK_BORDER - 1.0, WHITE);
        }
        let close = Rect::new(rect.x1 - TAB_CLOSE - TAB_GAP * 2.0, rect.y0, rect.x1 - TAB_GAP, rect.y1);
        let speaker = (tab.muted || self.sounding.contains(&tab.channel)).then(|| Rect::new(close.x0 - TAB_SPEAKER, rect.y0, close.x0, rect.y1));
        // The mark of a page costing the machine (REQ-6626), drawn here by
        // the interface, outside the page's picture, so no page can draw,
        // hide or imitate it.
        let before_buttons = speaker.map_or(close.x0, |speaker| speaker.x0);
        let cost = tab.costly.then(|| {
            let middle = f64::midpoint(rect.y0, rect.y1);
            Rect::new(before_buttons - COSTLY_MARK - TAB_GAP, middle - COSTLY_MARK / 2.0, before_buttons - TAB_GAP, middle + COSTLY_MARK / 2.0)
        });
        canvas.push_clip(Rect::new(rect.x0, rect.y0, cost.map_or(before_buttons, |cost| cost.x0 - TAB_GAP), rect.y1));
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
        if let Some(cost) = cost {
            canvas.fill(cost, 0.0, COSTLY);
            border(canvas, cost, 1.0, ink);
        }
        if let Some(speaker) = speaker {
            speaker_glyph(canvas, speaker, tab.muted, ink);
        }
        centred(canvas, close, "\u{00D7}", GLYPH, ink);
        TabRegion { index, tab: rect, close, speaker, cost }
    }

    /// A dragged tab where the pointer has it, and a line where it will
    /// go, over everything.
    pub(in crate::browser) fn draw_tab_drag<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
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
}

fn draw_window_buttons<C: Canvas + ?Sized>(canvas: &mut C, regions: &Regions) {
    for (rect, colour) in [(regions.minimise, GREEN), (regions.maximise, YELLOW), (regions.close, RED)] {
        canvas.fill(rect, 0.0, colour);
    }
    let centre = |rect: Rect| Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let half = WINDOW_GLYPH / 2.0;
    let thick = WINDOW_GLYPH_THICKNESS;
    let minimise = centre(regions.minimise);
    canvas.fill(Rect::new(minimise.x - half, minimise.y + half - thick, minimise.x + half, minimise.y + half), 0.0, BLACK);
    let maximise = centre(regions.maximise);
    border(canvas, Rect::new(maximise.x - half, maximise.y - half, maximise.x + half, maximise.y + half), thick, BLACK);
    // The close cross: two bars, each a thin parallelogram from corner to corner.
    cross(canvas, centre(regions.close), half, thick, BLACK);
}
