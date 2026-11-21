//! Rasterising: the damaged box drawn again from the frame's calls, each
//! call filled with anti-aliased edges into the window's words.

pub(crate) mod coverage;
pub(crate) mod paint;
pub(crate) mod path;
pub(crate) mod surface;
#[cfg(test)]
mod tests;

use interface_core::geometry::Rect;
use interface_core::theme::Colour;

use crate::record::{Command, Frame};
use crate::{DeviceBox, Painter};
use coverage::{Coverage, Line, alpha};
use paint::{Axis, GradientPaint, ImagePaint, Pixels, Sampling, opaque};
use surface::{Rgba, Source, Surface, covered, share};

/// A rectangle in device pixels: left, top, right, bottom.
pub(crate) type DeviceRect = (f64, f64, f64, f64);

/// What a fill paints with.
pub(crate) enum Paint<'a> {
    Solid(Rgba),
    /// An image, mapped to device pixels by its two axes.
    Image {
        pixels: &'a Pixels,
        sampling: Sampling,
        across: Axis,
        down: Axis,
    },
    Gradient(&'a GradientPaint),
}

/// The working store fills share, kept from frame to frame.
#[derive(Default)]
pub(crate) struct Scratch {
    pub(crate) coverage: Coverage,
    pub(crate) lines: Vec<Line>,
    colours: Vec<Rgba>,
    words: Vec<u32>,
    mask: Vec<u8>,
}

/// Fills the device rectangle `rect` with `paint`: its edges' pixels by
/// the share of each the rectangle covers.
pub(crate) fn fill_rect(surface: &mut Surface, scratch: &mut Scratch, rect: DeviceRect, paint: &Paint<'_>) {
    let (x0, x1) = (rect.0.min(rect.2), rect.0.max(rect.2));
    let (y0, y1) = (rect.1.min(rect.3), rect.1.max(rect.3));
    let scissor = surface.scissor();
    let (left, right) = covered(x0, x1, scissor.x0, scissor.x1);
    let (top, bottom) = covered(y0, y1, scissor.y0, scissor.y1);
    if left >= right || top >= bottom {
        return;
    }
    let width = usize::from(right.saturating_sub(left));
    let mut image = match *paint {
        Paint::Image { pixels, sampling, across, down } => Some(ImagePaint::new(pixels, sampling, across, down, left, right)),
        _ => None,
    };
    // The share of each column across the rectangle covers: only its first
    // and last can be less than whole.
    let last = right.saturating_sub(1);
    let column_share = |column: u16| if column == left || column == last { share(x0, x1, column) } else { 1.0 };
    let (left_share, right_share) = (column_share(left), column_share(last));
    let Scratch { colours, words, mask, .. } = scratch;
    for row in top..bottom {
        let source = match (paint, &mut image) {
            (Paint::Solid(colour), _) => Source::Solid(*colour),
            (Paint::Gradient(gradient), _) => {
                gradient.row(row, left, width, colours);
                Source::Row(colours)
            }
            (Paint::Image { .. }, Some(image)) => {
                image.row(row, left, width, words);
                Source::Opaque(words)
            }
            (Paint::Image { .. }, None) => return,
        };
        let down = share(y0, y1, row);
        if alpha(down) == u8::MAX {
            // A whole row: its first and last pixels by their share, the
            // rest whole.
            let (mut start, mut end) = (0, width);
            if alpha(left_share) < u8::MAX {
                surface.span(row, left, 1, source, Some(&[alpha(left_share)]));
                start = 1;
            }
            if end > start && alpha(right_share) < u8::MAX {
                end = end.saturating_sub(1);
                let at = left.saturating_add(u16::try_from(end).unwrap_or(u16::MAX));
                surface.span(row, at, 1, source.skip(end), Some(&[alpha(right_share)]));
            }
            if end > start {
                let at = left.saturating_add(u16::try_from(start).unwrap_or(u16::MAX));
                surface.span(row, at, end.saturating_sub(start), source.skip(start), None);
            }
        } else {
            mask.clear();
            mask.extend((left..right).map(|column| alpha(column_share(column) * down)));
            surface.span(row, left, width, source, Some(mask));
        }
    }
}

/// Fills the inside of the edges in `scratch.lines` with `paint`, one
/// colour or a gradient.
pub(crate) fn fill_lines(surface: &mut Surface, scratch: &mut Scratch, paint: &Paint<'_>) {
    let Scratch { coverage, lines, colours, .. } = scratch;
    let area = coverage::bounds(lines, surface.scissor());
    coverage.fill(lines, area, |row, column, length, mask| match paint {
        Paint::Solid(colour) => surface.span(row, column, length, Source::Solid(*colour), mask),
        Paint::Gradient(gradient) => {
            gradient.row(row, column, length, colours);
            surface.span(row, column, length, Source::Row(colours), mask);
        }
        Paint::Image { .. } => {}
    });
}

impl Painter {
    /// Draws `damage` from `frame`'s calls, those that reach it, into the
    /// window's copy.
    pub(crate) fn draw_box(&mut self, frame: &Frame, damage: DeviceBox) {
        self.surface.begin(damage);
        for call in frame.calls() {
            match call.command() {
                Command::PushClip(area) => {
                    let (x0, y0, x1, y1) = self.device(*area);
                    self.surface.push_clip(x0, y0, x1, y1);
                }
                Command::PopClip => self.surface.pop_clip(),
                _ if !call.bounds().meets(damage) => {}
                Command::Fill { area, radius, paint } => self.draw_fill(*area, *radius, *paint),
                Command::Polygon { points, paint } => self.draw_polygon(points, *paint),
                Command::Dim { area, amount } => self.draw_dim(*area, *amount),
                Command::Gradient { area, radius, stops } => self.draw_gradient(*area, *radius, stops),
                Command::Page { key, area, .. } => self.draw_page(*key, *area),
                Command::Text { area, text, style, paint } => self.draw_text(*area, text, *style, *paint),
                Command::Still { still, area, .. } => self.draw_still(*still, *area),
            }
        }
        self.surface.end();
    }

    /// A logical rectangle in device pixels.
    pub(crate) fn device(&self, rect: Rect) -> DeviceRect {
        (rect.x0 * self.scale, rect.y0 * self.scale, rect.x1 * self.scale, rect.y1 * self.scale)
    }

    pub(crate) fn draw_fill(&mut self, area: Rect, radius: f64, paint: Colour) {
        let rect = self.device(area);
        if radius > 0.0 {
            self.scratch.lines.clear();
            path::rounded_rect(&mut self.scratch.lines, rect, radius * self.scale);
            fill_lines(&mut self.surface, &mut self.scratch, &Paint::Solid(opaque(paint)));
        } else {
            fill_rect(&mut self.surface, &mut self.scratch, rect, &Paint::Solid(opaque(paint)));
        }
    }

    fn draw_polygon(&mut self, points: &[interface_core::geometry::Point], paint: Colour) {
        let scale = self.scale;
        self.scratch.lines.clear();
        path::polygon(&mut self.scratch.lines, points.iter().map(|point| (point.x * scale, point.y * scale)));
        fill_lines(&mut self.surface, &mut self.scratch, &Paint::Solid(opaque(paint)));
    }

    fn draw_dim(&mut self, area: Rect, amount: f32) {
        // Black at `amount`'s strength, premultiplied: no colour, only alpha.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0..=255 and rounded")]
        let strength = (amount.clamp(0.0, 1.0) * f32::from(u8::MAX)).round() as u8;
        let rect = self.device(area);
        fill_rect(&mut self.surface, &mut self.scratch, rect, &Paint::Solid([0, 0, 0, strength]));
    }

    fn draw_gradient(&mut self, area: Rect, radius: f64, stops: &[(f32, Colour)]) {
        let rect = self.device(area);
        let gradient = GradientPaint::new((rect.0, rect.1), (rect.2, rect.3), stops);
        self.scratch.lines.clear();
        path::rounded_rect(&mut self.scratch.lines, rect, radius * self.scale);
        fill_lines(&mut self.surface, &mut self.scratch, &Paint::Gradient(&gradient));
    }

    /// Draws the part `part` of an image, in its own pixels, with the whole
    /// image laid over the logical rectangle `place`.
    pub(crate) fn draw_image(&mut self, pixels: &Pixels, sampling: Sampling, place: Rect, part: DeviceRect) {
        let (width, height) = (f64::from(pixels.width()), f64::from(pixels.height()));
        let (across, down) = (place.width() / width, place.height() / height);
        if !(across > 0.0 && down > 0.0 && across.is_finite() && down.is_finite()) {
            return;
        }
        let scale = self.scale;
        let rect = (
            (place.x0 + part.0 * across) * scale,
            (place.y0 + part.1 * down) * scale,
            (place.x0 + part.2 * across) * scale,
            (place.y0 + part.3 * down) * scale,
        );
        // A device pixel to the image's: the inverse of the placement.
        let across = Axis { scale: 1.0 / (across * scale), offset: -place.x0 / across };
        let down = Axis { scale: 1.0 / (down * scale), offset: -place.y0 / down };
        fill_rect(&mut self.surface, &mut self.scratch, rect, &Paint::Image { pixels, sampling, across, down });
    }
}
