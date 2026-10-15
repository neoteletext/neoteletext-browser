//! The wall's transforms, as the prototype's CSS defines them, applied to
//! points: the fan turns each page about its own centre, and the tilt leans
//! the wall back about its top centre, seen in perspective.

use vello_cpu::kurbo::{Point, Vec2};

/// The tilt, from the prototype's `.wall--tilt`: rotateX(22deg)
/// rotateZ(-4deg) scale(0.94), seen with `perspective: 1800px`.
const TILT_X_DEGREES: f64 = 22.0;
const TILT_Z_DEGREES: f64 = -4.0;
const TILT_SCALE: f64 = 0.94;
const PERSPECTIVE: f64 = 1800.0;
/// The fan, from `.wall--fan .egui-card:nth-child(5n + k)`: an angle in
/// degrees and a lift in pixels, repeating every five pages.
const FAN: [(f64, f64); 5] = [(-2.5, -3.0), (-3.5, 0.0), (2.0, 8.0), (-1.0, -6.0), (4.0, 4.0)];

/// How far each transform is applied, from 0 to 1, and where the wall is.
#[derive(Clone, Copy)]
pub struct Pose {
    pub tilt: f64,
    pub fan: f64,
    pub wall_origin: Point,
    pub perspective_origin: Point,
}

impl Pose {
    pub const FLAT: Self = Self { tilt: 0.0, fan: 0.0, wall_origin: Point::ORIGIN, perspective_origin: Point::ORIGIN };

    pub fn transformed(&self) -> bool {
        self.tilt > f64::EPSILON || self.fan > f64::EPSILON
    }

    pub fn place(&self, index: usize, centre: Point, point: Point) -> Point {
        // `nth-child` counts from 1.
        let (degrees, lift) = FAN[(index + 1) % FAN.len()];
        let (sine, cosine) = (degrees * self.fan).to_radians().sin_cos();
        let offset = point - centre;
        let point = centre + Vec2::new(offset.x * cosine - offset.y * sine, offset.x * sine + offset.y * cosine + lift * self.fan);

        let scale = 1.0 + (TILT_SCALE - 1.0) * self.tilt;
        let (sine_z, cosine_z) = (TILT_Z_DEGREES * self.tilt).to_radians().sin_cos();
        let (sine_x, cosine_x) = (TILT_X_DEGREES * self.tilt).to_radians().sin_cos();
        let local = (point - self.wall_origin) * scale;
        let turned = Vec2::new(local.x * cosine_z - local.y * sine_z, local.x * sine_z + local.y * cosine_z);
        // rotateX: y shortens and the point comes towards the viewer (CSS z).
        let depth = turned.y * sine_x;
        let flat = self.wall_origin + Vec2::new(turned.x, turned.y * cosine_x);
        self.perspective_origin + (flat - self.perspective_origin) * (PERSPECTIVE / (PERSPECTIVE - depth))
    }
}
