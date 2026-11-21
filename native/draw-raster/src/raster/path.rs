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

    /// A cubic Bézier curve to `to` by `first` and `second`, flattened: as
    /// one chord where the curve keeps within [`FLATNESS`] of it, else by
    /// R. Levien's method ("Flattening cubic Béziers", the method of his
    /// 2019 article on quadratics carried over): the cubic approximated by
    /// quadratics within a tenth of the tolerance, and the chords spread
    /// over all of them evenly by the same parabola measure, within the
    /// rest of the tolerance. The tolerance and the spacing are those the
    /// interface's rounded corners were drawn with before.
    pub(crate) fn cubic_to(&mut self, first: (f64, f64), second: (f64, f64), to: (f64, f64)) {
        let from = self.at;
        // The curve lies within three quarters of its control points'
        // greater distance from its chord: their Bernstein weights sum to
        // 3t(1 − t), at most three quarters.
        let reach = distance_to_segment(first, from, to).max(distance_to_segment(second, from, to));
        if reach <= FLATNESS * 4.0 / 3.0 {
            self.line_to(to);
            return;
        }
        let count = quadratic_count(from, first, second, to);
        let cubic = |along: f64| {
            let rest = 1.0 - along;
            let weights = (rest * rest * rest, 3.0 * rest * rest * along, 3.0 * rest * along * along, along * along * along);
            (
                weights.0 * from.0 + weights.1 * first.0 + weights.2 * second.0 + weights.3 * to.0,
                weights.0 * from.1 + weights.1 * first.1 + weights.2 * second.1 + weights.3 * to.1,
            )
        };
        // Each quadratic through the cubic's points at its two ends and its
        // middle, its control point set so that it passes the middle one.
        let tolerance = FLATNESS * (1.0 - CUBIC_TO_QUADRATIC_SHARE);
        let root_tolerance = tolerance.sqrt();
        let mut pieces = [Parabola::default(); MAX_QUADRATICS];
        let mut total = 0.0;
        for (index, piece) in (0..count).zip(pieces.iter_mut()) {
            let start = cubic(f64::from(index) / f64::from(count));
            let end = cubic(f64::from(index.saturating_add(1)) / f64::from(count));
            let middle = cubic((2.0 * f64::from(index) + 1.0) / (2.0 * f64::from(count)));
            let control = (2.0 * middle.0 - f64::midpoint(start.0, end.0), 2.0 * middle.1 - f64::midpoint(start.1, end.1));
            *piece = Parabola::new(start, control, end, root_tolerance);
            total += piece.measure;
        }
        let chords = chord_count(0.5 * total / root_tolerance);
        let step = total / f64::from(chords);
        // The chords' ends are spaced `step` apart along the summed measure,
        // each found in the quadratic it falls in.
        let mut emitted = 0_u32;
        let mut summed = 0.0;
        let mut carried = 0.0;
        for piece in pieces.iter().take(usize::try_from(count).unwrap_or(0)) {
            summed += piece.measure;
            let reached = summed / step;
            let next = 1.0 + reached.floor();
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a count of chords, whole and at most MAX_CHORDS")]
            let next_index = next.clamp(0.0, f64::from(MAX_CHORDS)) as u32;
            let new = next_index.saturating_sub(emitted);
            if new > 0 && piece.measure > 0.0 {
                let spacing = step / piece.measure;
                let offset = carried * spacing;
                for index in 0..new {
                    let place = emitted.saturating_add(index);
                    // The first point is the curve's start, already drawn,
                    // and the last its end, drawn below.
                    if place == 0 || place >= chords {
                        continue;
                    }
                    let point = piece.point(offset + f64::from(index) * spacing);
                    if point.0.is_finite() && point.1.is_finite() {
                        self.line_to(point);
                    }
                }
            }
            carried = next - reached;
            emitted = next_index;
        }
        self.line_to(to);
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
/// than half its shorter side, each corner a quarter circle drawn as one
/// cubic Bézier curve (its control arms 4/3 × tan(π / 8) of the radius,
/// the usual approximation, within 0.03 % of the radius) and flattened as
/// every cubic is, as the interface's corners were drawn before.
pub(crate) fn rounded_rect(lines: &mut Vec<Line>, (x0, y0, x1, y1): (f64, f64, f64, f64), radius: f64) {
    /// A quarter circle's cubic control arm, as a share of the radius.
    const ARM: f64 = 4.0 / 3.0 * 0.414_213_562_373_095_03;
    let (x0, x1) = (x0.min(x1), x0.max(x1));
    let (y0, y1) = (y0.min(y1), y0.max(y1));
    let radius = radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
    let arm = radius * ARM;
    let mut edges = Edges::new(lines);
    // Clockwise on the screen (y down) from the top-left corner's start.
    edges.move_to((x0, y0 + radius));
    edges.cubic_to((x0, y0 + radius - arm), (x0 + radius - arm, y0), (x0 + radius, y0));
    edges.line_to((x1 - radius, y0));
    edges.cubic_to((x1 - radius + arm, y0), (x1, y0 + radius - arm), (x1, y0 + radius));
    edges.line_to((x1, y1 - radius));
    edges.cubic_to((x1, y1 - radius + arm), (x1 - radius + arm, y1), (x1 - radius, y1));
    edges.line_to((x0 + radius, y1));
    edges.cubic_to((x0 + radius - arm, y1), (x0, y1 - radius + arm), (x0, y1 - radius));
    edges.close();
}

/// The share of the tolerance a cubic's approximation by quadratics may
/// take; the chords take the rest.
const CUBIC_TO_QUADRATIC_SHARE: f64 = 0.1;
/// The most quadratics one cubic is approximated by: past this a cubic is
/// drawn coarser rather than without bound.
const MAX_QUADRATICS: usize = 16;

/// How many quadratics approximate the cubic within its share of the
/// tolerance: the error of n quadratics falls as the sixth power of n, so
/// n is the sixth root of the error measure, rounded up (Levien, "Fitting
/// cubic Béziers with quadratics": the error of one is √3 / 36 of the
/// distance between 3 × the second control point − the end and 3 × the
/// first − the start).
fn quadratic_count(from: (f64, f64), first: (f64, f64), second: (f64, f64), to: (f64, f64)) -> u32 {
    let accuracy = FLATNESS * CUBIC_TO_QUADRATIC_SHARE;
    let (across, down) = ((3.0 * second.0 - to.0) - (3.0 * first.0 - from.0), (3.0 * second.1 - to.1) - (3.0 * first.1 - from.1));
    let ratio = (across * across + down * down) / (432.0 * accuracy * accuracy);
    let mut count = 1_u32;
    while count < u32::try_from(MAX_QUADRATICS).unwrap_or(1) && ratio > f64::from(count).powi(6) {
        count = count.saturating_add(1);
    }
    count
}

/// A quadratic mapped onto the parabola y = x², with the measure of its
/// chords at a tolerance (Levien, 2019): what spreads chords along it.
#[derive(Clone, Copy, Default)]
struct Parabola {
    start: (f64, f64),
    control: (f64, f64),
    end: (f64, f64),
    integral_start: f64,
    integral_span: f64,
    parameter_start: f64,
    parameter_scale: f64,
    /// Twice the root of the tolerance times the chords it needs.
    measure: f64,
}

impl Parabola {
    fn new(start: (f64, f64), control: (f64, f64), end: (f64, f64), root_tolerance: f64) -> Self {
        let first = (control.0 - start.0, control.1 - start.1);
        let second = (end.0 - control.0, end.1 - control.1);
        let bend = (first.0 - second.0, first.1 - second.1);
        let cross = (end.0 - start.0) * bend.1 - (end.1 - start.1) * bend.0;
        let low = (first.0 * bend.0 + first.1 * bend.1) / cross;
        let high = (second.0 * bend.0 + second.1 * bend.1) / cross;
        let scale = (cross / (bend.0.hypot(bend.1) * (high - low))).abs();
        let (integral_start, integral_end) = (parabola_integral(low), parabola_integral(high));
        let measure = if scale.is_finite() {
            let span = (integral_end - integral_start).abs();
            if (low >= 0.0) == (high >= 0.0) { span * scale.sqrt() } else { span * root_tolerance / parabola_integral(root_tolerance / scale.sqrt()) }
        } else {
            0.0
        };
        let measure = if measure.is_finite() { measure } else { 0.0 };
        let (parameter_start, parameter_end) = (parabola_inverse_integral(integral_start), parabola_inverse_integral(integral_end));
        Self {
            start,
            control,
            end,
            integral_start,
            integral_span: integral_end - integral_start,
            parameter_start,
            parameter_scale: 1.0 / (parameter_end - parameter_start),
            measure,
        }
    }

    /// The point a share `along` of the way along the parabola measure.
    fn point(&self, along: f64) -> (f64, f64) {
        let integral = self.integral_start + self.integral_span * along;
        let parameter = (parabola_inverse_integral(integral) - self.parameter_start) * self.parameter_scale;
        let rest = 1.0 - parameter;
        let (weight_start, weight_middle, weight_end) = (rest * rest, 2.0 * rest * parameter, parameter * parameter);
        (
            weight_start * self.start.0 + weight_middle * self.control.0 + weight_end * self.end.0,
            weight_start * self.start.1 + weight_middle * self.control.1 + weight_end * self.end.1,
        )
    }
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
