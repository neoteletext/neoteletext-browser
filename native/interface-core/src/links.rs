//! Hyperlinks as the interface shows them (`hyperlinks.md` sections 9 and
//! 10): where each link of the open page lies, drawn as a hairline under
//! its cells at the screen's own resolution, finer than any line a page can
//! draw; and what the trusted strip shows of a link, as the trusted core
//! wrote it. The interface holds no link's target, only where to draw it
//! and what kind of place it leads to (requirement 17.3).

use alloc::string::String;
use alloc::vec::Vec;

use crate::canvas::{COLUMNS, Canvas, PAGE_HEIGHT, ROWS};
use crate::geometry::Rect;
use crate::theme::{BLACK, LINK, WHITE};
use crate::ui::border;

/// The lines of the page image one teletext dot takes: a cell of 12 by 10
/// dots drawn in 20 lines (EN 300 706 clause 2, the engine's 12-by-20
/// mode). A page draws no long horizontal line thinner than one dot: its
/// characters, mosaics and smooth mosaics are all made of whole dots, and
/// character rounding adds half-dot steps only to diagonals.
pub const DOT_LINES: u16 = 2;
/// Dots down a page: 25 rows of 10.
const DOTS_DOWN: u16 = PAGE_HEIGHT / DOT_LINES;
/// Links are drawn only when a dot is at least one and a half device
/// pixels high, so that the one-pixel hairline is at most two thirds of the
/// page's finest line (requirement 17.2). At two pixels, as first written,
/// a page shown at its own size on an ordinary screen would show no links.
pub const MIN_DOT_PIXELS: f64 = 1.5;
/// From four device pixels a dot, the underline is two pixels thick.
const THICK_DOT_PIXELS: f64 = 4.0;
/// The underline: one device pixel, or two on a large page.
const THIN_PIXELS: f64 = 1.0;
const THICK_PIXELS: f64 = 2.0;
/// The underline under the pointer, or of the focused link, is one device
/// pixel thicker.
const HIGHLIGHT_PIXELS: f64 = 1.0;
/// The contrasting edge on either side of the underline.
const EDGE_PIXELS: f64 = 1.0;
/// The focus ring around the focused link's cells.
const FOCUS_PIXELS: f64 = 2.0;

/// What kind of place a link leads to: the interface shows the pointer and
/// the hairline by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// A page of this channel: followed at once.
    Page = 1,
    /// Another channel: followed after the site-change screen.
    Channel = 2,
    /// A web page: opened in the system's web browser after its prompt.
    Web = 3,
}

impl LinkKind {
    /// The kind a protocol byte names, or `None` for any other byte.
    #[must_use]
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Page),
            2 => Some(Self::Channel),
            3 => Some(Self::Web),
            _ => None,
        }
    }

    /// Whether following it leaves the channel.
    #[must_use]
    pub fn leaves(self) -> bool {
        self != Self::Page
    }
}

/// One link of a page: a run of cells on one row, the trusted core's
/// checks already applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkRun {
    /// The row, 1 to 23.
    pub row: u8,
    /// The first column.
    pub column: u8,
    /// How many cells.
    pub length: u8,
    /// Drawn in double height: the run also covers the row below, and its
    /// underline is under that row.
    pub double: bool,
    /// The cells' background is light: the underline's edge is black
    /// rather than white.
    pub light: bool,
    /// Where it leads.
    pub kind: LinkKind,
}

impl LinkRun {
    /// Whether the cell at `column`, `row` is part of the link.
    #[must_use]
    pub fn contains(&self, column: u8, row: u8) -> bool {
        let last_row = if self.double { self.row.saturating_add(1) } else { self.row };
        (self.row..=last_row).contains(&row) && (self.column..self.column.saturating_add(self.length)).contains(&column)
    }
}

/// The links of one image of a page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageLinks {
    /// The image they belong to: drawn only over that image.
    pub version: u32,
    /// Each link, in reading order.
    pub runs: Vec<LinkRun>,
    /// Page numbers that, typed while the page is open, follow a link
    /// rather than going to that page (`LK` lines and the channel's
    /// aliases).
    pub remapped: Vec<u16>,
}

impl PageLinks {
    /// The link at `column`, `row`, by its place in reading order.
    #[must_use]
    pub fn run_at(&self, column: u8, row: u8) -> Option<usize> {
        self.runs.iter().position(|run| run.contains(column, row))
    }
}

/// What the trusted strip shows while the pointer or the keyboard focus is
/// on a link: its destination, as the trusted core wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pointing {
    /// What kind of place it is.
    pub kind: LinkKind,
    /// Where it leads.
    pub text: String,
}

/// The two questions the trusted strip asks before a link leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// The site-change screen: the link leads to another channel
    /// (requirement 8.4).
    Leave = 1,
    /// The web-browser prompt: the link leads to a web page
    /// (`hyperlinks.md` section 12).
    Web = 2,
}

impl PromptKind {
    /// The kind a protocol byte names, or `None` for any other byte.
    #[must_use]
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Leave),
            2 => Some(Self::Web),
            _ => None,
        }
    }
}

/// A question the trusted strip asks, drawn only there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// Which question: a confirmation names it, so a confirmation meant for
    /// an earlier prompt confirms nothing.
    pub id: u32,
    /// What it asks.
    pub kind: PromptKind,
    /// The destination, as the trusted core wrote it.
    pub destination: String,
    /// The destination would be reached over an encrypted connection.
    pub secure: bool,
}

/// A cell of a page image, for the trusted core: where the pointer or the
/// keyboard's focus is on a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkCell {
    /// The page.
    pub key: crate::canvas::PageKey,
    /// The version of its image on screen.
    pub version: u32,
    /// The column, 0 to 39.
    pub column: u8,
    /// The row, 0 to 24.
    pub row: u8,
}

/// The answers to the strip's question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptButton {
    /// Go there, or open it in the web browser.
    Go,
    /// Stay.
    Cancel,
}

/// The device pixels one teletext dot takes when the page is drawn in
/// `rect` on `canvas`.
fn dot_pixels<C: Canvas + ?Sized>(canvas: &C, rect: Rect) -> f64 {
    rect.height() / f64::from(DOTS_DOWN) * canvas.device_scale()
}

/// Whether links are drawn over a page drawn in `rect`: a dot must be at
/// least one and a half device pixels high.
#[must_use]
pub fn shown<C: Canvas + ?Sized>(canvas: &C, rect: Rect) -> bool {
    dot_pixels(canvas, rect) >= MIN_DOT_PIXELS
}

/// The cells of `run` on a page drawn in `rect`.
#[must_use]
pub fn cells(rect: Rect, run: &LinkRun) -> Rect {
    let (width, height) = (rect.width() / f64::from(COLUMNS), rect.height() / f64::from(ROWS));
    let rows = if run.double { 2.0 } else { 1.0 };
    let x0 = rect.x0 + f64::from(run.column) * width;
    let y0 = rect.y0 + f64::from(run.row) * height;
    Rect::new(x0, y0, x0 + f64::from(run.length) * width, y0 + rows * height)
}

/// Where the underline of `run` lies on a page drawn in `rect`, with its
/// thickness in device pixels: along the bottom of the run's cells.
#[must_use]
pub fn underline(rect: Rect, run: &LinkRun, pixels: f64, scale: f64) -> Rect {
    let cells = cells(rect, run);
    Rect::new(cells.x0, cells.y1 - pixels / scale, cells.x1, cells.y1)
}

/// Draws the links of a page drawn in `rect`: a blue hairline under each,
/// with a contrasting edge either side, finer than any line of the page
/// (`hyperlinks.md` section 9.2). `highlighted` is the link under the
/// pointer, `focused` the keyboard's.
pub fn draw<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, runs: &[LinkRun], highlighted: Option<usize>, focused: Option<usize>) {
    let dot = dot_pixels(canvas, rect);
    if dot < MIN_DOT_PIXELS {
        return;
    }
    let scale = canvas.device_scale();
    let base = if dot >= THICK_DOT_PIXELS { THICK_PIXELS } else { THIN_PIXELS };
    let edge = EDGE_PIXELS / scale;
    for (index, run) in runs.iter().enumerate() {
        let marked = highlighted == Some(index) || focused == Some(index);
        let line = underline(rect, run, base + if marked { HIGHLIGHT_PIXELS } else { 0.0 }, scale);
        canvas.fill(Rect::new(line.x0, line.y0 - edge, line.x1, line.y1 + edge), 0.0, if run.light { BLACK } else { WHITE });
        canvas.fill(line, 0.0, LINK);
        if focused == Some(index) {
            let ring = FOCUS_PIXELS / scale;
            let around = cells(rect, run);
            border(canvas, Rect::new(around.x0 - ring, around.y0 - ring, around.x1 + ring, around.y1 + ring), ring, LINK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_canvas::Recorder;

    const PAGE: Rect = Rect::new(0.0, 0.0, 480.0, 500.0);

    fn run(row: u8, column: u8, length: u8, double: bool) -> LinkRun {
        LinkRun { row, column, length, double, light: false, kind: LinkKind::Page }
    }

    #[test]
    fn a_run_covers_its_cells_and_the_row_below_in_double_height() {
        let single = run(5, 10, 3, false);
        assert!(single.contains(10, 5) && single.contains(12, 5));
        assert!(!single.contains(13, 5) && !single.contains(9, 5) && !single.contains(10, 6));
        let double = run(5, 10, 3, true);
        assert!(double.contains(11, 6) && !double.contains(11, 7));
    }

    #[test]
    fn the_underline_is_along_the_bottom_of_the_cells_and_finer_than_a_dot() {
        // At native size a dot is two lines: at scale 2 that is four device
        // pixels, and the underline is two of them.
        let line = underline(PAGE, &run(5, 10, 3, false), THICK_PIXELS, 2.0);
        assert_eq!((line.x0, line.x1), (120.0, 156.0));
        assert_eq!((line.y0, line.y1), (119.0, 120.0), "two device pixels at scale 2, ending at row 6's top");
        let dot_height = f64::from(DOT_LINES);
        assert!(line.height() < dot_height, "finer than one dot of the page");
        // Double height: under the row below.
        assert_eq!(underline(PAGE, &run(5, 10, 3, true), THIN_PIXELS, 1.0).y1, 140.0);
    }

    #[test]
    fn links_are_drawn_only_when_a_dot_is_one_and_a_half_device_pixels() {
        let mut canvas = Recorder::new();
        canvas.scale = 1.0;
        // A dot is two lines of the native image: two device pixels at scale 1.
        assert!(shown(&canvas, PAGE));
        // A little under the native size on an ordinary screen still shows them.
        assert!(shown(&canvas, Rect::new(0.0, 0.0, 478.0, 498.0)));
        assert!(shown(&canvas, Rect::new(0.0, 0.0, 360.0, 375.0)), "a dot of one and a half pixels");
        assert!(!shown(&canvas, Rect::new(0.0, 0.0, 359.0, 374.0)));
        assert!(!shown(&canvas, Rect::new(0.0, 0.0, 240.0, 250.0)), "half size: one pixel a dot");
        canvas.scale = 2.0;
        assert!(shown(&canvas, Rect::new(0.0, 0.0, 240.0, 250.0)));
        canvas.fills.clear();
        draw(&mut canvas, Rect::new(0.0, 0.0, 120.0, 125.0), &[run(5, 1, 3, false)], None, None);
        assert!(canvas.fills.is_empty(), "nothing drawn at half a pixel a dot");
        draw(&mut canvas, PAGE, &[run(5, 1, 3, false)], None, None);
        assert_eq!(canvas.fills.len(), 2, "the edge, then the blue line");
        assert_eq!(canvas.fills[1].1, LINK);
        assert_eq!(canvas.fills[0].1, WHITE, "a dark background gets a white edge");
    }

    #[test]
    fn the_highlighted_link_is_thicker_and_the_focused_one_is_ringed() {
        let mut canvas = Recorder::new();
        canvas.scale = 2.0;
        let runs = [run(5, 1, 3, false), LinkRun { light: true, ..run(7, 1, 3, false) }];
        draw(&mut canvas, PAGE, &runs, Some(0), None);
        let thickness = |canvas: &Recorder, index: usize| canvas.fills[index].0.height();
        assert!(thickness(&canvas, 1) > thickness(&canvas, 3), "the pointer's link is thicker");
        assert_eq!(canvas.fills[2].1, BLACK, "a light background gets a black edge");
        canvas.fills.clear();
        draw(&mut canvas, PAGE, &runs, None, Some(1));
        assert_eq!(canvas.fills.len(), 2 + 2 + 4, "two underlines, then the focus ring's four sides");
    }
}
