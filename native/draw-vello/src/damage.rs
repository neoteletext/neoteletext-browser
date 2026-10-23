//! Damage: which device pixels a frame changed from the last
//! (`TECH-DEBT.md`, damage-limited drawing). A frame's drawing calls are
//! kept and compared with the last frame's; only the box that holds every
//! call that differs is drawn again.

use crate::record::{Command, Frame, Recorded};
use crate::{Painter, RGBA_BYTES};
use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};
use interface_core::geometry::Rect;
use vello_cpu::Pixmap;

#[cfg(test)]
mod tests;

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

impl Painter {
    /// The boxes holding every call of `frame` that differs from the last
    /// frame's.
    pub(crate) fn damage(&self, frame: &Frame) -> Damage {
        let (old, new) = (&self.last, frame);
        let same = |before: &Recorded, after: &Recorded| before.command == after.command && old.rects(before.clip) == new.rects(after.clip);
        let (prefix, suffix) = common_ends(&old.calls, &new.calls, same);
        let before = &old.calls[prefix..old.calls.len() - suffix];
        let after = &new.calls[prefix..new.calls.len() - suffix];
        let mut damage = Damage::default();
        if before.len() == after.len() {
            // Call by call: where the two are the same, the same is drawn at
            // that place in both frames, as in the common ends.
            for (was, now) in before.iter().zip(after).filter(|(was, now)| !same(was, now)) {
                if let Some(dots) = self.narrowed(old, was, new, now) {
                    damage.add(dots);
                } else {
                    damage.add(was.bounds);
                    damage.add(now.bounds);
                }
            }
        } else {
            for call in before.iter().chain(after) {
                damage.add(call.bounds);
            }
        }
        damage
    }

    /// For a page whose image alone changed, where it is drawn, inside the
    /// same clips, the pixels of the dots that changed.
    fn narrowed(&self, old: &Frame, was: &Recorded, new: &Frame, now: &Recorded) -> Option<DeviceBox> {
        let (Command::Page { key, area, .. }, Command::Page { key: key_now, area: area_now, .. }) = (&was.command, &now.command) else { return None };
        if key != key_now || area != area_now || old.rects(was.clip) != new.rects(now.clip) {
            return None;
        }
        let dots = self.changed.get(key)?;
        let (across, down) = (area.width() / f64::from(PAGE_WIDTH), area.height() / f64::from(PAGE_HEIGHT));
        let changed = Rect {
            x0: area.x0 + f64::from(dots.x0) * across,
            y0: area.y0 + f64::from(dots.y0) * down,
            x1: area.x0 + f64::from(dots.x1) * across,
            y1: area.y0 + f64::from(dots.y1) * down,
        };
        let clip = new.clips.get(now.clip).map_or_else(DeviceBox::default, |(_, area)| *area);
        Some(DeviceBox::covering(changed, self.scale, self.surface()).intersect(clip))
    }
}

/// The box of dots whose colour differs between a held image and a new
/// one's red, green, blue and alpha bytes.
pub(crate) fn changed_dots(held: &Pixmap, rgba: &[u8]) -> DeviceBox {
    let mut found = DeviceBox::default();
    let width = usize::from(PAGE_WIDTH);
    let (dots, _) = rgba.as_chunks::<RGBA_BYTES>();
    for (index, (pixel, &[red, green, blue, _])) in held.data().iter().zip(dots).enumerate() {
        if [pixel.r, pixel.g, pixel.b] != [red, green, blue] {
            let (Ok(x), Ok(y)) = (u16::try_from(index % width), u16::try_from(index / width)) else { continue };
            found = found.union(DeviceBox { x0: x, y0: y, x1: x.saturating_add(1), y1: y.saturating_add(1) });
        }
    }
    found
}
