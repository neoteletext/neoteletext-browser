//! The drawing interface of design section 4.1: everything the interface
//! draws goes through these calls, and each drawing back end implements
//! them. Coordinates are logical pixels; a back end applies the display
//! scale.

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
    /// Text with its top-left corner at `rect`'s top-left, clipped to `rect`.
    fn text(&mut self, rect: Rect, text: &str, style: Style, colour: Colour);
    /// The width of `text` in logical pixels.
    fn text_width(&mut self, text: &str, style: Style) -> f64;
    /// The height of one line of text in `style`, in logical pixels.
    fn line_height(&mut self, style: Style) -> f64;
}
