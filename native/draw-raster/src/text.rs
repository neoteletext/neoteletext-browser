//! Text: laid out and shaped by Parley, cached by its string and style,
//! with small capitals where the style asks for them, and drawn.

use interface_core::canvas::Style;
use interface_core::geometry::Rect;
use interface_core::theme::Colour;
use parley::{FontFamilyName, FontWeight, GenericFamily, Layout, PositionedLayoutItem, StyleProperty};
use skrifa::instance::NormalizedCoord;

use crate::Painter;
use crate::raster::coverage::{self, Line};
use crate::raster::paint::opaque;
use crate::raster::surface::Source;

/// Letters written in lower case are drawn as capitals this much smaller
/// than the true capitals.
const SMALL_CAPITAL: f32 = 0.8;
/// How far a glyph's ink may reach outside its laid-out box, as a share
/// of the type's size: a stroke past the advance, an accent above the
/// ascent. A quarter of an em is beyond any in the shipped fonts.
pub(crate) const INK_OVERHANG: f64 = 0.25;
/// More laid-out strings than one frame draws; past this the cache is
/// emptied and refills with what is on screen.
pub(crate) const LAYOUT_CACHE_LIMIT: usize = 4096;

/// A [`Style`] as a cache key: its size by its bits, and its flags.
pub(crate) type StyleKey = (u32, bool, bool, bool);

fn style_key(style: Style) -> StyleKey {
    (style.size.to_bits(), style.monospace, style.bold, style.caps)
}

/// `text` in small capitals: every letter upper case, and the byte ranges
/// of the result that were lower case.
pub(crate) fn small_capitals(text: &str) -> (String, Vec<core::ops::Range<usize>>) {
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

impl Painter {
    /// The layout of `text` in `style`, made on first use.
    pub(crate) fn layout(&mut self, text: &str, style: Style) -> Option<&Layout<()>> {
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

    /// Draws `text` from `area`'s top-left corner, cut at its edges: each
    /// glyph's outline filled, its origin where Parley placed it and, when
    /// hinted, its baseline on a whole device pixel.
    pub(crate) fn draw_text(&mut self, area: Rect, text: &str, style: Style, paint: Colour) {
        if self.layout(text, style).is_none() {
            return;
        }
        let (scale, clip) = (self.scale, self.device(area));
        // The layout is borrowed from the cache while the glyphs are drawn:
        // the fields are borrowed apart.
        let Self { surface, scratch, layouts_made, glyphs, .. } = self;
        let Some(layout) = layouts_made.get(&style_key(style)).and_then(|texts| texts.get(text)) else { return };
        surface.push_clip(clip.0, clip.1, clip.2, clip.3);
        let colour = Source::Solid(opaque(paint));
        let mut coordinates: Vec<NormalizedCoord> = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else { continue };
                // The run's variation settings carry its weight: the shipped
                // fonts are variable fonts, with weight as an axis.
                coordinates.clear();
                coordinates.extend(run.run().normalized_coords().iter().map(|coordinate| NormalizedCoord::from_bits(coordinate.to_bits())));
                let Some(mut face) = glyphs.face(run.run().font(), run.run().font_size() * scale_f32(scale), &coordinates) else { continue };
                let hinted = face.hinted();
                for glyph in run.positioned_glyphs() {
                    let x = (area.x0 + f64::from(glyph.x)) * scale;
                    let y = (area.y0 + f64::from(glyph.y)) * scale;
                    let y = if hinted { y.round() } else { y };
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "a device coordinate; past f32's range it saturates to infinity, and an edge not finite is dropped"
                    )]
                    let (x, y) = (x as f32, y as f32);
                    scratch.lines.clear();
                    scratch.lines.extend(face.glyph(glyph.id).iter().map(|edge| Line { x0: edge.x0 + x, y0: edge.y0 + y, x1: edge.x1 + x, y1: edge.y1 + y }));
                    let reach = coverage::bounds(&scratch.lines, surface.scissor());
                    scratch.coverage.fill(&scratch.lines, reach, |row, column, length, mask| surface.span(row, column, length, colour, mask));
                }
            }
        }
        surface.pop_clip();
    }
}

/// A scale as the font size's type.
fn scale_f32(scale: f64) -> f32 {
    #[expect(clippy::cast_possible_truncation, reason = "a display scale, a small number")]
    let scale = scale as f32;
    scale
}
