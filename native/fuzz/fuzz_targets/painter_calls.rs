//! The drawing back end's own rasteriser (DEC-7168): whatever the drawing
//! calls (rectangles, rounded corners, polygons, dimming, gradients, clips
//! and text, at any coordinates, infinite and not-a-number among them, at
//! any scale), drawing a frame whole and then its damage keeps every bound
//! and leaves the window its size.
#![no_main]

use draw_raster::{Family, Painter};
use interface_core::canvas::{Canvas, Style};
use interface_core::geometry::{Point, Rect};
use interface_core::theme::Colour;
use libfuzzer_sys::fuzz_target;

/// A window small enough that each input draws quickly.
const WIDTH: u16 = 96;
const HEIGHT: u16 = 64;
/// The most calls one input makes.
const MAX_CALLS: usize = 64;
/// The most corners one polygon has.
const MAX_CORNERS: usize = 16;

/// The bytes as a stream of numbers, zero once they run out.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((&first, rest)) = self.0.split_first() else { return 0 };
        self.0 = rest;
        first
    }

    /// A coordinate: mostly in and around the window, sometimes any value
    /// a double holds.
    fn number(&mut self) -> f64 {
        match self.byte() % 8 {
            0 => f64::from_bits(u64::from_le_bytes(core::array::from_fn(|_| self.byte()))),
            kind => f64::from(i16::from_le_bytes([self.byte(), self.byte()])) / f64::from(kind),
        }
    }

    fn rect(&mut self) -> Rect {
        Rect { x0: self.number(), y0: self.number(), x1: self.number(), y1: self.number() }
    }

    fn colour(&mut self) -> Colour {
        Colour { red: self.byte(), green: self.byte(), blue: self.byte() }
    }
}

thread_local! {
    /// One painter for every input: making one loads the system's fonts,
    /// which takes far longer than drawing.
    static PAINTER: core::cell::RefCell<Painter> = core::cell::RefCell::new(Painter::new(Family::Roboto));
}

fuzz_target!(|data: &[u8]| PAINTER.with_borrow_mut(|painter| draw(painter, data)));

fn draw(painter: &mut Painter, data: &[u8]) {
    let mut bytes = Bytes(data);
    // Whatever the last input left, this one starts from a whole frame.
    painter.redraw_whole();
    let scale = f64::from(bytes.byte() % 4 + 1) / 2.0;
    for _ in 0..2 {
        painter.begin(WIDTH, HEIGHT, scale);
        let mut input = Bytes(bytes.0);
        for _ in 0..MAX_CALLS {
            if input.0.is_empty() {
                break;
            }
            match input.byte() % 8 {
                0 => {
                    let (area, radius, colour) = (input.rect(), input.number(), input.colour());
                    painter.fill(area, radius, colour);
                }
                1 => {
                    let corners = usize::from(input.byte()) % MAX_CORNERS;
                    let points: Vec<Point> = (0..corners).map(|_| Point { x: input.number(), y: input.number() }).collect();
                    let colour = input.colour();
                    painter.polygon(&points, colour);
                }
                2 => {
                    let area = input.rect();
                    painter.dim(area, f32::from(input.byte()) / 200.0);
                }
                3 => {
                    let area = input.rect();
                    painter.push_clip(area);
                }
                4 => painter.pop_clip(),
                5 => {
                    let (area, radius) = (input.rect(), input.number());
                    let stops = [(0.0, input.colour()), (f32::from(input.byte()) / 255.0, input.colour()), (1.0, input.colour())];
                    painter.gradient(area, radius, &stops);
                }
                _ => {
                    let area = input.rect();
                    let length = usize::from(input.byte() % 12);
                    let text: String = (0..length).map(|_| char::from(b' ' + input.byte() % 95)).collect();
                    let style = Style { size: f32::from(input.byte() % 40 + 1), monospace: false, bold: input.byte() % 2 == 1, caps: input.byte() % 2 == 1 };
                    painter.text(area, &text, style, input.colour());
                }
            }
        }
        let mut words = vec![0_u32; usize::from(WIDTH) * usize::from(HEIGHT)];
        painter.finish(&mut words);
        assert_eq!(words.len(), usize::from(WIDTH) * usize::from(HEIGHT));
        // The second time round draws only what changed, if anything did.
        bytes = Bytes(input.0);
    }
}
