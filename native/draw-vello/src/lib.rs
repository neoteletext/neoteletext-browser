//! The interface core's drawing interface on the processor (design section
//! 4.1): `vello_cpu` draws every pixel with no graphics processor, and
//! Parley lays out and shapes text. The interface's own text is in a font
//! shipped inside the program, Roboto or Open Sans (both from Google Fonts,
//! under the SIL Open Font License 1.1, `fonts/`); system fonts fill in
//! scripts those fonts lack.
//!
//! A frame's drawing calls are kept and compared with the last frame's;
//! only the box of device pixels that holds every call that changed is
//! drawn again (`damage`).

// No panicking constructs: it draws the pages and the text the trusted
// core sends, in the interface's process (finding F-105). The same list is
// in each crate that reads input from outside its process; scripts/check.sh
// checks that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

mod damage;
mod pages;
mod raster;
mod record;
#[cfg(test)]
mod tests;
mod text;

pub use damage::{Damage, DeviceBox, MAX_BOXES};

use std::collections::HashMap;
use std::sync::Arc;

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH, PageKey};
use interface_core::geometry::Rect;
use interface_core::theme::Colour;
use parley::fontique::Blob;
use parley::{FontContext, Layout, LayoutContext};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{self, Point};
use vello_cpu::{Pixmap, RenderContext, Resources};

use crate::pages::PageImage;
use crate::record::Frame;
use crate::text::StyleKey;

/// How closely curves are flattened into lines, in pixels.
const TOLERANCE: f64 = 0.1;
const OPAQUE: u8 = u8::MAX;
const RGBA_BYTES: usize = 4;
const PAGE_DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
const ROBOTO: &[u8] = include_bytes!("../fonts/Roboto.ttf");
const OPEN_SANS: &[u8] = include_bytes!("../fonts/OpenSans.ttf");

/// The shipped font the interface's text is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Roboto.
    Roboto,
    /// Open Sans.
    OpenSans,
}

impl Family {
    fn name(self) -> &'static str {
        match self {
            Self::Roboto => "Roboto",
            Self::OpenSans => "Open Sans",
        }
    }
}

fn colour(colour: Colour) -> AlphaColor<Srgb> {
    AlphaColor::from_rgb8(colour.red, colour.green, colour.blue)
}

fn rect(rect: Rect) -> kurbo::Rect {
    kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1)
}

fn point(point: interface_core::geometry::Point) -> Point {
    Point::new(point.x, point.y)
}

/// The drawing back end on the processor: draws into a pixel buffer with
/// `vello_cpu` and lays text out with `parley`.
pub struct Painter {
    /// Draws the damaged box, sized to it.
    context: RenderContext,
    resources: Resources,
    /// The damaged box's pixels, drawn.
    target: Pixmap,
    fonts: FontContext,
    layouts: LayoutContext<()>,
    pages: HashMap<PageKey, PageImage>,
    /// The number the next new page image takes.
    next_image: u64,
    /// The dots of each page that changed since the last frame was drawn,
    /// in the page's own dots.
    changed: HashMap<PageKey, DeviceBox>,
    /// The frame being drawn, and the last one drawn.
    frame: Frame,
    last: Frame,
    /// The whole window, one 0RGB word a pixel, as the last frame left it.
    words: Vec<u32>,
    /// The surface's size in device pixels.
    width: u16,
    height: u16,
    /// The next frame is drawn whole: the first, or after a resize.
    whole: bool,
    /// Where the damaged box's top-left corner lies, while it is drawn.
    origin: (f64, f64),
    /// Text laid out once and reused: the interface draws the same
    /// strings every frame, and measures each before drawing it. By style,
    /// then by text, so a lookup borrows the text rather than copying it.
    layouts_made: HashMap<StyleKey, HashMap<String, Layout<()>>>,
    /// How many layouts are kept, against [`LAYOUT_CACHE_LIMIT`](crate::text::LAYOUT_CACHE_LIMIT).
    layouts_kept: usize,
    family: Family,
    /// Logical pixels to device pixels.
    pub scale: f64,
}

/// Why a page image was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum PageError {
    /// Not 480 by 500 indices.
    Size,
    /// An index past the end of the palette.
    Index,
}

impl Painter {
    /// A painter drawing text in `family`, at a scale of one.
    #[must_use]
    pub fn new(family: Family) -> Self {
        let mut fonts = FontContext::new();
        for data in [ROBOTO, OPEN_SANS] {
            fonts.collection.register_fonts(Blob::new(Arc::new(data)), None);
        }
        Self {
            context: RenderContext::new(1, 1),
            resources: Resources::new(),
            target: Pixmap::new(1, 1),
            fonts,
            layouts: LayoutContext::new(),
            pages: HashMap::new(),
            next_image: 0,
            changed: HashMap::new(),
            frame: Frame::new(DeviceBox::default()),
            last: Frame::default(),
            words: Vec::new(),
            width: 0,
            height: 0,
            whole: true,
            origin: (0.0, 0.0),
            layouts_made: HashMap::new(),
            layouts_kept: 0,
            family,
            scale: 1.0,
        }
    }

    /// Starts a frame of `width` by `height` device pixels, at `scale`
    /// device pixels to each logical pixel.
    pub fn begin(&mut self, width: u16, height: u16, scale: f64) {
        #[expect(clippy::float_cmp, reason = "any change of scale at all redraws the whole window")]
        let same = self.width == width && self.height == height && self.scale == scale;
        if !same {
            self.width = width;
            self.height = height;
            self.words = vec![0; usize::from(width).saturating_mul(usize::from(height))];
            self.whole = true;
        }
        self.scale = scale;
        self.frame = Frame::new(self.surface());
    }

    /// The next frame is drawn whole, as when the window was uncovered and
    /// the system asks for every pixel again.
    pub fn redraw_whole(&mut self) {
        self.whole = true;
    }

    fn surface(&self) -> DeviceBox {
        DeviceBox::whole(self.width, self.height)
    }

    /// Draws what changed since the last frame into the painter's copy of
    /// the window, and says where: empty when nothing did.
    pub fn render(&mut self) -> Damage {
        let frame = std::mem::take(&mut self.frame);
        let damage = if self.whole {
            let mut whole = Damage::default();
            whole.add(self.surface());
            whole
        } else {
            self.damage(&frame)
        };
        self.whole = false;
        self.changed.clear();
        for &area in damage.boxes() {
            self.draw_box(&frame, area);
        }
        self.last = frame;
        damage
    }

    /// Writes the window into a surface's `words`, as one 0RGB word a
    /// pixel: only `damage` when the surface holds the last frame (an
    /// `age` of one), or all of it.
    pub fn copy_to(&self, words: &mut [u32], damage: &Damage, age: u8) {
        if age != 1 || words.len() != self.words.len() {
            words.iter_mut().zip(&self.words).for_each(|(word, &drawn)| *word = drawn);
            return;
        }
        let width = usize::from(self.width);
        for area in damage.boxes() {
            for row in area.y0..area.y1 {
                let Some(start) = usize::from(row).checked_mul(width).and_then(|start| start.checked_add(usize::from(area.x0))) else { continue };
                let range = start..start.saturating_add(usize::from(area.width()));
                if let (Some(to), Some(from)) = (words.get_mut(range.clone()), self.words.get(range)) {
                    to.copy_from_slice(from);
                }
            }
        }
    }

    /// Renders the frame and writes the whole window into `words`.
    pub fn finish(&mut self, words: &mut [u32]) {
        let damage = self.render();
        self.copy_to(words, &damage, 0);
    }
}
