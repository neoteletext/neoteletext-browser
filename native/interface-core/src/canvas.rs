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

/// A ReGIS picture's width in pixels (the VT340's screen; the ReGIS
/// renderer's DEC-6296).
pub const REGIS_WIDTH: u16 = 800;
/// A ReGIS picture's height in pixels.
pub const REGIS_HEIGHT: u16 = 480;
/// The colours of a ReGIS picture: each pixel is one of sixteen.
pub const REGIS_COLOURS: usize = 16;
/// A picture drawn by the ReGIS renderer (DEC-6335) is 800 by 480 pixels,
/// wider for its height than a page's image, so it is drawn across the
/// image's whole width and this many of its 500 lines, which keeps the
/// picture's own proportions (480 dots to 288 lines is 800 to 480).
pub const REGIS_LINES: u16 = 288;
/// The image's line a ReGIS picture's top row is drawn on: the picture is
/// centred down the image.
pub const REGIS_TOP_LINE: u16 = (PAGE_HEIGHT - REGIS_LINES) / 2;

/// The lines of the teletext raster a NAPLPS page's display area is drawn
/// on (the NAPLPS renderer's REQ-5819, DEC-6667): 480 by 462 pixels, which
/// on the raster's pixels, 16/13 as wide as they are high (DEC-2049), keep
/// the NAPLPS display's shape of 1.28 to 1.
pub const NAPLPS_DISPLAY_LINES: u16 = 462;
/// The raster's line a NAPLPS display area's top row is drawn on: the 38
/// lines left over are shared equally above and below it (REQ-5819).
pub const NAPLPS_TOP_LINE: u16 = (PAGE_HEIGHT - NAPLPS_DISPLAY_LINES) / 2;
const _: () = assert!(NAPLPS_TOP_LINE == 19 && NAPLPS_TOP_LINE * 2 + NAPLPS_DISPLAY_LINES == PAGE_HEIGHT, "REQ-5819: 19 border lines above and below");

/// The language a page's picture is drawn in, which says its size: which
/// renderer made it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PictureLanguage {
    /// ReGIS (DEC-6335): 800 by 480 pixels, the VT340's screen.
    #[default]
    Regis,
    /// NAPLPS, ANSI X3.110-1983 (the NAPLPS renderer's REQ-6408): the
    /// teletext raster itself, 480 by 500, the page's display area on
    /// lines 19 to 480 and the lines above and below it in the page's
    /// background colour (REQ-5819, DEC-6667).
    Naplps,
}

impl PictureLanguage {
    /// The picture's width in pixels.
    #[must_use]
    pub const fn width(self) -> u16 {
        match self {
            Self::Regis => REGIS_WIDTH,
            Self::Naplps => PAGE_WIDTH,
        }
    }

    /// The picture's height in pixels.
    #[must_use]
    pub const fn height(self) -> u16 {
        match self {
            Self::Regis => REGIS_HEIGHT,
            Self::Naplps => PAGE_HEIGHT,
        }
    }

    /// The picture's pixels.
    #[must_use]
    pub const fn pixels(self) -> usize {
        (self.width() as usize).saturating_mul(self.height() as usize)
    }
}

/// The widest picture of any language: what a pointer's pixel is below
/// before the picture it is on is known.
pub const PICTURE_MAX_WIDTH: u16 = if REGIS_WIDTH > PAGE_WIDTH { REGIS_WIDTH } else { PAGE_WIDTH };
/// The tallest picture of any language.
pub const PICTURE_MAX_HEIGHT: u16 = if REGIS_HEIGHT > PAGE_HEIGHT { REGIS_HEIGHT } else { PAGE_HEIGHT };

/// Where the parts of a page drawn as a picture in `language` sit in its
/// image drawn in `image`. A NAPLPS picture is the teletext raster
/// (REQ-5819), so it fills the image as a teletext page does, whatever the
/// fit; row 0 of its teletext page is drawn over the raster's lines 0 to
/// 19 and the row of coloured-key prompts, when shown, over lines 480 to
/// 499, where the teletext page has them, covering the picture's first and
/// last lines (renderer parity PAR-31). A ReGIS picture is laid out by
/// [`regis_layout`].
#[must_use]
pub fn picture_layout(language: PictureLanguage, image: Rect, contain: bool, keyed: bool) -> RegisLayout {
    match language {
        PictureLanguage::Regis => regis_layout(image, contain, keyed),
        PictureLanguage::Naplps => {
            let row = image.height() / f64::from(ROWS);
            let keys = keyed.then(|| Rect::new(image.x0, image.y1 - row, image.x1, image.y1));
            RegisLayout { header: Rect::new(image.x0, image.y0, image.x1, image.y0 + row), picture: image, keys, stacked: false }
        }
    }
}

/// Where a ReGIS picture sits in a page's image drawn in `image`.
#[must_use]
pub fn regis_within(image: Rect) -> Rect {
    let line = image.height() / f64::from(PAGE_HEIGHT);
    let top = image.y0 + line * f64::from(REGIS_TOP_LINE);
    Rect::new(image.x0, top, image.x1, top + line * f64::from(REGIS_LINES))
}

/// A ReGIS picture's height for each unit of its width: 480 rows to 800
/// pixels.
pub const REGIS_HEIGHT_PER_WIDTH: f64 = REGIS_HEIGHT as f64 / REGIS_WIDTH as f64;

/// Where a ReGIS picture sits when it is contained in `area` (REQ-6363):
/// as large as fits at its own shape, limited by whichever of the width
/// and the height runs out first, and centred. In an area of the picture's
/// own shape that is the whole area.
#[must_use]
pub fn regis_contained(area: Rect) -> Rect {
    let width = area.width().min(area.height() / REGIS_HEIGHT_PER_WIDTH).max(0.0);
    let height = width * REGIS_HEIGHT_PER_WIDTH;
    let (left, top) = ((area.x0 + area.x1 - width) / 2.0, (area.y0 + area.y1 - height) / 2.0);
    Rect::new(left, top, left + width, top + height)
}

/// A teletext row's height for each unit of its page's width: 20 dots to
/// 480.
pub const ROW_HEIGHT_PER_WIDTH: f64 = CELL_HEIGHT as f64 / PAGE_WIDTH as f64;

/// The height for each unit of width of a contained ReGIS picture with its
/// teletext page's row 0 above it (renderer parity PAR-31): the picture's,
/// and a teletext row's at the same width.
pub const REGIS_HEADED_HEIGHT_PER_WIDTH: f64 = REGIS_HEIGHT_PER_WIDTH + ROW_HEIGHT_PER_WIDTH;

/// The same with the row of coloured-key prompts beneath it as well
/// (REQ-6379): two teletext rows' at the picture's width.
pub const REGIS_KEYED_HEIGHT_PER_WIDTH: f64 = REGIS_HEADED_HEIGHT_PER_WIDTH + ROW_HEIGHT_PER_WIDTH;

/// Where the parts of a page drawn in ReGIS sit in its image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegisLayout {
    /// Row 0 of the teletext page beneath: the receiver's status area and
    /// the channel's header and clock, drawn above the picture at the scale
    /// row 24 is drawn at (renderer parity PAR-31).
    pub header: Rect,
    /// The picture.
    pub picture: Rect,
    /// Row 24 of the teletext page beneath, the coloured-key prompts,
    /// when the page shows it (REQ-6379).
    pub keys: Option<Rect>,
    /// The header, the picture and the row together are the page: the
    /// header is row 0 of the teletext page beneath, the picture stands for
    /// its rows 1 to 23, or 1 to 24 with no row of prompts, and the row is
    /// its row 24. False when the picture is drawn in the teletext page's
    /// own shape, where every row is where the teletext page has it.
    pub stacked: bool,
}

/// Where a ReGIS picture, and the row of coloured-key prompts when
/// `keyed`, sit in a page's image drawn in `image` (REQ-6363, REQ-6379).
/// Contained in the page area, the picture is as large as fits with the
/// row directly beneath it, at a teletext row's height for the picture's
/// width; contained to the teletext page, the picture is in its band and
/// the row where the teletext page has it.
#[must_use]
pub fn regis_layout(image: Rect, contain: bool, keyed: bool) -> RegisLayout {
    if !contain {
        let row = image.height() / f64::from(ROWS);
        let keys = keyed.then(|| Rect::new(image.x0, image.y1 - row, image.x1, image.y1));
        let header = Rect::new(image.x0, image.y0, image.x1, image.y0 + row);
        return RegisLayout { header, picture: regis_within(image), keys, stacked: false };
    }
    // Row 0, the picture and, when shown, row 24, stacked at the
    // picture's width: as large as fits, centred (PAR-31, REQ-6379).
    let shape = if keyed { REGIS_KEYED_HEIGHT_PER_WIDTH } else { REGIS_HEADED_HEIGHT_PER_WIDTH };
    let width = image.width().min(image.height() / shape).max(0.0);
    let (left, top) = ((image.x0 + image.x1 - width) / 2.0, (image.y0 + image.y1 - width * shape) / 2.0);
    let row = width * ROW_HEIGHT_PER_WIDTH;
    let header = Rect::new(left, top, left + width, top + row);
    let middle = top + row + width * REGIS_HEIGHT_PER_WIDTH;
    let picture = Rect::new(left, top + row, left + width, middle);
    let keys = keyed.then(|| Rect::new(left, middle, left + width, middle + row));
    RegisLayout { header, picture, keys, stacked: true }
}

impl RegisLayout {
    /// The point of `image` at which the teletext page beneath, drawn
    /// across the whole of `image`, would have the cell that is under
    /// `point` here: what finds a link or a prompt under the pointer. Where
    /// the picture and the row are stacked, the picture's height stands
    /// for rows 0 to 23 and the row is row 24; a point on neither is on no
    /// cell. Otherwise the page's rows are where the teletext page has them.
    #[must_use]
    pub fn grid_point(&self, image: Rect, point: Point) -> Option<Point> {
        if !self.stacked {
            return Some(point);
        }
        let rows = f64::from(ROWS);
        // The picture stands for the rows between the header's and the
        // prompts', or to the last row with no prompts.
        let below = if self.keys.is_some() { 1.0 } else { 0.0 };
        let down = if let Some(keys) = self.keys.filter(|keys| keys.contains(point)) {
            (rows - 1.0 + (point.y - keys.y0) / keys.height()) / rows
        } else if self.header.contains(point) {
            (point.y - self.header.y0) / self.header.height() / rows
        } else if self.picture.contains(point) {
            (1.0 + (point.y - self.picture.y0) / self.picture.height() * (rows - 1.0 - below)) / rows
        } else {
            return None;
        };
        let across = (point.x - self.picture.x0) / self.picture.width();
        Some(Point::new(image.x0 + across * image.width(), image.y0 + down * image.height()))
    }
}

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

/// How far past a whole multiple of its actual size the room for an open
/// page may grow while the page is still held at that multiple, as a share
/// of the multiple (REQ-6387): enough to feel as the window is dragged,
/// and never more than a twentieth of the page left as margin.
pub const STICK_SHARE: f64 = 0.05;

/// How long the label that names the size a page has come to be held at
/// stays, in milliseconds (REQ-6387).
pub const STICK_LABEL_MILLISECONDS: u64 = 1_500;

/// The width an open page's image is drawn at when its room allows
/// `width`, and the whole multiple of its actual size it is held at, if it
/// is (REQ-6387). `actual` is the page's own width in its own pixels and
/// `device_scale` the display's physical pixels for each unit of `width`.
/// While the room is a whole multiple of the actual size or up to
/// [`STICK_SHARE`] more, the page is exactly that multiple, each of its
/// pixels a whole number of the display's; otherwise it fills its room.
/// The band lies only above the multiple: a page cannot be larger than
/// its room.
#[must_use]
pub fn sticky_width(width: f64, actual: f64, device_scale: f64) -> (f64, Option<u32>) {
    if width <= 0.0 || actual <= 0.0 || device_scale <= 0.0 {
        return (width, None);
    }
    let multiples = width * device_scale / actual;
    let whole = libm::floor(multiples);
    if whole < 1.0 || multiples > whole * (1.0 + STICK_SHARE) {
        return (width, None);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "at least 1, and a display is not billions of pixels wide")]
    let held = whole.min(f64::from(u32::MAX)) as u32;
    (whole * actual / device_scale, Some(held))
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
    /// Keeps a still of a page under the number `still` (the tab
    /// filmstrip's, REQ-6640 item 3): the page with its screen as
    /// [`draw_page_on_screen`] draws it, or its ReGIS or NAPLPS picture as
    /// the page area lays it, made now from the page's latest image at
    /// [`STILL_WIDTH`] by [`STILL_HEIGHT`] pixels and never changed by a
    /// later image. A still kept before under the number is replaced.
    /// Returns false, keeping nothing, when the back end holds no image
    /// of the page. A back end without the memory for stills (a
    /// microcontroller's) keeps none, and the filmstrip shows stand-ins.
    fn keep_still(&mut self, still: u32, key: PageKey) -> bool {
        let _ = (still, key);
        false
    }
    /// Draws the still kept under `still`, scaled to fill `rect`; false
    /// when none is kept.
    fn still(&mut self, still: u32, rect: Rect) -> bool {
        let _ = (still, rect);
        false
    }
    /// Forgets the still kept under `still`, if one is.
    fn forget_still(&mut self, still: u32) {
        let _ = still;
    }
}

/// A still's width in pixels (REQ-6640 item 3): a page with its screen
/// shown about a fifth of a large window wide, 256 points on a window of
/// 1,280, is drawn from it at about one and a half of a display's pixels
/// for each of the still's on a display of two for each point.
pub const STILL_WIDTH: u16 = 384;
/// A still's height in pixels: the page-with-screen's own shape, 480 dots
/// to 500 lines (its screen is as wide in proportion both ways).
pub const STILL_HEIGHT: u16 = 400;
const _: () = assert!(STILL_WIDTH as u32 * PAGE_HEIGHT as u32 == STILL_HEIGHT as u32 * PAGE_WIDTH as u32, "a still is the page's shape");
/// The most stills held, one for each of as many tabs (REQ-6640 item 3):
/// 24 of 384 by 400 pixels of four bytes is about 15 megabytes, against a
/// window's 64 tabs at most (the protocol's `MAX_TABS`). The tabs left
/// longest ago lose theirs first and show a stand-in.
pub const MAX_STILLS: usize = 24;
