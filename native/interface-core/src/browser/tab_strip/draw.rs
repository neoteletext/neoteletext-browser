//! The tab strip drawn: the window's own buttons where it has them, the
//! tabs at a width that fits, the count of those that do not, the new-tab
//! button, and the overflow panel and a dragged tab over everything.

use alloc::format;

use super::{
    COUNT_WIDTH, MARKER_WIDTH, NEW_TAB, PANEL_ROW, PRIVATE_MARK, STRIP_HEIGHT, STRIP_MARGIN, TAB_CLOSE, TAB_GAP, TAB_MAX_WIDTH, TAB_MIN_WIDTH, TAB_SPEAKER,
    TAB_TEXT_INSET, TAB_TOP, TAB_TOP_INACTIVE, WINDOW_BUTTON_GAP, WINDOW_BUTTON_HEIGHT, WINDOW_BUTTON_WIDTH, arrangement,
};
use crate::browser::Browser;
use crate::browser::banks::BANK_BORDER;
use crate::browser::glyphs::{WINDOW_GLYPH, WINDOW_GLYPH_THICKNESS, speaker_glyph};
use crate::browser::regions::{Regions, TabRegion};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::theme::{BLACK, BUTTON, GLYPH, GREEN, PRIVATE, RED, ROW_STATUS, RULE, SELECTED, TAB_COLOURS, TAB_TITLE, WHITE, YELLOW};
use crate::ui::{HAIRLINE, border, centred, left_centred, outline};

impl Browser {
    pub(in crate::browser) fn draw_tab_strip<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64) {
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
        self.draw_tabs(canvas, left, right);
        self.regions.strip = Rect::new(0.0, 0.0, width, STRIP_HEIGHT);
    }

    /// Draws the strip's tabs between `left` and `right`, the new-tab
    /// button and the count; the panel is drawn last, over everything.
    fn draw_tabs<C: Canvas + ?Sized>(&mut self, canvas: &mut C, left: f64, right: f64) {
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
    pub(in crate::browser) fn draw_tab_overflow<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64) {
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
    let close = centre(regions.close);
    let normal = thick / 2.0 / core::f64::consts::SQRT_2;
    for direction in [1.0, -1.0] {
        let along = Point::new(half, half * direction);
        let across = Point::new(-normal * direction, normal);
        canvas.polygon(&[close - along + across, close + along + across, close + along - across, close - along - across], BLACK);
    }
}
