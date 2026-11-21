//! The interface core's drawing interface on the processor (design section
//! 4.1): its own rasteriser draws every pixel with no graphics processor,
//! anti-aliased by the exact area each pixel's inside covers, and Parley
//! lays out and shapes text, whose glyphs' outlines skrifa reads. The interface's own text is in a font
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
mod glyphs;
mod pages;
mod raster;
mod record;
#[cfg(test)]
mod tests;
mod text;

pub use damage::{Damage, DeviceBox, MAX_BOXES};
pub use pages::RegisPart;

use std::collections::HashMap;
use std::sync::Arc;

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH, PageKey};
use parley::fontique::Blob;
use parley::{FontContext, Layout, LayoutContext};

use crate::glyphs::Glyphs;
use crate::pages::{PageImage, RegisImage, Still};
use crate::raster::Scratch;
use crate::raster::surface::Surface;
use crate::record::Frame;
use crate::text::StyleKey;

const OPAQUE: u8 = u8::MAX;
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

/// The drawing back end on the processor: draws into the window's pixels
/// with its own rasteriser and lays text out with `parley`.
pub struct Painter {
    /// The whole window, one 0RGB word a pixel, as the last frame left it,
    /// and the clips open while a box is drawn.
    surface: Surface,
    /// The rasteriser's working store.
    scratch: Scratch,
    /// Glyph outlines, hinted at the sizes drawn.
    glyphs: Glyphs,
    fonts: FontContext,
    layouts: LayoutContext<()>,
    pages: HashMap<PageKey, PageImage>,
    /// The pictures of the pages drawn in ReGIS, each shown in its page's
    /// place (DEC-6335).
    regis: HashMap<PageKey, RegisImage>,
    /// A ReGIS picture is contained in the rectangle its page is drawn
    /// in; else in the teletext page's band of it (REQ-6363).
    regis_contain: bool,
    /// The pages drawn in ReGIS that show their row of coloured-key
    /// prompts beneath the picture (REQ-6379).
    regis_keyed: Vec<PageKey>,
    /// The stills of the tab filmstrip, by their numbers (REQ-6640 item
    /// 3), at most [`interface_core::canvas::MAX_STILLS`].
    stills: HashMap<u32, Still>,
    /// The number the next new page image takes.
    next_image: u64,
    /// The dots of each page that changed since the last frame was drawn,
    /// in the page's own dots.
    changed: HashMap<PageKey, DeviceBox>,
    /// The pixels of each ReGIS picture that changed since the last frame
    /// was drawn, in the picture's own pixels (REQ-6399): a box for each
    /// part apart from the others, so that two changes far apart (a key
    /// pressed and a meter lit) draw two small boxes, not the one that
    /// holds both (RULE-65).
    regis_changed: HashMap<PageKey, Damage>,
    /// The frame being drawn, and the last one drawn.
    frame: Frame,
    last: Frame,
    /// The surface's size in device pixels.
    width: u16,
    height: u16,
    /// The next frame is drawn whole: the first, or after a resize.
    whole: bool,
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
    /// Not a ReGIS picture's 800 by 480 pixels, two to a byte.
    RegisSize,
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
            surface: Surface::new(0, 0),
            scratch: Scratch::default(),
            glyphs: Glyphs::default(),
            fonts,
            layouts: LayoutContext::new(),
            pages: HashMap::new(),
            regis: HashMap::new(),
            regis_contain: true,
            regis_keyed: Vec::new(),
            stills: HashMap::new(),
            next_image: 0,
            changed: HashMap::new(),
            regis_changed: HashMap::new(),
            frame: Frame::new(DeviceBox::default()),
            last: Frame::default(),
            width: 0,
            height: 0,
            whole: true,
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
            self.surface = Surface::new(width, height);
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
        self.regis_changed.clear();
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
        let drawn = &self.surface.words;
        if age != 1 || words.len() != drawn.len() {
            words.iter_mut().zip(drawn).for_each(|(word, &drawn)| *word = drawn);
            return;
        }
        let width = usize::from(self.width);
        for area in damage.boxes() {
            for row in area.y0..area.y1 {
                let Some(start) = usize::from(row).checked_mul(width).and_then(|start| start.checked_add(usize::from(area.x0))) else { continue };
                let range = start..start.saturating_add(usize::from(area.width()));
                if let (Some(to), Some(from)) = (words.get_mut(range.clone()), drawn.get(range)) {
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
