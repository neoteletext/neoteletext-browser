//! The overview of all 800 pages along the bottom: a cell per page in
//! its bank's colour, the legend, the rings and dots of updates and
//! arrivals, the page under the pointer popped up at its native size, and
//! the button that folds it into the footer.

use alloc::format;

use crate::browser::Browser;
use crate::browser::frames::{PULSE_MILLISECONDS, PULSE_SCALE};
use crate::browser::glyphs::chevron;
use crate::browser::window::{HEADING_HEIGHT, MARGIN, PADDING};
use crate::canvas::{Canvas, PAGE_HEIGHT, PAGE_WIDTH, SCREEN_SCALE, draw_page_on_screen};
use crate::channel::{BANKS, PAGES, PAGES_PER_BANK, Status, bank_indices, page_index};
use crate::geometry::{Point, Rect};
use crate::text::{border, centred, left_centred, outline};
use crate::theme::{
    AXIS, BANK_COLOURS, BLACK, BUTTON, CELL_AVAILABLE, CELL_EMPTY, CELL_PULSE, HEADING, LEGEND, MUTED, PANEL, RULE, SELECTED, UPDATED, WHITE, YELLOW, mix,
};
use crate::wall::since;

const AXIS_WIDTH: f64 = 40.0;
const CELL_GAP: f64 = 2.0;
const LEGEND_SWATCH: f64 = 10.0;
const LEGEND_GAP: f64 = 6.0;
const LEGEND_SPACING: f64 = 18.0;
/// An updated row flashes this far towards amber at first.
pub(super) const ROW_FLASH: f64 = 0.6;
/// The overview's update ring: 1.2 seconds, growing to five cells wide,
/// two pixels thick.
pub(super) const UPDATE_RING_MILLISECONDS: u64 = 1_200;
const UPDATE_RING_SCALE: f64 = 4.0;
const UPDATE_RING_WIDTH: f64 = 2.0;
/// The update dot leaves a quarter of the cell all round.
const UPDATE_DOT_INSET: f64 = 0.25;
/// The button at the overview's top right that folds it into the footer
/// and opens it again, with its chevron.
const TOGGLE_SIZE: f64 = 28.0;
/// A page under the pointer in the overview pops up at its native size,
/// this far above its cell, with a shadow offset as the wall's hover.
const PREVIEW_GAP: f64 = 8.0;
const PREVIEW_SHADOW: f64 = 8.0;
const PREVIEW_SHADOW_DARKNESS: f32 = 0.6;

impl Browser {
    /// The overview's cell under `point`, when its page has arrived.
    pub(super) fn overview_page(&self, point: Point) -> Option<usize> {
        let index = self.cell_index(point)?;
        matches!(self.channel.pages.get(index), Some(Some(info)) if info.status == Status::Loaded).then_some(index)
    }

    /// Folds the overview into the footer, or opens it again.
    /// The View menu's overview: shown, or hidden with its room given to
    /// the pages (decision 109).
    pub fn toggle_overview_shown(&mut self) {
        self.overview.hidden = !self.overview.hidden;
        self.pulses.clear();
        self.previewed = None;
    }

    /// Whether the overview is shown at all, folded or not.
    #[must_use]
    pub fn overview_shown(&self) -> bool {
        !self.overview.hidden
    }

    pub(super) fn toggle_overview(&mut self) {
        self.overview.folded = !self.overview.folded;
        self.pulses.clear();
        self.previewed = None;
    }

    pub(super) fn cell_index(&self, point: Point) -> Option<usize> {
        let cells = self.regions.cells;
        if !cells.contains(point) {
            return None;
        }
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let (width, height) = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "the point is inside the grid")]
        let (column, row) = (((point.x - cells.x0) / width) as usize, ((point.y - cells.y0) / height) as usize);
        (column < PAGES_PER_BANK && row < BANKS).then(|| bank_indices(row).start.saturating_add(column))
    }

    /// The overview's legend and page counts in `heading`, right-aligned to
    /// end before `right`.
    fn draw_legend<C: Canvas + ?Sized>(&self, canvas: &mut C, heading: Rect, right: f64) {
        let middle = f64::midpoint(heading.y0, heading.y1);
        let (present, loaded) = self.channel.counts();
        let mut x = right - LEGEND_SPACING;
        let counts = format!("{present} of {PAGES} pages have content, {loaded} loaded");
        x -= canvas.text_width(&counts, LEGEND);
        left_centred(canvas, Rect::new(x, heading.y0, heading.x1, heading.y1), 0.0, &counts, LEGEND, MUTED);
        x -= LEGEND_SPACING * 2.0;
        for (word, colour, ring) in [
            ("Empty", CELL_EMPTY, false),
            ("Available", CELL_AVAILABLE, false),
            ("Loaded", YELLOW, false),
            ("Shown", YELLOW, true),
            ("Updated", UPDATED, false),
        ]
        .into_iter()
        .rev()
        {
            let width = canvas.text_width(word, LEGEND);
            x -= width;
            left_centred(canvas, Rect::new(x, heading.y0, x + width, heading.y1), 0.0, word, LEGEND, MUTED);
            x -= LEGEND_GAP + LEGEND_SWATCH;
            let swatch = Rect::new(x, middle - LEGEND_SWATCH / 2.0, x + LEGEND_SWATCH, middle + LEGEND_SWATCH / 2.0);
            canvas.fill(swatch, 0.0, colour);
            if ring {
                outline(canvas, swatch, WHITE);
            }
            x -= LEGEND_SPACING;
        }
    }

    pub(super) fn draw_overview<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect, now: u64) {
        canvas.fill(panel, 0.0, PANEL);
        outline(canvas, panel, RULE);
        let heading = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + HEADING_HEIGHT);
        left_centred(canvas, heading, PADDING, &format!("All {PAGES} pages · overview"), HEADING, WHITE);
        let middle = f64::midpoint(heading.y0, heading.y1);
        let toggle = Rect::new(heading.x1 - PADDING - TOGGLE_SIZE, middle - TOGGLE_SIZE / 2.0, heading.x1 - PADDING, middle + TOGGLE_SIZE / 2.0);
        self.regions.overview = panel;
        self.regions.overview_toggle = toggle;
        canvas.fill(toggle, 0.0, BUTTON);
        outline(canvas, toggle, RULE);
        // Down folds it away; up brings it back.
        chevron(canvas, toggle, !self.overview.folded, WHITE);
        if self.overview.folded {
            return;
        }
        self.draw_legend(canvas, heading, toggle.x0);

        let cells = Rect::new(panel.x0 + PADDING + AXIS_WIDTH, heading.y1, panel.x1 - PADDING, panel.y1 - PADDING);
        self.regions.cells = cells;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let size = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
        self.draw_cells(canvas, panel, cells, size, now);
        self.draw_rings(canvas, cells, size, now);
        self.draw_pulses(canvas, cells, size, now);
    }

    /// The bank axis and a cell for each of the 800 pages in `cells`, each
    /// `size` wide and high: its state's colour, outlined on screen, ringed
    /// when chosen, dotted for a minute after an update.
    fn draw_cells<C: Canvas + ?Sized>(&self, canvas: &mut C, panel: Rect, cells: Rect, size: (f64, f64), now: u64) {
        let (width, height) = size;
        // Which cells are on screen, looked up once for each rather than
        // searched for in the list of visible pages 800 times.
        let mut on_screen = [false; PAGES];
        for key in self.visible.iter().filter(|key| key.channel == self.channel.channel) {
            if let Some(cell) = page_index(key.page).and_then(|index| on_screen.get_mut(index)) {
                *cell = true;
            }
        }
        for (bank, &bank_colour) in BANK_COLOURS.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "eight banks")]
            let top = cells.y0 + bank as f64 * height;
            let axis = Rect::new(panel.x0 + PADDING, top, cells.x0, top + height);
            left_centred(canvas, axis, 0.0, &format!("{}00", bank.saturating_add(1)), AXIS, if bank == self.bank { WHITE } else { MUTED });
            if bank == self.bank {
                canvas.fill(Rect::new(axis.x0 - PADDING / 2.0, top + CELL_GAP, axis.x0 - PADDING / 2.0 + CELL_GAP, top + height - CELL_GAP), 0.0, WHITE);
            }
            for (column, index) in bank_indices(bank).enumerate() {
                #[allow(clippy::cast_precision_loss, reason = "small counts")]
                let left = cells.x0 + column as f64 * width;
                let cell = Rect::new(left, top, left + width - CELL_GAP, top + height - CELL_GAP);
                let colour = match self.channel.pages.get(index) {
                    Some(Some(info)) if info.status == Status::Loaded => bank_colour,
                    Some(Some(_)) => CELL_AVAILABLE,
                    _ => CELL_EMPTY,
                };
                canvas.fill(cell, 0.0, colour);
                if on_screen.get(index).copied().unwrap_or(false) {
                    outline(canvas, cell, WHITE);
                }
                if self.selected == Some(index) {
                    border(canvas, Rect::new(cell.x0 - CELL_GAP, cell.y0 - CELL_GAP, cell.x1 + CELL_GAP, cell.y1 + CELL_GAP), CELL_GAP, SELECTED);
                }
                // An amber dot for a minute after each update.
                if since(self.updated_at(index), now).is_some() {
                    let inset = cell.width().min(cell.height()) * UPDATE_DOT_INSET;
                    let dot = Rect::new(cell.x0 + inset, cell.y0 + inset, cell.x1 - inset, cell.y1 - inset);
                    canvas.fill(dot, dot.width().min(dot.height()) / 2.0, UPDATED);
                }
            }
        }
    }

    /// A ring bursting out of each page just updated, growing and fading.
    fn draw_rings<C: Canvas + ?Sized>(&self, canvas: &mut C, cells: Rect, size: (f64, f64), now: u64) {
        let (width, height) = size;
        for &(index, at) in &self.updates {
            let elapsed = now.saturating_sub(at);
            if elapsed >= UPDATE_RING_MILLISECONDS {
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "under two seconds")]
            let progress = elapsed as f64 / UPDATE_RING_MILLISECONDS as f64;
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let centre = Point::new(cells.x0 + ((index % PAGES_PER_BANK) as f64 + 0.5) * width, cells.y0 + ((index / PAGES_PER_BANK) as f64 + 0.5) * height);
            let radius = width.min(height) * (1.0 + UPDATE_RING_SCALE * progress) / 2.0;
            let ring = Rect::new(centre.x - radius, centre.y - radius, centre.x + radius, centre.y + radius);
            border(canvas, ring, UPDATE_RING_WIDTH, mix(UPDATED, PANEL, progress));
        }
    }

    /// A dot on each page just received, shrinking away.
    fn draw_pulses<C: Canvas + ?Sized>(&self, canvas: &mut C, cells: Rect, size: (f64, f64), now: u64) {
        let (width, height) = size;
        for &(index, start) in &self.pulses {
            let elapsed = now.saturating_sub(start);
            if elapsed >= PULSE_MILLISECONDS {
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "under a second")]
            let remaining = 1.0 - elapsed as f64 / PULSE_MILLISECONDS as f64;
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let centre = Point::new(cells.x0 + ((index % PAGES_PER_BANK) as f64 + 0.5) * width, cells.y0 + ((index / PAGES_PER_BANK) as f64 + 0.5) * height);
            let radius = width.min(height) * PULSE_SCALE * remaining / 2.0;
            canvas.fill(Rect::new(centre.x - radius, centre.y - radius, centre.x + radius, centre.y + radius), radius, CELL_PULSE);
        }
    }

    /// The overview's page under the pointer, at its native size, above
    /// its cell. It never covers the tab strip or the trusted strip, where
    /// a page image could pass for the address: on a short window it
    /// shrinks to fit below them.
    pub(super) fn draw_preview<C: Canvas + ?Sized>(&self, canvas: &mut C, width: f64) {
        let Some(index) = self.previewed else { return };
        let Some(key) = self.page_key(index) else { return };
        let cells = self.regions.cells;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let (cell_width, cell_height) = (cells.width() / PAGES_PER_BANK as f64, cells.height() / BANKS as f64);
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let (centre, cell_top) = (cells.x0 + ((index % PAGES_PER_BANK) as f64 + 0.5) * cell_width, cells.y0 + (index / PAGES_PER_BANK) as f64 * cell_height);
        let bottom = cell_top - PREVIEW_GAP;
        // The image at most its native size, with its screen around it.
        let scale = ((bottom - self.body.y0) / (f64::from(PAGE_HEIGHT) * SCREEN_SCALE)).min(1.0);
        if scale <= 0.0 {
            return;
        }
        let (page_width, page_height) = (f64::from(PAGE_WIDTH) * scale * SCREEN_SCALE, f64::from(PAGE_HEIGHT) * scale * SCREEN_SCALE);
        let left = (centre - page_width / 2.0).clamp(MARGIN, (width - MARGIN - page_width).max(MARGIN));
        let rect = Rect::new(left, bottom - page_height, left + page_width, bottom);
        canvas.dim(Rect::new(rect.x0 + PREVIEW_SHADOW, rect.y0 + PREVIEW_SHADOW, rect.x1 + PREVIEW_SHADOW, rect.y1 + PREVIEW_SHADOW), PREVIEW_SHADOW_DARKNESS);
        if canvas.screen(key).is_none() {
            // Asked for when the pointer arrived; it shows when rendered.
            canvas.fill(rect, 0.0, BLACK);
            centred(canvas, rect, &format!("{:03X}", key.page), HEADING, MUTED);
        } else {
            draw_page_on_screen(canvas, key, rect, BLACK);
        }
        outline(canvas, rect, WHITE);
    }
}

#[cfg(test)]
mod tests;
