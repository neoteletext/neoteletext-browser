//! The surface a damaged box is drawn into: the window's words, the box,
//! and the clips around what is drawn, with source-over compositing of
//! premultiplied colour under a coverage mask.
//!
//! Compositing is Porter and Duff's source-over (Porter and Duff,
//! "Compositing Digital Images", SIGGRAPH 1984, section 4), on premultiplied
//! eight-bit channels: `result = source × mask + destination × (1 −
//! source alpha × mask)`, each product divided by 255 as `(x + 255) >> 8`.
//!
//! A clip is a layer: what is drawn inside it is composited into it, and
//! the layer into what lies beneath through the clip's own coverage. Where
//! the clip covers a pixel whole, drawing into the layer and then the layer
//! into what lies beneath is drawing straight into what lies beneath, so
//! only the pixels the clip's edge covers in part are held apart: at most
//! its first and last row and column.

use crate::DeviceBox;
use crate::raster::coverage::alpha;

/// The channels of one premultiplied pixel: red, green, blue, alpha.
pub(crate) type Rgba = [u8; 4];

const FULL: u8 = u8::MAX;
const FULL_WIDE: u32 = 255;
/// `(x + 255) >> 8`: a product of two channels back to one, rounded up.
const DIVIDE_SHIFT: u32 = 8;
/// Where each channel lies in a 0RGB word.
const RED_SHIFT: u32 = 16;
const GREEN_SHIFT: u32 = 8;
const CHANNEL_MASK: u32 = 0xFF;

/// A product of two channels, divided by 255 rounding up.
#[expect(clippy::arithmetic_side_effects, reason = "the sum of two products of bytes, at most 2 × 255 × 255, far inside a u32")]
const fn divide(product: u32) -> u32 {
    (product + FULL_WIDE) >> DIVIDE_SHIFT
}

/// One channel of `source` laid over one of `under`, the source's alpha
/// `alpha`, under `mask`.
#[expect(clippy::arithmetic_side_effects, reason = "bytes and their products, at most 2 × 255 × 255; the divided value is at most 255")]
#[expect(clippy::cast_possible_truncation, reason = "a divided product of bytes is at most 255")]
const fn over(under: u8, source: u8, alpha: u8, mask: u8) -> u8 {
    let (under, source, alpha, mask) = (under as u32, source as u32, alpha as u32, mask as u32);
    if mask == FULL_WIDE {
        (divide(under * (FULL_WIDE - alpha)) + source) as u8
    } else {
        let remaining = FULL_WIDE - divide(alpha * mask);
        divide(under * remaining + source * mask) as u8
    }
}

/// A pixel of a layer with `source` laid over it under `mask`.
const fn over_pixel(under: Rgba, source: Rgba, mask: u8) -> Rgba {
    let [red, green, blue, alpha] = source;
    [over(under[0], red, alpha, mask), over(under[1], green, alpha, mask), over(under[2], blue, alpha, mask), over(under[3], alpha, alpha, mask)]
}

/// A word of the window, opaque, with `source` laid over it under `mask`.
const fn over_word(under: u32, source: Rgba, mask: u8) -> u32 {
    let [red, green, blue, alpha] = source;
    if mask == FULL && alpha == FULL {
        return word(source);
    }
    let (under_red, under_green, under_blue) =
        (((under >> RED_SHIFT) & CHANNEL_MASK) as u8, ((under >> GREEN_SHIFT) & CHANNEL_MASK) as u8, (under & CHANNEL_MASK) as u8);
    word([over(under_red, red, alpha, mask), over(under_green, green, alpha, mask), over(under_blue, blue, alpha, mask), FULL])
}

/// An opaque pixel as a 0RGB word.
pub(crate) const fn word([red, green, blue, _]: Rgba) -> u32 {
    (red as u32) << RED_SHIFT | (green as u32) << GREEN_SHIFT | blue as u32
}

/// A 0RGB word as an opaque pixel.
const fn pixel_of(word: u32) -> Rgba {
    [((word >> RED_SHIFT) & CHANNEL_MASK) as u8, ((word >> GREEN_SHIFT) & CHANNEL_MASK) as u8, (word & CHANNEL_MASK) as u8, FULL]
}

/// What a run of pixels is painted with: one colour, a colour for each,
/// or an opaque 0RGB word for each, as an image's are.
#[derive(Clone, Copy)]
pub(crate) enum Source<'a> {
    Solid(Rgba),
    Row(&'a [Rgba]),
    Opaque(&'a [u32]),
}

impl Source<'_> {
    /// The source from the `skip`th pixel on.
    pub(crate) fn skip(self, skip: usize) -> Self {
        match self {
            Self::Solid(colour) => Self::Solid(colour),
            Self::Row(row) => Self::Row(row.get(skip..).unwrap_or_default()),
            Self::Opaque(row) => Self::Opaque(row.get(skip..).unwrap_or_default()),
        }
    }

    /// The colour of the `index`th pixel.
    fn at(self, index: usize) -> Rgba {
        match self {
            Self::Solid(colour) => colour,
            Self::Row(row) => row.get(index).copied().unwrap_or_default(),
            Self::Opaque(row) => row.get(index).copied().map_or([0; 4], pixel_of),
        }
    }
}

/// A mask from the `skip`th pixel on.
fn skip_mask(mask: Option<&[u8]>, skip: usize) -> Option<&[u8]> {
    mask.map(|mask| mask.get(skip..).unwrap_or_default())
}

fn mask_at(mask: Option<&[u8]>, index: usize) -> u8 {
    mask.map_or(FULL, |mask| mask.get(index).copied().unwrap_or(0))
}

/// How much of the pixel `pixel..pixel + 1` the span from `low` to `high`
/// covers, along one axis.
pub(crate) fn share(low: f64, high: f64, pixel: u16) -> f32 {
    let (start, end) = (f64::from(pixel), f64::from(pixel) + 1.0);
    #[expect(clippy::cast_possible_truncation, reason = "a share of one pixel, between 0 and 1")]
    let share = (high.min(end) - low.max(start)).clamp(0.0, 1.0) as f32;
    share
}

/// A device coordinate as the pixel it lies in, clamped to `from..=to`.
fn pixel(value: f64, from: u16, to: u16) -> u16 {
    if value.is_nan() || value <= f64::from(from) {
        from
    } else if value >= f64::from(to) {
        to
    } else {
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "inside from..to, both u16")]
        let pixel = value as u16;
        pixel
    }
}

/// The pixels along one axis the span from `low` to `high` covers at
/// all, inside `from..to`: those it covers by less than half a 255th are
/// left out, as their alpha rounds to none.
pub(crate) fn covered(low: f64, high: f64, from: u16, to: u16) -> (u16, u16) {
    let mut first = pixel(low.floor(), from, to);
    if first < to && alpha(share(low, high, first)) == 0 {
        first = first.saturating_add(1);
    }
    let mut last = pixel(high.ceil(), from, to).max(first);
    if last > first && alpha(share(low, high, last.saturating_sub(1))) == 0 {
        last = last.saturating_sub(1);
    }
    (first, last)
}

/// One side of a clip: how much of its first or last row or column it
/// covers, and, where that is only in part, what was drawn there.
struct Edge {
    share: f32,
    /// The pixels drawn into the edge when it is held apart: a row across
    /// the whole scissor, or a column over the rows between the two rows.
    held: Option<Vec<Rgba>>,
}

impl Edge {
    /// An edge covering `share` of its pixels, held apart when that is
    /// less than whole, of `length` pixels.
    fn new(share: f32, apart: bool, length: usize) -> Self {
        Self { share, held: (apart && alpha(share) < FULL).then(|| vec![[0; 4]; length]) }
    }

    const fn apart(&self) -> bool {
        self.held.is_some()
    }
}

/// One clip: the pixels it lets through and its four edges.
struct Layer {
    scissor: DeviceBox,
    left: Edge,
    right: Edge,
    top: Edge,
    bottom: Edge,
}

impl Layer {
    /// The first row the edge columns hold.
    fn inner_top(&self) -> u16 {
        if self.top.apart() { self.scissor.y0.saturating_add(1) } else { self.scissor.y0 }
    }

    /// The clip's coverage of the pixel at `x`, `y`, as an alpha.
    fn coverage(&self, x: u16, y: u16) -> u8 {
        let across = if x == self.scissor.x0 {
            self.left.share
        } else if x.saturating_add(1) == self.scissor.x1 {
            self.right.share
        } else {
            1.0
        };
        let down = if y == self.scissor.y0 {
            self.top.share
        } else if y.saturating_add(1) == self.scissor.y1 {
            self.bottom.share
        } else {
            1.0
        };
        alpha(across * down)
    }
}

/// The window's pixels, the box being drawn, and the clips now open.
pub(crate) struct Surface {
    /// The whole window, one 0RGB word a pixel.
    pub(crate) words: Vec<u32>,
    width: u16,
    height: u16,
    /// The box being drawn: nothing outside it changes.
    area: DeviceBox,
    layers: Vec<Layer>,
}

impl Surface {
    /// A surface `width` by `height` pixels, black.
    pub(crate) fn new(width: u16, height: u16) -> Self {
        Self { words: vec![0; usize::from(width).saturating_mul(usize::from(height))], width, height, area: DeviceBox::default(), layers: Vec::new() }
    }

    /// Starts drawing `area` afresh: its pixels black, as a surface drawn
    /// on nothing, and no clip open.
    pub(crate) fn begin(&mut self, area: DeviceBox) {
        self.area = area.intersect(DeviceBox::whole(self.width, self.height));
        self.layers.clear();
        let width = usize::from(self.width);
        for row in self.area.y0..self.area.y1 {
            let start = usize::from(row).saturating_mul(width).saturating_add(usize::from(self.area.x0));
            if let Some(words) = self.words.get_mut(start..start.saturating_add(usize::from(self.area.width()))) {
                words.fill(0);
            }
        }
    }

    /// Closes every clip still open.
    pub(crate) fn end(&mut self) {
        while !self.layers.is_empty() {
            self.pop_clip();
        }
    }

    /// The pixels drawing may change now: the box, inside every open clip.
    pub(crate) fn scissor(&self) -> DeviceBox {
        self.layers.last().map_or(self.area, |layer| layer.scissor)
    }

    /// Opens a clip to the device rectangle `x0..x1` by `y0..y1`.
    pub(crate) fn push_clip(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) {
        let outer = self.scissor();
        let (left, right) = covered(x0, x1, outer.x0, outer.x1);
        let (top, bottom) = covered(y0, y1, outer.y0, outer.y1);
        let scissor = DeviceBox { x0: left, y0: top, x1: right, y1: bottom };
        let scissor = if scissor.is_empty() { DeviceBox::default() } else { scissor };
        let last = |end: u16| end.saturating_sub(1);
        let (left, right) = (share(x0, x1, scissor.x0), share(x0, x1, last(scissor.x1)));
        let (top, bottom) = (share(y0, y1, scissor.y0), share(y0, y1, last(scissor.y1)));
        let empty = scissor.is_empty();
        let (across, down) = (usize::from(scissor.width()), usize::from(scissor.height()));
        let top = Edge::new(top, !empty, across);
        // A single column or row is its own first and last.
        let bottom = if down > 1 { Edge::new(bottom, !empty, across) } else { Edge { share: top.share, held: None } };
        let inner_rows = down.saturating_sub(usize::from(top.apart())).saturating_sub(usize::from(bottom.apart()));
        let left = Edge::new(left, !empty, inner_rows);
        let right = if across > 1 { Edge::new(right, !empty, inner_rows) } else { Edge { share: left.share, held: None } };
        self.layers.push(Layer { scissor, left, right, top, bottom });
    }

    /// Closes the innermost clip: what was drawn into its edges is laid over
    /// what lies beneath through its coverage there.
    pub(crate) fn pop_clip(&mut self) {
        let Some(layer) = self.layers.pop() else { return };
        let scissor = layer.scissor;
        let put = |surface: &mut Self, x: u16, y: u16, pixel: Rgba| {
            if pixel != [0; 4] {
                surface.span(y, x, 1, Source::Solid(pixel), Some(&[layer.coverage(x, y)]));
            }
        };
        for (row, edge) in [(scissor.y0, &layer.top), (scissor.y1.saturating_sub(1), &layer.bottom)] {
            for (x, &pixel) in (scissor.x0..scissor.x1).zip(edge.held.iter().flatten()) {
                put(self, x, row, pixel);
            }
        }
        for (column, edge) in [(scissor.x0, &layer.left), (scissor.x1.saturating_sub(1), &layer.right)] {
            for (y, &pixel) in (layer.inner_top()..).zip(edge.held.iter().flatten()) {
                put(self, column, y, pixel);
            }
        }
    }

    /// Lays `source` over the `length` pixels of row `y` from column `x`,
    /// under `mask` (whole where there is none), inside the open clips.
    pub(crate) fn span(&mut self, y: u16, x: u16, length: usize, source: Source<'_>, mask: Option<&[u8]>) {
        let scissor = self.scissor();
        if y < scissor.y0 || y >= scissor.y1 {
            return;
        }
        // The run cut to the scissor.
        let end = usize::from(x).saturating_add(length).min(usize::from(scissor.x1));
        let start = usize::from(x.max(scissor.x0));
        if start >= end {
            return;
        }
        let skip = start.saturating_sub(usize::from(x));
        let (mut source, mut mask) = (source.skip(skip), skip_mask(mask, skip));
        let (mut start, mut end) = (start, end);
        // Through each clip from the innermost: where its edge covers a
        // pixel in part, the pixel is drawn into the clip.
        for layer in self.layers.iter_mut().rev() {
            let inner_top = layer.inner_top();
            if let (true, Some(held)) = (y == layer.scissor.y0, layer.top.held.as_mut()) {
                return Self::into_row(held, layer.scissor.x0, start, end, source, mask);
            }
            if let (true, Some(held)) = (y.saturating_add(1) == layer.scissor.y1, layer.bottom.held.as_mut()) {
                return Self::into_row(held, layer.scissor.x0, start, end, source, mask);
            }
            let row = usize::from(y.saturating_sub(inner_top));
            if let (true, Some(held)) = (start == usize::from(layer.scissor.x0), layer.left.held.as_mut()) {
                if let Some(pixel) = held.get_mut(row) {
                    *pixel = over_pixel(*pixel, source.at(0), mask_at(mask, 0));
                }
                start = start.saturating_add(1);
                (source, mask) = (source.skip(1), skip_mask(mask, 1));
            }
            if let (true, Some(held)) = (end == usize::from(layer.scissor.x1) && start < end, layer.right.held.as_mut()) {
                end = end.saturating_sub(1);
                let last = end.saturating_sub(start);
                if let Some(pixel) = held.get_mut(row) {
                    *pixel = over_pixel(*pixel, source.at(last), mask_at(mask, last));
                }
            }
            if start >= end {
                return;
            }
        }
        let row = usize::from(y).saturating_mul(usize::from(self.width));
        let Some(words) = self.words.get_mut(row.saturating_add(start)..row.saturating_add(end)) else { return };
        match (source, mask) {
            (Source::Solid(colour), None) if colour[3] == FULL => words.fill(word(colour)),
            (Source::Solid(colour), None) => words.iter_mut().for_each(|under| *under = over_word(*under, colour, FULL)),
            (Source::Solid(colour), Some(mask)) => words.iter_mut().zip(mask).for_each(|(under, &mask)| *under = over_word(*under, colour, mask)),
            (Source::Row(row), None) => words.iter_mut().zip(row).for_each(|(under, &colour)| *under = over_word(*under, colour, FULL)),
            (Source::Row(row), Some(mask)) => {
                words.iter_mut().zip(row).zip(mask).for_each(|((under, &colour), &mask)| *under = over_word(*under, colour, mask));
            }
            (Source::Opaque(row), None) => {
                if let Some(row) = row.get(..words.len()) {
                    words.copy_from_slice(row);
                }
            }
            (Source::Opaque(row), Some(mask)) => {
                words.iter_mut().zip(row).zip(mask).for_each(|((under, &colour), &mask)| *under = over_word(*under, pixel_of(colour), mask));
            }
        }
    }

    /// Lays a run from column `start` to `end` over a held row that starts
    /// at column `from`.
    fn into_row(row: &mut [Rgba], from: u16, start: usize, end: usize, source: Source<'_>, mask: Option<&[u8]>) {
        let offset = start.saturating_sub(usize::from(from));
        let Some(held) = row.get_mut(offset..offset.saturating_add(end.saturating_sub(start))) else { return };
        for (index, pixel) in held.iter_mut().enumerate() {
            *pixel = over_pixel(*pixel, source.at(index), mask_at(mask, index));
        }
    }
}
