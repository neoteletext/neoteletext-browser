//! Drawing helpers on tiny-skia: colours, rounded rectangles, and pages
//! placed with the wall's transforms.

use tiny_skia::{Color, ColorU8, FillRule, FilterQuality, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Point, Rect, Transform};

use crate::pages::Page;

/// Theme colours are 8-bit, which tiny-skia can build at compile time.
pub const fn colour(red: u8, green: u8, blue: u8) -> ColorU8 {
    ColorU8::from_rgba(red, green, blue, u8::MAX)
}

pub fn float(colour: ColorU8) -> Color {
    Color::from_rgba8(colour.red(), colour.green(), colour.blue(), colour.alpha())
}

/// The prototype's dark theme (`src/shared/tokens.css`).
pub const RAISED: ColorU8 = colour(0x1b, 0x1e, 0x25);
pub const FIELD: ColorU8 = colour(0x0f, 0x11, 0x15);
pub const WELL: ColorU8 = colour(0x0c, 0x0e, 0x12);
pub const FRAME: ColorU8 = colour(0x11, 0x13, 0x18);
pub const INK: ColorU8 = colour(0xe6, 0xe9, 0xef);
pub const MUTED: ColorU8 = colour(0x93, 0x99, 0xa6);
pub const LINE: ColorU8 = colour(0x2b, 0x2f, 0x38);
pub const LINE_SOFT: ColorU8 = colour(0x20, 0x24, 0x2b);
pub const ACCENT: ColorU8 = colour(0x8f, 0xa1, 0xff);

pub fn paint(colour: ColorU8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(float(colour));
    paint.anti_alias = true;
    paint
}

/// A rectangle with rounded corners, in logical pixels.
pub fn rounded(rect: Rect, radius: f32) -> Option<Path> {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let (left, top, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let mut path = PathBuilder::new();
    path.move_to(left + radius, top);
    path.line_to(right - radius, top);
    path.quad_to(right, top, right, top + radius);
    path.line_to(right, bottom - radius);
    path.quad_to(right, bottom, right - radius, bottom);
    path.line_to(left + radius, bottom);
    path.quad_to(left, bottom, left, bottom - radius);
    path.line_to(left, top + radius);
    path.quad_to(left, top, left + radius, top);
    path.close();
    path.finish()
}

pub fn fill(pixmap: &mut Pixmap, scale: f32, rect: Option<Rect>, radius: f32, colour: ColorU8) {
    if let Some(path) = rect.and_then(|rect| rounded(rect, radius)) {
        pixmap.fill_path(&path, &paint(colour), FillRule::Winding, Transform::from_scale(scale, scale), None);
    }
}

/// The tilt, from the prototype's `.wall--tilt`: rotateX(22deg)
/// rotateZ(-4deg) scale(0.94), seen with `perspective: 1800px`.
const TILT_X_DEGREES: f32 = 22.0;
const TILT_Z_DEGREES: f32 = -4.0;
const TILT_SCALE: f32 = 0.94;
const PERSPECTIVE: f32 = 1800.0;
/// The fan, from `.wall--fan .egui-card:nth-child(5n + k)`: an angle in
/// degrees and a lift in pixels, repeating every five pages.
const FAN: [(f32, f32); 5] = [(-2.5, -3.0), (-3.5, 0.0), (2.0, 8.0), (-1.0, -6.0), (4.0, 4.0)];

/// How far each transform has been applied, from 0 to 1, as they animate.
#[derive(Clone, Copy)]
pub struct Pose {
    pub tilt: f32,
    pub fan: f32,
    /// The wall's top centre, and its centre, in logical pixels.
    pub wall_origin: Point,
    pub perspective_origin: Point,
}

impl Pose {
    pub fn fan_of(index: usize) -> (f32, f32) {
        // `nth-child` counts from 1.
        FAN[(index + 1) % FAN.len()]
    }

    fn fan(&self, index: usize, centre: Point, point: Point) -> Point {
        let (degrees, lift) = Self::fan_of(index);
        let (sine, cosine) = (degrees * self.fan).to_radians().sin_cos();
        let (x, y) = (point.x - centre.x, point.y - centre.y);
        Point::from_xy(centre.x + x * cosine - y * sine, centre.y + x * sine + y * cosine + lift * self.fan)
    }

    fn tilt(&self, point: Point) -> Point {
        let scale = 1.0 + (TILT_SCALE - 1.0) * self.tilt;
        let (sine_z, cosine_z) = (TILT_Z_DEGREES * self.tilt).to_radians().sin_cos();
        let (sine_x, cosine_x) = (TILT_X_DEGREES * self.tilt).to_radians().sin_cos();
        let (x, y) = ((point.x - self.wall_origin.x) * scale, (point.y - self.wall_origin.y) * scale);
        let (x, y) = (x * cosine_z - y * sine_z, x * sine_z + y * cosine_z);
        // rotateX: y shortens and the point comes towards the viewer (CSS z).
        let depth = y * sine_x;
        let flat = Point::from_xy(self.wall_origin.x + x, self.wall_origin.y + y * cosine_x);
        let magnification = PERSPECTIVE / (PERSPECTIVE - depth);
        Point::from_xy(
            self.perspective_origin.x + (flat.x - self.perspective_origin.x) * magnification,
            self.perspective_origin.y + (flat.y - self.perspective_origin.y) * magnification,
        )
    }

    pub fn place(&self, index: usize, centre: Point, point: Point) -> Point {
        self.tilt(self.fan(index, centre, point))
    }
}

/// The affine transform taking an image of `width` by `height` onto the
/// parallelogram with these three corners (device pixels). tiny-skia maps
/// x' = sx·x + kx·y + tx and y' = ky·x + sy·y + ty.
fn onto(width: f32, height: f32, top_left: Point, top_right: Point, bottom_left: Point) -> Transform {
    let across = ((top_right.x - top_left.x) / width, (top_right.y - top_left.y) / width);
    let down = ((bottom_left.x - top_left.x) / height, (bottom_left.y - top_left.y) / height);
    Transform::from_row(across.0, across.1, down.0, down.1, top_left.x, top_left.y)
}

/// Strips overlap their neighbour by this share of their height, so the
/// edges between them never show the background.
const STRIP_OVERLAP: f32 = 0.04;

/// Draws a page into `rect` (logical pixels), as the wall's pose places it.
/// Untilted, one affine transform is exact; tilted, the page is drawn as
/// strips, each exact for its own corners.
pub fn page(pixmap: &mut Pixmap, scale: f32, page: &Page, index: usize, rect: Rect, centre: Point, pose: &Pose) {
    let nearest = PixmapPaint { quality: FilterQuality::Nearest, ..PixmapPaint::default() };
    let corner = |x: f32, y: f32| {
        let point = pose.place(index, centre, Point::from_xy(x, y));
        Point::from_xy(point.x * scale, point.y * scale)
    };
    #[allow(clippy::cast_precision_loss, reason = "page sizes are small")]
    let (width, height) = (page.image.width() as f32, page.image.height() as f32);
    if pose.tilt <= f32::EPSILON {
        let transform = onto(width, height, corner(rect.left(), rect.top()), corner(rect.right(), rect.top()), corner(rect.left(), rect.bottom()));
        pixmap.draw_pixmap(0, 0, page.image.as_ref(), &nearest, transform, None);
        return;
    }
    #[allow(clippy::cast_precision_loss, reason = "a few strips")]
    let strip_height = rect.height() / page.strips.len() as f32;
    for (number, strip) in page.strips.iter().enumerate() {
        #[allow(clippy::cast_precision_loss, reason = "a few strips")]
        let top = rect.top() + number as f32 * strip_height;
        let bottom = top + strip_height * (1.0 + STRIP_OVERLAP);
        #[allow(clippy::cast_precision_loss, reason = "strip sizes are small")]
        let transform = onto(width, strip.height() as f32, corner(rect.left(), top), corner(rect.right(), top), corner(rect.left(), bottom));
        pixmap.draw_pixmap(0, 0, strip.as_ref(), &nearest, transform, None);
    }
}
