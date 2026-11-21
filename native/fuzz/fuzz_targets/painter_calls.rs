//! The drawing back end's own rasteriser (DEC-7168, DEC-7196): whatever the
//! drawing calls (rectangles, rounded corners, polygons, dimming,
//! gradients, clips, text in any script with combining marks, teletext
//! pages, ReGIS and NAPLPS pictures whole and in parts, their rows and
//! stills, at any coordinates, infinite and not-a-number among them, at
//! any scale and in windows of any small size), drawing a frame whole and
//! then its damage keeps every bound and leaves the window its size.
#![no_main]

use draw_raster::{Family, Painter, RegisPart};
use interface_core::canvas::{Canvas, MAX_STILLS, PAGE_HEIGHT, PAGE_WIDTH, PageKey, PictureLanguage, REGIS_COLOURS, REGIS_KEYED_HEIGHT_PER_WIDTH, Style};
use interface_core::geometry::{Point, Rect};
use interface_core::theme::Colour;
use libfuzzer_sys::fuzz_target;

/// The largest window, small enough that each input draws quickly; the
/// window's width and height are each from none to this.
const MAX_WIDTH: u16 = 128;
const MAX_HEIGHT: u16 = 96;
/// The most calls one input makes.
const MAX_CALLS: usize = 64;
/// The most calls that set a page's image or picture, each a whole page
/// of indices or pixels, in one input.
const MAX_IMAGES: usize = 4;
/// The most corners one polygon has.
const MAX_CORNERS: usize = 16;
/// The most characters one text call draws.
const MAX_CHARACTERS: usize = 24;
/// The pages drawn: a few, so that images set are drawn again.
const PAGES: u8 = 4;
/// The stills' numbers: a few more than are kept, so the oldest goes.
#[expect(clippy::cast_possible_truncation, reason = "MAX_STILLS is 24, far inside a byte")]
const STILLS: u8 = MAX_STILLS as u8 + 4;
/// The largest whole multiple a picture is drawn at to be sampled
/// nearest.
const MAX_MULTIPLE: u8 = 3;
/// A teletext page's dots.
const PAGE_DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
/// Scales at the edges of what a double holds and of sense.
const EXTREME_SCALES: [f64; 14] =
    [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MIN_POSITIVE, 1e-300, 1e-9, 1e9, 1e300, f64::MAX, 3.0, 8.0];
/// Where text is taken from: each a first scalar value and a count, so
/// that the scripts the shipped fonts lack (and so fall back to the
/// system's fonts) and the combining marks are reached.
const SCRIPTS: [(u32, u32); 12] = [
    (0x20, 0x5F),     // Basic Latin
    (0xA0, 0x160),    // Latin-1 and Latin Extended-A
    (0x300, 0x70),    // combining diacritical marks
    (0x370, 0x190),   // Greek and Cyrillic
    (0x590, 0x170),   // Hebrew and Arabic
    (0x900, 0x400),   // the Indic scripts
    (0xE00, 0x100),   // Thai and Lao
    (0x1100, 0x100),  // Hangul Jamo
    (0x3040, 0x60),   // Hiragana
    (0x4E00, 0x5200), // CJK unified ideographs
    (0x1F300, 0x300), // pictographs and emoji
    (0x0, 0x11_0000), // any scalar value
];

/// The bytes as a stream of numbers, zero once they run out.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((&first, rest)) = self.0.split_first() else { return 0 };
        self.0 = rest;
        first
    }

    fn word(&mut self) -> u16 {
        u16::from_le_bytes([self.byte(), self.byte()])
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

    /// A scale: mostly the usual halves from one half to two, sometimes
    /// an extreme, sometimes any double.
    fn scale(&mut self) -> f64 {
        match self.byte() % 8 {
            0 => self.number(),
            1 => *EXTREME_SCALES.get(usize::from(self.byte()) % EXTREME_SCALES.len()).unwrap_or(&1.0),
            _ => f64::from(self.byte() % 4 + 1) / 2.0,
        }
    }

    fn key(&mut self) -> PageKey {
        PageKey { channel: 1, page: 0x100 + u16::from(self.byte() % PAGES) }
    }

    /// Text of up to [`MAX_CHARACTERS`] scalar values, each from one of
    /// [`SCRIPTS`].
    fn text(&mut self) -> String {
        let length = usize::from(self.byte()) % (MAX_CHARACTERS + 1);
        (0..length)
            .filter_map(|_| {
                let (first, count) = *SCRIPTS.get(usize::from(self.byte()) % SCRIPTS.len()).unwrap_or(&SCRIPTS[0]);
                char::from_u32(first + u32::from(self.word()) % count)
            })
            .collect()
    }

    /// A page's area: any rectangle, or one at which a picture in
    /// `language` is drawn at a whole multiple of its size at `scale`, so
    /// that it is sampled nearest rather than smoothly.
    fn page_area(&mut self, scale: f64) -> Rect {
        if self.byte().is_multiple_of(2) {
            return self.rect();
        }
        let multiple = f64::from(self.byte() % MAX_MULTIPLE + 1);
        let (x, y) = (f64::from(i8::from_le_bytes([self.byte()])), f64::from(i8::from_le_bytes([self.byte()])));
        let (width, height) = if self.byte().is_multiple_of(2) {
            // A ReGIS picture contained in the area is as wide as the area
            // when the area is at least the stacked rows' height.
            let width = f64::from(PictureLanguage::Regis.width()) * multiple / scale;
            (width, width * REGIS_KEYED_HEIGHT_PER_WIDTH)
        } else {
            // A NAPLPS picture fills the area.
            (f64::from(PAGE_WIDTH) * multiple / scale, f64::from(PAGE_HEIGHT) * multiple / scale)
        };
        Rect::new(x, y, x + width, y + height)
    }

    /// `count` bytes repeating a pattern taken from the input.
    fn pattern(&mut self, count: usize) -> Vec<u8> {
        let length = usize::from(self.byte() % 8) + 1;
        let pattern: Vec<u8> = (0..length).map(|_| self.byte()).collect();
        pattern.iter().copied().cycle().take(count).collect()
    }

    /// A length that is usually `exact` and sometimes one off, so that
    /// both acceptance and refusal are reached.
    fn length(&mut self, exact: usize) -> usize {
        match self.byte() % 8 {
            0 => exact.saturating_sub(1),
            1 => exact + 1,
            _ => exact,
        }
    }

    fn set_page(&mut self, painter: &mut Painter) {
        let key = self.key();
        let palette: Vec<[u8; 3]> = (0..usize::from(self.byte() % 40)).map(|_| [self.byte(), self.byte(), self.byte()]).collect();
        let screen = self.byte() % 48;
        let count = self.length(PAGE_DOTS);
        let mut indices = self.pattern(count);
        // Mostly indices inside the palette, so the page is taken.
        if !self.byte().is_multiple_of(4) {
            let colours = u8::try_from(palette.len().max(1)).unwrap_or(u8::MAX);
            for index in &mut indices {
                *index %= colours;
            }
        }
        let _ = painter.set_page(key, &palette, screen, &indices);
    }

    fn set_regis(&mut self, painter: &mut Painter) {
        let key = self.key();
        let language = if self.byte().is_multiple_of(2) { PictureLanguage::Regis } else { PictureLanguage::Naplps };
        let palette: [[u8; 3]; REGIS_COLOURS] = core::array::from_fn(|_| [self.byte(), self.byte(), self.byte()]);
        let number = u32::from(self.byte());
        let part = if self.byte().is_multiple_of(3) {
            RegisPart::whole(number, language)
        } else {
            // A part, mostly inside the picture and of even width.
            let (x, y) = (self.word() % (language.width() + 8), self.word() % (language.height() + 8));
            let width = self.word() % (language.width() / 4);
            let width = if self.byte().is_multiple_of(8) { width } else { width & !1 };
            RegisPart { language, number, base: u32::from(self.byte()), x, y, width, height: self.word() % (language.height() / 4) }
        };
        let count = self.length(usize::from(part.width) * usize::from(part.height) / 2);
        let packed = self.pattern(count);
        let _ = painter.set_regis(key, &palette, part, &packed);
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
    let (width, height) = (u16::from(bytes.byte()) % (MAX_WIDTH + 1), u16::from(bytes.byte()) % (MAX_HEIGHT + 1));
    let scale = bytes.scale();
    let mut images = 0;
    for _ in 0..2 {
        painter.begin(width, height, scale);
        let mut input = Bytes(bytes.0);
        for _ in 0..MAX_CALLS {
            if input.0.is_empty() {
                break;
            }
            match input.byte() % 16 {
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
                6 | 7 => {
                    let area = input.rect();
                    let text = input.text();
                    let style = Style {
                        size: f32::from(input.byte() % 40 + 1),
                        monospace: input.byte().is_multiple_of(4),
                        bold: input.byte() % 2 == 1,
                        caps: input.byte() % 2 == 1,
                    };
                    let _ = painter.text_width(&text, style);
                    painter.text(area, &text, style, input.colour());
                }
                8 if images < MAX_IMAGES => {
                    images += 1;
                    input.set_page(painter);
                }
                9 if images < MAX_IMAGES => {
                    images += 1;
                    input.set_regis(painter);
                }
                10 | 11 => {
                    let key = input.key();
                    let area = input.page_area(scale);
                    let _ = painter.page(key, area);
                    let _ = painter.screen(key);
                }
                12 => {
                    let (still, key) = (u32::from(input.byte() % STILLS), input.key());
                    let _ = painter.keep_still(still, key);
                }
                13 => {
                    let still = u32::from(input.byte() % STILLS);
                    let area = input.rect();
                    let _ = painter.still(still, area);
                }
                14 => painter.set_regis_contain(input.byte().is_multiple_of(2)),
                _ => {
                    let keyed: Vec<PageKey> = (0..input.byte() % (PAGES + 1)).map(|_| input.key()).collect();
                    painter.set_regis_keyed(keyed);
                }
            }
        }
        let mut words = vec![0_u32; usize::from(width) * usize::from(height)];
        painter.finish(&mut words);
        assert_eq!(words.len(), usize::from(width) * usize::from(height));
        // The second time round draws only what changed, if anything did.
        bytes = Bytes(input.0);
    }
}
