//! The vector glyphs the window draws itself, so that no icon font is
//! needed: the overview's chevron, the panels' close cross and the
//! console's maximise and restore squares, Back's and Forward's arrows,
//! the speaker, the padlock, the house, the reload arrow, the gear and
//! the star, each with the measurements it is drawn from.

use alloc::vec::Vec;

use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::text::{border, cross};
use crate::theme::{BUTTON, Colour, FIELD};

pub(super) const WINDOW_GLYPH: f64 = 14.0;
pub(super) const WINDOW_GLYPH_THICKNESS: f64 = 3.0;
/// The speaker glyph's half height; its parts are fractions of it.
pub(super) const SPEAKER_HALF: f64 = 7.0;
/// The speaker's lines: its sound waves and the cross of a muted one.
pub(super) const SPEAKER_LINE: f64 = 2.0;
pub(super) const LOCK_WIDTH: f64 = 16.0;
pub(super) const LOCK_HEIGHT: f64 = 12.0;
pub(super) const SHACKLE_WIDTH: f64 = 10.0;
pub(super) const SHACKLE_HEIGHT: f64 = 8.0;
pub(super) const SHACKLE_THICKNESS: f64 = 2.5;
/// The star's size, in the strip and beside a bookmark's suggestion.
pub(super) const STAR_RADIUS: f64 = 11.0;
pub(super) const STAR_INNER: f64 = 0.45;
pub(super) const STAR_POINTS: usize = 5;
pub(super) const ARROW_LENGTH: f64 = 20.0;
pub(super) const ARROW_HEAD: f64 = 9.0;
pub(super) const ARROW_THICKNESS: f64 = 3.0;
/// The Home button's house: its width, the roof's height, and the body's.
pub(super) const HOUSE_WIDTH: f64 = 20.0;
pub(super) const ROOF_HEIGHT: f64 = 9.0;
pub(super) const HOUSE_BODY_HEIGHT: f64 = 9.0;
pub(super) const HOUSE_BODY_INSET: f64 = 3.0;
pub(super) const DOOR_WIDTH: f64 = 5.0;
pub(super) const DOOR_HEIGHT: f64 = 6.0;
/// The Reload button's circling arrow: the ring's radius to its middle,
/// its thickness, the gap at the top right where the arrow's head is, the
/// head's length, and the corners the curve is drawn with.
pub(super) const RELOAD_RADIUS: f64 = 8.0;
pub(super) const RELOAD_THICKNESS: f64 = 3.0;
pub(super) const RELOAD_GAP_DEGREES: f64 = 70.0;
pub(super) const RELOAD_HEAD: f64 = 6.0;
/// How far the head's base reaches either side of the ring's middle.
pub(super) const RELOAD_HEAD_REACH: f64 = 5.5;
pub(super) const RELOAD_SEGMENTS: usize = 24;
/// The gear: its teeth, their reach, and the hub.
pub(super) const GEAR_TEETH: usize = 8;
pub(super) const GEAR_OUTER: f64 = 11.0;
pub(super) const GEAR_INNER: f64 = 8.0;
pub(super) const GEAR_HUB: f64 = 3.5;
pub(super) const CHEVRON_WIDTH: f64 = 12.0;
pub(super) const CHEVRON_HEIGHT: f64 = 6.0;
pub(super) const CHEVRON_THICKNESS: f64 = 2.5;
/// A docked panel's close cross, as wide as the chevron beside it,
/// reaching half the glyph each way, as the zoomed page's does.
pub(super) const CLOSE_GLYPH: f64 = CHEVRON_WIDTH;
pub(super) const CLOSE_THICKNESS: f64 = CHEVRON_THICKNESS;
/// The console's maximise square, as wide as the chevron and the cross
/// beside it, its lines thinner than theirs since a square's edges are
/// straight; restored, a second square sits behind it this far up and to
/// the right, as the common desktops draw their restore button.
pub(super) const MAXIMISE_GLYPH: f64 = CHEVRON_WIDTH;
pub(super) const MAXIMISE_THICKNESS: f64 = 2.0;
pub(super) const RESTORE_OFFSET: f64 = 3.0;

/// A chevron in `rect`, pointing down or up.
pub(super) fn chevron<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, down: bool, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let (tip, base) = if down { (CHEVRON_HEIGHT / 2.0, -CHEVRON_HEIGHT / 2.0) } else { (-CHEVRON_HEIGHT / 2.0, CHEVRON_HEIGHT / 2.0) };
    let thickness = if down { -CHEVRON_THICKNESS } else { CHEVRON_THICKNESS };
    let half = CHEVRON_WIDTH / 2.0;
    canvas.polygon(
        &[
            Point::new(centre.x - half, centre.y + base),
            Point::new(centre.x, centre.y + tip),
            Point::new(centre.x + half, centre.y + base),
            Point::new(centre.x + half, centre.y + base + thickness),
            Point::new(centre.x, centre.y + tip + thickness),
            Point::new(centre.x - half, centre.y + base + thickness),
        ],
        colour,
    );
}

/// A docked panel's close cross, centred in `rect`.
pub(super) fn close_glyph<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, colour: Colour) {
    cross(canvas, Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1)), CLOSE_GLYPH / 2.0, CLOSE_THICKNESS, colour);
}

/// The maximise square centred in `rect`, or, once maximised, the restore
/// glyph: a smaller square in front of another, the front one filled
/// with the button's `background` so it hides the back one's corner.
pub(super) fn maximise_glyph<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, maximised: bool, background: Colour, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let half = MAXIMISE_GLYPH / 2.0;
    let square = Rect::new(centre.x - half, centre.y - half, centre.x + half, centre.y + half);
    if !maximised {
        border(canvas, square, MAXIMISE_THICKNESS, colour);
        return;
    }
    let back = Rect::new(square.x0 + RESTORE_OFFSET, square.y0, square.x1, square.y1 - RESTORE_OFFSET);
    let front = Rect::new(square.x0, square.y0 + RESTORE_OFFSET, square.x1 - RESTORE_OFFSET, square.y1);
    border(canvas, back, MAXIMISE_THICKNESS, colour);
    canvas.fill(front, 0.0, background);
    border(canvas, front, MAXIMISE_THICKNESS, colour);
}

/// A left- or right-pointing arrow centred in `rect`.
pub(super) fn arrow<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, pointing_left: bool, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let sign = if pointing_left { -1.0 } else { 1.0 };
    let tip = Point::new(centre.x + sign * ARROW_LENGTH / 2.0, centre.y);
    let tail = centre.x - sign * ARROW_LENGTH / 2.0;
    let shaft = Rect::new(
        tail.min(tip.x - sign * ARROW_THICKNESS),
        centre.y - ARROW_THICKNESS / 2.0,
        tail.max(tip.x - sign * ARROW_THICKNESS),
        centre.y + ARROW_THICKNESS / 2.0,
    );
    canvas.fill(shaft, 0.0, colour);
    for direction in [1.0, -1.0] {
        let back = Point::new(tip.x - sign * ARROW_HEAD, tip.y + direction * ARROW_HEAD);
        let thick = sign * ARROW_THICKNESS * 0.7;
        canvas.polygon(&[tip, back, Point::new(back.x + thick, back.y), Point::new(tip.x + thick, tip.y)], colour);
    }
}

/// A loudspeaker centred in `rect`: with sound waves, or crossed out when
/// muted, as Chrome's tab shows it.
pub(super) fn speaker_glyph<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, muted: bool, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let half = SPEAKER_HALF;
    // The magnet: a square of half the height, left of centre.
    let back = centre.x - half;
    canvas.fill(Rect::new(back, centre.y - half / 2.0, back + half / 2.0, centre.y + half / 2.0), 0.0, colour);
    // The cone opens from the magnet to the full height at the centre.
    let neck = back + half / 2.0;
    canvas.polygon(
        &[
            Point::new(neck, centre.y - half / 2.0),
            Point::new(centre.x, centre.y - half),
            Point::new(centre.x, centre.y + half),
            Point::new(neck, centre.y + half / 2.0),
        ],
        colour,
    );
    let right = centre.x + half / 4.0;
    if muted {
        // A cross to the right of the cone, two bars corner to corner.
        cross(canvas, Point::new(right + half / 2.0, centre.y), half / 2.0, SPEAKER_LINE, colour);
    } else {
        // Two waves, drawn as bars rising with their distance.
        for (offset, height) in [(0.0, half / 2.0), (half / 2.0, half)] {
            let x = right + offset;
            canvas.fill(
                Rect::new(x, centre.y - height / 2.0 - SPEAKER_LINE / 2.0, x + SPEAKER_LINE, centre.y + height / 2.0 + SPEAKER_LINE / 2.0),
                SPEAKER_LINE / 2.0,
                colour,
            );
        }
    }
}

/// A closed padlock with its body's left edge at `at.x`, centred on `at.y`.
pub(super) fn padlock<C: Canvas + ?Sized>(canvas: &mut C, at: Point, colour: Colour) {
    let body = Rect::new(at.x, at.y - LOCK_HEIGHT / 2.0 + SHACKLE_HEIGHT / 2.0, at.x + LOCK_WIDTH, at.y + LOCK_HEIGHT / 2.0 + SHACKLE_HEIGHT / 2.0);
    canvas.fill(body, 1.5, colour);
    let shackle = Rect::new(
        at.x + (LOCK_WIDTH - SHACKLE_WIDTH) / 2.0,
        body.y0 - SHACKLE_HEIGHT,
        at.x + f64::midpoint(LOCK_WIDTH, SHACKLE_WIDTH),
        body.y0 + SHACKLE_THICKNESS,
    );
    canvas.fill(shackle, SHACKLE_WIDTH / 2.0, colour);
    canvas.fill(
        Rect::new(shackle.x0 + SHACKLE_THICKNESS, shackle.y0 + SHACKLE_THICKNESS, shackle.x1 - SHACKLE_THICKNESS, body.y0),
        (SHACKLE_WIDTH - SHACKLE_THICKNESS * 2.0) / 2.0,
        FIELD,
    );
}

/// The Home button's house: a roof over a body with a door.
pub(super) fn house<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let top = centre.y - f64::midpoint(ROOF_HEIGHT, HOUSE_BODY_HEIGHT);
    let eaves = top + ROOF_HEIGHT;
    canvas.polygon(&[Point::new(centre.x, top), Point::new(centre.x + HOUSE_WIDTH / 2.0, eaves), Point::new(centre.x - HOUSE_WIDTH / 2.0, eaves)], colour);
    let body = Rect::new(centre.x - HOUSE_WIDTH / 2.0 + HOUSE_BODY_INSET, eaves, centre.x + HOUSE_WIDTH / 2.0 - HOUSE_BODY_INSET, eaves + HOUSE_BODY_HEIGHT);
    canvas.fill(body, 0.0, colour);
    canvas.fill(Rect::new(centre.x - DOOR_WIDTH / 2.0, body.y1 - DOOR_HEIGHT, centre.x + DOOR_WIDTH / 2.0, body.y1), 0.0, BUTTON);
}

/// The Reload button's arrow, circling clockwise to its head at the top.
pub(super) fn reload_glyph<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let (outer, inner) = (RELOAD_RADIUS + RELOAD_THICKNESS / 2.0, RELOAD_RADIUS - RELOAD_THICKNESS / 2.0);
    // Angles grow clockwise on the screen, whose y grows downwards; the
    // ring runs from the gap's right end round to the top.
    let end = -core::f64::consts::FRAC_PI_2;
    let start = end + RELOAD_GAP_DEGREES.to_radians();
    let finish = end + core::f64::consts::TAU;
    let at = |angle: f64, radius: f64| {
        let (sine, cosine) = libm::sincos(angle);
        Point::new(centre.x + radius * cosine, centre.y + radius * sine)
    };
    let mut points = Vec::with_capacity((RELOAD_SEGMENTS + 1) * 2);
    for step in 0..=RELOAD_SEGMENTS {
        #[allow(clippy::cast_precision_loss, reason = "a few corners")]
        let angle = start + (finish - start) * step as f64 / RELOAD_SEGMENTS as f64;
        points.push(at(angle, outer));
    }
    for step in (0..=RELOAD_SEGMENTS).rev() {
        #[allow(clippy::cast_precision_loss, reason = "a few corners")]
        let angle = start + (finish - start) * step as f64 / RELOAD_SEGMENTS as f64;
        points.push(at(angle, inner));
    }
    canvas.polygon(&points, colour);
    // The head, at the top, pointing the way the ring turns: to the right.
    let top = at(finish, RELOAD_RADIUS);
    canvas
        .polygon(&[Point::new(top.x, top.y - RELOAD_HEAD_REACH), Point::new(top.x + RELOAD_HEAD, top.y), Point::new(top.x, top.y + RELOAD_HEAD_REACH)], colour);
}

/// The settings button's gear: teeth round a hub.
pub(super) fn gear<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, colour: Colour) {
    let centre = Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
    let mut points = Vec::with_capacity(GEAR_TEETH * 4);
    for corner in 0..GEAR_TEETH * 4 {
        #[allow(clippy::cast_precision_loss, reason = "a few corners")]
        let angle = core::f64::consts::TAU * corner as f64 / (GEAR_TEETH * 4) as f64;
        // Each tooth: two corners out, two in.
        let radius = if corner % 4 < 2 { GEAR_OUTER } else { GEAR_INNER };
        let (sine, cosine) = libm::sincos(angle);
        points.push(Point::new(centre.x + radius * cosine, centre.y + radius * sine));
    }
    canvas.polygon(&points, colour);
    canvas.fill(Rect::new(centre.x - GEAR_HUB, centre.y - GEAR_HUB, centre.x + GEAR_HUB, centre.y + GEAR_HUB), GEAR_HUB, BUTTON);
}

/// A five-pointed star centred on `centre`.
pub(super) fn star<C: Canvas + ?Sized>(canvas: &mut C, centre: Point, colour: Colour) {
    let mut points = Vec::with_capacity(STAR_POINTS * 2);
    for point in 0..STAR_POINTS * 2 {
        #[allow(clippy::cast_precision_loss, reason = "ten points")]
        let angle = core::f64::consts::PI * point as f64 / STAR_POINTS as f64 - core::f64::consts::FRAC_PI_2;
        let radius = if point % 2 == 0 { STAR_RADIUS } else { STAR_RADIUS * STAR_INNER };
        let (sine, cosine) = libm::sincos(angle);
        points.push(Point::new(centre.x + radius * cosine, centre.y + radius * sine));
    }
    canvas.polygon(&points, colour);
}
