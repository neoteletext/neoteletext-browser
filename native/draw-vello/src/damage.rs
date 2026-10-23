//! Damage: which device pixels a frame changed from the last
//! (`TECH-DEBT.md`, damage-limited drawing). A frame's drawing calls are
//! kept and compared with the last frame's; only the box that holds every
//! call that differs is drawn again.

use interface_core::geometry::Rect;

/// Device pixels a shape's edge is taken to reach past its geometry, for
/// the rounding of transforms: `floor` and `ceil` already take in every
/// pixel an edge's antialiasing touches, and one more on each side keeps a
/// coordinate that lands a hair past a pixel's edge from losing it.
const MARGIN: f64 = 1.0;

/// A box of device pixels: `x0` and `y0` inside it, `x1` and `y1` just
/// past it. Empty when either side is zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceBox {
    /// The left column.
    pub x0: u16,
    /// The top row.
    pub y0: u16,
    /// Just past the right column.
    pub x1: u16,
    /// Just past the bottom row.
    pub y1: u16,
}

/// A device coordinate as a pixel index, clamped to `0..=limit`.
fn clamp(value: f64, limit: u16) -> u16 {
    if value.is_nan() || value <= 0.0 {
        0
    } else if value >= f64::from(limit) {
        limit
    } else {
        // In range and whole: the conversion cannot truncate.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a whole number between 0 and a u16, checked above")]
        let pixel = value as u16;
        pixel
    }
}

impl DeviceBox {
    /// The whole of a surface `width` by `height` pixels.
    #[must_use]
    pub const fn whole(width: u16, height: u16) -> Self {
        Self { x0: 0, y0: 0, x1: width, y1: height }
    }

    /// The device pixels the logical `rect` touches at `scale`, inside
    /// `surface`.
    #[must_use]
    pub fn covering(rect: Rect, scale: f64, surface: Self) -> Self {
        let (x0, x1) = (rect.x0.min(rect.x1), rect.x0.max(rect.x1));
        let (y0, y1) = (rect.y0.min(rect.y1), rect.y0.max(rect.y1));
        let found = Self {
            x0: clamp((x0 * scale).floor() - MARGIN, surface.x1),
            y0: clamp((y0 * scale).floor() - MARGIN, surface.y1),
            x1: clamp((x1 * scale).ceil() + MARGIN, surface.x1),
            y1: clamp((y1 * scale).ceil() + MARGIN, surface.y1),
        };
        found.intersect(surface)
    }

    /// No pixels.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    /// Columns.
    #[must_use]
    pub const fn width(self) -> u16 {
        self.x1.saturating_sub(self.x0)
    }

    /// Rows.
    #[must_use]
    pub const fn height(self) -> u16 {
        self.y1.saturating_sub(self.y0)
    }

    /// Pixels.
    #[must_use]
    pub fn area(self) -> u32 {
        u32::from(self.width()) * u32::from(self.height())
    }

    /// The smallest box holding both.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self { x0: self.x0.min(other.x0), y0: self.y0.min(other.y0), x1: self.x1.max(other.x1), y1: self.y1.max(other.y1) }
    }

    /// The pixels in both; empty when they do not meet.
    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        let found = Self { x0: self.x0.max(other.x0), y0: self.y0.max(other.y0), x1: self.x1.min(other.x1), y1: self.y1.min(other.y1) };
        if found.is_empty() { Self::default() } else { found }
    }

    /// Whether they share a pixel.
    #[must_use]
    pub fn meets(self, other: Self) -> bool {
        !self.intersect(other).is_empty()
    }
}

/// The most boxes a frame's damage is kept as: each is drawn on its own,
/// and past this the two whose joining adds least are joined. A wall of
/// pages whose header clocks all tick is a box for each page in view.
pub const MAX_BOXES: usize = 16;

/// A frame's damage: a few boxes, none overlapping another it was joined
/// with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    boxes: Vec<DeviceBox>,
}

impl Damage {
    /// The boxes.
    #[must_use]
    pub fn boxes(&self) -> &[DeviceBox] {
        &self.boxes
    }

    /// Nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    /// Adds `area`, joining it with any box it meets.
    pub fn add(&mut self, area: DeviceBox) {
        if area.is_empty() {
            return;
        }
        let mut joined = area;
        // A joined box can meet others it did not before.
        while let Some(index) = self.boxes.iter().position(|held| held.meets(joined)) {
            joined = joined.union(self.boxes.swap_remove(index));
        }
        if self.boxes.len() < MAX_BOXES {
            self.boxes.push(joined);
            return;
        }
        let growth = |held: &DeviceBox| held.union(joined).area().saturating_sub(held.area());
        let Some(index) = self.boxes.iter().enumerate().min_by_key(|(_, held)| growth(held)).map(|(index, _)| index) else { return };
        let merged = self.boxes.swap_remove(index).union(joined);
        self.add(merged);
    }
}

/// How many of the calls at the start, and then at the end, of `old` and
/// `new` are the same: what lies between is what changed. The two counts
/// never overlap in either list.
pub fn common_ends<T>(old: &[T], new: &[T], same: impl Fn(&T, &T) -> bool) -> (usize, usize) {
    let prefix = old.iter().zip(new).take_while(|(before, after)| same(before, after)).count();
    let rest = old.len().min(new.len()).saturating_sub(prefix);
    let suffix = old.iter().rev().zip(new.iter().rev()).take(rest).take_while(|(before, after)| same(before, after)).count();
    (prefix, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACE: DeviceBox = DeviceBox::whole(100, 50);

    #[test]
    fn a_rect_covers_every_pixel_it_touches_and_a_margin() {
        let rect = Rect { x0: 10.25, y0: 4.5, x1: 20.75, y1: 5.0 };
        assert_eq!(DeviceBox::covering(rect, 2.0, SURFACE), DeviceBox { x0: 19, y0: 8, x1: 43, y1: 11 });
    }

    #[test]
    fn a_rect_is_held_inside_the_surface() {
        let rect = Rect { x0: -30.0, y0: -1.0, x1: 400.0, y1: 20.0 };
        assert_eq!(DeviceBox::covering(rect, 1.0, SURFACE), DeviceBox { x0: 0, y0: 0, x1: 100, y1: 21 });
        let outside = Rect { x0: 200.0, y0: 0.0, x1: 300.0, y1: 10.0 };
        assert!(DeviceBox::covering(outside, 1.0, SURFACE).is_empty());
    }

    #[test]
    fn an_empty_box_adds_nothing_to_a_union() {
        let one = DeviceBox { x0: 5, y0: 5, x1: 6, y1: 7 };
        assert_eq!(one.union(DeviceBox::default()), one);
        assert_eq!(DeviceBox::default().union(one), one);
        assert_eq!(one.union(DeviceBox { x0: 1, y0: 9, x1: 2, y1: 10 }), DeviceBox { x0: 1, y0: 5, x1: 6, y1: 10 });
        assert!(!one.meets(DeviceBox { x0: 6, y0: 5, x1: 8, y1: 7 }), "boxes that only touch share no pixel");
    }

    #[test]
    fn damage_joins_boxes_that_meet_and_keeps_apart_those_that_do_not() {
        let mut damage = Damage::default();
        damage.add(DeviceBox { x0: 0, y0: 0, x1: 4, y1: 4 });
        damage.add(DeviceBox { x0: 50, y0: 0, x1: 54, y1: 4 });
        damage.add(DeviceBox::default());
        assert_eq!(damage.boxes().len(), 2);
        // Meets both: all three become one.
        damage.add(DeviceBox { x0: 2, y0: 1, x1: 52, y1: 2 });
        assert_eq!(damage.boxes(), [DeviceBox { x0: 0, y0: 0, x1: 54, y1: 4 }]);
    }

    #[test]
    fn past_the_most_boxes_the_nearest_are_joined() {
        let mut damage = Damage::default();
        let far = u16::try_from(MAX_BOXES).unwrap() * 10;
        for step in 0..=u16::try_from(MAX_BOXES).unwrap() {
            damage.add(DeviceBox { x0: step * 10, y0: 0, x1: step * 10 + 2, y1: 2 });
        }
        assert_eq!(damage.boxes().len(), MAX_BOXES);
        let covered: u32 = damage.boxes().iter().map(|area| area.area()).sum();
        assert!(covered < 2 * 2 * u32::try_from(MAX_BOXES).unwrap() + 12 * 2, "only two neighbours joined: {covered}");
        assert!(damage.boxes().iter().any(|area| area.x1 == far + 2), "the last box is kept");
    }

    #[test]
    fn the_calls_that_changed_lie_between_the_common_ends() {
        let same = |a: &u8, b: &u8| a == b;
        assert_eq!(common_ends(&[1, 2, 3, 4], &[1, 2, 3, 4], same), (4, 0));
        assert_eq!(common_ends(&[1, 2, 3, 4], &[1, 9, 3, 4], same), (1, 2));
        assert_eq!(common_ends(&[1, 2, 4], &[1, 2, 3, 4], same), (2, 1));
        assert_eq!(common_ends(&[1, 1], &[1, 1, 1], same), (2, 0), "the ends never overlap");
        assert_eq!(common_ends(&[], &[5], same), (0, 0));
    }
}
