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

mod damage;

pub use damage::{Damage, DeviceBox, MAX_BOXES};

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

use crate::damage::common_ends;

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

/// How far a glyph's ink may reach outside its laid-out box, as a share
/// of the type's size: a stroke past the advance, an accent above the
/// ascent. A quarter of an em is beyond any in the shipped fonts.
const INK_OVERHANG: f64 = 0.25;

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

/// One drawing call, kept to be compared with the last frame's and drawn
/// where the frame changed.
#[derive(Debug, PartialEq)]
enum Command {
    Fill {
        area: Rect,
        radius: f64,
        paint: Colour,
    },
    Polygon {
        points: Vec<interface_core::geometry::Point>,
        paint: Colour,
    },
    Dim {
        area: Rect,
        amount: f32,
    },
    PushClip(Rect),
    PopClip,
    Gradient {
        area: Rect,
        radius: f64,
        stops: Vec<(f32, Colour)>,
    },
    /// A page's image, by the number of the image it was.
    Page {
        key: PageKey,
        area: Rect,
        image: u64,
    },
    Text {
        area: Rect,
        text: String,
        style: Style,
        paint: Colour,
    },
}

/// A call as kept: with the clips it was drawn inside, and the device
/// pixels it can change.
#[derive(Debug)]
struct Recorded {
    command: Command,
    /// The clips around it, an index into [`Frame::clips`].
    clip: usize,
    /// The pixels it can change: its shape's, inside its clips. A clip's
    /// own are its rect's, since what it holds changes only there.
    bounds: DeviceBox,
}

/// A frame's calls.
#[derive(Debug, Default)]
struct Frame {
    calls: Vec<Recorded>,
    /// Every set of clips a call was drawn inside, outermost first, and the
    /// pixels they leave; the first is none, the whole surface.
    clips: Vec<(Vec<Rect>, DeviceBox)>,
    /// While drawing, the clips now around a call, as indices into `clips`.
    stack: Vec<usize>,
}

impl Frame {
    fn new(surface: DeviceBox) -> Self {
        Self { calls: Vec::new(), clips: vec![(Vec::new(), surface)], stack: vec![0] }
    }

    fn top(&self) -> usize {
        self.stack.last().copied().unwrap_or(0)
    }

    fn clip_box(&self) -> DeviceBox {
        self.clips.get(self.top()).map_or_else(DeviceBox::default, |(_, area)| *area)
    }

    fn rects(&self, clip: usize) -> &[Rect] {
        self.clips.get(clip).map_or(&[], |(rects, _)| rects.as_slice())
    }
}

/// A page's image as the painter holds it.
struct PageImage {
    image: Image,
    pixmap: Arc<Pixmap>,
    /// The colour of the screen around it.
    screen: Colour,
    /// Which image it is: a new one is a new number.
    number: u64,
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
    /// How many layouts are kept, against [`LAYOUT_CACHE_LIMIT`].
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
            self.words = vec![0; usize::from(width) * usize::from(height)];
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
            let all = words.len().min(self.words.len());
            words[..all].copy_from_slice(&self.words[..all]);
            return;
        }
        let width = usize::from(self.width);
        for area in damage.boxes() {
            for row in area.y0..area.y1 {
                let start = usize::from(row) * width + usize::from(area.x0);
                let end = start + usize::from(area.width());
                words[start..end].copy_from_slice(&self.words[start..end]);
            }
        }
    }

    /// Renders the frame and writes the whole window into `words`.
    pub fn finish(&mut self, words: &mut [u32]) {
        let damage = self.render();
        self.copy_to(words, &damage, 0);
    }

    /// The boxes holding every call of `frame` that differs from the last
    /// frame's.
    fn damage(&self, frame: &Frame) -> Damage {
        let (old, new) = (&self.last, frame);
        let same = |before: &Recorded, after: &Recorded| before.command == after.command && old.rects(before.clip) == new.rects(after.clip);
        let (prefix, suffix) = common_ends(&old.calls, &new.calls, same);
        let before = &old.calls[prefix..old.calls.len() - suffix];
        let after = &new.calls[prefix..new.calls.len() - suffix];
        let mut damage = Damage::default();
        if before.len() == after.len() {
            // Call by call: where the two are the same, the same is drawn at
            // that place in both frames, as in the common ends.
            for (was, now) in before.iter().zip(after).filter(|(was, now)| !same(was, now)) {
                if let Some(dots) = self.narrowed(old, was, new, now) {
                    damage.add(dots);
                } else {
                    damage.add(was.bounds);
                    damage.add(now.bounds);
                }
            }
        } else {
            for call in before.iter().chain(after) {
                damage.add(call.bounds);
            }
        }
        damage
    }

    /// For a page whose image alone changed, where it is drawn, inside the
    /// same clips, the pixels of the dots that changed.
    fn narrowed(&self, old: &Frame, was: &Recorded, new: &Frame, now: &Recorded) -> Option<DeviceBox> {
        let (Command::Page { key, area, .. }, Command::Page { key: key_now, area: area_now, .. }) = (&was.command, &now.command) else { return None };
        if key != key_now || area != area_now || old.rects(was.clip) != new.rects(now.clip) {
            return None;
        }
        let dots = self.changed.get(key)?;
        let (across, down) = (area.width() / f64::from(PAGE_WIDTH), area.height() / f64::from(PAGE_HEIGHT));
        let changed = Rect {
            x0: area.x0 + f64::from(dots.x0) * across,
            y0: area.y0 + f64::from(dots.y0) * down,
            x1: area.x0 + f64::from(dots.x1) * across,
            y1: area.y0 + f64::from(dots.y1) * down,
        };
        let clip = new.clips.get(now.clip).map_or_else(DeviceBox::default, |(_, area)| *area);
        Some(DeviceBox::covering(changed, self.scale, self.surface()).intersect(clip))
    }

    /// Draws `damage` from `frame`'s calls, those that reach it, and writes
    /// it into the window's copy.
    fn draw_box(&mut self, frame: &Frame, damage: DeviceBox) {
        let (width, height) = (damage.width(), damage.height());
        if self.context.width() == width && self.context.height() == height {
            self.context.reset();
        } else {
            self.context.reset_and_resize(width, height);
            self.target = Pixmap::new(width, height);
        }
        self.origin = (f64::from(damage.x0), f64::from(damage.y0));
        for call in &frame.calls {
            match &call.command {
                Command::PushClip(area) => self.draw_push_clip(*area),
                Command::PopClip => self.context.pop_layer(),
                _ if !call.bounds.meets(damage) => {}
                Command::Fill { area, radius, paint } => self.draw_fill(*area, *radius, *paint),
                Command::Polygon { points, paint } => self.draw_polygon(points, *paint),
                Command::Dim { area, amount } => self.draw_dim(*area, *amount),
                Command::Gradient { area, radius, stops } => self.draw_gradient(*area, *radius, stops),
                Command::Page { key, area, .. } => self.draw_page(*key, *area),
                Command::Text { area, text, style, paint } => self.draw_text(*area, text, *style, *paint),
            }
        }
        self.context.flush();
        self.context.render(&mut self.target, &mut self.resources);
        let (full, part) = (usize::from(self.width), usize::from(width));
        for (row, pixels) in (usize::from(damage.y0)..).zip(self.target.data().chunks_exact(part.max(1))) {
            let start = row * full + usize::from(damage.x0);
            let Some(words) = self.words.get_mut(start..start + part) else { break };
            // Every pixel drawn is opaque, so the premultiplied colour is
            // the colour itself.
            for (word, pixel) in words.iter_mut().zip(pixels) {
                *word = u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b);
            }
        }
    }

    /// Keeps a painting call, with the pixels it can change.
    fn record(&mut self, command: Command, shape: Rect) {
        let bounds = DeviceBox::covering(shape, self.scale, self.surface()).intersect(self.frame.clip_box());
        let clip = self.frame.top();
        self.frame.calls.push(Recorded { command, clip, bounds });
    }

    /// Replaces a page's image whole (requirement 9.6), from palette indices.
    ///
    /// # Errors
    /// When the indices are not a full page or one is outside the palette.
    pub fn set_page(&mut self, key: PageKey, palette: &[[u8; 3]], screen: u8, indices: &[u8]) -> Result<(), PageError> {
        if indices.len() != PAGE_DOTS {
            return Err(PageError::Size);
        }
        let [red, green, blue] = *palette.get(usize::from(screen)).ok_or(PageError::Index)?;
        let screen = Colour { red, green, blue };
        let mut rgba = Vec::with_capacity(PAGE_DOTS * RGBA_BYTES);
        for &index in indices {
            let [red, green, blue] = *palette.get(usize::from(index)).ok_or(PageError::Index)?;
            rgba.extend_from_slice(&[red, green, blue, OPAQUE]);
        }
        let held = self.pages.get(&key);
        if held.is_some_and(|held| held.screen != screen) {
            // The screen colour is drawn around the page, by other calls,
            // which differ in their own right; the image is whole again.
            self.changed.insert(key, DeviceBox::whole(PAGE_WIDTH, PAGE_HEIGHT));
        } else {
            let dots = held.map_or(DeviceBox::whole(PAGE_WIDTH, PAGE_HEIGHT), |held| changed_dots(&held.pixmap, &rgba));
            if dots.is_empty() {
                return Ok(());
            }
            let since = self.changed.get(&key).copied().unwrap_or_default();
            self.changed.insert(key, since.union(dots));
        }
        let pixmap = pixmap(rgba);
        let number = self.next_image;
        self.next_image = number.wrapping_add(1);
        self.pages.insert(key, PageImage { image: image(Arc::clone(&pixmap)), pixmap, screen, number });
        Ok(())
    }

    /// Forgets the images of pages not in `keep`.
    pub fn retain_pages(&mut self, keep: impl Fn(&PageKey) -> bool) {
        self.pages.retain(|key, _| keep(key));
    }

    /// Logical pixels to the damaged box's device pixels.
    fn base(&self) -> Affine {
        Affine::translate((-self.origin.0, -self.origin.1)) * Affine::scale(self.scale)
    }

    /// The layout of `text` in `style`, made on first use.
    fn layout(&mut self, text: &str, style: Style) -> Option<&Layout<()>> {
        let key = style_key(style);
        if !self.layouts_made.get(&key).is_some_and(|texts| texts.contains_key(text)) {
            if self.layouts_kept >= LAYOUT_CACHE_LIMIT {
                self.layouts_made.clear();
                self.layouts_kept = 0;
            }
            let layout = self.make_layout(text, style);
            self.layouts_made.entry(key).or_default().insert(String::from(text), layout);
            self.layouts_kept = self.layouts_kept.saturating_add(1);
        }
        self.layouts_made.get(&key)?.get(text)
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

/// The box of dots whose colour differs between a held image and a new
/// one's red, green, blue and alpha bytes.
fn changed_dots(held: &Pixmap, rgba: &[u8]) -> DeviceBox {
    let mut found = DeviceBox::default();
    let width = usize::from(PAGE_WIDTH);
    let (dots, _) = rgba.as_chunks::<RGBA_BYTES>();
    for (index, (pixel, &[red, green, blue, _])) in held.data().iter().zip(dots).enumerate() {
        if [pixel.r, pixel.g, pixel.b] != [red, green, blue] {
            let (Ok(x), Ok(y)) = (u16::try_from(index % width), u16::try_from(index / width)) else { continue };
            found = found.union(DeviceBox { x0: x, y0: y, x1: x.saturating_add(1), y1: y.saturating_add(1) });
        }
    }
    found
}

/// The box around `points`.
fn around(points: &[interface_core::geometry::Point]) -> Rect {
    let mut found = Rect { x0: f64::INFINITY, y0: f64::INFINITY, x1: f64::NEG_INFINITY, y1: f64::NEG_INFINITY };
    for at in points {
        found = Rect { x0: found.x0.min(at.x), y0: found.y0.min(at.y), x1: found.x1.max(at.x), y1: found.y1.max(at.y) };
    }
    if points.is_empty() { Rect::default() } else { found }
}

/// The calls as `vello_cpu` draws them, into the damaged box.
impl Painter {
    fn draw_fill(&mut self, area: Rect, radius: f64, paint: Colour) {
        self.context.set_transform(self.base());
        self.context.set_paint(colour(paint));
        if radius > 0.0 {
            self.context.fill_path(&RoundedRect::from_rect(rect(area), radius).to_path(TOLERANCE));
        } else {
            self.context.fill_rect(&rect(area));
        }
    }

    fn draw_polygon(&mut self, points: &[interface_core::geometry::Point], paint: Colour) {
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

    fn draw_dim(&mut self, area: Rect, amount: f32) {
        self.context.set_transform(self.base());
        self.context.set_paint(AlphaColor::<Srgb>::new([0.0, 0.0, 0.0, amount.min(1.0)]));
        self.context.fill_rect(&rect(area));
    }

    fn draw_push_clip(&mut self, area: Rect) {
        self.context.set_transform(self.base());
        self.context.push_clip_layer(&rect(area).to_path(TOLERANCE));
    }

    fn draw_gradient(&mut self, area: Rect, radius: f64, stops: &[(f32, Colour)]) {
        let stops: Vec<(f32, AlphaColor<Srgb>)> = stops.iter().map(|&(offset, paint)| (offset, colour(paint))).collect();
        let gradient = Gradient::new_linear(Point::new(area.x0, area.y0), Point::new(area.x1, area.y1)).with_stops(stops.as_slice());
        self.context.set_transform(self.base());
        self.context.set_paint(gradient);
        self.context.fill_path(&RoundedRect::from_rect(rect(area), radius).to_path(TOLERANCE));
    }

    fn draw_page(&mut self, key: PageKey, area: Rect) {
        let Some(page) = self.pages.get(&key) else { return };
        let (width, height) = (f64::from(PAGE_WIDTH), f64::from(PAGE_HEIGHT));
        let placement = Affine::translate((area.x0, area.y0)) * Affine::scale_non_uniform(area.width() / width, area.height() / height);
        let image = page.image.clone();
        self.context.set_transform(self.base() * placement);
        self.context.set_paint(image);
        self.context.fill_rect(&kurbo::Rect::new(0.0, 0.0, width, height));
    }

    fn draw_text(&mut self, area: Rect, text: &str, style: Style, paint: Colour) {
        if self.layout(text, style).is_none() {
            return;
        }
        // The layout is borrowed from the cache while the context draws it:
        // the fields are borrowed apart.
        let base = self.base();
        let Self { context, resources, layouts_made, .. } = self;
        let Some(layout) = layouts_made.get(&style_key(style)).and_then(|texts| texts.get(text)) else { return };
        context.set_transform(base);
        context.push_clip_layer(&rect(area).to_path(TOLERANCE));
        let transform = base * Affine::translate((area.x0, area.y0));
        context.set_paint(colour(paint));
        let mut coordinates: Vec<i16> = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let glyphs = run.positioned_glyphs().map(|glyph| Glyph { id: glyph.id, x: glyph.x, y: glyph.y });
                context.set_transform(transform);
                // The run's variation settings carry its weight: the shipped fonts
                // are variable fonts, with weight as an axis.
                coordinates.clear();
                coordinates.extend(run.run().normalized_coords().iter().map(|coordinate| coordinate.to_bits()));
                let _ = context.glyph_run(resources, run.run().font()).font_size(run.run().font_size()).normalized_coords(&coordinates).fill_glyphs(glyphs);
            }
        }
        context.pop_layer();
    }
}

/// Each call is kept, to be drawn by [`Painter::render`] where the frame
/// changed.
impl Canvas for Painter {
    fn fill(&mut self, area: Rect, radius: f64, paint: Colour) {
        self.record(Command::Fill { area, radius, paint }, area);
    }

    fn polygon(&mut self, points: &[interface_core::geometry::Point], paint: Colour) {
        if points.is_empty() {
            return;
        }
        self.record(Command::Polygon { points: points.to_vec(), paint }, around(points));
    }

    fn dim(&mut self, area: Rect, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        self.record(Command::Dim { area, amount }, area);
    }

    fn push_clip(&mut self, area: Rect) {
        let parent = self.frame.top();
        let (mut rects, outer) = self.frame.clips.get(parent).cloned().unwrap_or_default();
        rects.push(area);
        let inside = DeviceBox::covering(area, self.scale, self.surface()).intersect(outer);
        let clip = self.frame.clips.len();
        self.frame.clips.push((rects, inside));
        // A clip changes what it holds only inside itself.
        self.frame.calls.push(Recorded { command: Command::PushClip(area), clip: parent, bounds: inside });
        self.frame.stack.push(clip);
    }

    fn pop_clip(&mut self) {
        if self.frame.stack.len() <= 1 {
            return;
        }
        let bounds = self.frame.clip_box();
        self.frame.stack.pop();
        let clip = self.frame.top();
        self.frame.calls.push(Recorded { command: Command::PopClip, clip, bounds });
    }

    fn gradient(&mut self, area: Rect, radius: f64, stops: &[(f32, Colour)]) {
        self.record(Command::Gradient { area, radius, stops: stops.to_vec() }, area);
    }

    fn screen(&mut self, key: PageKey) -> Option<Colour> {
        self.pages.get(&key).map(|page| page.screen)
    }

    fn page(&mut self, key: PageKey, area: Rect) -> bool {
        let Some(page) = self.pages.get(&key) else { return false };
        let image = page.number;
        self.record(Command::Page { key, area, image }, area);
        true
    }

    fn text(&mut self, area: Rect, text: &str, style: Style, paint: Colour) {
        // Text is drawn from the area's corner and cut at its edges; the
        // area is often far larger than the text, so the text's own box is
        // what it can change.
        let Some((width, height)) = self.layout(text, style).map(|layout| (f64::from(layout.width()), f64::from(layout.height()))) else { return };
        let over = f64::from(style.size) * INK_OVERHANG;
        let ink = Rect { x0: area.x0 - over, y0: area.y0 - over, x1: (area.x0 + width + over).min(area.x1), y1: (area.y0 + height + over).min(area.y1) };
        let shape = Rect { x0: ink.x0.max(area.x0), y0: ink.y0.max(area.y0), ..ink };
        self.record(Command::Text { area, text: text.to_owned(), style, paint }, shape);
    }

    fn text_width(&mut self, text: &str, style: Style) -> f64 {
        self.layout(text, style).map_or(0.0, |layout| f64::from(layout.width()))
    }

    fn line_height(&mut self, style: Style) -> f64 {
        self.layout("X", Style { caps: false, ..style }).map_or(0.0, |layout| f64::from(layout.height()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use decoder::Engine;
    use interface_core::browser::{Browser, Frame as WindowFrame, TabInfo};
    use interface_core::channel::{Channel, PageInfo, Status, page_index};

    const WIDTH: u16 = 720;
    const HEIGHT: u16 = 450;
    const CHANNEL: u32 = 3;
    /// Long after any transition, in milliseconds.
    const SETTLED: u64 = 10_000;
    const FRONT: PageKey = PageKey { channel: CHANNEL, page: 0x100 };

    /// A sample page as the engine renders it.
    struct Sample {
        page: u16,
        title: String,
        palette: Vec<[u8; 3]>,
        screen: u8,
        indices: Vec<u8>,
    }

    /// A channel of sample pages, decoded by the teletext engine.
    fn pages() -> Vec<Sample> {
        let mut engine = Engine::new();
        let mut arrived = engine.feed(&decoder::sample::tti(&[0x100, 0x101, 0x102, 0x103], &["Headlines", "World", "Sport", "Weather"]));
        arrived.extend(engine.finish());
        arrived
            .into_iter()
            .map(|(page, title)| {
                let rendered = engine.render(page).unwrap();
                Sample { page, title, palette: rendered.palette.clone(), screen: rendered.screen, indices: rendered.indices.clone() }
            })
            .collect()
    }

    fn browser(painter: &mut Painter, pages: &[Sample]) -> Browser {
        let mut channel = Channel::empty();
        channel.channel = CHANNEL;
        for sample in pages {
            channel.pages[page_index(sample.page).unwrap()] = Some(PageInfo { title: sample.title.clone(), status: Status::Loaded });
            painter.set_page(PageKey { channel: CHANNEL, page: sample.page }, &sample.palette, sample.screen, &sample.indices).unwrap();
        }
        let mut browser = Browser::new(WindowFrame { left_inset: 0.0, window_buttons: true });
        browser.set_tabs(vec![TabInfo { title: "sample".into(), colour: 0, channel: CHANNEL, muted: false, private: false }], 0);
        browser.set_channel(channel);
        browser
    }

    /// The window as `browser` draws it at `now`, by `painter`.
    fn frame(painter: &mut Painter, browser: &mut Browser, now: u64) -> (Vec<DeviceBox>, Vec<u32>) {
        painter.begin(WIDTH, HEIGHT, 1.0);
        browser.draw(painter, f64::from(WIDTH), f64::from(HEIGHT), now);
        let damage = painter.render();
        let mut words = vec![0; usize::from(WIDTH) * usize::from(HEIGHT)];
        painter.copy_to(&mut words, &damage, 0);
        (damage.boxes().to_vec(), words)
    }

    fn covered(boxes: &[DeviceBox]) -> u32 {
        boxes.iter().map(|area| area.area()).sum()
    }

    /// The same window drawn whole, every pixel, by another painter.
    fn fresh(pages: &[Sample], now: u64, change: impl Fn(&mut Painter, &mut Browser)) -> Vec<u32> {
        let mut painter = Painter::new(Family::Roboto);
        let mut browser = browser(&mut painter, pages);
        // The first frame reports which pages are on screen.
        frame(&mut painter, &mut browser, SETTLED);
        change(&mut painter, &mut browser);
        painter.redraw_whole();
        frame(&mut painter, &mut browser, now).1
    }

    fn differing(left: &[u32], right: &[u32]) -> usize {
        left.iter().zip(right).filter(|(a, b)| a != b).count()
    }

    #[test]
    fn a_frame_that_changed_nothing_is_not_drawn() {
        let pages = pages();
        let mut painter = Painter::new(Family::Roboto);
        let mut browser = browser(&mut painter, &pages);
        assert_eq!(frame(&mut painter, &mut browser, SETTLED).0, [DeviceBox::whole(WIDTH, HEIGHT)], "the first frame is whole");
        frame(&mut painter, &mut browser, SETTLED);
        assert_eq!(frame(&mut painter, &mut browser, SETTLED).0, []);
        painter.redraw_whole();
        assert_eq!(frame(&mut painter, &mut browser, SETTLED).0, [DeviceBox::whole(WIDTH, HEIGHT)], "asked to, every pixel");
    }

    #[test]
    fn a_page_whose_dots_changed_redraws_only_them_and_matches_a_whole_frame() {
        let pages = pages();
        let mut painter = Painter::new(Family::Roboto);
        let mut browser = browser(&mut painter, &pages);
        frame(&mut painter, &mut browser, SETTLED);
        frame(&mut painter, &mut browser, SETTLED);
        // As a clock's seconds tick in the header: a few dots of one row.
        let change = |painter: &mut Painter, _: &mut Browser| {
            let front = &pages[0];
            let mut ticked = front.indices.clone();
            let row = usize::from(PAGE_WIDTH) * 5;
            for dot in &mut ticked[row + 400..row + 412] {
                *dot = if *dot == 7 { 0 } else { 7 };
            }
            painter.set_page(FRONT, &front.palette, front.screen, &ticked).unwrap();
        };
        change(&mut painter, &mut browser);
        let (damage, words) = frame(&mut painter, &mut browser, SETTLED);
        assert!(!damage.is_empty() && covered(&damage) < 200, "only the dots that changed: {damage:?}");
        assert_eq!(differing(&words, &fresh(&pages, SETTLED, change)), 0);
    }

    #[test]
    fn a_hover_redraws_only_where_it_shows_and_matches_a_whole_frame() {
        let pages = pages();
        let mut painter = Painter::new(Family::Roboto);
        let mut browser = browser(&mut painter, &pages);
        frame(&mut painter, &mut browser, SETTLED);
        let shown: Vec<_> = browser.wall().shown().collect();
        assert!(!shown.is_empty(), "pages on the wall");
        let (_, rect) = shown[shown.len() - 1];
        let (x, y) = (f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1));
        browser.pointer_moved(x, y, SETTLED);
        // The hover fades in; long after, it shows.
        let (damage, words) = frame(&mut painter, &mut browser, SETTLED * 2);
        assert!(!damage.is_empty() && covered(&damage) < u32::from(WIDTH) * u32::from(HEIGHT) / 2, "a part of the window: {damage:?}");
        let expected = fresh(&pages, SETTLED * 2, |_, browser| {
            browser.pointer_moved(x, y, SETTLED);
        });
        assert_eq!(differing(&words, &expected), 0);
    }

    #[test]
    fn a_surface_that_holds_the_last_frame_takes_only_the_damage() {
        let mut painter = Painter::new(Family::Roboto);
        painter.begin(4, 3, 1.0);
        painter.fill(Rect { x0: 0.0, y0: 0.0, x1: 4.0, y1: 3.0 }, 0.0, Colour { red: 1, green: 2, blue: 3 });
        let whole = painter.render();
        let mut words = vec![0; 12];
        painter.copy_to(&mut words, &whole, 1);
        assert!(words.iter().all(|&word| word == 0x0001_0203));
        let mut held = vec![9; 12];
        let mut damage = Damage::default();
        damage.add(DeviceBox { x0: 1, y0: 1, x1: 3, y1: 2 });
        painter.copy_to(&mut held, &damage, 1);
        assert_eq!(held, [9, 9, 9, 9, 9, 0x0001_0203, 0x0001_0203, 9, 9, 9, 9, 9]);
        painter.copy_to(&mut held, &damage, 0);
        assert_eq!(held, words, "a fresh surface takes every pixel");
    }

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
        assert_eq!(painter.set_page(key, &[[0, 0, 0]], 0, &indices[1..]), Err(PageError::Size));
        assert_eq!(painter.set_page(key, &[[0, 0, 0]], 1, &indices), Err(PageError::Index), "the screen colour");
        indices[7] = 1;
        assert_eq!(painter.set_page(key, &[[0, 0, 0]], 0, &indices), Err(PageError::Index));
        indices[7] = 0;
        assert_eq!(painter.set_page(key, &[[9, 8, 7]], 0, &indices), Ok(()));
        assert_eq!(painter.screen(key), Some(Colour { red: 9, green: 8, blue: 7 }));
    }
}
