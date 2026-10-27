//! A page zoomed from its place on the wall to fill the area and back,
//! stepping from page to page by key, trackpad or wheel notch, and the
//! zoomed page drawn with its neighbours sliding and its close button.

use alloc::format;

use super::grid::{CAPTION_GAP, CAPTION_HEIGHT, CAPTION_NUMBER_WIDTH, FRAME, PAGE_ASPECT, WALL_PADDING};
use super::{Wall, WallPage};
use crate::canvas::{Canvas, PageKey};
use crate::geometry::{Point, Rect};
use crate::text::{cross, fitted_text, label, outline};
use crate::theme::{BLACK, BUTTON, CAPTION, CAPTION_TITLE, MUTED, PANEL, RULE, WHITE};
use crate::tween::{STANDARD, Tween};

/// The zoom from the wall and back: Material 3's medium duration of 300
/// milliseconds on its standard curve, `cubic-bezier(0.2, 0, 0, 1)`
/// (material-components-android `docs/theming/Motion.md`).
pub(crate) const ZOOM_MILLISECONDS: u64 = 300;
/// How dark the wall becomes behind a zoomed page.
const ZOOM_DIM: f64 = 0.75;
/// While zoomed, a trackpad moves one page for every 20 pixels it would
/// scroll (a mouse wheel moves by notches instead: `Wall::wheel_notch`),
/// and keeps at most two pages of movement in hand, so a fast flick does
/// not run on for long.
pub(super) const WHEEL_PER_PAGE: f64 = 20.0;
const WHEEL_CARRY_LIMIT: f64 = 2.0 * WHEEL_PER_PAGE;
/// A step to the next page slides the pages over 220 milliseconds on the
/// standard curve: quicker than the zoom, so wheeling through pages feels
/// fast, and easing to a stop with the page snapped into place.
pub(super) const SLIDE_MILLISECONDS: u64 = 220;
/// How many neighbours either side may be on screen during fast steps.
const SLIDE_REACH: i32 = 2;
/// The close button of a zoomed page.
const CLOSE_SIZE: f64 = 36.0;
const CLOSE_INSET: f64 = 10.0;
const CLOSE_GLYPH: f64 = 14.0;
const CLOSE_THICKNESS: f64 = 3.0;

impl Wall {
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

    pub(super) fn start_zoom(&mut self, key: PageKey, now: u64) {
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

    /// Where a zoomed page sits: as large as the area allows, keeping its
    /// shape, below a line for its number and title.
    pub(super) fn zoomed_rect(&self) -> Rect {
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
    pub(super) fn draw_zooming<C: Canvas + ?Sized>(&mut self, canvas: &mut C, key: PageKey, progress: f64) {
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
    pub(super) fn draw_zoomed<C: Canvas + ?Sized>(&mut self, canvas: &mut C, pages: &[WallPage<'_>], key: PageKey, now: u64) -> f64 {
        self.scroll = self.scroll.max(0.0);
        let area = self.area;
        let Some((current, page)) = pages.iter().enumerate().find(|(_, page)| page.key == key) else { return 0.0 };
        let rect = self.zoomed_rect();
        self.draw_sliding(canvas, pages, current, rect, true, now);
        // The caption line stays put on its own background; pages slide
        // beneath it.
        canvas.fill(Rect::new(area.x0, area.y0, area.x1, rect.y0 - FRAME), 0.0, PANEL);
        let caption = Point::new(rect.x0, area.y0 + WALL_PADDING);
        label(canvas, caption, &format!("{:03X}", key.page), CAPTION, WHITE);
        fitted_text(
            canvas,
            Rect::new(caption.x + CAPTION_NUMBER_WIDTH, caption.y + 1.0, rect.x1, caption.y + CAPTION_HEIGHT),
            page.title,
            CAPTION_TITLE,
            MUTED,
        );
        area.height() - WALL_PADDING * 2.0
    }

    /// The zoomed page and its neighbours on a vertical strip, the page at
    /// `rect`, offset by the slide, which eases to rest with the page
    /// exactly in place; each framed when `framed`.
    fn draw_sliding<C: Canvas + ?Sized>(&mut self, canvas: &mut C, pages: &[WallPage<'_>], current: usize, rect: Rect, framed: bool, now: u64) {
        let area = self.area;
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
            let image = self.draw_page(canvas, other.key, shifted);
            if neighbour == 0 && slide.abs() < f64::from(f32::EPSILON) {
                self.open = Some((other.key, image));
            }
            if framed {
                outline(canvas, Rect::new(shifted.x0 - FRAME, shifted.y0 - FRAME, shifted.x1 + FRAME, shifted.y1 + FRAME), RULE);
            }
        }
    }

    /// The open page alone in `area`, the largest it fits at its own
    /// proportions, centred, in its screen colour, with none of the wall's
    /// controls (decision 85); the next pages slide in as in the window.
    /// Returns where the page is drawn with its screen, or none when no
    /// page is open or it has left the wall.
    pub fn draw_full_screen<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect, pages: &[WallPage<'_>], now: u64) -> Option<Rect> {
        self.area = area;
        self.page_keys = pages.iter().map(|page| page.key).collect();
        self.drawn.clear();
        self.open = None;
        // The wall's controls are not drawn, so none of them takes a press.
        for control in [&mut self.close, &mut self.bar, &mut self.minus, &mut self.plus, &mut self.bar_zone] {
            *control = Rect::ZERO;
        }
        let key = self.zoomed.filter(|_| self.zoom_in)?;
        let Some(current) = pages.iter().position(|page| page.key == key) else {
            self.to_top();
            return None;
        };
        let width = area.width().min(area.height() / PAGE_ASPECT).max(0.0);
        let (left, top) = (f64::midpoint(area.x0, area.x1) - width / 2.0, f64::midpoint(area.y0, area.y1) - width * PAGE_ASPECT / 2.0);
        let rect = Rect::new(left, top, left + width, top + width * PAGE_ASPECT);
        // Around the page, its screen colour, as the page area draws it.
        let screen = canvas.screen(key).unwrap_or(BLACK);
        canvas.fill(area, 0.0, screen);
        canvas.push_clip(area);
        self.draw_sliding(canvas, pages, current, rect, false, now);
        canvas.pop_clip();
        Some(rect)
    }

    /// The close button of a zoomed page, at the area's top right.
    pub(super) fn draw_close<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let right = self.area.x1 - CLOSE_INSET;
        let top = self.area.y0 + CLOSE_INSET;
        self.close = Rect::new(right - CLOSE_SIZE, top, right, top + CLOSE_SIZE);
        canvas.fill(self.close, 0.0, BUTTON);
        outline(canvas, self.close, WHITE);
        let centre = Point::new(f64::midpoint(self.close.x0, self.close.x1), f64::midpoint(self.close.y0, self.close.y1));
        cross(canvas, centre, CLOSE_GLYPH / 2.0, CLOSE_THICKNESS, WHITE);
    }
}
