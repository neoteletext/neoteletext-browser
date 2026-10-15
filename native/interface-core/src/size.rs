//! The page size rules: how wide a page on the wall is, and when the wall
//! is a list instead. The slider shrinks the pages over its whole travel;
//! only its left edge itself is the list.

/// The smallest page on the grid, just right of the slider's left edge:
/// one sixth of the native 480 dots, 80 pixels, two pixels to a character
/// column, small enough to see a whole bank at once.
pub const MIN_WIDTH: f32 = 80.0;
/// Twice the native teletext raster width (480 dots, EN 300 706 clause 2 at
/// 12 dots a cell): whole-number scaling keeps every dot square.
pub const MAX_WIDTH: f32 = 960.0;
/// A single page may not exceed this share of the window.
const MAX_WINDOW_SHARE: f32 = 0.9;
pub const DEFAULT_WIDTH: f32 = 320.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Grid,
    List,
}

/// The list at the slider's left edge, where the width is its smallest;
/// the grid everywhere else.
#[must_use]
pub fn layout_for_width(width: f32) -> Layout {
    if width <= MIN_WIDTH { Layout::List } else { Layout::Grid }
}

#[must_use]
pub fn max_width_for_window(window_width: f32) -> f32 {
    libm::floorf(window_width * MAX_WINDOW_SHARE).clamp(MIN_WIDTH, MAX_WIDTH)
}

#[must_use]
pub fn clamp_width(width: f32, max: f32) -> f32 {
    width.clamp(MIN_WIDTH, max)
}

/// Grid pages never go below the minimum, even while the width animates
/// down to the list.
#[must_use]
pub fn grid_width(width: f32) -> f32 {
    width.max(MIN_WIDTH)
}

// The slider is logarithmic, and steeper towards its right: travel is
// raised to `SLIDER_CURVE` before it scales the width by a ratio. A pull
// to the left from the large sizes shrinks the page quickly (halving the
// widest takes about 14% of the bar), and the small sizes get the rest of
// the bar for fine control. The owner asked for leftward pulls to have
// more effect sooner than a plain logarithmic slider gave.
const SLIDER_CURVE: f32 = 3.0;

/// The width at `fraction` (0 to 1) of the slider's travel.
#[must_use]
pub fn width_at_fraction(fraction: f32, max: f32) -> f32 {
    let travel = libm::powf(fraction.clamp(0.0, 1.0), SLIDER_CURVE);
    MIN_WIDTH * libm::powf(max / MIN_WIDTH, travel)
}

/// Where along the slider's travel, 0 to 1, `width` sits: the inverse of
/// [`width_at_fraction`].
#[must_use]
pub fn fraction_of_width(width: f32, max: f32) -> f32 {
    let travel = (libm::logf(width / MIN_WIDTH) / libm::logf(max / MIN_WIDTH)).clamp(0.0, 1.0);
    libm::powf(travel, SLIDER_CURVE.recip())
}

/// Presses of − or + that halve or double the width.
const STEPS_PER_HALVING: f32 = 8.0;

/// The width one press of − (`direction` −1) or + (1) gives: a fixed ratio,
/// matching the slider.
#[must_use]
pub fn stepped_width(width: f32, direction: i8) -> f32 {
    width * libm::exp2f(f32::from(direction) / STEPS_PER_HALVING)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_left_edge_is_the_list() {
        assert_eq!(layout_for_width(width_at_fraction(0.0, MAX_WIDTH)), Layout::List);
        // Just right of the edge is the grid, at about the smallest size.
        let near = width_at_fraction(0.02, MAX_WIDTH);
        assert_eq!(layout_for_width(near), Layout::Grid);
        assert!(near < MIN_WIDTH * 1.01, "{near}");
        // The pages keep shrinking all the way down: well below the old
        // floor of 192 pixels.
        assert!(width_at_fraction(0.3, MAX_WIDTH) < 100.0);
    }

    #[test]
    fn widths_are_bounded() {
        assert!((max_width_for_window(50.0) - MIN_WIDTH).abs() < f32::EPSILON);
        assert!((max_width_for_window(4000.0) - MAX_WIDTH).abs() < f32::EPSILON);
        assert!((clamp_width(10.0, MAX_WIDTH) - MIN_WIDTH).abs() < f32::EPSILON);
    }

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 0.01
    }

    #[test]
    fn a_pull_left_from_the_widest_shrinks_the_page_quickly() {
        assert!(close(width_at_fraction(0.0, MAX_WIDTH), MIN_WIDTH));
        assert!(close(width_at_fraction(1.0, MAX_WIDTH), MAX_WIDTH));
        let half = fraction_of_width(MAX_WIDTH / 2.0, MAX_WIDTH);
        assert!(close(width_at_fraction(half, MAX_WIDTH), MAX_WIDTH / 2.0));
        // Halving from the widest takes under a sixth of the bar; a plain
        // logarithmic slider needed 37% of it, a linear one 59%.
        let first = 1.0 - half;
        assert!(first < 0.17, "{first}");
        // Halving again takes more travel: the small sizes get finer control.
        let second = half - fraction_of_width(MAX_WIDTH / 4.0, MAX_WIDTH);
        assert!(second > first, "{second} after {first}");
    }

    #[test]
    fn steps_halve_in_a_fixed_number_of_presses() {
        let mut width = MAX_WIDTH;
        for _ in 0..8 {
            width = stepped_width(width, -1);
        }
        assert!(close(width, MAX_WIDTH / 2.0), "{width}");
        assert!(close(stepped_width(stepped_width(DEFAULT_WIDTH, 1), -1), DEFAULT_WIDTH));
    }
}
