//! Anti-aliased coverage of a filled path: the exact area of each pixel
//! the path's inside covers, by the non-zero winding rule.
//!
//! Each straight edge adds, to every pixel it crosses, the signed area of
//! that pixel lying to the right of it, and to the pixel after, the rest
//! of its height; summed along a row, these give the winding-weighted area
//! of each pixel inside the path (the signed-area accumulation of
//! scan-converting rasterisers: Foley, van Dam, Feiner and Hughes,
//! "Computer Graphics: Principles and Practice", 2nd edition, section
//! 19.3, area sampling). A pixel's coverage is that area's magnitude, at
//! most one: the non-zero rule.

use crate::DeviceBox;

/// The alpha of a pixel covered whole.
const FULL: f32 = 255.0;
/// Rounds a coverage to the nearest alpha.
const HALF: f32 = 0.5;

/// A coverage from 0 to 1 as an alpha from 0 to 255, rounded to nearest.
pub(crate) fn alpha(coverage: f32) -> u8 {
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "between 0 and 255 by the clamp")]
    let alpha = (coverage.abs() * FULL + HALF).min(FULL) as u8;
    alpha
}

/// A straight edge of a path, in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    pub(crate) x0: f32,
    pub(crate) y0: f32,
    pub(crate) x1: f32,
    pub(crate) y1: f32,
}

/// The box of device pixels holding every one of `lines`, inside `limit`.
pub(crate) fn bounds(lines: &[Line], limit: DeviceBox) -> DeviceBox {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for line in lines {
        x0 = x0.min(line.x0).min(line.x1);
        y0 = y0.min(line.y0).min(line.y1);
        x1 = x1.max(line.x0).max(line.x1);
        y1 = y1.max(line.y0).max(line.y1);
    }
    if lines.is_empty() || !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return DeviceBox::default();
    }
    let pixel = |value: f32, limit: u16| -> u16 {
        // Clamped into the box's range first, so the conversion is exact.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0..=u16::MAX, whole")]
        let pixel = value.clamp(0.0, f32::from(limit)) as u16;
        pixel
    };
    let found = DeviceBox { x0: pixel(x0.floor(), limit.x1), y0: pixel(y0.floor(), limit.y1), x1: pixel(x1.ceil(), limit.x1), y1: pixel(y1.ceil(), limit.y1) };
    found.intersect(limit)
}

/// The rasteriser's working store, kept from fill to fill.
#[derive(Default)]
pub(crate) struct Coverage {
    /// For each row of the box, one more than its width of signed area:
    /// the coverage of a pixel is the sum of the row up to it.
    cells: Vec<f32>,
    /// For each row, the first and last cell an edge reached.
    touched: Vec<(usize, usize)>,
    /// A row's coverage, as alphas.
    mask: Vec<u8>,
}

impl Coverage {
    /// Fills the inside of `lines` within `area`: hands each run of a row
    /// that is covered to `run`, by its row, first column, length and
    /// alphas, `None` where every pixel of it is covered whole.
    pub(crate) fn fill(&mut self, lines: &[Line], area: DeviceBox, mut run: impl FnMut(u16, u16, usize, Option<&[u8]>)) {
        if area.is_empty() {
            return;
        }
        let (width, height) = (usize::from(area.width()), usize::from(area.height()));
        let stride = width.saturating_add(1);
        self.cells.clear();
        self.cells.resize(stride.saturating_mul(height), 0.0);
        self.touched.clear();
        self.touched.resize(height, (usize::MAX, 0));
        let origin = (f32::from(area.x0), f32::from(area.y0));
        for line in lines {
            self.add(*line, origin, width, height);
        }
        for ((row, cells), &(first, last)) in (area.y0..).zip(self.cells.chunks_exact(stride)).zip(&self.touched) {
            if first > last {
                continue;
            }
            // Before the first cell an edge reached, nothing is covered;
            // after the last, the coverage holds to the row's end.
            let mut sum = 0.0;
            self.mask.clear();
            for &cell in cells.get(first..=last.min(width.saturating_sub(1))).unwrap_or_default() {
                sum += cell;
                self.mask.push(alpha(sum));
            }
            let start = area.x0.saturating_add(u16::try_from(first).unwrap_or(u16::MAX));
            run(row, start, self.mask.len(), Some(&self.mask));
            let tail = alpha(sum);
            let from = last.saturating_add(1);
            if tail > 0 && from < width {
                let column = area.x0.saturating_add(u16::try_from(from).unwrap_or(u16::MAX));
                let length = width.saturating_sub(from);
                if tail == u8::MAX {
                    run(row, column, length, None);
                } else {
                    self.mask.clear();
                    self.mask.resize(length, tail);
                    run(row, column, length, Some(&self.mask));
                }
            }
        }
    }

    /// Adds one edge, `origin` the box's top-left corner.
    fn add(&mut self, line: Line, origin: (f32, f32), width: usize, height: usize) {
        let (x0, y0, x1, y1) = (line.x0 - origin.0, line.y0 - origin.1, line.x1 - origin.0, line.y1 - origin.1);
        #[expect(clippy::float_cmp, reason = "a level edge, exactly, adds no area")]
        let level = y0 == y1;
        if level || !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return;
        }
        // Downward, with the edge's direction as its sign.
        let (sign, (top_x, top_y), (bottom_x, bottom_y)) = if y0 < y1 { (1.0, (x0, y0), (x1, y1)) } else { (-1.0, (x1, y1), (x0, y0)) };
        #[expect(clippy::cast_precision_loss, reason = "a box's height, at most u16::MAX")]
        let rows = height as f32;
        let (from, to) = (top_y.max(0.0), bottom_y.min(rows));
        if from >= to {
            return;
        }
        let slope = (bottom_x - top_x) / (bottom_y - top_y);
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "between 0 and the box's height by the clamps above")]
        let (first, last) = (from.floor() as usize, (to.ceil() as usize).min(height));
        for row in first..last {
            #[expect(clippy::cast_precision_loss, reason = "a row of a box, at most u16::MAX")]
            let top = row as f32;
            let (start, end) = (from.max(top), to.min(top + 1.0));
            if start >= end {
                continue;
            }
            let (start_x, end_x) = (top_x + (start - top_y) * slope, top_x + (end - top_y) * slope);
            self.add_piece(row, start_x, end_x, (end - start) * sign, width);
        }
    }

    /// Adds the piece of an edge inside one row, from `start_x` to `end_x`,
    /// of signed height `height`.
    fn add_piece(&mut self, row: usize, start_x: f32, end_x: f32, height: f32, width: usize) {
        let stride = width.saturating_add(1);
        #[expect(clippy::cast_precision_loss, reason = "a box's width, at most u16::MAX")]
        let right_edge = width as f32;
        let (mut low, mut high) = (start_x.min(end_x), start_x.max(end_x));
        if low >= right_edge {
            // Right of the box: it covers nothing in it.
            return;
        }
        let Some(cells) = self.cells.get_mut(row.saturating_mul(stride)..row.saturating_mul(stride).saturating_add(stride)) else { return };
        let Some(touched) = self.touched.get_mut(row) else { return };
        let mut add = |cell: usize, value: f32| {
            if let Some(slot) = cells.get_mut(cell) {
                *slot += value;
                touched.0 = touched.0.min(cell);
                touched.1 = touched.1.max(cell);
            }
        };
        let spread = high - low;
        // The edge's height is spread evenly along its run across the row.
        let mut height = height;
        if spread > 0.0 {
            if high > right_edge {
                height *= (right_edge - low) / spread;
                high = right_edge;
            }
            if low < 0.0 {
                // Left of the box, the edge covers the whole of every pixel
                // in it, as an edge at its left side would.
                let left = height * (high.min(0.0) - low) / (high - low);
                add(0, left);
                height -= left;
                low = 0.0;
            }
        } else if low < 0.0 {
            add(0, height);
            return;
        }
        let spread = high - low;
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "between 0 and the box's width by the cuts above")]
        let mut cell = low.floor() as usize;
        if spread <= 0.0 {
            #[expect(clippy::cast_precision_loss, reason = "a column of a box, at most u16::MAX")]
            let inside = low - cell as f32;
            add(cell, height * (1.0 - inside));
            add(cell.saturating_add(1), height * inside);
            return;
        }
        let mut at = low;
        while at < high && cell < width {
            #[expect(clippy::cast_precision_loss, reason = "a column of a box, at most u16::MAX")]
            let left = cell as f32;
            let next = high.min(left + 1.0);
            let part = height * (next - at) / spread;
            let middle = (at + next) * HALF - left;
            add(cell, part * (1.0 - middle));
            add(cell.saturating_add(1), part * middle);
            at = next;
            cell = cell.saturating_add(1);
        }
    }
}
