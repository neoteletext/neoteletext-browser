//! Paths as straight edges in device pixels: polygons, rounded rectangles
//! and curves, flattened.

use crate::raster::coverage::Line;

/// How far a flattened curve's chords may lie from the curve, in device
/// pixels: a quarter of a pixel, the tolerance the interface's text and
/// shapes were drawn with before this rasteriser, with which every glyph
/// of the shipped fonts measured comes out the same, pixel for pixel
/// (DEC-7168).
pub(crate) const FLATNESS: f64 = 0.25;
/// The most chords one curve is flattened into: a quarter circle of a
/// radius of the largest window needs fewer at [`FLATNESS`], and a curve
/// past it is drawn coarser rather than without bound.
pub(crate) const MAX_CHORDS: u32 = 256;
const QUARTER_TURN: f64 = core::f64::consts::FRAC_PI_2;

/// How many chords keep a curve whose second difference is `bend` within
/// [`FLATNESS`]: a chord of a parameter step `h` lies at most `bend × h² /
/// 8` from the curve (the remainder of linear interpolation), so `h ≤ √(8
/// × FLATNESS / bend)`.
fn chords(bend: f64) -> u32 {
    chord_count((bend / (8.0 * FLATNESS)).sqrt())
}

/// A path made into edges as it is drawn: moves, lines, curves and closes.
pub(crate) struct Edges<'a> {
    lines: &'a mut Vec<Line>,
    start: (f64, f64),
    at: (f64, f64),
}

impl<'a> Edges<'a> {
    /// A path whose edges go into `lines`.
    pub(crate) fn new(lines: &'a mut Vec<Line>) -> Self {
        Self { lines, start: (0.0, 0.0), at: (0.0, 0.0) }
    }

    /// Starts a contour at `point`, closing the last.
    pub(crate) fn move_to(&mut self, point: (f64, f64)) {
        self.close();
        self.start = point;
        self.at = point;
    }

    /// An edge from the current point to `point`.
    pub(crate) fn line_to(&mut self, point: (f64, f64)) {
        #[expect(clippy::cast_possible_truncation, reason = "a device coordinate, far inside f32's range")]
        self.lines.push(Line { x0: self.at.0 as f32, y0: self.at.1 as f32, x1: point.0 as f32, y1: point.1 as f32 });
        self.at = point;
    }

    /// A quadratic Bézier curve to `to` by `control`, flattened: as one
    /// chord where the curve keeps within [`FLATNESS`] of it, else into
    /// chords spaced evenly along the curve's parabola, so that each lies
    /// within [`FLATNESS`] of it (R. Levien, "Flattening quadratic Béziers",
    /// 2019: the curve mapped onto the parabola y = x², and the chords
    /// spread by an approximation of the integral of the square root of
    /// its curvature). The tolerance and the spacing are those the text was
    /// drawn with before, so glyphs come out the same.
    pub(crate) fn quad_to(&mut self, control: (f64, f64), to: (f64, f64)) {
        let from = self.at;
        // The curve lies within half its control point's distance of its
        // chord: the control point's Bernstein weight, 2t(1 − t), is at
        // most a half.
        if distance_to_segment(control, from, to) <= 2.0 * FLATNESS {
            self.line_to(to);
            return;
        }
        let first = (control.0 - from.0, control.1 - from.1);
        let second = (to.0 - control.0, to.1 - control.1);
        let bend = (first.0 - second.0, first.1 - second.1);
        let cross = (to.0 - from.0) * bend.1 - (to.1 - from.1) * bend.0;
        let start = (first.0 * bend.0 + first.1 * bend.1) / cross;
        let end = (second.0 * bend.0 + second.1 * bend.1) / cross;
        let scale = (cross / (bend.0.hypot(bend.1) * (end - start))).abs();
        let (integral_start, integral_end) = (parabola_integral(start), parabola_integral(end));
        let root_tolerance = FLATNESS.sqrt();
        let measure = if scale.is_finite() {
            let span = (integral_end - integral_start).abs();
            let same_side = start.signum() == end.signum();
            if same_side {
                span * scale.sqrt()
            } else {
                // The curve passes the parabola's vertex, its tightest bend.
                span * root_tolerance / parabola_integral(root_tolerance / scale.sqrt())
            }
        } else {
            0.0
        };
        let count = chord_count(0.5 * measure / root_tolerance);
        let (parameter_start, parameter_end) = (parabola_inverse_integral(integral_start), parabola_inverse_integral(integral_end));
        for step in 1..count {
            let integral = integral_start + (integral_end - integral_start) * f64::from(step) / f64::from(count);
            let along = (parabola_inverse_integral(integral) - parameter_start) / (parameter_end - parameter_start);
            let rest = 1.0 - along;
            // The Bernstein weights of the three points.
            let (weight_start, weight_middle, weight_end) = (rest * rest, 2.0 * rest * along, along * along);
            self.line_to((
                weight_start * from.0 + weight_middle * control.0 + weight_end * to.0,
                weight_start * from.1 + weight_middle * control.1 + weight_end * to.1,
            ));
        }
        self.line_to(to);
    }

    /// A cubic Bézier curve to `to` by `first` and `second`, flattened.
    pub(crate) fn cubic_to(&mut self, first: (f64, f64), second: (f64, f64), to: (f64, f64)) {
        let from = self.at;
        // Its second derivative is linear along it, so greatest at an end:
        // six times the second difference there.
        let at_start = (from.0 - 2.0 * first.0 + second.0).hypot(from.1 - 2.0 * first.1 + second.1);
        let at_end = (first.0 - 2.0 * second.0 + to.0).hypot(first.1 - 2.0 * second.1 + to.1);
        let count = chords(6.0 * at_start.max(at_end));
        for step in 1..=count {
            let along = f64::from(step) / f64::from(count);
            let rest = 1.0 - along;
            // The Bernstein weights of the four points.
            let weights = (rest * rest * rest, 3.0 * rest * rest * along, 3.0 * rest * along * along, along * along * along);
            self.line_to((
                weights.0 * from.0 + weights.1 * first.0 + weights.2 * second.0 + weights.3 * to.0,
                weights.0 * from.1 + weights.1 * first.1 + weights.2 * second.1 + weights.3 * to.1,
            ));
        }
    }

    /// Closes the contour back to its start.
    pub(crate) fn close(&mut self) {
        if self.at != self.start {
            self.line_to(self.start);
        }
        self.at = self.start;
    }
}

/// A polygon through `points`, closed.
pub(crate) fn polygon(lines: &mut Vec<Line>, points: impl IntoIterator<Item = (f64, f64)>) {
    let mut edges = Edges::new(lines);
    let mut points = points.into_iter();
    let Some(first) = points.next() else { return };
    edges.move_to(first);
    for point in points {
        edges.line_to(point);
    }
    edges.close();
}

/// The rectangle `x0..x1` by `y0..y1` with corners of `radius`, no more
/// than half its shorter side, as chords of each corner's quarter circle
/// lying within [`FLATNESS`] of it.
pub(crate) fn rounded_rect(lines: &mut Vec<Line>, (x0, y0, x1, y1): (f64, f64, f64, f64), radius: f64) {
    let (x0, x1) = (x0.min(x1), x0.max(x1));
    let (y0, y1) = (y0.min(y1), y0.max(y1));
    let radius = radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
    // A chord of angle θ lies r × (1 − cos(θ / 2)) inside its arc.
    let step = if radius > FLATNESS { 2.0 * (1.0 - FLATNESS / radius).acos() } else { QUARTER_TURN };
    let count = chords_for_angle(step);
    // Each corner's centre and the angle its arc starts at, clockwise on
    // the screen (y down) from the top-left.
    let corners = [
        (x0 + radius, y0 + radius, 2.0 * QUARTER_TURN),
        (x1 - radius, y0 + radius, 3.0 * QUARTER_TURN),
        (x1 - radius, y1 - radius, 0.0),
        (x0 + radius, y1 - radius, QUARTER_TURN),
    ];
    let mut points = Vec::with_capacity(corners.len().saturating_mul(usize::try_from(count).unwrap_or(0).saturating_add(1)));
    for (centre_x, centre_y, start) in corners {
        for step in 0..=count {
            let angle = start + QUARTER_TURN * f64::from(step) / f64::from(count);
            points.push((centre_x + radius * angle.cos(), centre_y + radius * angle.sin()));
        }
    }
    polygon(lines, points);
}

/// The distance from `point` to the segment from `start` to `end`.
fn distance_to_segment(point: (f64, f64), start: (f64, f64), end: (f64, f64)) -> f64 {
    let (across, down) = (end.0 - start.0, end.1 - start.1);
    let length = across.mul_add(across, down * down);
    let along = if length > 0.0 { (((point.0 - start.0) * across + (point.1 - start.1) * down) / length).clamp(0.0, 1.0) } else { 0.0 };
    (point.0 - start.0 - along * across).hypot(point.1 - start.1 - along * down)
}

/// An approximation of the integral of the fourth root of the parabola y =
/// x²'s curvature from its vertex, the measure along it by which chords
/// of equal error are spaced evenly (Levien, 2019; its constant 0.67 fits
/// the integral closely over the whole line).
fn parabola_integral(x: f64) -> f64 {
    const FIT: f64 = 0.67;
    x / (1.0 - FIT + (FIT.powi(4) + 0.25 * x * x).sqrt().sqrt())
}

/// The inverse of [`parabola_integral`], approximated in the same way
/// (Levien, 2019; its constant 0.39).
fn parabola_inverse_integral(x: f64) -> f64 {
    const FIT: f64 = 0.39;
    x * (1.0 - FIT + (FIT * FIT + 0.25 * x * x).sqrt())
}

/// A count of chords, `needed` rounded up, at least one and at most
/// [`MAX_CHORDS`].
fn chord_count(needed: f64) -> u32 {
    let needed = needed.ceil();
    if needed.is_nan() || needed < 1.0 {
        1
    } else if needed >= f64::from(MAX_CHORDS) {
        MAX_CHORDS
    } else {
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "whole and between 1 and MAX_CHORDS")]
        let count = needed as u32;
        count
    }
}

/// How many equal chords of a quarter turn are each at most `step`.
fn chords_for_angle(step: f64) -> u32 {
    chord_count(QUARTER_TURN / step)
}
