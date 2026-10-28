//! The tab strip drawn: the window's own buttons where it has them, the
//! browser's version before them, the tabs at a width that fits, the count
//! of those that do not, the new-tab button, and the overflow panel and a
//! dragged tab over everything.

use alloc::format;
use alloc::vec::Vec;

use super::{
    COUNT_WIDTH, LEAST_TABS_ROOM, MARKER_WIDTH, NEW_TAB, PANEL_DIM, PANEL_EDGE, PANEL_ROW, PRIVATE_MARK, SHADOW, SHADOW_BAND, STRIP_HEIGHT, STRIP_MARGIN,
    TAB_CLOSE, TAB_GAP, TAB_SPEAKER, TAB_TEXT_INSET, TAB_TOP, TAB_TOP_INACTIVE, TAB_WIDTH, VERSION_GAP, WINDOW_BUTTON_GAP, WINDOW_BUTTON_HEIGHT,
    WINDOW_BUTTON_WIDTH, arrangement, rest_of,
};
use crate::browser::Browser;
use crate::browser::banks::BANK_BORDER;
use crate::browser::glyphs::{WINDOW_GLYPH, WINDOW_GLYPH_THICKNESS, speaker_glyph};
use crate::browser::regions::{Regions, TabRegion};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::text::{HAIRLINE, border, centred, cross, left_centred, outline};
use crate::theme::{BLACK, BUTTON, GLYPH, GREEN, MUTED, PRIVATE, RAISED, RED, ROW_STATUS, RULE, SELECTED, TAB_COLOURS, TAB_TITLE, WHITE, YELLOW};

impl Browser {
    pub(in crate::browser) fn draw_tab_strip<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64) {
        // Raised with its open overflow panel, as a dialog is (decision 102).
        canvas.fill(Rect::new(0.0, 0.0, width, STRIP_HEIGHT), 0.0, if self.tab_strip.open { RAISED } else { BLACK });
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
        self.draw_tabs(canvas, left, right);
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
    /// button and the count; the panel is drawn last, over everything.
    fn draw_tabs<C: Canvas + ?Sized>(&mut self, canvas: &mut C, left: f64, right: f64) {
        let room = right - left - NEW_TAB - TAB_GAP * 2.0;
        #[allow(clippy::cast_precision_loss, reason = "a few tabs")]
        let count = self.tabs.len().max(1) as f64;
        let fit = if count * (TAB_WIDTH + TAB_GAP) - TAB_GAP <= room {
            self.tabs.len()
        } else {
            // Each tab and its gap, with the count's room kept.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a count of tabs, at least one")]
            let fit = libm::floor((room - COUNT_WIDTH) / (TAB_WIDTH + TAB_GAP)).max(1.0) as usize;
            fit
        };
        let tab_width = TAB_WIDTH;
        let (shown, rest) = self.tabs_arranged(fit);
        self.tab_strip.overflowing = !rest.is_empty();
        self.tab_strip.held = self.tab_strip.open.then(|| (shown.clone(), self.tabs.len()));
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
        self.tab_strip.tabs_area = Rect::new(left, 0.0, x, STRIP_HEIGHT);
        self.regions.new_tab = Rect::new(x + TAB_GAP, TAB_TOP_INACTIVE, x + TAB_GAP + NEW_TAB, STRIP_HEIGHT - TAB_GAP);
        canvas.fill(self.regions.new_tab, 0.0, BUTTON);
        outline(canvas, self.regions.new_tab, RULE);
        centred(canvas, self.regions.new_tab, "+", GLYPH, WHITE);
        self.tab_strip.rest = (rest, tab_width, left, right);
    }

    /// Which tabs the strip shows and which wrap into the panel: while the
    /// panel is open, the tabs it opened with, so nothing moves under the
    /// pointer, unless tabs came or went; otherwise the first that fit,
    /// with the active one.
    fn tabs_arranged(&self, fit: usize) -> (Vec<usize>, Vec<usize>) {
        let count = self.tabs.len();
        if let Some((shown, held_count)) = self.tab_strip.held.as_ref().filter(|_| self.tab_strip.open)
            && *held_count == count
            && shown.len() == fit.min(count)
        {
            return (shown.clone(), rest_of(count, shown));
        }
        arrangement(count, self.active, fit)
    }

    /// One tab in its slot: the active one standing taller, with its sound
    /// and close buttons.
    fn draw_tab<C: Canvas + ?Sized>(&self, canvas: &mut C, index: usize, slot: Rect) -> TabRegion {
        let active = index == self.active;
        let rect = Rect::new(slot.x0, slot.y0 + if active { TAB_TOP } else { TAB_TOP_INACTIVE }, slot.x1, slot.y1);
        let coloured = self.tabs.get(index).and_then(|tab| Some((tab, *TAB_COLOURS.iter().cycle().nth(usize::from(tab.colour))?)));
        let Some((tab, (fill, ink))) = coloured else { return TabRegion { index, tab: rect, close: Rect::ZERO, speaker: None } };
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

    /// The overflow panel, joined under the strip, while it is open; and a
    /// dragged tab where the pointer has it. Drawn over the toolbar and the
    /// page area, as the menu's list is, raised as a dialog is: the window
    /// below dimmed, the strip and the panel one surface, a blue edge at
    /// its foot and a shadow below (decision 102).
    pub(in crate::browser) fn draw_tab_overflow<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64, height: f64) {
        self.tab_strip.panel = Rect::ZERO;
        let (rest, tab_width, left, right) = core::mem::take(&mut self.tab_strip.rest);
        if self.tab_strip.open && !rest.is_empty() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a count of tabs, at least one")]
            let per_row = libm::floor((right - left + TAB_GAP) / (tab_width + TAB_GAP)).max(1.0) as usize;
            let rows = rest.len().div_ceil(per_row);
            #[allow(clippy::cast_precision_loss, reason = "a few rows")]
            let panel = Rect::new(0.0, STRIP_HEIGHT - HAIRLINE, width, STRIP_HEIGHT + rows as f64 * PANEL_ROW + TAB_GAP);
            canvas.dim(Rect::new(0.0, panel.y1, width, height), PANEL_DIM);
            let mut band_top = panel.y1;
            for amount in SHADOW {
                canvas.dim(Rect::new(0.0, band_top, width, band_top + SHADOW_BAND), amount);
                band_top += SHADOW_BAND;
            }
            // The strip's own surface, with no seam: the strip grown down.
            canvas.fill(panel, 0.0, RAISED);
            canvas.fill(Rect::new(0.0, panel.y1 - PANEL_EDGE, width, panel.y1), 0.0, SELECTED);
            for (place, &index) in rest.iter().enumerate() {
                #[allow(clippy::cast_precision_loss, reason = "a few tabs")]
                let (row, column) = (place.checked_div(per_row).unwrap_or(0) as f64, place.checked_rem(per_row).unwrap_or(0) as f64);
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
    cross(canvas, centre(regions.close), half, thick, BLACK);
}
