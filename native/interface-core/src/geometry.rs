//! Points and rectangles, in logical pixels.

use core::ops::{Add, Mul, Sub};

/// A point, measured right and down from the window's top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Distance from the left edge.
    pub x: f64,
    /// Distance from the top edge.
    pub y: f64,
}

impl Point {
    /// The window's top-left corner.
    pub const ORIGIN: Self = Self { x: 0.0, y: 0.0 };

    /// The point at `x`, `y`.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

impl Add for Point {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for Point {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

impl Mul<f64> for Point {
    type Output = Self;
    fn mul(self, factor: f64) -> Self {
        Self::new(self.x * factor, self.y * factor)
    }
}

/// A rectangle from its top-left corner (`x0`, `y0`) to its bottom-right
/// corner (`x1`, `y1`); it contains its top and left edges but not its
/// bottom and right ones.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// The left edge.
    pub x0: f64,
    /// The top edge.
    pub y0: f64,
    /// The right edge.
    pub x1: f64,
    /// The bottom edge.
    pub y1: f64,
}

impl Rect {
    /// The empty rectangle at the origin: what a part not drawn is recorded as.
    pub const ZERO: Self = Self { x0: 0.0, y0: 0.0, x1: 0.0, y1: 0.0 };

    /// The rectangle from (`x0`, `y0`) to (`x1`, `y1`).
    #[must_use]
    pub const fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// The distance from the left edge to the right.
    #[must_use]
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }

    /// The distance from the top edge to the bottom.
    #[must_use]
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }

    /// Whether `point` is inside: on the top or left edge counts, on the
    /// bottom or right edge does not, so adjoining rectangles never share a point.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x0 && point.x < self.x1 && point.y >= self.y0 && point.y < self.y1
    }
}
