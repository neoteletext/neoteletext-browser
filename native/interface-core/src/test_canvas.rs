//! A canvas for tests: it records where pages are drawn and has an image
//! of every page, and measures text as half its size per character.

use alloc::vec::Vec;

use crate::canvas::{Canvas, PageKey, Style};
use crate::geometry::{Point, Rect};
use crate::theme::Colour;

/// A page image or a fill, as one call in the order of drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Painted {
    Page(PageKey, Rect),
    Fill(Rect, Colour),
    /// A still drawn, by its number.
    Still(u32, Rect),
}

pub struct Recorder {
    pub pages: Vec<(PageKey, Rect)>,
    /// The page images and the fills together, in the order drawn, for
    /// what must be drawn over what.
    pub painted: Vec<Painted>,
    pub texts: Vec<(Rect, alloc::string::String)>,
    pub fills: Vec<(Rect, Colour)>,
    /// Each polygon, and its colour.
    pub polygons: Vec<(Vec<Point>, Colour)>,
    /// Each darkening and how much, in the order drawn.
    pub dims: Vec<(Rect, f32)>,
    /// Each text's colour, in the order drawn.
    pub inks: Vec<(alloc::string::String, Colour)>,
    pub scale: f64,
    /// What every page's screen colour is; none before a page has arrived.
    pub screen: Option<Colour>,
    /// Each still kept, by its number, with the page it was made from,
    /// as the canvas holds them now.
    pub stills: Vec<(u32, PageKey)>,
    /// Every still taken, in order, kept or not.
    pub taken: Vec<(u32, PageKey)>,
    /// Whether a still can be made: as a page's image is held.
    pub has_images: bool,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            pages: Vec::new(),
            painted: Vec::new(),
            texts: Vec::new(),
            fills: Vec::new(),
            polygons: Vec::new(),
            dims: Vec::new(),
            inks: Vec::new(),
            scale: 2.0,
            screen: Some(crate::theme::BLACK),
            stills: Vec::new(),
            taken: Vec::new(),
            has_images: true,
        }
    }

    /// The colour `text` was last drawn in, if it was.
    pub fn coloured_text(&self, text: &str) -> Option<Colour> {
        self.inks.iter().rev().find(|(drawn, _)| drawn == text).map(|(_, colour)| *colour)
    }
}

impl Canvas for Recorder {
    fn fill(&mut self, rect: Rect, _: f64, colour: Colour) {
        self.fills.push((rect, colour));
        self.painted.push(Painted::Fill(rect, colour));
    }
    fn device_scale(&self) -> f64 {
        self.scale
    }
    fn dim(&mut self, rect: Rect, amount: f32) {
        self.dims.push((rect, amount));
    }
    fn polygon(&mut self, points: &[Point], colour: Colour) {
        self.polygons.push((points.to_vec(), colour));
    }
    fn push_clip(&mut self, _: Rect) {}
    fn pop_clip(&mut self) {}
    fn gradient(&mut self, _: Rect, _: f64, _: &[(f32, Colour)]) {}
    fn page(&mut self, key: PageKey, rect: Rect) -> bool {
        self.pages.push((key, rect));
        self.painted.push(Painted::Page(key, rect));
        true
    }
    fn screen(&mut self, _: PageKey) -> Option<Colour> {
        self.screen
    }
    fn text(&mut self, rect: Rect, text: &str, _: Style, colour: Colour) {
        self.texts.push((rect, text.into()));
        self.inks.push((text.into(), colour));
    }
    #[allow(clippy::cast_precision_loss, reason = "short strings")]
    fn text_width(&mut self, text: &str, style: Style) -> f64 {
        text.chars().count() as f64 * f64::from(style.size) / 2.0
    }
    fn line_height(&mut self, style: Style) -> f64 {
        f64::from(style.size)
    }
    fn keep_still(&mut self, still: u32, key: PageKey) -> bool {
        self.taken.push((still, key));
        self.stills.retain(|&(kept, _)| kept != still);
        if self.has_images {
            self.stills.push((still, key));
        }
        self.has_images
    }
    fn still(&mut self, still: u32, rect: Rect) -> bool {
        let kept = self.stills.iter().any(|&(kept, _)| kept == still);
        if kept {
            self.painted.push(Painted::Still(still, rect));
        }
        kept
    }
    fn forget_still(&mut self, still: u32) {
        self.stills.retain(|&(kept, _)| kept != still);
    }
}
