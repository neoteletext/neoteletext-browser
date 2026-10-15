//! Drawing on the CPU with `vello_cpu`: colours, rounded rectangles, gradients,
//! pages placed with the wall's transforms, and text laid out by Parley.

use std::sync::Arc;

use parley::{FontContext, Layout, LayoutContext, PositionedLayoutItem, StyleProperty};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, Point, Rect, RoundedRect, Shape};
use vello_cpu::peniko::{Gradient, ImageQuality, ImageSampler};
use vello_cpu::{Glyph, Image, ImageSource, Pixmap, RenderContext, Resources};

pub type Colour = AlphaColor<Srgb>;

/// The prototype's dark theme (`src/shared/tokens.css`).
pub const RAISED: Colour = Colour::from_rgb8(0x1b, 0x1e, 0x25);
pub const FIELD: Colour = Colour::from_rgb8(0x0f, 0x11, 0x15);
pub const WELL: Colour = Colour::from_rgb8(0x0c, 0x0e, 0x12);
pub const FRAME: Colour = Colour::from_rgb8(0x11, 0x13, 0x18);
pub const INK: Colour = Colour::from_rgb8(0xe6, 0xe9, 0xef);
pub const MUTED: Colour = Colour::from_rgb8(0x93, 0x99, 0xa6);
pub const LINE: Colour = Colour::from_rgb8(0x2b, 0x2f, 0x38);
pub const LINE_SOFT: Colour = Colour::from_rgb8(0x20, 0x24, 0x2b);
pub const ACCENT: Colour = Colour::from_rgb8(0x8f, 0xa1, 0xff);
pub const MOSAIC: [Colour; 4] = [
    Colour::from_rgb8(0xff, 0x4d, 0x4d),
    Colour::from_rgb8(0xff, 0xd8, 0x4d),
    Colour::from_rgb8(0x4d, 0xff, 0x88),
    Colour::from_rgb8(0x4d, 0xd2, 0xff),
];
/// Where the mosaic's gradient changes colour, as its CSS gradient.
const MOSAIC_STOPS: [f32; 4] = [0.0, 0.34, 0.67, 1.0];
/// How closely curves are flattened into lines, in pixels.
const TOLERANCE: f64 = 0.1;

/// Teletext is blocks: nearest-neighbour sampling keeps them sharp.
pub fn image(pixmap: Arc<Pixmap>) -> Image {
    Image { image: ImageSource::Pixmap(pixmap), sampler: ImageSampler { quality: ImageQuality::Low, ..ImageSampler::default() } }
}

/// Text styles: a size in logical pixels, and whether monospaced or bold.
#[derive(Clone, Copy)]
pub struct Style {
    pub size: f32,
    pub monospace: bool,
    pub bold: bool,
}

pub const LABEL: Style = Style { size: 11.0, monospace: false, bold: false };
pub const SMALL_MONO: Style = Style { size: 11.0, monospace: true, bold: false };
pub const CAPTION: Style = Style { size: 13.5, monospace: false, bold: false };
pub const OPTION: Style = Style { size: 12.0, monospace: false, bold: false };
pub const READOUT: Style = Style { size: 20.0, monospace: true, bold: false };
pub const UNIT: Style = Style { size: 13.0, monospace: true, bold: false };
pub const TITLE: Style = Style { size: 21.0, monospace: false, bold: true };
pub const LIST_TITLE: Style = Style { size: 17.0, monospace: false, bold: true };
pub const DETAIL: Style = Style { size: 12.5, monospace: true, bold: false };

pub struct Painter {
    pub context: RenderContext,
    pub resources: Resources,
    fonts: FontContext,
    layouts: LayoutContext<()>,
    /// Logical pixels to device pixels.
    pub scale: f64,
}

impl Painter {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            context: RenderContext::new(width, height),
            resources: Resources::new(),
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
            scale: 1.0,
        }
    }

    fn base(&self) -> Affine {
        Affine::scale(self.scale)
    }

    /// A rectangle, rounded when `radius` is above zero, in logical pixels.
    pub fn fill(&mut self, rect: Rect, radius: f64, colour: Colour) {
        self.context.set_transform(self.base());
        self.context.set_paint(colour);
        if radius > 0.0 {
            self.context.fill_path(&RoundedRect::from_rect(rect, radius).to_path(TOLERANCE));
        } else {
            self.context.fill_rect(&rect);
        }
    }

    pub fn mosaic(&mut self, rect: Rect, radius: f64) {
        let gradient = Gradient::new_linear(Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y1))
            .with_stops(MOSAIC_STOPS.into_iter().zip(MOSAIC).collect::<Vec<_>>().as_slice());
        self.context.set_transform(self.base());
        self.context.set_paint(gradient);
        self.context.fill_path(&RoundedRect::from_rect(rect, radius).to_path(TOLERANCE));
    }

    /// Draws `image` (of `width` by `height` dots) onto the parallelogram
    /// with these three corners, in logical pixels.
    pub fn image(&mut self, image: &Image, width: f64, height: f64, top_left: Point, top_right: Point, bottom_left: Point) {
        let across = (top_right - top_left) / width;
        let down = (bottom_left - top_left) / height;
        let placement = Affine::new([across.x, across.y, down.x, down.y, top_left.x, top_left.y]);
        self.context.set_transform(self.base() * placement);
        self.context.set_paint(image.clone());
        self.context.fill_rect(&Rect::new(0.0, 0.0, width, height));
    }

    fn layout(&mut self, text: &str, style: Style) -> Layout<()> {
        let mut builder = self.layouts.ranged_builder(&mut self.fonts, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(style.size));
        if style.monospace {
            builder.push_default(StyleProperty::FontFamily(parley::GenericFamily::Monospace.into()));
        }
        if style.bold {
            builder.push_default(StyleProperty::FontWeight(parley::FontWeight::SEMI_BOLD));
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout
    }

    /// The width of `text` in logical pixels.
    pub fn width(&mut self, text: &str, style: Style) -> f64 {
        f64::from(self.layout(text, style).width())
    }

    /// Draws `text` with its top-left corner at a logical position.
    pub fn text(&mut self, x: f64, y: f64, text: &str, style: Style, colour: Colour) {
        let layout = self.layout(text, style);
        let transform = self.base() * Affine::translate((x, y));
        self.context.set_paint(colour);
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                let glyphs = run.positioned_glyphs().map(|glyph| Glyph { id: glyph.id, x: glyph.x, y: glyph.y });
                self.context.set_transform(transform);
                let _ = self.context.glyph_run(&mut self.resources, run.run().font()).font_size(run.run().font_size()).fill_glyphs(glyphs);
            }
        }
    }
}
