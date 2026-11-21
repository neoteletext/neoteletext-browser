//! Glyph outlines, read from the font by skrifa, hinted vertically at the
//! size drawn, flattened into edges and kept for reuse.
//!
//! Hinting fits each outline's horizontal features (baselines, the tops
//! of lower-case letters, stems' ends) to whole device pixels, so text is
//! crisp at small sizes, and leaves positions across alone, so laid-out
//! widths are kept: skrifa's automatic hinter, or the font's own
//! TrueType instructions where it has them, in its smooth mode for
//! displays without a fixed subpixel order. Each glyph's baseline is put
//! on a whole pixel to match.

use std::collections::HashMap;

use parley::FontData;
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, SmoothMode, Target};
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::raster::coverage::Line;
use crate::raster::path::Edges;

/// Hinting that moves outlines only up and down, keeping their advances
/// (skrifa's `Target::Smooth`, its light mode for displays of any
/// subpixel order).
const HINTING: HintingOptions = HintingOptions {
    engine: Engine::AutoFallback,
    target: Target::Smooth { mode: SmoothMode::Lcd, symmetric_rendering: false, preserve_linear_metrics: true },
};
/// More glyphs than the interface's text shows at once in every size it
/// uses: when a glyph not yet held would pass this, every face's glyphs
/// are dropped (their hinting kept) and refill with what is drawn, so the
/// store never holds more, however long one face goes on being used.
pub(crate) const GLYPH_CACHE_LIMIT: usize = 8192;
/// More faces (a font at a size and weight) than the interface draws in;
/// past this one face is dropped to make room ([`Glyphs::face`]).
pub(crate) const FACE_CACHE_LIMIT: usize = 64;

/// A font at one size and one setting of its variation axes, by the
/// font's data, its index in its file, the size's bits and the axes'
/// coordinates.
type FaceKey = (u64, u32, u32, Vec<i16>);

/// A font at one size, with its hinting and the glyphs drawn in it.
struct Face {
    key: FaceKey,
    hinting: Option<HintingInstance>,
    /// Each glyph's edges, in device pixels from its origin, y down.
    glyphs: HashMap<u32, Vec<Line>>,
    /// The frame it was last used in, and when, by the count of faces
    /// asked for.
    frame: u64,
    used: u64,
}

/// The faces and glyphs drawn so far.
#[derive(Default)]
pub(crate) struct Glyphs {
    /// At most [`FACE_CACHE_LIMIT`], so found by looking through them.
    faces: Vec<Face>,
    /// Glyphs held in every face, at most [`GLYPH_CACHE_LIMIT`].
    kept: usize,
    /// The frame being drawn, counted by [`Glyphs::next_frame`].
    frame: u64,
    /// Faces asked for so far, to order their uses.
    asked: u64,
    /// Faces made so far, so a test can count rebuilds.
    #[cfg(test)]
    made: usize,
}

/// A glyph's outline as edges, y turned downward.
struct Pen<'a> {
    edges: Edges<'a>,
}

impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.edges.move_to((f64::from(x), -f64::from(y)));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.edges.line_to((f64::from(x), -f64::from(y)));
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
        self.edges.quad_to((f64::from(control_x), -f64::from(control_y)), (f64::from(x), -f64::from(y)));
    }

    fn curve_to(&mut self, first_x: f32, first_y: f32, second_x: f32, second_y: f32, x: f32, y: f32) {
        self.edges.cubic_to((f64::from(first_x), -f64::from(first_y)), (f64::from(second_x), -f64::from(second_y)), (f64::from(x), -f64::from(y)));
    }

    fn close(&mut self) {
        self.edges.close();
    }
}

/// One run's font at its size, ready to give glyphs.
pub(crate) struct RunFace<'a> {
    faces: &'a mut [Face],
    index: usize,
    font: FontRef<'a>,
    size: f32,
    coordinates: &'a [NormalizedCoord],
    kept: &'a mut usize,
}

impl Glyphs {
    /// A new frame begins: faces used in it and the frame before are the
    /// ones its text is drawn in.
    pub(crate) fn next_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// `font` at `size` device pixels to the em, its axes at `coordinates`;
    /// `None` when its data is not a font skrifa reads.
    pub(crate) fn face<'a>(&'a mut self, font: &'a FontData, size: f32, coordinates: &'a [NormalizedCoord]) -> Option<RunFace<'a>> {
        let reference = FontRef::from_index(font.data.data(), font.index).ok()?;
        let key: FaceKey = (font.data.id(), font.index, size.to_bits(), coordinates.iter().map(|coordinate| coordinate.to_bits()).collect());
        self.asked = self.asked.wrapping_add(1);
        let index = if let Some(index) = self.faces.iter().position(|face| face.key == key) {
            index
        } else {
            if self.faces.len() >= FACE_CACHE_LIMIT
                && let Some(evicted) = self.evictable()
            {
                let face = self.faces.swap_remove(evicted);
                self.kept = self.kept.saturating_sub(face.glyphs.len());
            }
            let hinting = HintingInstance::new(&reference.outline_glyphs(), Size::new(size), LocationRef::new(coordinates), HINTING).ok();
            self.faces.push(Face { key, hinting, glyphs: HashMap::new(), frame: self.frame, used: self.asked });
            #[cfg(test)]
            {
                self.made = self.made.saturating_add(1);
            }
            self.faces.len().saturating_sub(1)
        };
        let face = self.faces.get_mut(index)?;
        face.frame = self.frame;
        face.used = self.asked;
        Some(RunFace { faces: &mut self.faces, index, font: reference, size, coordinates, kept: &mut self.kept })
    }

    /// The face to drop for a new one: the least recently used of those
    /// not used in this frame or the one before; when every face was, the
    /// most recently used. Dropping the least recently used then would
    /// drop, on each draw of text that needs more faces than the limit,
    /// the very face asked for next, so every face would be made again on
    /// every frame; dropping the latest makes again only the faces past
    /// the limit.
    fn evictable(&self) -> Option<usize> {
        let current = |face: &Face| self.frame.wrapping_sub(face.frame) <= 1;
        let stale = self.faces.iter().enumerate().filter(|(_, face)| !current(face)).min_by_key(|(_, face)| face.used);
        stale.or_else(|| self.faces.iter().enumerate().max_by_key(|(_, face)| face.used)).map(|(index, _)| index)
    }

    /// The glyphs and faces held.
    #[cfg(test)]
    pub(crate) fn held(&self) -> (usize, usize) {
        (self.faces.iter().map(|face| face.glyphs.len()).sum(), self.faces.len())
    }
}

impl RunFace<'_> {
    /// Whether the face is hinted, so that glyphs sit on whole pixels down.
    pub(crate) fn hinted(&self) -> bool {
        self.faces.get(self.index).is_some_and(|face| face.hinting.is_some())
    }

    /// The edges of glyph `id`, from its origin; none for a glyph with no
    /// outline.
    pub(crate) fn glyph(&mut self, id: u32) -> &[Line] {
        let Self { faces, index, font, size, coordinates, kept } = self;
        let held = faces.get(*index).is_some_and(|face| face.glyphs.contains_key(&id));
        if !held && **kept >= GLYPH_CACHE_LIMIT {
            // Dropped rather than cleared, so their tables' room goes too.
            for face in faces.iter_mut() {
                face.glyphs = HashMap::new();
            }
            **kept = 0;
        }
        let Some(Face { hinting, glyphs, .. }) = faces.get_mut(*index) else { return &[] };
        glyphs.entry(id).or_insert_with(|| {
            let mut lines = Vec::new();
            if let Some(outline) = font.outline_glyphs().get(GlyphId::new(id)) {
                let settings = hinting
                    .as_ref()
                    .map_or_else(|| DrawSettings::unhinted(Size::new(*size), LocationRef::new(coordinates)), |hinting| DrawSettings::hinted(hinting, false));
                let mut pen = Pen { edges: Edges::new(&mut lines) };
                let drawn = outline.draw(settings, &mut pen).is_ok();
                pen.edges.close();
                if !drawn {
                    lines.clear();
                }
            }
            **kept = kept.saturating_add(1);
            lines
        })
    }
}

#[cfg(test)]
mod tests;
