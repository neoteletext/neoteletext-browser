//! The interface core's drawing interface on the processor (design section
//! 4.1): `vello_cpu` draws every pixel with no graphics processor, and
//! Parley lays out and shapes text. The interface's own text is in a font
//! shipped inside the program, Roboto or Open Sans (both from Google Fonts,
//! under the SIL Open Font License 1.1, `fonts/`); system fonts fill in
//! scripts those fonts lack.

use std::collections::HashMap;
use std::sync::Arc;

use interface_core::canvas::{Canvas, PAGE_HEIGHT, PAGE_WIDTH, PageKey, Style};
use interface_core::geometry::Rect;
use interface_core::theme::Colour;
use parley::fontique::Blob;
use parley::{FontContext, FontFamilyName, FontWeight, GenericFamily, Layout, LayoutContext, PositionedLayoutItem, StyleProperty};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{self, Affine, BezPath, Point, RoundedRect, Shape};
use vello_cpu::peniko::{Gradient, ImageAlphaType, ImageQuality, ImageSampler};
use vello_cpu::{Glyph, Image, ImageSource, PixelMetadata, Pixmap, RenderContext, Resources};

/// How closely curves are flattened into lines, in pixels.
const TOLERANCE: f64 = 0.1;
const OPAQUE: u8 = u8::MAX;
const RGBA_BYTES: usize = 4;
const PAGE_DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
/// Letters written in lower case are drawn as capitals this much smaller
/// than the true capitals.
const SMALL_CAPITAL: f32 = 0.8;
const ROBOTO: &[u8] = include_bytes!("../fonts/Roboto.ttf");
const OPEN_SANS: &[u8] = include_bytes!("../fonts/OpenSans.ttf");

/// More laid-out strings than one frame draws; past this the cache is
/// emptied and refills with what is on screen.
const LAYOUT_CACHE_LIMIT: usize = 4096;

/// A [`Style`] as a cache key: its size by its bits, and its flags.
type StyleKey = (u32, bool, bool, bool);

fn style_key(style: Style) -> StyleKey {
    (style.size.to_bits(), style.monospace, style.bold, style.caps)
}

/// The shipped font the interface's text is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Roboto,
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

/// `text` in small capitals: every letter upper case, and the byte ranges
/// of the result that were lower case.
fn small_capitals(text: &str) -> (String, Vec<core::ops::Range<usize>>) {
    let mut out = String::with_capacity(text.len());
    let mut small: Vec<core::ops::Range<usize>> = Vec::new();
    for character in text.chars() {
        let start = out.len();
        if character.is_lowercase() {
            out.extend(character.to_uppercase());
            match small.last_mut() {
                Some(range) if range.end == start => range.end = out.len(),
                _ => small.push(start..out.len()),
            }
        } else {
            out.push(character);
        }
    }
    (out, small)
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

/// Teletext is blocks: nearest-neighbour sampling keeps them sharp.
fn image(pixmap: Arc<Pixmap>) -> Image {
    Image { image: ImageSource::Pixmap(pixmap), sampler: ImageSampler { quality: ImageQuality::Low, ..ImageSampler::default() } }
}

fn pixmap(rgba: Vec<u8>) -> Arc<Pixmap> {
    // Every teletext pixel is opaque, which lets the renderer skip blending.
    let metadata = PixelMetadata { may_have_transparency: false, alpha_type: ImageAlphaType::Alpha };
    Arc::new(Pixmap::from_parts(rgba, PAGE_WIDTH, PAGE_HEIGHT, metadata))
}

pub struct Painter {
    context: RenderContext,
    resources: Resources,
    target: Pixmap,
    fonts: FontContext,
    layouts: LayoutContext<()>,
    pages: HashMap<PageKey, Image>,
    /// Text laid out once and reused: the interface draws the same
    /// strings every frame, and measures each before drawing it.
    layouts_made: HashMap<(String, StyleKey), Layout<()>>,
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
            layouts_made: HashMap::new(),
            family,
            scale: 1.0,
        }
    }

    /// Starts a frame of `width` by `height` device pixels, at `scale`
    /// device pixels to each logical pixel.
    pub fn begin(&mut self, width: u16, height: u16, scale: f64) {
        if self.context.width() == width && self.context.height() == height {
            self.context.reset();
        } else {
            self.context.reset_and_resize(width, height);
            self.target = Pixmap::new(width, height);
        }
        self.scale = scale;
    }

    /// Renders the frame and writes it as one 0RGB word a pixel, as window
    /// surfaces take them.
    pub fn finish(&mut self, words: &mut [u32]) {
        self.context.flush();
        self.context.render(&mut self.target, &mut self.resources);
        // Every pixel drawn is opaque, so the premultiplied colour is the
        // colour itself.
        for (word, pixel) in words.iter_mut().zip(self.target.data()) {
            *word = u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b);
        }
    }

    /// Replaces a page's image whole (requirement 9.6), from palette indices.
    ///
    /// # Errors
    /// When the indices are not a full page or one is outside the palette.
    pub fn set_page(&mut self, key: PageKey, palette: &[[u8; 3]], indices: &[u8]) -> Result<(), PageError> {
        if indices.len() != PAGE_DOTS {
            return Err(PageError::Size);
        }
        let mut rgba = Vec::with_capacity(PAGE_DOTS * RGBA_BYTES);
        for &index in indices {
            let [red, green, blue] = *palette.get(usize::from(index)).ok_or(PageError::Index)?;
            rgba.extend_from_slice(&[red, green, blue, OPAQUE]);
        }
        self.pages.insert(key, image(pixmap(rgba)));
        Ok(())
    }

    /// Forgets the images of pages not in `keep`.
    pub fn retain_pages(&mut self, keep: impl Fn(&PageKey) -> bool) {
        self.pages.retain(|key, _| keep(key));
    }

    fn base(&self) -> Affine {
        Affine::scale(self.scale)
    }

    /// The layout of `text` in `style`, made on first use.
    fn layout(&mut self, text: &str, style: Style) -> &Layout<()> {
        let key = (String::from(text), style_key(style));
        if !self.layouts_made.contains_key(&key) {
            if self.layouts_made.len() >= LAYOUT_CACHE_LIMIT {
                self.layouts_made.clear();
            }
            let layout = self.make_layout(text, style);
            self.layouts_made.insert(key.clone(), layout);
        }
        &self.layouts_made[&key]
    }

    fn make_layout(&mut self, text: &str, style: Style) -> Layout<()> {
        let (text, small) = if style.caps { small_capitals(text) } else { (String::from(text), Vec::new()) };
        let mut builder = self.layouts.ranged_builder(&mut self.fonts, &text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(style.size));
        if style.monospace {
            builder.push_default(GenericFamily::Monospace);
        } else {
            builder.push_default(FontFamilyName::named(self.family.name()));
        }
        if style.bold {
            builder.push_default(StyleProperty::FontWeight(FontWeight::BOLD));
        }
        for range in small {
            builder.push(StyleProperty::FontSize(style.size * SMALL_CAPITAL), range);
        }
        let mut layout = builder.build(&text);
        layout.break_all_lines(None);
        layout
    }
}

impl Canvas for Painter {
    fn fill(&mut self, area: Rect, radius: f64, paint: Colour) {
        self.context.set_transform(self.base());
        self.context.set_paint(colour(paint));
        if radius > 0.0 {
            self.context.fill_path(&RoundedRect::from_rect(rect(area), radius).to_path(TOLERANCE));
        } else {
            self.context.fill_rect(&rect(area));
        }
    }

    fn polygon(&mut self, points: &[interface_core::geometry::Point], paint: Colour) {
        let mut path = BezPath::new();
        let mut points = points.iter().map(|&at| point(at));
        let Some(first) = points.next() else { return };
        path.move_to(first);
        for at in points {
            path.line_to(at);
        }
        path.close_path();
        self.context.set_transform(self.base());
        self.context.set_paint(colour(paint));
        self.context.fill_path(&path);
    }

    fn dim(&mut self, area: Rect, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        self.context.set_transform(self.base());
        self.context.set_paint(AlphaColor::<Srgb>::new([0.0, 0.0, 0.0, amount.min(1.0)]));
        self.context.fill_rect(&rect(area));
    }

    fn push_clip(&mut self, area: Rect) {
        self.context.set_transform(self.base());
        self.context.push_clip_layer(&rect(area).to_path(TOLERANCE));
    }

    fn pop_clip(&mut self) {
        self.context.pop_layer();
    }

    fn gradient(&mut self, area: Rect, radius: f64, stops: &[(f32, Colour)]) {
        let stops: Vec<(f32, AlphaColor<Srgb>)> = stops.iter().map(|&(offset, paint)| (offset, colour(paint))).collect();
        let gradient = Gradient::new_linear(Point::new(area.x0, area.y0), Point::new(area.x1, area.y1)).with_stops(stops.as_slice());
        self.context.set_transform(self.base());
        self.context.set_paint(gradient);
        self.context.fill_path(&RoundedRect::from_rect(rect(area), radius).to_path(TOLERANCE));
    }

    fn page(&mut self, key: PageKey, area: Rect) -> bool {
        let Some(page) = self.pages.get(&key) else { return false };
        let (width, height) = (f64::from(PAGE_WIDTH), f64::from(PAGE_HEIGHT));
        let placement = Affine::translate((area.x0, area.y0)) * Affine::scale_non_uniform(area.width() / width, area.height() / height);
        self.context.set_transform(self.base() * placement);
        self.context.set_paint(page.clone());
        self.context.fill_rect(&kurbo::Rect::new(0.0, 0.0, width, height));
        true
    }

    fn text(&mut self, area: Rect, text: &str, style: Style, paint: Colour) {
        // Taken out while drawing, so the context can be borrowed, then put back.
        self.layout(text, style);
        let key = (String::from(text), style_key(style));
        let Some(layout) = self.layouts_made.remove(&key) else { return };
        self.context.set_transform(self.base());
        self.context.push_clip_layer(&rect(area).to_path(TOLERANCE));
        let transform = self.base() * Affine::translate((area.x0, area.y0));
        self.context.set_paint(colour(paint));
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let glyphs = run.positioned_glyphs().map(|glyph| Glyph { id: glyph.id, x: glyph.x, y: glyph.y });
                self.context.set_transform(transform);
                // The run's variation settings carry its weight: the shipped fonts
                // are variable fonts, with weight as an axis.
                let coordinates: Vec<i16> = run.run().normalized_coords().iter().map(|coordinate| coordinate.to_bits()).collect();
                let _ = self
                    .context
                    .glyph_run(&mut self.resources, run.run().font())
                    .font_size(run.run().font_size())
                    .normalized_coords(&coordinates)
                    .fill_glyphs(glyphs);
            }
        }
        self.context.pop_layer();
        self.layouts_made.insert(key, layout);
    }

    fn text_width(&mut self, text: &str, style: Style) -> f64 {
        f64::from(self.layout(text, style).width())
    }

    fn line_height(&mut self, style: Style) -> f64 {
        f64::from(self.layout("X", Style { caps: false, ..style }).height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_capitals_mark_what_was_lower_case() {
        let (text, small) = small_capitals("Page size ß");
        assert_eq!(text, "PAGE SIZE SS");
        assert_eq!(small, [1..4, 5..9, 10..12]);
    }

    #[test]
    fn a_page_with_an_index_outside_its_palette_is_refused() {
        let mut painter = Painter::new(Family::Roboto);
        let key = PageKey { channel: 1, page: 0x100 };
        let mut indices = vec![0; PAGE_DOTS];
        assert_eq!(painter.set_page(key, &[[0, 0, 0]], &indices[1..]), Err(PageError::Size));
        indices[7] = 1;
        assert_eq!(painter.set_page(key, &[[0, 0, 0]], &indices), Err(PageError::Index));
        indices[7] = 0;
        assert_eq!(painter.set_page(key, &[[0, 0, 0]], &indices), Ok(()));
    }
}
