//! The page under the pointer popping out of the wall, and the one it
//! left settling back.

use super::Wall;
use super::grid::FRAME;
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::text::outline;
use crate::theme::WHITE;
use crate::tween::{EASE_OUT, Tween};

/// A page under the pointer pops out: 6% larger, over 150 milliseconds
/// ease-out, as the prototype's width transition, with a shadow offset by
/// 8 pixels at 60% darkness.
pub(super) const HOVER_MILLISECONDS: u64 = 150;
pub(super) const HOVER_GROWTH: f64 = 0.06;
const HOVER_SHADOW: f64 = 8.0;
const HOVER_SHADOW_DARKNESS: f64 = 0.6;

impl Wall {
    /// Follows the pointer: the page under it, on the wall, pops out.
    /// Returns whether that changed which page it is.
    pub fn hover(&mut self, point: Point, now: u64) -> bool {
        let over = if self.zoomed.is_some() || !self.area.contains(point) {
            None
        } else {
            self.drawn.iter().rev().find(|drawn| drawn.rect.contains(point)).map(|drawn| drawn.key)
        };
        if over == self.hovered {
            return false;
        }
        // The page left settles back from however far it had popped.
        if let Some(old) = self.hovered {
            let mut leave = Tween::new(self.hover.value(now), HOVER_MILLISECONDS, EASE_OUT);
            leave.set(0.0, now);
            self.leaving = Some(old);
            self.leave = leave;
        }
        self.hovered = over;
        self.hover = Tween::new(0.0, HOVER_MILLISECONDS, EASE_OUT);
        self.hover.set(1.0, now);
        true
    }

    /// Draws the hovered page, and the one it left, popped out over their
    /// slots: larger by up to `HOVER_GROWTH`, with a shadow beneath and a
    /// light frame.
    pub(super) fn draw_popped<C: Canvas + ?Sized>(&mut self, canvas: &mut C, now: u64) {
        let popped = [(self.leaving, self.leave.value(now)), (self.hovered, self.hover.value(now))];
        for (key, amount) in popped {
            let Some(key) = key else { continue };
            let amount = f64::from(amount);
            if amount <= 0.0 {
                continue;
            }
            let Some(slot) = self.drawn.iter().find(|drawn| drawn.key == key).map(|drawn| drawn.rect) else { continue };
            let grow = 1.0 + HOVER_GROWTH * amount;
            let centre = Point::new(f64::midpoint(slot.x0, slot.x1), f64::midpoint(slot.y0, slot.y1));
            let (half_width, half_height) = (slot.width() * grow / 2.0, slot.height() * grow / 2.0);
            let rect = Rect::new(centre.x - half_width, centre.y - half_height, centre.x + half_width, centre.y + half_height);
            let drop = HOVER_SHADOW * amount;
            #[allow(clippy::cast_possible_truncation, reason = "a share between 0 and 1")]
            canvas.dim(Rect::new(rect.x0 + drop, rect.y0 + drop, rect.x1 + drop, rect.y1 + drop), (HOVER_SHADOW_DARKNESS * amount) as f32);
            self.draw_page(canvas, key, rect);
            outline(canvas, Rect::new(rect.x0 - FRAME, rect.y0 - FRAME, rect.x1 + FRAME, rect.y1 + FRAME), WHITE);
        }
    }
}
