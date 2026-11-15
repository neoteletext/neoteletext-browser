//! The pointer shown where the wheel steps pages (REQ-6356): a mouse with
//! an arrow above it, below it or both, for the ways there are items, its
//! wheel blinking. The owner chose the design from twenty candidates on 16
//! November 2026 (number 5, the solid mouse with a lit wheel). The pictures
//! are drawn here from the design's geometry, in a dark-background and a
//! light-background variant, as red, green, blue and opacity for the
//! windowing system's custom pointer.

use alloc::vec::Vec;

use crate::geometry::Point;
use crate::theme::{BLACK, CYAN, Colour, WHITE};

/// The design's square, in points: the size a pointer has on every desktop.
pub const SIZE: u16 = 32;
/// The pointer's active point, the square's centre, from its left and top.
pub const HOTSPOT: u16 = SIZE / 2;
/// The wheel's flash (REQ-6356 item 3, as the owner set it on 16 November
/// 2026: "make it flash 3 times each 150ms, then leave it lit for the next
/// 10 seconds"): it is unlit this long and lit this long, [`FLASHES`]
/// times, from the moment the pointer takes this shape.
pub const FLASH_MILLISECONDS: u64 = 150;
/// How many times the wheel flashes before it stays lit.
pub const FLASHES: u64 = 3;
/// How long the wheel then stays lit, before it flashes again.
pub const LIT_MILLISECONDS: u64 = 10_000;
/// The flashes, each an unlit and a lit part.
const FLASHING_MILLISECONDS: u64 = FLASHES * 2 * FLASH_MILLISECONDS;
/// One round of flashes and the lit time after them.
const ROUND_MILLISECONDS: u64 = FLASHING_MILLISECONDS + LIT_MILLISECONDS;
/// The largest scale a picture is made at: beyond any display's today.
pub const MAX_SCALE: u16 = 4;

/// Samples across and down each pixel, for smooth edges.
const SAMPLES: u16 = 4;
/// The edge round the body and the arrows, in the opposite tone.
const EDGE: f64 = 1.0;
/// The lines inside the body: half their width.
const HALF_LINE: f64 = 0.5;
/// The body: its left, top, width, height and corner radius.
const BODY: (f64, f64, f64, f64, f64) = (10.0, 8.0, 12.0, 16.0, 5.52);
/// Where the line between the buttons and the rest of the body lies.
const SPLIT: f64 = 14.72;
/// The wheel: its left, top, width, height and corner radius.
const WHEEL: (f64, f64, f64, f64, f64) = (14.7, 9.6, 2.6, 3.52, 1.3);
/// The square's middle, where the line between the two buttons runs.
const MIDDLE: f64 = 16.0;
/// The up arrow's corners; the down arrow is its mirror in the middle.
const UP_ARROW: [(f64, f64); 3] = [(16.0, 0.2), (19.4, 5.98), (12.6, 5.98)];
/// The light variant's body: the browser's panel black.
const DARK_BODY: Colour = Colour { red: 0x10, green: 0x10, blue: 0x14 };
/// The light variant's inner lines and unlit wheel.
const DARK_BODY_LINE: Colour = Colour { red: 0x6a, green: 0x6a, blue: 0x78 };
/// A byte's most, as a float.
const FULL: f64 = 255.0;

/// What lies behind the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ground {
    /// A dark background: the white mouse.
    Dark,
    /// A light background: the dark mouse.
    Light,
}

/// The wheel pointer's state: the ways there are items, and its variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WheelCursor {
    /// There is an item before the one shown.
    pub up: bool,
    /// There is an item after the one shown.
    pub down: bool,
    /// The variant, for what lies behind the pointer.
    pub ground: Ground,
}

/// Whether the wheel is lit, `shown` milliseconds after the pointer took
/// this shape: three flashes, each unlit then lit, then lit for ten
/// seconds, and round again.
#[must_use]
pub fn lit(shown: u64) -> bool {
    let within = shown % ROUND_MILLISECONDS;
    within >= FLASHING_MILLISECONDS || !(within / FLASH_MILLISECONDS).is_multiple_of(2)
}

/// How long until the wheel next changes, `shown` milliseconds after the
/// pointer took this shape.
#[must_use]
pub fn until_blink(shown: u64) -> u64 {
    let within = shown % ROUND_MILLISECONDS;
    let next =
        if within >= FLASHING_MILLISECONDS { ROUND_MILLISECONDS } else { (within / FLASH_MILLISECONDS).saturating_add(1).saturating_mul(FLASH_MILLISECONDS) };
    next.saturating_sub(within).max(1)
}

/// The colours of one variant.
struct Palette {
    body: Colour,
    edge: Colour,
    line: Colour,
    arrow: Colour,
    wheel_lit: Colour,
    wheel_unlit: Colour,
}

impl Ground {
    fn palette(self) -> Palette {
        match self {
            Self::Dark => Palette { body: WHITE, edge: BLACK, line: BLACK, arrow: CYAN, wheel_lit: CYAN, wheel_unlit: BLACK },
            Self::Light => Palette { body: DARK_BODY, edge: WHITE, line: DARK_BODY_LINE, arrow: DARK_BODY, wheel_lit: CYAN, wheel_unlit: DARK_BODY_LINE },
        }
    }
}

/// How far `point` is outside a rounded rectangle, negative inside it.
fn rounded((left, top, width, height, radius): (f64, f64, f64, f64, f64), point: Point) -> f64 {
    let (half_width, half_height) = (width / 2.0, height / 2.0);
    let across = libm::fabs(point.x - (left + half_width)) - (half_width - radius);
    let down = libm::fabs(point.y - (top + half_height)) - (half_height - radius);
    let outside = libm::hypot(across.max(0.0), down.max(0.0));
    outside + across.max(down).min(0.0) - radius
}

/// How far `point` is outside the triangle `corners`, negative inside it.
fn triangle(corners: [(f64, f64); 3], point: Point) -> f64 {
    let [first, second, third] = corners;
    let mut nearest = f64::INFINITY;
    let mut sides = [0.0_f64; 3];
    for (side, (from, to)) in sides.iter_mut().zip([(first, second), (second, third), (third, first)]) {
        let (edge_x, edge_y) = (to.0 - from.0, to.1 - from.1);
        let (to_point_x, to_point_y) = (point.x - from.0, point.y - from.1);
        let along = ((to_point_x * edge_x + to_point_y * edge_y) / (edge_x * edge_x + edge_y * edge_y)).clamp(0.0, 1.0);
        nearest = nearest.min(libm::hypot(to_point_x - edge_x * along, to_point_y - edge_y * along));
        *side = edge_x * to_point_y - edge_y * to_point_x;
    }
    let inside = sides.iter().all(|&side| side >= 0.0) || sides.iter().all(|&side| side <= 0.0);
    if inside { -nearest } else { nearest }
}

/// The colour of the design at `point`, in points; none where it is clear.
fn colour_at(cursor: WheelCursor, wheel_lit: bool, palette: &Palette, point: Point) -> Option<Colour> {
    let down_arrow = UP_ARROW.map(|(x, y)| (x, f64::from(SIZE) - y));
    let arrows = [(cursor.up, UP_ARROW), (cursor.down, down_arrow)];
    let body = rounded(BODY, point);
    if body <= 0.0 {
        let wheel = rounded(WHEEL, point);
        if wheel <= 0.0 {
            return Some(if wheel_lit { palette.wheel_lit } else { palette.wheel_unlit });
        }
        let on_split = libm::fabs(point.y - SPLIT) <= HALF_LINE;
        let between_buttons = libm::fabs(point.x - MIDDLE) <= HALF_LINE && point.y <= SPLIT;
        return Some(if wheel <= HALF_LINE || on_split || between_buttons { palette.line } else { palette.body });
    }
    let arrow = arrows.iter().filter(|(shown, _)| *shown).map(|&(_, corners)| triangle(corners, point)).fold(f64::INFINITY, f64::min);
    if arrow <= 0.0 {
        Some(palette.arrow)
    } else if arrow <= EDGE || body <= EDGE {
        Some(palette.edge)
    } else {
        None
    }
}

/// The picture of `cursor` with its wheel lit or not, `scale` pixels to a
/// point (1 to [`MAX_SCALE`]): rows of red, green, blue and opacity, not
/// multiplied together, [`SIZE`] times `scale` pixels square.
#[must_use]
pub fn image(cursor: WheelCursor, wheel_lit: bool, scale: u16) -> Vec<u8> {
    let scale = scale.clamp(1, MAX_SCALE);
    let side = SIZE.saturating_mul(scale);
    let palette = cursor.ground.palette();
    let step = 1.0 / (f64::from(scale) * f64::from(SAMPLES));
    let mut pixels = Vec::with_capacity(usize::from(side).saturating_mul(usize::from(side)).saturating_mul(4));
    for y in 0..side {
        for x in 0..side {
            let (mut red, mut green, mut blue, mut covered) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
            for sample_y in 0..SAMPLES {
                for sample_x in 0..SAMPLES {
                    let across = (f64::from(x) * f64::from(SAMPLES) + f64::from(sample_x) + 0.5) * step;
                    let down = (f64::from(y) * f64::from(SAMPLES) + f64::from(sample_y) + 0.5) * step;
                    if let Some(colour) = colour_at(cursor, wheel_lit, &palette, Point::new(across, down)) {
                        red += f64::from(colour.red);
                        green += f64::from(colour.green);
                        blue += f64::from(colour.blue);
                        covered += 1.0;
                    }
                }
            }
            // Rounded and within a byte by construction: averages of bytes.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to a byte first")]
            let byte = |value: f64| libm::round(value).clamp(0.0, FULL) as u8;
            let samples = f64::from(SAMPLES) * f64::from(SAMPLES);
            if covered > 0.0 {
                pixels.extend_from_slice(&[byte(red / covered), byte(green / covered), byte(blue / covered), byte(FULL * covered / samples)]);
            } else {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    #![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing, reason = "a test")]
    use super::*;

    /// The pixel of the picture at scale 1 at `x`, `y`.
    fn pixel(picture: &[u8], x: usize, y: usize) -> [u8; 4] {
        let at = (y * usize::from(SIZE) + x) * 4;
        [picture[at], picture[at + 1], picture[at + 2], picture[at + 3]]
    }

    const BOTH: WheelCursor = WheelCursor { up: true, down: true, ground: Ground::Dark };
    const CYAN_PIXEL: [u8; 4] = [CYAN.red, CYAN.green, CYAN.blue, 255];

    /// REQ-6356 item 3: three flashes of 150 milliseconds unlit and 150
    /// lit, then lit for ten seconds, then round again, with a wake at
    /// each change and none between.
    #[test]
    fn the_wheel_flashes_three_times_then_stays_lit_ten_seconds() {
        let changes: Vec<(u64, bool)> = [0, 149, 150, 299, 300, 450, 600, 750, 899, 900, 5_000, 10_899].map(|shown| (shown, lit(shown))).to_vec();
        assert_eq!(
            changes,
            [
                (0, false),
                (149, false),
                (150, true),
                (299, true),
                (300, false),
                (450, true),
                (600, false),
                (750, true),
                (899, true),
                (900, true),
                (5_000, true),
                (10_899, true)
            ]
        );
        assert!(!lit(10_900) && lit(10_900 + 150), "and round again");
        assert_eq!((until_blink(0), until_blink(149), until_blink(150), until_blink(750)), (150, 1, 150, 150));
        assert_eq!((until_blink(900), until_blink(5_000), until_blink(10_899)), (10_000, 5_900, 1), "no wake while it stays lit");
    }

    /// REQ-6356 items 1 and 2: an arrow only for a way there are items.
    #[test]
    fn an_arrow_is_drawn_only_for_a_way_there_are_items() {
        let (above, below) = ((16, 4), (16, 27));
        for (up, down) in [(true, true), (true, false), (false, true)] {
            let picture = image(WheelCursor { up, down, ground: Ground::Dark }, true, 1);
            assert_eq!(picture.len(), usize::from(SIZE) * usize::from(SIZE) * 4);
            assert_eq!(pixel(&picture, above.0, above.1) == CYAN_PIXEL, up, "the up arrow, up {up} down {down}");
            assert_eq!(pixel(&picture, below.0, below.1) == CYAN_PIXEL, down, "the down arrow, up {up} down {down}");
            assert_eq!(pixel(&picture, 0, 16)[3], 0, "clear beside the mouse");
        }
    }

    /// REQ-6356 item 3: only the wheel differs between lit and unlit.
    #[test]
    fn only_the_wheel_blinks() {
        let (lit_picture, unlit_picture) = (image(BOTH, true, 1), image(BOTH, false, 1));
        assert_eq!(pixel(&lit_picture, 16, 11), CYAN_PIXEL, "the wheel lit");
        assert_eq!(pixel(&unlit_picture, 16, 11), [0, 0, 0, 255], "the wheel unlit");
        for y in 0..usize::from(SIZE) {
            for x in 0..usize::from(SIZE) {
                let in_wheel = (14..=17).contains(&x) && (9..=13).contains(&y);
                assert!(in_wheel || pixel(&lit_picture, x, y) == pixel(&unlit_picture, x, y), "{x},{y} is not the wheel's");
            }
        }
    }

    /// REQ-6356 item 4: a white body on dark, a dark body on light, each
    /// edged in the other tone.
    #[test]
    fn each_ground_has_its_variant() {
        let dark = image(BOTH, true, 1);
        let light = image(WheelCursor { ground: Ground::Light, ..BOTH }, true, 1);
        assert_eq!(pixel(&dark, 13, 20), [255, 255, 255, 255], "a white body");
        assert_eq!(pixel(&light, 13, 20), [DARK_BODY.red, DARK_BODY.green, DARK_BODY.blue, 255], "a dark body");
        assert_eq!(pixel(&light, 16, 11), CYAN_PIXEL, "its wheel lit in cyan too");
        assert_eq!(pixel(&light, 16, 4), [DARK_BODY.red, DARK_BODY.green, DARK_BODY.blue, 255], "its arrows dark");
        // Just outside the body's left side, at its middle: the edge.
        assert!(pixel(&dark, 9, 20)[0] < 64 && pixel(&dark, 9, 20)[3] > 128, "edged in black: {:?}", pixel(&dark, 9, 20));
        assert!(pixel(&light, 9, 20)[0] > 192 && pixel(&light, 9, 20)[3] > 128, "edged in white: {:?}", pixel(&light, 9, 20));
    }

    /// The picture grows with the display's scale, within its bound.
    #[test]
    fn the_picture_is_made_at_the_displays_scale() {
        assert_eq!(image(BOTH, true, 2).len(), 64 * 64 * 4);
        assert_eq!(image(BOTH, true, 99).len(), image(BOTH, true, MAX_SCALE).len());
    }
}
