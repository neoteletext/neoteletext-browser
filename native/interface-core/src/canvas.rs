//! The drawing interface of design section 4.1: everything the interface
//! draws goes through these calls, and each drawing back end implements
//! them. Coordinates are logical pixels; a back end applies the display
//! scale.

use neoteletext_core::consts::{ASPECT_DENOMINATOR, ASPECT_NUMERATOR};

use crate::geometry::{Point, Rect};
use crate::theme::Colour;

/// A teletext page, as a channel and a page number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PageKey {
    /// The channel, as the trusted core numbers it.
    pub channel: u32,
    /// The page number, three hexadecimal digits as teletext writes it (0x100 is page 100).
    pub page: u16,
}

/// The native teletext raster: 40 columns of 12 dots and 25 rows of 20
/// lines (EN 300 706 clause 2; the engine's `CELL_WIDTH` and `CELL_HEIGHT`).
/// Character cells across a page.
pub const COLUMNS: u16 = 40;
/// Character rows down a page.
pub const ROWS: u16 = 25;
/// Dots across one character cell.
pub const CELL_WIDTH: u16 = 12;
/// Lines down one character cell.
pub const CELL_HEIGHT: u16 = 20;
/// A page's width in dots at its native size.
pub const PAGE_WIDTH: u16 = COLUMNS * CELL_WIDTH;
/// A page's height in lines at its native size.
pub const PAGE_HEIGHT: u16 = ROWS * CELL_HEIGHT;
/// The screen around a page, in dots across on each side: one character
/// cell. EN 300 706 colours the screen area outside the page's rows in the
/// page's screen colour (Table 4, Default Screen Colour; clause 12.3, Full
/// Screen Colour) but gives it no size; a television shows it as its
/// border, and the browser draws it one cell wide (decision 60).
pub const SCREEN_BORDER: u16 = CELL_WIDTH;

/// How much larger a page with its screen is than its image, both ways.
pub const SCREEN_SCALE: f64 = (PAGE_WIDTH as f64 + 2.0 * SCREEN_BORDER as f64) / PAGE_WIDTH as f64;

/// The shape a page is drawn in, which only the drawing transform changes:
/// the page's raster and every coordinate sent to its channel stay the same
/// (REQ-737).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageShape {
    /// The raster's own proportions, every dot as wide as a line is high.
    #[default]
    Raster,
    /// As a 4:3 television shows the raster: wider by the engine's aspect
    /// factor, its height unchanged (DEC-2049).
    Corrected,
}

impl PageShape {
    /// The shape for the viewer's View menu setting (REQ-737 item 2).
    #[must_use]
    pub const fn of_setting(corrected: bool) -> Self {
        if corrected { Self::Corrected } else { Self::Raster }
    }

    /// How many times wider a page is drawn than the raster's proportions.
    #[must_use]
    pub const fn stretch(self) -> f64 {
        #[allow(clippy::cast_precision_loss, reason = "the engine's 16 and 13, exact in any float")]
        match self {
            Self::Raster => 1.0,
            Self::Corrected => ASPECT_NUMERATOR as f64 / ASPECT_DENOMINATOR as f64,
        }
    }

    /// A page's height for each unit of its width.
    #[must_use]
    pub const fn height_per_width(self) -> f64 {
        PAGE_HEIGHT as f64 / PAGE_WIDTH as f64 / self.stretch()
    }
}

/// Where a page's image sits when the page, with its screen, is drawn in
/// `rect`: smaller by [`SCREEN_SCALE`] both ways and centred, so the screen
/// is [`SCREEN_BORDER`] dots of the drawn page at the sides, and as wide in
/// proportion above and below. The shape is `rect`'s own: whoever lays a
/// page out gives it the page's proportions (REQ-740, DEC-953).
#[must_use]
pub fn image_within(rect: Rect) -> Rect {
    let (width, height) = (rect.width() / SCREEN_SCALE, rect.height() / SCREEN_SCALE);
    let (left, top) = (f64::midpoint(rect.x0, rect.x1) - width / 2.0, f64::midpoint(rect.y0, rect.y1) - height / 2.0);
    Rect::new(left, top, left + width, top + height)
}

/// Draws a page with its screen in `rect`, or `stand_in` there when the
/// back end holds no image of it yet; returns where the image is, for
/// hit-testing and links.
pub fn draw_page_on_screen<C: Canvas + ?Sized>(canvas: &mut C, key: PageKey, rect: Rect, stand_in: Colour) -> Rect {
    let image = image_within(rect);
    match canvas.screen(key) {
        Some(screen) => {
            canvas.fill(rect, 0.0, screen);
            canvas.page(key, image);
        }
        None => canvas.fill(rect, 0.0, stand_in),
    }
    image
}

/// Text styles: a size in logical pixels, whether monospaced or bold, and
/// whether drawn in small capitals: every letter a capital, with letters
/// written in lower case drawn as smaller capitals. A back end with bitmap
/// fonts picks the nearest font it has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// The size in logical pixels.
    pub size: f32,
    /// Every character the same width.
    pub monospace: bool,
    /// Bold.
    pub bold: bool,
    /// Small capitals.
    pub caps: bool,
}

/// Everything the interface draws with; each drawing back end implements it.
pub trait Canvas {
    /// A rectangle, rounded when `radius` is above zero.
    fn fill(&mut self, rect: Rect, radius: f64, colour: Colour);
    /// Darkens `rect` towards black by `amount`, from 0 (no change) to 1
    /// (black). A back end without transparency (a microcontroller's) may
    /// leave it undarkened.
    fn dim(&mut self, rect: Rect, amount: f32);
    /// A filled polygon, closed from its last point to its first.
    fn polygon(&mut self, points: &[Point], colour: Colour);
    /// Clips everything drawn until the matching [`Canvas::pop_clip`] to
    /// `rect`, within any clip already in force.
    fn push_clip(&mut self, rect: Rect);
    /// Ends the clip of the matching [`Canvas::push_clip`].
    fn pop_clip(&mut self);
    /// A rounded rectangle filled with a linear gradient from its top-left
    /// to its bottom-right corner, through `stops` (offset, colour).
    fn gradient(&mut self, rect: Rect, radius: f64, stops: &[(f32, Colour)]);
    /// A page's latest image, scaled to fill `rect`. Returns false when
    /// the back end holds no image of that page yet.
    fn page(&mut self, key: PageKey, rect: Rect) -> bool;
    /// The colour of the screen around a page's latest image, as its frame
    /// said; none when the back end holds no image of that page yet.
    fn screen(&mut self, key: PageKey) -> Option<Colour>;
    /// Text with its top-left corner at `rect`'s top-left, clipped to `rect`.
    fn text(&mut self, rect: Rect, text: &str, style: Style, colour: Colour);
    /// The width of `text` in logical pixels.
    fn text_width(&mut self, text: &str, style: Style) -> f64;
    /// The height of one line of text in `style`, in logical pixels.
    fn line_height(&mut self, style: Style) -> f64;
    /// Device pixels per logical pixel: what the browser draws finer than a
    /// page's own dots is measured in device pixels (`hyperlinks.md`
    /// section 9.2).
    fn device_scale(&self) -> f64 {
        1.0
    }
}
