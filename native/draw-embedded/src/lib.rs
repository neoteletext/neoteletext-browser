//! The interface core's drawing interface for microcontrollers (design
//! section 7): `embedded-graphics` onto any display driver, with bitmap
//! fonts, and no standard library.
//!
//! What it cuts: text is ASCII in fixed-size bitmap fonts, with small
//! capitals drawn as plain capitals; gradients are drawn as bands of their
//! stop colours.

#![no_std]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_7X13, FONT_7X13_BOLD, FONT_9X15, FONT_9X15_BOLD, FONT_10X20};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb888;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle, RoundedRectangle, Triangle};
use embedded_graphics::text::{Baseline, Text};
use interface_core::canvas::{Canvas, PAGE_HEIGHT, PAGE_WIDTH, PageKey, Style};
use interface_core::geometry::Point as LogicalPoint;
use interface_core::geometry::Rect;
use interface_core::theme::Colour;

const PAGE_DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
/// The Colour Map of Presentation Level 2.5: four tables of eight.
const MAX_COLOURS: usize = 32;

/// Font sizes, in logical pixels, below which the next smaller bitmap font
/// is used. The fonts' own heights are 10, 13, 15 and 20 pixels.
const SMALL_BELOW: f32 = 12.0;
const MEDIUM_BELOW: f32 = 15.0;
const LARGE_BELOW: f32 = 19.0;

/// Why a page image was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum PageError {
    /// Not 480 by 500 indices.
    Size,
    /// More colours than teletext has, or an index past the palette's end.
    Palette,
}

struct PageImage {
    palette: Vec<Rgb888>,
    indices: Vec<u8>,
}

/// A display and the pages it can show. Each page is 240 kilobytes of
/// indices, so a small chip keeps only the visible pages
/// ([`Screen::retain_pages`]).
pub struct Screen<D> {
    pub target: D,
    pages: BTreeMap<PageKey, PageImage>,
    /// Clips in force, innermost last.
    clips: Vec<Rectangle>,
}

fn to_i32(value: f64) -> i32 {
    #[allow(clippy::cast_possible_truncation, reason = "clamped to the range of i32 first")]
    let whole = libm::round(value).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    whole
}

fn rectangle(area: Rect) -> Rectangle {
    Rectangle::with_corners(Point::new(to_i32(area.x0), to_i32(area.y0)), Point::new(to_i32(area.x1) - 1, to_i32(area.y1) - 1))
}

fn rgb(colour: Colour) -> Rgb888 {
    Rgb888::new(colour.red, colour.green, colour.blue)
}

fn font(style: Style) -> &'static MonoFont<'static> {
    match (style.size, style.bold) {
        (size, _) if size < SMALL_BELOW => &FONT_6X10,
        (size, false) if size < MEDIUM_BELOW => &FONT_7X13,
        (size, true) if size < MEDIUM_BELOW => &FONT_7X13_BOLD,
        (size, false) if size < LARGE_BELOW => &FONT_9X15,
        (size, true) if size < LARGE_BELOW => &FONT_9X15_BOLD,
        _ => &FONT_10X20,
    }
}

impl<D> Screen<D>
where
    D: DrawTarget,
    D::Color: From<Rgb888>,
{
    pub fn new(target: D) -> Self {
        Self { target, pages: BTreeMap::new(), clips: Vec::new() }
    }

    /// The area drawing may touch: the display, within every clip.
    fn clip(&self) -> Rectangle {
        self.clips.last().copied().unwrap_or_else(|| self.target.bounding_box())
    }

    /// Replaces a page's image whole (requirement 9.6), from palette indices.
    ///
    /// # Errors
    /// When the indices are not a full page, or the palette or an index is
    /// out of range.
    pub fn set_page(&mut self, key: PageKey, palette: &[[u8; 3]], indices: &[u8]) -> Result<(), PageError> {
        if indices.len() != PAGE_DOTS {
            return Err(PageError::Size);
        }
        if palette.is_empty() || palette.len() > MAX_COLOURS || indices.iter().any(|&index| usize::from(index) >= palette.len()) {
            return Err(PageError::Palette);
        }
        let palette = palette.iter().map(|&[red, green, blue]| Rgb888::new(red, green, blue)).collect();
        self.pages.insert(key, PageImage { palette, indices: indices.to_vec() });
        Ok(())
    }

    /// Forgets the images of pages not in `keep`.
    pub fn retain_pages(&mut self, keep: impl Fn(&PageKey) -> bool) {
        self.pages.retain(|key, _| keep(key));
    }
}

// A display that fails to draw has nothing better to do than carry on with
// the next shape, so drawing errors are dropped throughout.
impl<D> Canvas for Screen<D>
where
    D: DrawTarget,
    D::Color: From<Rgb888>,
{
    fn fill(&mut self, area: Rect, radius: f64, colour: Colour) {
        let style = PrimitiveStyle::with_fill(D::Color::from(rgb(colour)));
        let shape = rectangle(area);
        let clip = self.clip();
        let mut target = self.target.clipped(&clip);
        if radius > 0.0 {
            #[allow(clippy::cast_sign_loss, reason = "a positive radius")]
            let corner = to_i32(radius) as u32;
            let _ = RoundedRectangle::with_equal_corners(shape, Size::new(corner, corner)).into_styled(style).draw(&mut target);
        } else {
            let _ = shape.into_styled(style).draw(&mut target);
        }
    }

    fn polygon(&mut self, points: &[LogicalPoint], colour: Colour) {
        // A fan of triangles from the first point: exact for the convex
        // shapes the interface draws.
        let style = PrimitiveStyle::with_fill(D::Color::from(rgb(colour)));
        let point = |at: LogicalPoint| Point::new(to_i32(at.x), to_i32(at.y));
        let Some(&first) = points.first() else { return };
        let clip = self.clip();
        let mut target = self.target.clipped(&clip);
        for pair in points[1..].windows(2) {
            let _ = Triangle::new(point(first), point(pair[0]), point(pair[1])).into_styled(style).draw(&mut target);
        }
    }

    /// No transparency on a microcontroller's display: the wall stays as
    /// it is behind a zooming page.
    fn dim(&mut self, _area: Rect, _amount: f32) {}

    fn push_clip(&mut self, area: Rect) {
        let clip = self.clip().intersection(&rectangle(area));
        self.clips.push(clip);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn gradient(&mut self, area: Rect, _radius: f64, stops: &[(f32, Colour)]) {
        // Each stop's colour from its offset to the next stop's.
        for (index, &(offset, colour)) in stops.iter().enumerate() {
            let end = stops.get(index + 1).map_or(1.0, |&(next, _)| next);
            let left = area.x0 + area.width() * f64::from(offset);
            let right = area.x0 + area.width() * f64::from(end);
            self.fill(Rect::new(left, area.y0, right.max(left + 1.0), area.y1), 0.0, colour);
        }
    }

    fn page(&mut self, key: PageKey, place: Rect) -> bool {
        let Some(page) = self.pages.get(&key) else { return false };
        let destination = rectangle(place);
        let area = destination.intersection(&self.clip());
        let (width, height) = (destination.size.width, destination.size.height);
        if width == 0 || height == 0 || area.size.width == 0 || area.size.height == 0 {
            return true;
        }
        // Nearest-neighbour: each display pixel takes the dot under its corner.
        let colours = area.points().map(|at| {
            let local = at - destination.top_left;
            #[allow(clippy::cast_sign_loss, reason = "points inside the destination are at or after its top-left")]
            let (x, y) = (local.x as u32, local.y as u32);
            let column = x * u32::from(PAGE_WIDTH) / width;
            let row = y * u32::from(PAGE_HEIGHT) / height;
            let index = page.indices[row as usize * usize::from(PAGE_WIDTH) + column as usize];
            D::Color::from(page.palette[usize::from(index)])
        });
        let _ = self.target.fill_contiguous(&area, colours);
        true
    }

    fn text(&mut self, area: Rect, text: &str, style: Style, colour: Colour) {
        let character = MonoTextStyle::new(font(style), D::Color::from(rgb(colour)));
        let place = rectangle(area);
        let clip = self.clip().intersection(&place);
        let mut clipped = self.target.clipped(&clip);
        let text: String = if style.caps { text.chars().flat_map(char::to_uppercase).collect() } else { String::from(text) };
        let _ = Text::with_baseline(&text, place.top_left, character, Baseline::Top).draw(&mut clipped);
    }

    fn text_width(&mut self, text: &str, style: Style) -> f64 {
        let font = font(style);
        let advance = font.character_size.width + font.character_spacing;
        #[allow(clippy::cast_possible_truncation, reason = "interface strings are short")]
        let characters = text.chars().count() as u32;
        f64::from(characters * advance)
    }

    fn line_height(&mut self, style: Style) -> f64 {
        f64::from(font(style).character_size.height)
    }
}
