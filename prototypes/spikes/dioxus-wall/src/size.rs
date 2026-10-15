//! The thumbnail width rules of the React prototype (`src/tab/thumbnailSize.ts`),
//! carried over unchanged except for the widest setting, which here is twice
//! a teletext page's native width, and the window-width ceiling, left out
//! because Dioxus native does not report the window's width to the app.

/// Smallest width at which a page and its caption stay legible.
pub const MIN_WIDTH: f32 = 192.0;
/// Slider travel below `MIN_WIDTH` belongs to the list layout.
const LIST_ZONE: f32 = 48.0;
pub const LIST_STOP: f32 = MIN_WIDTH - LIST_ZONE;
/// Hysteresis: the layout switches at a different point in each direction,
/// so a hand wobbling near the boundary cannot flip it back and forth.
const LEAVE_MARGIN: f32 = 8.0;
const LIST_ENTER_BELOW: f32 = LIST_STOP + LIST_ZONE / 2.0;
const LIST_LEAVE_ABOVE: f32 = MIN_WIDTH + LEAVE_MARGIN;
/// Twice the native teletext raster width (480 dots, EN 300 706 clause 2 at
/// 12 dots a cell): whole-number scaling keeps every dot square.
pub const MAX_WIDTH: f32 = 960.0;
pub const DEFAULT_WIDTH: f32 = 320.0;
/// The time the thumb must stay past a threshold before the layout switches,
/// so a fast sweep across the band does not trigger it.
pub const SWITCH_DWELL_SECONDS: f32 = 0.120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Grid,
    List,
}

/// A width with no direction of travel: the zone boundary decides.
pub fn layout_for_width(width: f32) -> Layout {
    if width < MIN_WIDTH { Layout::List } else { Layout::Grid }
}

pub fn next_layout(current: Layout, width: f32) -> Layout {
    match current {
        Layout::Grid if width < LIST_ENTER_BELOW => Layout::List,
        Layout::List if width > LIST_LEAVE_ABOVE => Layout::Grid,
        unchanged => unchanged,
    }
}

pub fn clamp_width(width: f32, max: f32) -> f32 {
    width.clamp(LIST_STOP, max)
}

/// Grid pages never go below the minimum, even while the thumb sits in the
/// hysteresis band on its way into or out of the list.
pub fn grid_width(width: f32) -> f32 {
    width.max(MIN_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_switches_with_hysteresis() {
        // Into the list only halfway into the list zone.
        assert_eq!(next_layout(Layout::Grid, LIST_ENTER_BELOW + 1.0), Layout::Grid);
        assert_eq!(next_layout(Layout::Grid, LIST_ENTER_BELOW - 1.0), Layout::List);
        // Out of it only clear of the grid minimum.
        assert_eq!(next_layout(Layout::List, MIN_WIDTH + 1.0), Layout::List);
        assert_eq!(next_layout(Layout::List, LIST_LEAVE_ABOVE + 1.0), Layout::Grid);
    }

    #[test]
    fn restored_width_uses_the_zone_boundary() {
        assert_eq!(layout_for_width(MIN_WIDTH - 1.0), Layout::List);
        assert_eq!(layout_for_width(MIN_WIDTH), Layout::Grid);
    }

    #[test]
    fn widths_are_bounded() {
        assert!((clamp_width(10.0, MAX_WIDTH) - LIST_STOP).abs() < f32::EPSILON);
        assert!((grid_width(LIST_STOP) - MIN_WIDTH).abs() < f32::EPSILON);
    }
}
