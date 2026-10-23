//! Rasterising: the damaged box drawn again from the frame's calls, and
//! each call as `vello_cpu` draws it.

use interface_core::geometry::Rect;
use interface_core::theme::Colour;
use vello_cpu::Pixmap;
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, BezPath, Point, RoundedRect, Shape};
use vello_cpu::peniko::Gradient;

use crate::record::{Command, Frame};
use crate::{DeviceBox, Painter, TOLERANCE, colour, point, rect};

impl Painter {
    /// Draws `damage` from `frame`'s calls, those that reach it, and writes
    /// it into the window's copy.
    pub(crate) fn draw_box(&mut self, frame: &Frame, damage: DeviceBox) {
        let (width, height) = (damage.width(), damage.height());
        if self.context.width() == width && self.context.height() == height {
            self.context.reset();
        } else {
            self.context.reset_and_resize(width, height);
            self.target = Pixmap::new(width, height);
        }
        self.origin = (f64::from(damage.x0), f64::from(damage.y0));
        for call in &frame.calls {
            match &call.command {
                Command::PushClip(area) => self.draw_push_clip(*area),
                Command::PopClip => self.context.pop_layer(),
                _ if !call.bounds.meets(damage) => {}
                Command::Fill { area, radius, paint } => self.draw_fill(*area, *radius, *paint),
                Command::Polygon { points, paint } => self.draw_polygon(points, *paint),
                Command::Dim { area, amount } => self.draw_dim(*area, *amount),
                Command::Gradient { area, radius, stops } => self.draw_gradient(*area, *radius, stops),
                Command::Page { key, area, .. } => self.draw_page(*key, *area),
                Command::Text { area, text, style, paint } => self.draw_text(*area, text, *style, *paint),
            }
        }
        self.context.flush();
        self.context.render(&mut self.target, &mut self.resources);
        let (full, part) = (usize::from(self.width), usize::from(width));
        for (row, pixels) in (usize::from(damage.y0)..).zip(self.target.data().chunks_exact(part.max(1))) {
            let start = row * full + usize::from(damage.x0);
            let Some(words) = self.words.get_mut(start..start + part) else { break };
            // Every pixel drawn is opaque, so the premultiplied colour is
            // the colour itself.
            for (word, pixel) in words.iter_mut().zip(pixels) {
                *word = u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b);
            }
        }
    }

    /// Logical pixels to the damaged box's device pixels.
    pub(crate) fn base(&self) -> Affine {
        Affine::translate((-self.origin.0, -self.origin.1)) * Affine::scale(self.scale)
    }
}

/// The calls as `vello_cpu` draws them, into the damaged box.
impl Painter {
    fn draw_fill(&mut self, area: Rect, radius: f64, paint: Colour) {
        self.context.set_transform(self.base());
        self.context.set_paint(colour(paint));
        if radius > 0.0 {
            self.context.fill_path(&RoundedRect::from_rect(rect(area), radius).to_path(TOLERANCE));
        } else {
            self.context.fill_rect(&rect(area));
        }
    }

    fn draw_polygon(&mut self, points: &[interface_core::geometry::Point], paint: Colour) {
        let mut path = BezPath::new();
        let mut points = points.iter().map(|&at| point(at));
        let Some(first) = points.next() else { return };
        path.move_to(first);
        for at in points {
            path.line_to(at);
        }
        path.close_path();
        self.context.set_transform(self.base());
        self.context.set_paint(colour(paint));
        self.context.fill_path(&path);
    }

    fn draw_dim(&mut self, area: Rect, amount: f32) {
        self.context.set_transform(self.base());
        self.context.set_paint(AlphaColor::<Srgb>::new([0.0, 0.0, 0.0, amount.min(1.0)]));
        self.context.fill_rect(&rect(area));
    }

    fn draw_push_clip(&mut self, area: Rect) {
        self.context.set_transform(self.base());
        self.context.push_clip_layer(&rect(area).to_path(TOLERANCE));
    }

    fn draw_gradient(&mut self, area: Rect, radius: f64, stops: &[(f32, Colour)]) {
        let stops: Vec<(f32, AlphaColor<Srgb>)> = stops.iter().map(|&(offset, paint)| (offset, colour(paint))).collect();
        let gradient = Gradient::new_linear(Point::new(area.x0, area.y0), Point::new(area.x1, area.y1)).with_stops(stops.as_slice());
        self.context.set_transform(self.base());
        self.context.set_paint(gradient);
        self.context.fill_path(&RoundedRect::from_rect(rect(area), radius).to_path(TOLERANCE));
    }
}
