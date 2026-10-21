//! The zoomable page area: the page size control along its top, and below
//! it the wall of the selected bank's pages, in a grid that grows and
//! shrinks with the page size or, below a size, a list. Because this code
//! lays out every page itself, it hit-tests the pointer against the same
//! rectangles it draws (requirement 12.3).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::canvas::{CELL_HEIGHT, CELL_WIDTH, COLUMNS, Canvas, PAGE_HEIGHT, PAGE_WIDTH, PageKey, ROWS};
use crate::geometry::{Point, Rect};
use crate::size::{
    DEFAULT_WIDTH, Layout, MAX_WIDTH, clamp_width, fraction_of_width, grid_width, layout_for_width, max_width_for_window, stepped_width, width_at_fraction,
};
use crate::theme::{
    BUTTON, CAPTION, CAPTION_TITLE, CONTROL, MUTED, PANEL, READOUT, ROW_NUMBER, ROW_TITLE, RULE, RULE_SOFT, SEGMENT_OFF, SELECTED, WHITE, WINDOW,
};
use crate::theme::{UPDATED, mix};
use crate::tween::{EASE_OUT, STANDARD, Tween};
use crate::ui::{border, centred, label, outline};

// ---- The control row along the panel's top ----
const CONTROLS_HEIGHT: f64 = 48.0;
const PADDING: f64 = 12.0;
const BUTTON_SIZE: f64 = 32.0;
const SEGMENT_WIDTH: f64 = 10.0;
const SEGMENT_GAP: f64 = 3.0;
const SEGMENT_HEIGHT: f64 = 18.0;
const THUMB_WIDTH: f64 = 10.0;
const THUMB_OVERHANG: f64 = 5.0;
const READOUT_WIDTH: f64 = 76.0;
const CONTROL_GAP: f64 = 10.0;
const PERCENT: f64 = 100.0;
/// Where the label's text starts below the row's top.
const LABEL_DROP: f64 = 12.0;

// ---- The wall ----
const WALL_PADDING: f64 = 16.0;
const COLUMN_GAP: f64 = 12.0;
const ROW_GAP: f64 = 26.0;
const CAPTION_GAP: f64 = 6.0;
const CAPTION_HEIGHT: f64 = 18.0;
const PAGE_ASPECT: f64 = PAGE_HEIGHT as f64 / PAGE_WIDTH as f64;
const LIST_PAGE_WIDTH: f64 = 120.0;
const LIST_ROW_PADDING: f64 = 8.0;
const LIST_GAP: f64 = 16.0;
const LIST_TITLE_RISE: f64 = 22.0;
const LIST_DETAIL_DROP: f64 = 2.0;
const SCROLL_BAR_WIDTH: f64 = 4.0;
const SELECTED_BORDER: f64 = 3.0;
/// The gap between a page and its frame.
const FRAME: f64 = 1.0;
const CAPTION_NUMBER_WIDTH: f64 = 44.0;
/// The zoom from the wall and back: Material 3's medium duration of 300
/// milliseconds on its standard curve, `cubic-bezier(0.2, 0, 0, 1)`
/// (material-components-android `docs/theming/Motion.md`).
const ZOOM_MILLISECONDS: u64 = 300;
/// How dark the wall becomes behind a zoomed page.
const ZOOM_DIM: f64 = 0.75;
/// A page under the pointer pops out: 6% larger, over 150 milliseconds
/// ease-out, as the prototype's width transition, with a shadow offset by
/// 8 pixels at 60% darkness.
const HOVER_MILLISECONDS: u64 = 150;
const HOVER_GROWTH: f64 = 0.06;
const HOVER_SHADOW: f64 = 8.0;
const HOVER_SHADOW_DARKNESS: f64 = 0.6;
/// While zoomed, a trackpad moves one page for every 20 pixels it would
/// scroll (a mouse wheel moves by notches instead: `Wall::wheel_notch`),
/// and keeps at most two pages of movement in hand, so a fast flick does
/// not run on for long.
const WHEEL_PER_PAGE: f64 = 20.0;
const WHEEL_CARRY_LIMIT: f64 = 2.0 * WHEEL_PER_PAGE;
/// A step to the next page slides the pages over 220 milliseconds on the
/// standard curve: quicker than the zoom, so wheeling through pages feels
/// fast, and easing to a stop with the page snapped into place.
const SLIDE_MILLISECONDS: u64 = 220;
/// How many neighbours either side may be on screen during fast steps.
const SLIDE_REACH: i32 = 2;
/// The close button of a zoomed page.
const CLOSE_SIZE: f64 = 36.0;
const CLOSE_INSET: f64 = 10.0;
const CLOSE_GLYPH: f64 = 14.0;
const CLOSE_THICKNESS: f64 = 3.0;

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

/// How strongly an update still shows, from 1 just after it to 0 once
/// `UPDATE_GLOW_MILLISECONDS` have passed.
#[must_use]
pub fn glow(updated: Option<u64>, now: u64) -> f64 {
    updated.map_or(0.0, |at| {
        #[allow(clippy::cast_precision_loss, reason = "a few seconds")]
        let share = now.saturating_sub(at) as f64 / UPDATE_GLOW_MILLISECONDS as f64;
        (1.0 - share).max(0.0)
    })
}

/// "Updated 12s": how long ago a page was updated, for a minute after it
/// was.
#[must_use]
pub fn since(updated: Option<u64>, now: u64) -> Option<String> {
    let elapsed = now.saturating_sub(updated?);
    (elapsed < UPDATE_MARK_MILLISECONDS).then(|| format!("Updated {}s", elapsed / MILLISECONDS_PER_SECOND))
}

/// An update glows for 3 seconds, and is marked with its age for a minute.
pub const UPDATE_GLOW_MILLISECONDS: u64 = 3_000;
/// How long an update is marked with its age.
pub const UPDATE_MARK_MILLISECONDS: u64 = 60_000;
const MILLISECONDS_PER_SECOND: u64 = 1_000;
/// The width of an updated page's glowing frame, at its brightest.
const GLOW_WIDTH: f64 = 4.0;

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

/// Where a page was last drawn, for hit-testing.
struct Drawn {
    key: PageKey,
    rect: Rect,
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

    /// Zooms `key` from its place on the wall to fill the area. The slider
    /// goes to its right end, which is the zoomed state; its place before
    /// comes back when the zoom closes.
    pub fn zoom(&mut self, key: PageKey, now: u64) {
        if !self.zoom_in && self.width < self.max {
            self.width_before_zoom = Some(self.width);
        }
        // The thumb moves; the wall behind keeps its size during the zoom.
        self.width = self.max;
        self.start_zoom(key, now);
    }

    fn start_zoom(&mut self, key: PageKey, now: u64) {
        self.zoomed = Some(key);
        self.zoom_in = true;
        self.zoom.set(1.0, now);
        self.slide = Tween::new(0.0, SLIDE_MILLISECONDS, STANDARD);
        self.wheel_carry = 0.0;
    }

    /// Moves the zoomed page `direction` pages along the wall (1 the next,
    /// −1 the one before), sliding the pages up or down and easing them to
    /// rest exactly in place. A step during a slide carries on from where
    /// the slide has reached. Returns the new page, or `None` at either
    /// end of the wall.
    pub fn step_zoom(&mut self, direction: i8, now: u64) -> Option<PageKey> {
        if !self.zoom_in {
            return None;
        }
        let current = self.page_keys.iter().position(|&key| Some(key) == self.zoomed)?;
        let next = current.checked_add_signed(isize::from(direction))?;
        let key = *self.page_keys.get(next)?;
        self.zoomed = Some(key);
        let mut slide = Tween::new(self.slide.value(now) + f32::from(direction), SLIDE_MILLISECONDS, STANDARD);
        slide.set(0.0, now);
        self.slide = slide;
        Some(key)
    }

    /// Turns the wheel's movement into page steps while zoomed, one page for
    /// every `WHEEL_PER_PAGE` pixels; the wheel turned towards the user (a
    /// positive distance) goes to the next page. Returns the page now
    /// zoomed, when it changed.
    pub fn wheel_pages(&mut self, distance: f64, now: u64) -> Option<PageKey> {
        self.wheel_carry = (self.wheel_carry + distance).clamp(-WHEEL_CARRY_LIMIT, WHEEL_CARRY_LIMIT);
        let mut moved = None;
        while self.wheel_carry.abs() >= WHEEL_PER_PAGE {
            let direction: i8 = if self.wheel_carry > 0.0 { 1 } else { -1 };
            self.wheel_carry -= WHEEL_PER_PAGE * f64::from(direction);
            match self.step_zoom(direction, now) {
                Some(key) => moved = Some(key),
                // At an end of the wall: the rest of the movement is spent.
                None => self.wheel_carry = 0.0,
            }
        }
        moved
    }

    /// A mouse wheel's notch while zoomed: one page for any notch, however
    /// small its reported size, and at most one page per slide. macOS
    /// reports a slow notch as a tenth of a line and speeds up quick ones
    /// (measured: 0.1, then 0.9 and 2.8 within 80 milliseconds), so the
    /// size says nothing about how many notches were turned; feeds that
    /// show one item at a time count only the direction, and ignore the
    /// wheel until the step's animation is done. A quick flick therefore
    /// moves one page, and a wheel kept turning moves one page per slide.
    pub fn wheel_notch(&mut self, lines: f64, now: u64) -> Option<PageKey> {
        if lines == 0.0 || self.last_notch.is_some_and(|at| now.saturating_sub(at) < SLIDE_MILLISECONDS) {
            return None;
        }
        let moved = self.step_zoom(if lines > 0.0 { 1 } else { -1 }, now);
        if moved.is_some() {
            self.last_notch = Some(now);
        }
        moved
    }

    /// Shows `key` in place of the zoomed page, with no transition, as when
    /// another page is chosen from the list.
    pub fn switch_zoom(&mut self, key: PageKey) {
        if self.zoom_in {
            self.zoomed = Some(key);
        }
    }

    /// Zooms back to the wall, from wherever the zoom has reached; returns
    /// whether a page was zoomed.
    pub fn unzoom(&mut self, now: u64) -> bool {
        if !self.zoom_in {
            return false;
        }
        self.zoom_in = false;
        self.zoom.set(0.0, now);
        if let Some(width) = self.width_before_zoom.take() {
            self.width = width;
        }
        true
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

    /// A new page size from the slider, its buttons, keys or the wheel.
    /// The slider's right end is the zoomed state: reaching it zooms the
    /// chosen page (or the first on screen), and leaving it zooms out.
    fn set_width(&mut self, width: f32, now: u64) {
        self.width = clamp_width(width, self.max);
        self.shown_width.set(self.width, now);
        let at_max = self.width >= self.max;
        if at_max && !self.zoom_in {
            let first = self.drawn.first().map(|drawn| drawn.key);
            if let Some(key) = self.focus.filter(|key| self.page_keys.contains(key)).or(first) {
                self.width_before_zoom = None;
                self.start_zoom(key, now);
            }
        } else if !at_max && self.zoom_in {
            self.width_before_zoom = None;
            self.zoom_in = false;
            self.zoom.set(0.0, now);
        }
    }

    /// Steps the page size one notch smaller (`direction` below zero) or larger.
    pub fn step(&mut self, direction: i8, now: u64) {
        self.set_width(stepped_width(self.width, direction), now);
    }

    fn width_at(&self, x: f64) -> f32 {
        let fraction = ((x - self.bar.x0) / self.bar.width()).clamp(0.0, 1.0);
        #[allow(clippy::cast_possible_truncation, reason = "a fraction between 0 and 1")]
        let fraction = fraction as f32;
        width_at_fraction(fraction, self.max)
    }

    /// Returns whether the press was on the wall's controls.
    pub fn pointer_pressed(&mut self, point: Point, now: u64) -> bool {
        if self.zoom_in && self.close.contains(point) {
            self.unzoom(now);
            return true;
        }
        if self.bar.contains(point) {
            self.dragging = true;
            self.set_width(self.width_at(point.x), now);
            return true;
        }
        if self.minus.contains(point) {
            self.step(-1, now);
            return true;
        }
        if self.plus.contains(point) {
            self.step(1, now);
            return true;
        }
        false
    }

    /// Returns whether the size changed.
    pub fn pointer_moved(&mut self, point: Point, now: u64) -> bool {
        if self.dragging {
            self.set_width(self.width_at(point.x), now);
        }
        self.dragging
    }

    /// The main button was released: any drag of the page size bar ends.
    pub fn pointer_released(&mut self) {
        self.dragging = false;
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

    /// Slides the page size bar by a wheel's `distance`, as the scroll
    /// would move: the thumb moves the same number of pixels, the wheel
    /// turned away from the user (a negative distance) moving it right,
    /// towards larger pages.
    pub fn wheel_bar(&mut self, distance: f64, now: u64) {
        if self.bar.width() <= 0.0 {
            return;
        }
        let fraction = f64::from(fraction_of_width(self.width, self.max));
        let moved = (fraction - distance / self.bar.width()).clamp(0.0, 1.0);
        #[allow(clippy::cast_possible_truncation, reason = "a fraction between 0 and 1")]
        let width = width_at_fraction(moved as f32, self.max);
        self.set_width(width, now);
    }

    /// The cell under `point`.
    #[must_use]
    pub fn cell_at(&self, point: Point) -> Option<CellClick> {
        if !self.area.contains(point) {
            return None;
        }
        // Topmost first: a zooming page is drawn over the wall.
        let drawn = self.drawn.iter().rev().find(|drawn| drawn.rect.contains(point))?;
        let (column, row) = cell_in(drawn.rect, point)?;
        Some(CellClick { key: drawn.key, version: 0, column, row })
    }

    /// The page zoomed in full and at rest, and where it was drawn in the
    /// last frame.
    #[must_use]
    pub fn open(&self) -> Option<(PageKey, Rect)> {
        self.open
    }

    /// The page area, below the controls, as last drawn.
    #[must_use]
    pub fn area(&self) -> Rect {
        self.area
    }

    /// Where each page was drawn in the last frame: for screen readers'
    /// bounds and for tests.
    pub fn shown(&self) -> impl Iterator<Item = (PageKey, Rect)> + '_ {
        self.drawn.iter().map(|drawn| (drawn.key, drawn.rect))
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

    /// Whether the pointer is over a page a click would zoom.
    #[must_use]
    pub fn over_page(&self) -> bool {
        self.hovered.is_some()
    }

    /// Draws the hovered page, and the one it left, popped out over their
    /// slots: larger by up to `HOVER_GROWTH`, with a shadow beneath and a
    /// light frame.
    fn draw_popped<C: Canvas + ?Sized>(&mut self, canvas: &mut C, now: u64) {
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

    /// The list at the slider's left edge, the grid everywhere else.
    fn layout(&self) -> Layout {
        layout_for_width(self.width)
    }

    /// Whether anything on the wall is moving at `now`.
    #[must_use]
    pub fn animating(&self, now: u64) -> bool {
        self.shown_width.moving(now) || self.zoom.moving(now) || self.slide.moving(now) || self.hover.moving(now) || self.leave.moving(now)
    }

    /// Draws the panel in `panel`, with `pages` on the wall and `selected`
    /// outlined.
    pub fn draw<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect, pages: &[WallPage<'_>], selected: Option<PageKey>, now: u64) {
        canvas.fill(panel, 0.0, PANEL);
        self.area = Rect::new(panel.x0, panel.y0 + CONTROLS_HEIGHT, panel.x1, panel.y1);
        #[allow(clippy::cast_possible_truncation, reason = "panel widths fit easily in f32")]
        let available = (self.area.width() - WALL_PADDING * 2.0) as f32;
        self.max = max_width_for_window(available);
        // A new window size keeps the zoomed state at the slider's end, and
        // never zooms by itself.
        if self.zoom_in || self.width > self.max {
            self.width = if self.zoom_in { self.max } else { clamp_width(self.width, self.max) };
            if !self.zoom_in || self.width_before_zoom.is_none() {
                self.shown_width.set(self.width, now);
            }
        }
        self.focus = selected;
        self.page_keys = pages.iter().map(|page| page.key).collect();
        self.drawn.clear();
        self.open = None;
        // A zoomed page that has left the wall (another bank, another
        // channel) ends the zoom at once.
        if let Some(key) = self.zoomed
            && !pages.iter().any(|page| page.key == key)
        {
            self.to_top();
        }
        let progress = f64::from(self.zoom.value(now));
        let full = self.zoom_in && progress >= 1.0 - f64::from(f32::EPSILON);
        canvas.push_clip(self.area);
        let content = match self.zoomed {
            Some(key) if full => self.draw_zoomed(canvas, pages, key, now),
            _ => {
                let content = match self.layout() {
                    Layout::Grid => self.draw_grid(canvas, pages, selected, now),
                    Layout::List => self.draw_list(canvas, pages, selected),
                };
                self.draw_popped(canvas, now);
                if let Some(key) = self.zoomed {
                    self.draw_zooming(canvas, key, progress);
                }
                content
            }
        };
        canvas.pop_clip();
        if full {
            self.draw_close(canvas);
        }
        if !self.zoom_in && !self.zoom.moving(now) {
            self.zoomed = None;
        }
        let view = self.area.height();
        let total = content + WALL_PADDING * 2.0;
        let max_scroll = (total - view).max(0.0);
        self.scroll = self.scroll.min(max_scroll);
        if max_scroll > 0.0 {
            let bar = view * view / total;
            let top = self.area.y0 + (view - bar) * self.scroll / max_scroll;
            let left = self.area.x1 - SCROLL_BAR_WIDTH * 2.0;
            canvas.fill(Rect::new(left, top, left + SCROLL_BAR_WIDTH, top + bar), SCROLL_BAR_WIDTH / 2.0, RULE);
        }
        self.draw_controls(canvas, panel);
        outline(canvas, panel, RULE);
    }

    fn draw_page<C: Canvas + ?Sized>(&mut self, canvas: &mut C, key: PageKey, rect: Rect) {
        // With no image yet a dark frame stands in, and the page still
        // counts as visible, so that the trusted core renders it.
        if !canvas.page(key, rect) {
            canvas.fill(rect, 0.0, WINDOW);
        }
        self.drawn.push(Drawn { key, rect });
    }

    /// Where a zoomed page sits: as large as the area allows, keeping its
    /// shape, below a line for its number and title.
    fn zoomed_rect(&self) -> Rect {
        let area = self.area;
        let room = Rect::new(area.x0 + WALL_PADDING, area.y0 + WALL_PADDING + CAPTION_HEIGHT + CAPTION_GAP, area.x1 - WALL_PADDING, area.y1 - WALL_PADDING);
        let width = room.width().min(room.height() / PAGE_ASPECT).max(0.0);
        let left = f64::midpoint(room.x0, room.x1) - width / 2.0;
        Rect::new(left, room.y0, left + width, room.y0 + width * PAGE_ASPECT)
    }

    /// The page part of the way between its place on the wall and its
    /// zoomed place, over the wall dimmed by the same share: the shared
    /// element transition of Material's container transform and the web's
    /// FLIP technique. Both ends are measured afresh each frame, so the
    /// page lands right even if the wall scrolls or the window changes size
    /// meanwhile. A page scrolled off the wall grows from the area's middle.
    fn draw_zooming<C: Canvas + ?Sized>(&mut self, canvas: &mut C, key: PageKey, progress: f64) {
        let target = self.zoomed_rect();
        let centre = Point::new(f64::midpoint(target.x0, target.x1), f64::midpoint(target.y0, target.y1));
        let start = self.drawn.iter().find(|drawn| drawn.key == key).map_or(Rect::new(centre.x, centre.y, centre.x, centre.y), |drawn| drawn.rect);
        let between = |from: f64, to: f64| from + (to - from) * progress;
        let rect = Rect::new(between(start.x0, target.x0), between(start.y0, target.y0), between(start.x1, target.x1), between(start.y1, target.y1));
        #[allow(clippy::cast_possible_truncation, reason = "a share between 0 and 1")]
        canvas.dim(self.area, (progress * ZOOM_DIM) as f32);
        self.draw_page(canvas, key, rect);
        outline(canvas, Rect::new(rect.x0 - FRAME, rect.y0 - FRAME, rect.x1 + FRAME, rect.y1 + FRAME), RULE);
    }

    /// Draws one page alone, as large as the area allows, with its number
    /// and title above it; returns the height it fills.
    fn draw_zoomed<C: Canvas + ?Sized>(&mut self, canvas: &mut C, pages: &[WallPage<'_>], key: PageKey, now: u64) -> f64 {
        self.scroll = self.scroll.max(0.0);
        let area = self.area;
        let Some(current) = pages.iter().position(|page| page.key == key) else { return 0.0 };
        let page = &pages[current];
        let rect = self.zoomed_rect();
        // The zoomed page and its neighbours on a vertical strip, offset by
        // the slide, which eases to rest with the page exactly in place.
        let slide = f64::from(self.slide.value(now));
        // A full area apart, so at rest no neighbour shows.
        let pitch = area.height();
        for neighbour in -SLIDE_REACH..=SLIDE_REACH {
            let offset = f64::from(neighbour) + slide;
            let shifted = Rect::new(rect.x0, rect.y0 + offset * pitch, rect.x1, rect.y1 + offset * pitch);
            if shifted.y1 <= area.y0 || shifted.y0 >= area.y1 {
                continue;
            }
            let Some(other) = current.checked_add_signed(neighbour as isize).and_then(|index| pages.get(index)) else { continue };
            self.draw_page(canvas, other.key, shifted);
            if neighbour == 0 && slide.abs() < f64::from(f32::EPSILON) {
                self.open = Some((other.key, shifted));
            }
            outline(canvas, Rect::new(shifted.x0 - FRAME, shifted.y0 - FRAME, shifted.x1 + FRAME, shifted.y1 + FRAME), RULE);
        }
        // The caption line stays put on its own background; pages slide
        // beneath it.
        canvas.fill(Rect::new(area.x0, area.y0, area.x1, rect.y0 - FRAME), 0.0, PANEL);
        let caption = Point::new(rect.x0, area.y0 + WALL_PADDING);
        label(canvas, caption, &format!("{:03X}", key.page), CAPTION, WHITE);
        canvas.text(Rect::new(caption.x + CAPTION_NUMBER_WIDTH, caption.y + 1.0, rect.x1, caption.y + CAPTION_HEIGHT), page.title, CAPTION_TITLE, MUTED);
        area.height() - WALL_PADDING * 2.0
    }

    /// The close button of a zoomed page, at the area's top right.
    fn draw_close<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let right = self.area.x1 - CLOSE_INSET;
        let top = self.area.y0 + CLOSE_INSET;
        self.close = Rect::new(right - CLOSE_SIZE, top, right, top + CLOSE_SIZE);
        canvas.fill(self.close, 0.0, BUTTON);
        outline(canvas, self.close, WHITE);
        let centre = Point::new(f64::midpoint(self.close.x0, self.close.x1), f64::midpoint(self.close.y0, self.close.y1));
        let half = CLOSE_GLYPH / 2.0;
        let normal = CLOSE_THICKNESS / 2.0 / core::f64::consts::SQRT_2;
        for direction in [1.0, -1.0] {
            let along = Point::new(half, half * direction);
            let across = Point::new(-normal * direction, normal);
            canvas.polygon(&[centre - along + across, centre + along + across, centre + along - across, centre - along - across], WHITE);
        }
    }

    /// Scrolls so that the row at `top` to `bottom` (in unscrolled
    /// content coordinates) is in view.
    fn bring_into_view(&mut self, top: f64, bottom: f64) {
        let view = self.area.height() - WALL_PADDING * 2.0;
        if top < self.scroll {
            self.scroll = top;
        } else if bottom > self.scroll + view {
            self.scroll = bottom - view;
        }
    }

    /// Draws the grid and returns its height.
    fn draw_grid<C: Canvas + ?Sized>(&mut self, canvas: &mut C, pages: &[WallPage<'_>], selected: Option<PageKey>, now: u64) -> f64 {
        let area = self.area;
        let available = area.width() - WALL_PADDING * 2.0;
        let card_width = f64::from(grid_width(self.shown_width.value(now)));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small, positive column count")]
        let columns = (libm::floor((available + COLUMN_GAP) / (card_width + COLUMN_GAP)) as usize).max(1);
        let rows = pages.len().div_ceil(columns);
        let page_height = card_width * PAGE_ASPECT;
        let card_height = page_height + CAPTION_GAP + CAPTION_HEIGHT;
        let pitch = card_height + ROW_GAP;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let content = rows as f64 * card_height + rows.saturating_sub(1) as f64 * ROW_GAP;
        if let Some(key) = self.reveal.take()
            && let Some(index) = pages.iter().position(|page| page.key == key)
        {
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let top = (index / columns) as f64 * pitch;
            self.bring_into_view(top, top + card_height);
        }
        let top = area.y0 + WALL_PADDING - self.scroll;
        let centre_x = f64::midpoint(area.x0, area.x1);
        for (index, page) in pages.iter().enumerate() {
            let (row, column) = (index / columns, index % columns);
            let in_row = columns.min(pages.len() - row * columns);
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_width = in_row as f64 * card_width + (in_row - 1) as f64 * COLUMN_GAP;
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let left = centre_x - row_width / 2.0 + column as f64 * (card_width + COLUMN_GAP);
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let card_top = top + row as f64 * pitch;
            // Only pages in the panel are drawn and reported visible
            // (requirement 9.5).
            if card_top > area.y1 || card_top + card_height < area.y0 {
                continue;
            }
            let rect = Rect::new(left, card_top, left + card_width, card_top + page_height);
            self.draw_page(canvas, page.key, rect);
            // A frame, so a page's black edges stand apart from the panel;
            // amber and thick just after the page was updated, fading back.
            let glowing = glow(page.updated, now);
            if glowing > 0.0 {
                let width = FRAME + (GLOW_WIDTH - FRAME) * glowing;
                border(canvas, Rect::new(rect.x0 - width, rect.y0 - width, rect.x1 + width, rect.y1 + width), width, mix(RULE, UPDATED, glowing));
            } else {
                outline(canvas, Rect::new(rect.x0 - FRAME, rect.y0 - FRAME, rect.x1 + FRAME, rect.y1 + FRAME), RULE);
            }
            if selected == Some(page.key) {
                let ring = Rect::new(rect.x0 - SELECTED_BORDER, rect.y0 - SELECTED_BORDER, rect.x1 + SELECTED_BORDER, rect.y1 + SELECTED_BORDER);
                for width in 0..3_u8 {
                    let inset = f64::from(width);
                    outline(canvas, Rect::new(ring.x0 + inset, ring.y0 + inset, ring.x1 - inset, ring.y1 - inset), WHITE);
                }
            }
            let caption = Point::new(left, card_top + page_height + CAPTION_GAP);
            label(canvas, caption, &format!("{:03X}", page.key.page), CAPTION, WHITE);
            let title_at = Point::new(caption.x + CAPTION_NUMBER_WIDTH, caption.y + 1.0);
            let mut title_end = caption.x + card_width;
            if let Some(age) = since(page.updated, now) {
                let width = canvas.text_width(&age, CAPTION);
                title_end -= width + CAPTION_GAP;
                label(canvas, Point::new(caption.x + card_width - width, caption.y), &age, CAPTION, UPDATED);
            }
            canvas.text(Rect::new(title_at.x, title_at.y, title_end, title_at.y + CAPTION_HEIGHT), page.title, CAPTION_TITLE, MUTED);
        }
        content
    }

    /// Draws the list and returns its height.
    fn draw_list<C: Canvas + ?Sized>(&mut self, canvas: &mut C, pages: &[WallPage<'_>], selected: Option<PageKey>) -> f64 {
        let area = self.area;
        let left = area.x0 + WALL_PADDING;
        let right = area.x1 - WALL_PADDING;
        let page_height = LIST_PAGE_WIDTH * PAGE_ASPECT;
        let row_height = page_height + LIST_ROW_PADDING * 2.0;
        if let Some(key) = self.reveal.take()
            && let Some(index) = pages.iter().position(|page| page.key == key)
        {
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let top = index as f64 * row_height;
            self.bring_into_view(top, top + row_height);
        }
        let top = area.y0 + WALL_PADDING - self.scroll;
        for (index, page) in pages.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_top = top + index as f64 * row_height;
            if row_top > area.y1 || row_top + row_height < area.y0 {
                continue;
            }
            let row = Rect::new(left, row_top, right, row_top + row_height);
            if selected == Some(page.key) {
                canvas.fill(row, 0.0, SELECTED);
            }
            canvas.fill(Rect::new(left, row.y1 - 1.0, right, row.y1), 0.0, RULE_SOFT);
            let rect = Rect::new(
                left + LIST_ROW_PADDING,
                row_top + LIST_ROW_PADDING,
                left + LIST_ROW_PADDING + LIST_PAGE_WIDTH,
                row_top + LIST_ROW_PADDING + page_height,
            );
            self.draw_page(canvas, page.key, rect);
            let text_left = rect.x1 + LIST_GAP;
            let middle = rect.y0 + page_height / 2.0;
            label(canvas, Point::new(text_left, middle - LIST_TITLE_RISE), &format!("{:03X}", page.key.page), ROW_NUMBER, WHITE);
            canvas.text(Rect::new(text_left, middle + LIST_DETAIL_DROP, right, row.y1), page.title, ROW_TITLE, MUTED);
        }
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let height = row_height * pages.len() as f64;
        height
    }

    fn draw_controls<C: Canvas + ?Sized>(&mut self, canvas: &mut C, panel: Rect) {
        let row = Rect::new(panel.x0, panel.y0, panel.x1, panel.y0 + CONTROLS_HEIGHT);
        canvas.fill(row, 0.0, PANEL);
        canvas.fill(Rect::new(row.x0, row.y1 - 1.0, row.x1, row.y1), 0.0, RULE);
        let middle = f64::midpoint(row.y0, row.y1);
        let mut x = row.x0 + PADDING;
        label(canvas, Point::new(x, row.y0 + LABEL_DROP), "Page size", CONTROL, WHITE);
        x += canvas.text_width("Page size", CONTROL) + CONTROL_GAP * 1.5;

        let right = row.x1 - PADDING;

        // − [segments] + readout
        self.minus = Rect::new(x, middle - BUTTON_SIZE / 2.0, x + BUTTON_SIZE, middle + BUTTON_SIZE / 2.0);
        let readout = Rect::new(right - READOUT_WIDTH, row.y0, right, row.y1);
        self.plus = Rect::new(readout.x0 - CONTROL_GAP - BUTTON_SIZE, self.minus.y0, readout.x0 - CONTROL_GAP, self.minus.y1);
        for (button, glyph) in [(self.minus, "−"), (self.plus, "+")] {
            canvas.fill(button, 0.0, BUTTON);
            outline(canvas, button, WHITE);
            centred(canvas, button, glyph, CONTROL, WHITE);
        }
        self.bar = Rect::new(self.minus.x1 + CONTROL_GAP, middle - SEGMENT_HEIGHT / 2.0, self.plus.x0 - CONTROL_GAP, middle + SEGMENT_HEIGHT / 2.0);
        self.bar_zone = Rect::new(self.bar.x0, row.y0, self.bar.x1, row.y1);
        let fraction = f64::from(fraction_of_width(self.width, self.max));
        let thumb = self.bar.x0 + fraction * (self.bar.width() - THUMB_WIDTH);
        let mut segment = self.bar.x0;
        while segment + SEGMENT_WIDTH <= self.bar.x1 {
            canvas.fill(Rect::new(segment, self.bar.y0, segment + SEGMENT_WIDTH, self.bar.y1), 0.0, if segment < thumb { SELECTED } else { SEGMENT_OFF });
            segment += SEGMENT_WIDTH + SEGMENT_GAP;
        }
        canvas.fill(Rect::new(thumb, self.bar.y0 - THUMB_OVERHANG, thumb + THUMB_WIDTH, self.bar.y1 + THUMB_OVERHANG), 0.0, WHITE);

        let text = if self.layout() == Layout::List {
            String::from("List")
        } else {
            format!("{}%", libm::round(f64::from(grid_width(self.width)) / f64::from(PAGE_WIDTH) * PERCENT))
        };
        let width = canvas.text_width(&text, READOUT);
        label(canvas, Point::new(readout.x1 - width, row.y0 + LABEL_DROP - 1.0), &text, READOUT, if self.layout() == Layout::List { MUTED } else { WHITE });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_canvas::Recorder;

    const PANEL: Rect = Rect::new(0.0, 0.0, 1000.0, 700.0);

    fn pages(count: u16) -> Vec<WallPage<'static>> {
        (0..count).map(|offset| WallPage { key: PageKey { channel: 1, page: 0x100 + offset }, title: "Title", updated: None }).collect()
    }

    #[test]
    fn only_pages_on_screen_are_visible() {
        let mut wall = Wall::new();
        let mut canvas = Recorder::new();
        wall.draw(&mut canvas, PANEL, &pages(24), None, 0);
        let visible = wall.visible();
        assert!(!visible.is_empty() && visible.len() < 24, "{} of 24 visible", visible.len());
        assert_eq!(visible[0], PageKey { channel: 1, page: 0x100 });
    }

    #[test]
    fn a_revealed_page_is_drawn() {
        let mut wall = Wall::new();
        let mut canvas = Recorder::new();
        let pages = pages(24);
        wall.draw(&mut canvas, PANEL, &pages, None, 0);
        let last = pages[23].key;
        assert!(!wall.visible().contains(&last));
        wall.reveal(last);
        wall.draw(&mut canvas, PANEL, &pages, None, 0);
        assert!(wall.visible().contains(&last));
    }

    #[test]
    fn a_click_finds_its_cell() {
        let mut wall = Wall::new();
        let mut canvas = Recorder::new();
        wall.draw(&mut canvas, PANEL, &pages(24), None, 0);
        let (key, rect) = canvas.pages.iter().copied().find(|(key, _)| key.page == 0x101).unwrap();
        // The middle of column 10, row 3.
        let point = Point::new(
            rect.x0 + (10.5 * f64::from(CELL_WIDTH)) / f64::from(PAGE_WIDTH) * rect.width(),
            rect.y0 + (3.5 * f64::from(CELL_HEIGHT)) / f64::from(PAGE_HEIGHT) * rect.height(),
        );
        assert_eq!(wall.cell_at(point), Some(CellClick { key, version: 0, column: 10, row: 3 }));
        assert_eq!(wall.cell_at(Point::new(rect.x0 - 2.0, rect.y0)), None, "the gap between pages");
    }

    /// Where `key` was drawn last in the frame just drawn: the topmost.
    fn last_rect(canvas: &Recorder, key: PageKey) -> Rect {
        canvas.pages.iter().rev().find(|(drawn, _)| *drawn == key).map(|&(_, rect)| rect).unwrap()
    }

    fn draw_at(wall: &mut Wall, pages: &[WallPage<'_>], now: u64) -> Recorder {
        let mut canvas = Recorder::new();
        wall.draw(&mut canvas, PANEL, pages, None, now);
        canvas
    }

    #[test]
    fn a_zoomed_page_grows_from_its_place_to_fill_the_area() {
        let mut wall = Wall::new();
        let pages = pages(24);
        // The second page: on screen, in the first row.
        let key = pages[1].key;
        let slot = last_rect(&draw_at(&mut wall, &pages, 0), key);
        wall.zoom(key, 0);
        // Halfway through, the page is between its slot and its zoomed place.
        let middle = last_rect(&draw_at(&mut wall, &pages, ZOOM_MILLISECONDS / 2), key);
        let full = wall.zoomed_rect();
        assert!(middle.width() > slot.width() && middle.width() < full.width(), "{slot:?} {middle:?} {full:?}");
        assert!(wall.animating(ZOOM_MILLISECONDS / 2));
        // At the end it fills the area alone.
        let end = draw_at(&mut wall, &pages, ZOOM_MILLISECONDS);
        assert_eq!(wall.visible(), [key], "only the zoomed page is shown");
        assert_eq!(last_rect(&end, key), full);
        assert!(!wall.animating(ZOOM_MILLISECONDS));
    }

    #[test]
    fn the_close_button_zooms_back_to_the_wall() {
        let mut wall = Wall::new();
        let pages = pages(24);
        // The second page: on screen, in the first row.
        let key = pages[1].key;
        let slot = last_rect(&draw_at(&mut wall, &pages, 0), key);
        wall.zoom(key, 0);
        draw_at(&mut wall, &pages, ZOOM_MILLISECONDS);
        let close = wall.close;
        let now = 2 * ZOOM_MILLISECONDS;
        assert!(wall.pointer_pressed(Point::new(f64::midpoint(close.x0, close.x1), f64::midpoint(close.y0, close.y1)), now));
        assert_eq!(wall.zoomed(), None);
        // On its way back, the wall shows again behind it.
        draw_at(&mut wall, &pages, now + ZOOM_MILLISECONDS / 2);
        assert!(wall.visible().len() > 1);
        let back = draw_at(&mut wall, &pages, now + ZOOM_MILLISECONDS);
        assert_eq!(last_rect(&back, key), slot, "home in its slot");
        assert!(!wall.animating(now + ZOOM_MILLISECONDS));
    }

    #[test]
    fn escape_part_way_reverses_without_a_jump() {
        let mut wall = Wall::new();
        let pages = pages(24);
        // The second page: on screen, in the first row.
        let key = pages[1].key;
        draw_at(&mut wall, &pages, 0);
        wall.zoom(key, 0);
        let part = ZOOM_MILLISECONDS / 3;
        let before = last_rect(&draw_at(&mut wall, &pages, part), key);
        assert!(wall.unzoom(part));
        let after = last_rect(&draw_at(&mut wall, &pages, part), key);
        assert!((before.width() - after.width()).abs() < 1e-6, "{before:?} then {after:?}");
        let later = last_rect(&draw_at(&mut wall, &pages, part + ZOOM_MILLISECONDS / 3), key);
        assert!(later.width() < after.width(), "heading back");
    }

    #[test]
    fn a_hovered_page_pops_out_and_settles_back() {
        let mut wall = Wall::new();
        let pages = pages(24);
        let key = pages[1].key;
        let slot = last_rect(&draw_at(&mut wall, &pages, 0), key);
        let centre = Point::new(f64::midpoint(slot.x0, slot.x1), f64::midpoint(slot.y0, slot.y1));
        assert!(wall.hover(centre, 0));
        assert!(wall.over_page());
        let popped = last_rect(&draw_at(&mut wall, &pages, HOVER_MILLISECONDS), key);
        assert!(popped.width() > slot.width() * (1.0 + HOVER_GROWTH / 2.0), "{slot:?} {popped:?}");
        assert!((f64::midpoint(popped.x0, popped.x1) - centre.x).abs() < 1e-6, "grows about its centre");
        assert_eq!(wall.visible().iter().filter(|&&visible| visible == key).count(), 1, "still one page");
        // Away from every page, it settles back.
        assert!(wall.hover(Point::new(-1.0, -1.0), HOVER_MILLISECONDS));
        assert!(!wall.over_page());
        let settled = last_rect(&draw_at(&mut wall, &pages, 3 * HOVER_MILLISECONDS), key);
        assert_eq!(settled, slot);
    }

    #[test]
    fn the_slider_end_is_the_zoomed_state() {
        let mut wall = Wall::new();
        let pages = pages(24);
        draw_at(&mut wall, &pages, 0);
        // Wheeled all the way right: the first page on screen zooms.
        wall.wheel_bar(-10_000.0, 0);
        assert_eq!(wall.zoomed(), Some(pages[0].key));
        // A little way back from the end: the zoom closes.
        wall.wheel_bar(30.0, 1_000);
        assert_eq!(wall.zoomed(), None);
    }

    #[test]
    fn a_click_zoom_moves_the_thumb_to_the_end_and_escape_puts_it_back() {
        let mut wall = Wall::new();
        let pages = pages(24);
        draw_at(&mut wall, &pages, 0);
        let before = wall.width;
        wall.zoom(pages[1].key, 0);
        assert!((wall.width - wall.max).abs() < f32::EPSILON);
        assert!(wall.unzoom(10));
        assert!((wall.width - before).abs() < f32::EPSILON);
    }

    #[test]
    fn the_wheel_steps_through_pages_and_each_snaps_into_place() {
        let mut wall = Wall::new();
        let pages = pages(24);
        draw_at(&mut wall, &pages, 0);
        wall.zoom(pages[1].key, 0);
        let start = 1_000;
        draw_at(&mut wall, &pages, start);
        let full = wall.zoomed_rect();
        // Towards the user: the next page.
        assert_eq!(wall.wheel_pages(WHEEL_PER_PAGE, start), Some(pages[2].key));
        // Part-way, both pages are on the strip, the new one rising.
        let middle = draw_at(&mut wall, &pages, start + SLIDE_MILLISECONDS / 3);
        assert_eq!(wall.visible(), [pages[1].key, pages[2].key]);
        assert!(last_rect(&middle, pages[2].key).y0 > full.y0);
        // At the end, the new page alone, exactly in place.
        let rested = draw_at(&mut wall, &pages, start + SLIDE_MILLISECONDS);
        assert_eq!(wall.visible(), [pages[2].key]);
        assert_eq!(last_rect(&rested, pages[2].key), full);
        // Away from the user, back; and no page before the first.
        assert_eq!(wall.wheel_pages(-WHEEL_PER_PAGE, start + 1_000), Some(pages[1].key));
        assert_eq!(wall.wheel_pages(-WHEEL_PER_PAGE, start + 1_000), Some(pages[0].key));
        assert_eq!(wall.wheel_pages(-WHEEL_PER_PAGE, start + 1_000), None);
        // Small movements add up to a step.
        assert_eq!(wall.wheel_pages(WHEEL_PER_PAGE / 2.0, start + 2_000), None);
        assert_eq!(wall.wheel_pages(WHEEL_PER_PAGE / 2.0, start + 2_000), Some(pages[1].key));
    }

    #[test]
    fn every_notch_of_a_mouse_wheel_moves_a_page() {
        // A slow notch on macOS, as measured: a tenth of a line.
        const SLOW_NOTCH: f64 = 0.1;
        let mut wall = Wall::new();
        let pages = pages(24);
        draw_at(&mut wall, &pages, 0);
        wall.zoom(pages[1].key, 0);
        assert_eq!(wall.wheel_notch(SLOW_NOTCH, 1_000), Some(pages[2].key));
        // A quick flick's sped-up events during that slide move no further.
        assert_eq!(wall.wheel_notch(0.9, 1_020), None);
        assert_eq!(wall.wheel_notch(2.8, 1_080), None);
        // The next notch after the slide moves again, either way.
        assert_eq!(wall.wheel_notch(-SLOW_NOTCH, 1_000 + SLIDE_MILLISECONDS), Some(pages[1].key));
        assert_eq!(wall.wheel_notch(-SLOW_NOTCH, 1_000 + 2 * SLIDE_MILLISECONDS), Some(pages[0].key));
        // At the first page there is nowhere to go.
        assert_eq!(wall.wheel_notch(-SLOW_NOTCH, 1_000 + 3 * SLIDE_MILLISECONDS), None);
    }

    #[test]
    fn a_step_during_a_slide_carries_on_without_a_jump() {
        let mut wall = Wall::new();
        let pages = pages(24);
        draw_at(&mut wall, &pages, 0);
        wall.zoom(pages[1].key, 0);
        wall.step_zoom(1, 1_000);
        let part = 1_000 + SLIDE_MILLISECONDS / 4;
        let offset = wall.slide.value(part);
        wall.step_zoom(1, part);
        assert!((wall.slide.value(part) - (offset + 1.0)).abs() < 1e-6, "the strip stays where it was");
        assert_eq!(wall.zoomed(), Some(pages[3].key));
    }

    #[test]
    fn a_zoomed_page_that_leaves_the_wall_ends_the_zoom() {
        let mut wall = Wall::new();
        wall.zoom(PageKey { channel: 9, page: 0x500 }, 0);
        draw_at(&mut wall, &pages(4), 0);
        assert_eq!(wall.zoomed(), None);
    }

    #[test]
    fn the_buttons_step_the_page_size() {
        let mut wall = Wall::new();
        let mut canvas = Recorder::new();
        wall.draw(&mut canvas, PANEL, &pages(4), None, 0);
        let before = wall.width;
        let plus = wall.plus;
        assert!(wall.pointer_pressed(Point::new(f64::midpoint(plus.x0, plus.x1), f64::midpoint(plus.y0, plus.y1)), 0));
        assert!((wall.width - stepped_width(before, 1)).abs() < f32::EPSILON);
    }
}
