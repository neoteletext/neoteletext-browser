//! The page size control along the panel's top: its buttons, its bar of
//! segments with a thumb, and the readout; the presses, drags and wheel
//! turns that set the size, whose right end is the zoomed state.

use alloc::format;
use alloc::string::String;

use super::Wall;
use crate::canvas::{Canvas, PAGE_WIDTH};
use crate::geometry::{Point, Rect};
use crate::size::{Layout, clamp_width, fraction_of_width, grid_width, stepped_width, width_at_fraction};
use crate::theme::{BUTTON, CONTROL, MUTED, PANEL, READOUT, RULE, SEGMENT_OFF, SELECTED, WHITE};
use crate::ui::{centred, label, outline};

pub(super) const CONTROLS_HEIGHT: f64 = 48.0;
const PADDING: f64 = 12.0;
const BUTTON_SIZE: f64 = 32.0;
const SEGMENT_WIDTH: f64 = 10.0;
const SEGMENT_GAP: f64 = 3.0;
const SEGMENT_HEIGHT: f64 = 18.0;
const THUMB_WIDTH: f64 = 10.0;
const THUMB_OVERHANG: f64 = 5.0;
const READOUT_WIDTH: f64 = 76.0;
const CONTROL_GAP: f64 = 10.0;
const PERCENT: f64 = 100.0;
/// Where the label's text starts below the row's top.
const LABEL_DROP: f64 = 12.0;

impl Wall {
    /// A new page size from the slider, its buttons, keys or the wheel.
    /// The slider's right end is the zoomed state: reaching it zooms the
    /// chosen page (or the first on screen), and leaving it zooms out.
    fn set_width(&mut self, width: f32, now: u64) {
        self.width = clamp_width(width, self.max);
        self.shown_width.set(self.width, now);
        let at_max = self.width >= self.max;
        if at_max && !self.zoom_in {
            let first = self.drawn.first().map(|drawn| drawn.key);
            if let Some(key) = self.focus.filter(|key| self.page_keys.contains(key)).or(first) {
                self.width_before_zoom = None;
                self.start_zoom(key, now);
            }
        } else if !at_max && self.zoom_in {
            self.width_before_zoom = None;
            self.zoom_in = false;
            self.zoom.set(0.0, now);
        }
    }

    /// Steps the page size one notch smaller (`direction` below zero) or larger.
    pub fn step(&mut self, direction: i8, now: u64) {
        self.set_width(stepped_width(self.width, direction), now);
    }

    fn width_at(&self, x: f64) -> f32 {
        let fraction = ((x - self.bar.x0) / self.bar.width()).clamp(0.0, 1.0);
        #[allow(clippy::cast_possible_truncation, reason = "a fraction between 0 and 1")]
        let fraction = fraction as f32;
        width_at_fraction(fraction, self.max)
    }

    /// Returns whether the press was on the wall's controls.
    pub fn pointer_pressed(&mut self, point: Point, now: u64) -> bool {
        if self.zoom_in && self.close.contains(point) {
            self.unzoom(now);
            return true;
        }
        if self.bar.contains(point) {
            self.dragging = true;
            self.set_width(self.width_at(point.x), now);
            return true;
        }
        if self.minus.contains(point) {
            self.step(-1, now);
            return true;
        }
        if self.plus.contains(point) {
            self.step(1, now);
            return true;
        }
        false
    }

    /// Returns whether the size changed.
    pub fn pointer_moved(&mut self, point: Point, now: u64) -> bool {
        if self.dragging {
            self.set_width(self.width_at(point.x), now);
        }
        self.dragging
    }

    /// The main button was released: any drag of the page size bar ends.
    pub fn pointer_released(&mut self) {
        self.dragging = false;
    }

    /// Slides the page size bar by a wheel's `distance`, as the scroll
    /// would move: the thumb moves the same number of pixels, the wheel
    /// turned away from the user (a negative distance) moving it right,
    /// towards larger pages.
    pub fn wheel_bar(&mut self, distance: f64, now: u64) {
        if self.bar.width() <= 0.0 {
            return;
        }
        let fraction = f64::from(fraction_of_width(self.width, self.max));
        let moved = (fraction - distance / self.bar.width()).clamp(0.0, 1.0);
        #[allow(clippy::cast_possible_truncation, reason = "a fraction between 0 and 1")]
        let width = width_at_fraction(moved as f32, self.max);
        self.set_width(width, now);
    }

    pub(super) fn draw_controls<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect) {
        let row = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + CONTROLS_HEIGHT);
        canvas.fill(row, 0.0, PANEL);
        canvas.fill(Rect::new(row.x0, row.y1 - 1.0, row.x1, row.y1), 0.0, RULE);
        let middle = f64::midpoint(row.y0, row.y1);
        let mut x = row.x0 + PADDING;
        label(canvas, Point::new(x, row.y0 + LABEL_DROP), "Page size", CONTROL, WHITE);
        x += canvas.text_width("Page size", CONTROL) + CONTROL_GAP * 1.5;

        let right = row.x1 - PADDING;

        // − [segments] + readout
        self.minus = Rect::new(x, middle - BUTTON_SIZE / 2.0, x + BUTTON_SIZE, middle + BUTTON_SIZE / 2.0);
        let readout = Rect::new(right - READOUT_WIDTH, row.y0, right, row.y1);
        self.plus = Rect::new(readout.x0 - CONTROL_GAP - BUTTON_SIZE, self.minus.y0, readout.x0 - CONTROL_GAP, self.minus.y1);
        for (button, glyph) in [(self.minus, "−"), (self.plus, "+")] {
            canvas.fill(button, 0.0, BUTTON);
            outline(canvas, button, WHITE);
            centred(canvas, button, glyph, CONTROL, WHITE);
        }
        self.bar = Rect::new(self.minus.x1 + CONTROL_GAP, middle - SEGMENT_HEIGHT / 2.0, self.plus.x0 - CONTROL_GAP, middle + SEGMENT_HEIGHT / 2.0);
        self.bar_zone = Rect::new(self.bar.x0, row.y0, self.bar.x1, row.y1);
        let fraction = f64::from(fraction_of_width(self.width, self.max));
        let thumb = self.bar.x0 + fraction * (self.bar.width() - THUMB_WIDTH);
        let mut segment = self.bar.x0;
        while segment + SEGMENT_WIDTH <= self.bar.x1 {
            canvas.fill(Rect::new(segment, self.bar.y0, segment + SEGMENT_WIDTH, self.bar.y1), 0.0, if segment < thumb { SELECTED } else { SEGMENT_OFF });
            segment += SEGMENT_WIDTH + SEGMENT_GAP;
        }
        canvas.fill(Rect::new(thumb, self.bar.y0 - THUMB_OVERHANG, thumb + THUMB_WIDTH, self.bar.y1 + THUMB_OVERHANG), 0.0, WHITE);

        let text = if self.layout() == Layout::List {
            String::from("List")
        } else {
            format!("{}%", libm::round(f64::from(grid_width(self.width)) / f64::from(PAGE_WIDTH) * PERCENT))
        };
        let width = canvas.text_width(&text, READOUT);
        label(canvas, Point::new(readout.x1 - width, row.y0 + LABEL_DROP - 1.0), &text, READOUT, if self.layout() == Layout::List { MUTED } else { WHITE });
    }
}
