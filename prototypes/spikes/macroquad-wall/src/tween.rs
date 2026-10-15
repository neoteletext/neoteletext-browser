//! Animated values with the prototype's CSS timing functions.

use std::time::{Duration, Instant};

/// A CSS `cubic-bezier(x1, y1, x2, y2)` timing function.
#[derive(Clone, Copy)]
pub struct Curve {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

/// CSS `ease-out` (CSS Easing Functions Level 1, section 2.2).
pub const EASE_OUT: Curve = Curve { x1: 0.0, y1: 0.0, x2: 0.58, y2: 1.0 };
/// The prototype's transform curve, `cubic-bezier(0.2, 0.7, 0.1, 1)` (Wall.css).
pub const TRANSFORM_EASE: Curve = Curve { x1: 0.2, y1: 0.7, x2: 0.1, y2: 1.0 };

/// Newton iterations to find the curve's parameter for a time; the curve is
/// smooth, and this many settle well below a pixel.
const SOLVE_STEPS: usize = 8;

fn bezier(a: f32, b: f32, t: f32) -> f32 {
    // One axis of a cubic Bézier from (0, 0) to (1, 1) with control values a and b.
    let u = 1.0 - t;
    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
}

fn bezier_slope(a: f32, b: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * a + 6.0 * u * t * (b - a) + 3.0 * t * t * (1.0 - b)
}

impl Curve {
    pub fn at(self, progress: f32) -> f32 {
        let mut t = progress;
        for _ in 0..SOLVE_STEPS {
            let slope = bezier_slope(self.x1, self.x2, t);
            if slope.abs() < f32::EPSILON {
                break;
            }
            t = (t - (bezier(self.x1, self.x2, t) - progress) / slope).clamp(0.0, 1.0);
        }
        bezier(self.y1, self.y2, t)
    }
}

/// A value that moves to each new target over a fixed time.
pub struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
    curve: Curve,
}

impl Tween {
    pub fn new(value: f32, duration: Duration, curve: Curve) -> Self {
        Self { from: value, to: value, start: Instant::now(), duration, curve }
    }

    pub fn value(&self, now: Instant) -> f32 {
        let progress = (now.saturating_duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32()).min(1.0);
        self.from + (self.to - self.from) * self.curve.at(progress)
    }

    pub fn set(&mut self, target: f32, now: Instant) {
        if (target - self.to).abs() > f32::EPSILON {
            self.from = self.value(now);
            self.to = target;
            self.start = now;
        }
    }

    pub fn moving(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) < self.duration && (self.to - self.from).abs() > f32::EPSILON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_at_zero_and_end_at_one() {
        for curve in [EASE_OUT, TRANSFORM_EASE] {
            assert!(curve.at(0.0).abs() < 1e-4);
            assert!((curve.at(1.0) - 1.0).abs() < 1e-4);
            assert!(curve.at(0.5) > 0.5, "an ease-out is past halfway at half time");
        }
    }
}
