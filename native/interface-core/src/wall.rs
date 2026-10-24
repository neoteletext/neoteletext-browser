//! The zoomable page area: the page size control along its top, and below
//! it the wall of the selected bank's pages, in a grid that grows and
//! shrinks with the page size or, below a size, a list. Because this code
//! lays out every page itself, it hit-tests the pointer against the same
//! rectangles it draws (requirement 12.3).

use alloc::vec::Vec;

use crate::canvas::{CELL_HEIGHT, CELL_WIDTH, COLUMNS, PAGE_HEIGHT, PAGE_WIDTH, PageKey, ROWS};
use crate::geometry::{Point, Rect};
use crate::size::{DEFAULT_WIDTH, Layout, MAX_WIDTH, layout_for_width};
use crate::tween::{EASE_OUT, STANDARD, Tween};
use crate::wall::hover::HOVER_MILLISECONDS;
use crate::wall::zoom::SLIDE_MILLISECONDS;
pub(crate) use crate::wall::zoom::ZOOM_MILLISECONDS;

mod draw;
mod grid;
mod hover;
mod size_control;
mod updates;
mod zoom;

pub use updates::{UPDATE_GLOW_MILLISECONDS, UPDATE_MARK_MILLISECONDS, glow, since};

/// The prototype's CSS transition for the page size: 160 ms ease-out.
const WIDTH_TRANSITION_MILLISECONDS: u64 = 160;

/// One page the wall shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WallPage<'a> {
    /// Which page.
    pub key: PageKey,
    /// Its title, shown in its caption.
    pub title: &'a str,
    /// When the page last had a new version, in milliseconds, if it has.
    pub updated: Option<u64>,
}

/// A click on one character cell of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellClick {
    /// The page clicked.
    pub key: PageKey,
    /// The version of the page's image on screen. The wall knows no
    /// versions; the browser fills it in.
    pub version: u32,
    /// The cell's column, 0 to 39.
    pub column: u8,
    /// The cell's row, 0 to 24.
    pub row: u8,
}

/// The cell of a page drawn in `rect` under `point`, as column and row.
#[must_use]
pub fn cell_in(rect: Rect, point: Point) -> Option<(u8, u8)> {
    if !rect.contains(point) {
        return None;
    }
    let x = (point.x - rect.x0) / rect.width() * f64::from(PAGE_WIDTH);
    let y = (point.y - rect.y0) / rect.height() * f64::from(PAGE_HEIGHT);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "both are within the page raster")]
    let (column, row) = ((x / f64::from(CELL_WIDTH)) as u8, (y / f64::from(CELL_HEIGHT)) as u8);
    (u16::from(column) < COLUMNS && u16::from(row) < ROWS).then_some((column, row))
}

/// Where a page was last drawn: its place with its screen, which the
/// layout and its animations move, and its image within, which cells and
/// links are found in.
struct Drawn {
    key: PageKey,
    rect: Rect,
    image: Rect,
}

/// The page area: a bank's pages as a grid or a list, the page size
/// control above them, and a page zoomed to fill the area.
pub struct Wall {
    width: f32,
    max: f32,
    shown_width: Tween,
    scroll: f64,
    /// A page to bring into view at the next draw.
    reveal: Option<PageKey>,
    dragging: bool,
    /// The controls, where they were last drawn.
    bar: Rect,
    minus: Rect,
    plus: Rect,
    /// The bar with the control row's height, where the wheel slides it.
    bar_zone: Rect,
    /// The page under the pointer, popping out, and how far.
    hovered: Option<PageKey>,
    hover: Tween,
    /// The page the pointer left, settling back.
    leaving: Option<PageKey>,
    leave: Tween,
    /// The page a zoom from the slider shows: the one chosen in the list,
    /// or else the first on screen.
    focus: Option<PageKey>,
    /// The pages on the wall, in order, as last drawn.
    page_keys: Vec<PageKey>,
    /// The page size before a click zoomed a page, restored when it closes.
    width_before_zoom: Option<f32>,
    /// While zoomed, how far the pages are from resting in place, in pages:
    /// positive while the next page slides up into place.
    slide: Tween,
    /// Wheel movement not yet turned into a page step.
    wheel_carry: f64,
    /// When a mouse wheel's notch last moved a page.
    last_notch: Option<u64>,
    /// The page zooming in, shown alone, or zooming back out.
    zoomed: Option<PageKey>,
    /// Whether the zoom is heading in (true) or back to the wall.
    zoom_in: bool,
    /// From 0 on the wall to 1 filling the area.
    zoom: Tween,
    close: Rect,
    area: Rect,
    drawn: Vec<Drawn>,
    /// The page zoomed in full and at rest, and where it was drawn: links
    /// are shown and followed only there (`hyperlinks.md` section 10.1).
    open: Option<(PageKey, Rect)>,
    /// The last frame drew something part of the way through a move, so
    /// the frame that ends it is still owed, however late it is asked for.
    drawn_moving: bool,
}

impl Default for Wall {
    fn default() -> Self {
        Self::new()
    }
}

impl Wall {
    /// A wall at the default page size, scrolled to the top, nothing zoomed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            max: MAX_WIDTH,
            shown_width: Tween::new(DEFAULT_WIDTH, WIDTH_TRANSITION_MILLISECONDS, EASE_OUT),
            scroll: 0.0,
            reveal: None,
            dragging: false,
            bar: Rect::ZERO,
            minus: Rect::ZERO,
            plus: Rect::ZERO,
            bar_zone: Rect::ZERO,
            hovered: None,
            hover: Tween::new(0.0, HOVER_MILLISECONDS, EASE_OUT),
            leaving: None,
            leave: Tween::new(0.0, HOVER_MILLISECONDS, EASE_OUT),
            focus: None,
            page_keys: Vec::new(),
            width_before_zoom: None,
            slide: Tween::new(0.0, SLIDE_MILLISECONDS, STANDARD),
            wheel_carry: 0.0,
            last_notch: None,
            zoomed: None,
            zoom_in: false,
            zoom: Tween::new(0.0, ZOOM_MILLISECONDS, STANDARD),
            close: Rect::ZERO,
            area: Rect::ZERO,
            drawn: Vec::new(),
            open: None,
            drawn_moving: false,
        }
    }

    /// Starts the wall at its top, as for a newly chosen bank, and leaves
    /// any zoomed page at once.
    pub fn to_top(&mut self) {
        self.scroll = 0.0;
        self.zoomed = None;
        self.zoom_in = false;
        self.zoom = Tween::new(0.0, ZOOM_MILLISECONDS, STANDARD);
    }

    /// The page zoomed in, or zooming in.
    #[must_use]
    pub fn zoomed(&self) -> Option<PageKey> {
        self.zoomed.filter(|_| self.zoom_in)
    }

    /// Brings `key` into view at the next draw.
    pub fn reveal(&mut self, key: PageKey) {
        self.reveal = Some(key);
    }

    /// Whether `point` is in the page area, below the controls.
    #[must_use]
    pub fn contains(&self, point: Point) -> bool {
        self.area.contains(point)
    }

    /// Scrolls the pages by `distance` logical pixels, down when positive, never above the top.
    pub fn scroll_by(&mut self, distance: f64) {
        self.scroll = (self.scroll + distance).max(0.0);
    }

    #[cfg(test)]
    pub(crate) fn bar_zone_for_tests(&self) -> Rect {
        self.bar_zone
    }

    #[cfg(test)]
    pub(crate) fn width_for_tests(&self) -> f32 {
        self.width
    }

    #[cfg(test)]
    pub(crate) fn layout_for_tests(&self) -> Layout {
        self.layout()
    }

    /// Whether `point` is over the page size bar, anywhere in the height
    /// of the control row.
    #[must_use]
    pub fn over_bar(&self, point: Point) -> bool {
        self.bar_zone.contains(point)
    }

    /// The cell under `point`.
    #[must_use]
    pub fn cell_at(&self, point: Point) -> Option<CellClick> {
        if !self.area.contains(point) {
            return None;
        }
        // Topmost first: a zooming page is drawn over the wall.
        let drawn = self.drawn.iter().rev().find(|drawn| drawn.image.contains(point))?;
        let (column, row) = cell_in(drawn.image, point)?;
        Some(CellClick { key: drawn.key, version: 0, column, row })
    }

    /// The page zoomed in full and at rest, and where it was drawn in the
    /// last frame.
    #[must_use]
    pub fn open(&self) -> Option<(PageKey, Rect)> {
        self.open
    }

    /// A zoomed page's close button, as last drawn; nothing while no page
    /// is zoomed.
    #[must_use]
    pub fn close_button(&self) -> Rect {
        if self.zoom_in { self.close } else { Rect::ZERO }
    }

    /// The page size's smaller and larger buttons, as last drawn.
    #[must_use]
    pub const fn size_buttons(&self) -> (Rect, Rect) {
        (self.minus, self.plus)
    }

    /// The page area, below the controls, as last drawn.
    #[must_use]
    pub fn area(&self) -> Rect {
        self.area
    }

    /// Where each page was drawn in the last frame: for screen readers'
    /// bounds and for tests.
    pub fn shown(&self) -> impl Iterator<Item = (PageKey, Rect)> + '_ {
        self.drawn.iter().map(|drawn| (drawn.key, drawn.image))
    }

    /// The pages drawn in the last frame, sorted and without repeats.
    #[must_use]
    pub fn visible(&self) -> Vec<PageKey> {
        let mut visible: Vec<PageKey> = self.drawn.iter().map(|drawn| drawn.key).collect();
        visible.sort_unstable();
        // A page drawn twice, popped out or zooming over its own slot, is
        // still one page.
        visible.dedup();
        visible
    }

    /// Whether the pointer is over a page a click would zoom.
    #[must_use]
    pub fn over_page(&self) -> bool {
        self.hovered.is_some()
    }

    /// The list at the slider's left edge, the grid everywhere else.
    fn layout(&self) -> Layout {
        layout_for_width(self.width)
    }

    /// Whether anything on the wall is moving at `now`, or the last frame
    /// drew it part of the way and the frame that ends the move is owed.
    #[must_use]
    pub fn animating(&self, now: u64) -> bool {
        self.drawn_moving || self.moving(now)
    }

    /// Whether anything on the wall is on its way at `now`.
    fn moving(&self, now: u64) -> bool {
        self.shown_width.moving(now) || self.zoom.moving(now) || self.slide.moving(now) || self.hover.moving(now) || self.leave.moving(now)
    }
}

#[cfg(test)]
mod tests;
