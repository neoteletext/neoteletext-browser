//! A canvas for tests: it records where pages are drawn and has an image
//! of every page, and measures text as half its size per character.

use alloc::vec::Vec;

use crate::canvas::{Canvas, PageKey, Style};
use crate::geometry::{Point, Rect};
use crate::theme::Colour;

pub struct Recorder {
    pub pages: Vec<(PageKey, Rect)>,
    pub texts: Vec<(Rect, alloc::string::String)>,
    pub fills: Vec<(Rect, Colour)>,
    pub scale: f64,
}

impl Recorder {
    pub fn new() -> Self {
        Self { pages: Vec::new(), texts: Vec::new(), fills: Vec::new(), scale: 2.0 }
    }
}

impl Canvas for Recorder {
    fn fill(&mut self, rect: Rect, _: f64, colour: Colour) {
        self.fills.push((rect, colour));
    }
    fn device_scale(&self) -> f64 {
        self.scale
    }
    fn dim(&mut self, _: Rect, _: f32) {}
    fn polygon(&mut self, _: &[Point], _: Colour) {}
    fn push_clip(&mut self, _: Rect) {}
    fn pop_clip(&mut self) {}
    fn gradient(&mut self, _: Rect, _: f64, _: &[(f32, Colour)]) {}
    fn page(&mut self, key: PageKey, rect: Rect) -> bool {
        self.pages.push((key, rect));
        true
    }
    fn screen(&mut self, _: PageKey) -> Option<Colour> {
        Some(crate::theme::BLACK)
    }
    fn text(&mut self, rect: Rect, text: &str, _: Style, _: Colour) {
        self.texts.push((rect, text.into()));
    }
    #[allow(clippy::cast_precision_loss, reason = "short strings")]
    fn text_width(&mut self, text: &str, style: Style) -> f64 {
        text.chars().count() as f64 * f64::from(style.size) / 2.0
    }
    fn line_height(&mut self, style: Style) -> f64 {
        f64::from(style.size)
    }
}
