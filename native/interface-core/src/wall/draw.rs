//! The page area drawn: the layout or the zoomed page, the scroll bar
//! and the control row, and each page with its screen, kept for
//! hit-testing (requirement 12.3).

use super::grid::{SCROLL_BAR_WIDTH, WALL_PADDING};
use super::size_control::CONTROLS_HEIGHT;
use super::{Drawn, Wall, WallPage};
use crate::canvas::{Canvas, PageKey, draw_page_on_screen};
use crate::geometry::Rect;
use crate::size::{Layout, clamp_width, max_width_for_window};
use crate::text::outline;
use crate::theme::{PANEL, RULE, WINDOW};

impl Wall {
    /// Draws the panel in `panel`, with `pages` on the wall and `selected`
    /// outlined.
    pub fn draw<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect, pages: &[WallPage<'_>], selected: Option<PageKey>, now: u64) {
        canvas.fill(panel, 0.0, PANEL);
        self.area = Rect::new(panel.x0, panel.y0 + CONTROLS_HEIGHT, panel.x1, panel.y1);
        #[allow(clippy::cast_possible_truncation, reason = "panel widths fit easily in f32")]
        let available = (self.area.width() - WALL_PADDING * 2.0) as f32;
        self.max = max_width_for_window(available);
        // A new window size keeps the zoomed state at the slider's end, and
        // never zooms by itself.
        if self.zoom_in || self.width > self.max {
            self.width = if self.zoom_in { self.max } else { clamp_width(self.width, self.max) };
            if !self.zoom_in || self.width_before_zoom.is_none() {
                self.shown_width.set(self.width, now);
            }
        }
        self.focus = selected;
        self.page_keys = pages.iter().map(|page| page.key).collect();
        self.drawn.clear();
        self.open = None;
        // A zoomed page that has left the wall (another bank, another
        // channel) ends the zoom at once.
        if let Some(key) = self.zoomed
            && !pages.iter().any(|page| page.key == key)
        {
            self.to_top();
        }
        let progress = f64::from(self.zoom.value(now));
        let full = self.zoom_in && progress >= 1.0 - f64::from(f32::EPSILON);
        canvas.push_clip(self.area);
        let content = match self.zoomed {
            Some(key) if full => self.draw_zoomed(canvas, pages, key, now),
            _ => {
                let content = match self.layout() {
                    Layout::Grid => self.draw_grid(canvas, pages, selected, now),
                    Layout::List => self.draw_list(canvas, pages, selected),
                };
                self.draw_popped(canvas, now);
                if let Some(key) = self.zoomed {
                    self.draw_zooming(canvas, key, progress);
                }
                content
            }
        };
        canvas.pop_clip();
        if full {
            self.draw_close(canvas);
        }
        if !self.zoom_in && !self.zoom.moving(now) {
            self.zoomed = None;
        }
        let view = self.area.height();
        let total = content + WALL_PADDING * 2.0;
        let max_scroll = (total - view).max(0.0);
        self.scroll = self.scroll.min(max_scroll);
        if max_scroll > 0.0 {
            let bar = view * view / total;
            let top = self.area.y0 + (view - bar) * self.scroll / max_scroll;
            let left = self.area.x1 - SCROLL_BAR_WIDTH * 2.0;
            canvas.fill(Rect::new(left, top, left + SCROLL_BAR_WIDTH, top + bar), SCROLL_BAR_WIDTH / 2.0, RULE);
        }
        self.draw_controls(canvas, panel);
        outline(canvas, panel, RULE);
        self.drawn_moving = self.moving(now);
    }

    /// Draws a page with its screen around it; returns where its image is.
    pub(super) fn draw_page<C: Canvas + ?Sized>(&mut self, canvas: &mut C, key: PageKey, rect: Rect) -> Rect {
        // With no image yet a dark frame stands in, and the page still
        // counts as visible, so that the trusted core renders it.
        let image = draw_page_on_screen(canvas, key, rect, WINDOW);
        self.drawn.push(Drawn { key, rect, image });
        image
    }
}
