//! What a fill paints with besides one colour: an image, sampled at each
//! pixel's centre, or a linear gradient.

use interface_core::theme::Colour;

use crate::raster::surface::Rgba;

const FULL: u8 = u8::MAX;
const FULL_WIDE: u16 = 255;
/// Rounds a share to the nearest step of 1/255.
const HALF: f64 = 0.5;
/// A pixel's centre, from its top-left corner.
const CENTRE: f64 = 0.5;

/// An opaque colour as a pixel.
pub(crate) const fn opaque(colour: Colour) -> Rgba {
    [colour.red, colour.green, colour.blue, FULL]
}

/// An image's pixels, row by row, each opaque, as 0RGB words: the
/// window's own form, so a pixel sampled nearest is copied as it is.
#[derive(Clone)]
pub(crate) struct Pixels {
    width: u16,
    height: u16,
    data: Vec<u32>,
}

impl Pixels {
    /// `data` as an image `width` by `height`; `None` when it is not that
    /// many pixels.
    pub(crate) fn new(width: u16, height: u16, data: Vec<u32>) -> Option<Self> {
        (data.len() == usize::from(width).saturating_mul(usize::from(height))).then_some(Self { width, height, data })
    }

    pub(crate) const fn width(&self) -> u16 {
        self.width
    }

    pub(crate) const fn height(&self) -> u16 {
        self.height
    }

    pub(crate) fn data(&self) -> &[u32] {
        &self.data
    }

    pub(crate) fn data_mut(&mut self) -> &mut [u32] {
        &mut self.data
    }

    fn row(&self, row: usize) -> &[u32] {
        let width = usize::from(self.width);
        let start = row.saturating_mul(width);
        self.data.get(start..start.saturating_add(width)).unwrap_or_default()
    }
}

/// How an image is sampled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sampling {
    /// The pixel under each pixel's centre: blocks stay sharp.
    Nearest,
    /// The four pixels around it, weighted by nearness: a picture drawn
    /// smaller keeps its thin lines.
    Smooth,
}

/// One axis of the mapping from device pixels to an image's: `image =
/// device × scale + offset`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Axis {
    pub(crate) scale: f64,
    pub(crate) offset: f64,
}

/// Where along one axis a device pixel samples an image: for nearest
/// sampling the one pixel, for smooth the two either side and the weight
/// of the second, in 255ths.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Tap {
    first: usize,
    second: usize,
    weight: u16,
}

/// The pixel of an image's `size` a position falls in, the image's edge
/// pixels carried on outward.
fn clamp_index(position: f64, size: u16) -> usize {
    let last = f64::from(size.saturating_sub(1));
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0..=size - 1, a u16")]
    let index = position.clamp(0.0, last) as usize;
    index
}

fn tap(axis: Axis, pixel: u16, size: u16, sampling: Sampling) -> Tap {
    let position = (f64::from(pixel) + CENTRE) * axis.scale + axis.offset;
    match sampling {
        Sampling::Nearest => {
            let index = clamp_index(position.floor(), size);
            Tap { first: index, second: index, weight: 0 }
        }
        Sampling::Smooth => {
            // The pixels whose centres lie either side of the position.
            let shifted = position + CENTRE;
            let base = shifted.floor();
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a share between 0 and 1, scaled to 0..=255")]
            let weight = ((shifted - base) * f64::from(FULL_WIDE) + HALF) as u16;
            Tap { first: clamp_index(base - 1.0, size), second: clamp_index(base, size), weight: weight.min(FULL_WIDE) }
        }
    }
}

/// The red and blue channels of a 0RGB word, each in a lane of sixteen
/// bits; the green in the other.
const RED_BLUE: u32 = 0x00FF_00FF;
const GREEN: u32 = 0x0000_FF00;
/// Each lane's rounding term, 255, added before the shift by eight.
const ROUND_RED_BLUE: u32 = 0x00FF_00FF;
const ROUND_GREEN: u32 = 0x0000_00FF;
const LANE_SHIFT: u32 = 8;

/// Two opaque 0RGB words mixed, `weight` 255ths of the second, each
/// channel `(first × (255 − weight) + second × weight + 255) >> 8`; red
/// and blue together, a channel to each sixteen-bit lane, which the sum
/// fits (at most 255 × 255 + 255).
#[expect(clippy::arithmetic_side_effects, reason = "each lane holds at most 255 × 255 + 255, inside its sixteen bits; weight is at most 255")]
pub(crate) const fn mix(first: u32, second: u32, weight: u16) -> u32 {
    let (weight, rest) = (weight as u32, FULL_WIDE as u32 - weight as u32);
    let red_blue = (((first & RED_BLUE) * rest + (second & RED_BLUE) * weight + ROUND_RED_BLUE) >> LANE_SHIFT) & RED_BLUE;
    let green = ((((first & GREEN) >> LANE_SHIFT) * rest + ((second & GREEN) >> LANE_SHIFT) * weight + ROUND_GREEN) >> LANE_SHIFT) & (GREEN >> LANE_SHIFT);
    red_blue | (green << LANE_SHIFT)
}

/// An image laid over device pixels, with the place in it of each column
/// of the pixels it paints worked out once.
pub(crate) struct ImagePaint<'a> {
    pixels: &'a Pixels,
    sampling: Sampling,
    down: Axis,
    /// The first device column the taps are for.
    from: u16,
    columns: Vec<Tap>,
    /// The taps down, first column and length of the last row made: a row
    /// that samples the same is not made again.
    last: Option<(Tap, u16, usize)>,
    /// For smooth sampling, the image's two rows mixed down, over the
    /// columns the taps reach.
    mixed: Vec<u32>,
}

impl<'a> ImagePaint<'a> {
    /// `pixels` mapped to device pixels by `across` and `down`, for the
    /// columns `from..to`.
    pub(crate) fn new(pixels: &'a Pixels, sampling: Sampling, across: Axis, down: Axis, from: u16, to: u16) -> Self {
        let columns = (from..to).map(|column| tap(across, column, pixels.width, sampling)).collect();
        Self { pixels, sampling, down, from, columns, last: None, mixed: Vec::new() }
    }

    /// The colours of `length` pixels of row `y` from column `x`, as 0RGB
    /// words, into `out`; left as they are when the last row made samples
    /// the same, as rows of an image drawn larger do.
    pub(crate) fn row(&mut self, y: u16, x: u16, length: usize, out: &mut Vec<u32>) {
        let row = tap(self.down, y, self.pixels.height, self.sampling);
        if self.last == Some((row, x, length)) {
            return;
        }
        self.last = Some((row, x, length));
        out.clear();
        let skip = usize::from(x.saturating_sub(self.from));
        let columns = self.columns.get(skip..skip.saturating_add(length)).unwrap_or_default();
        let (upper, lower) = (self.pixels.row(row.first), self.pixels.row(row.second));
        match self.sampling {
            Sampling::Nearest => out.extend(columns.iter().map(|column| upper.get(column.first).copied().unwrap_or_default())),
            Sampling::Smooth => {
                // Down first, once for each column of the image the taps
                // reach, then across for each pixel: a weight of none
                // leaves the first exactly as it is.
                let low = columns.iter().map(|column| column.first).min().unwrap_or(0);
                let high = columns.iter().map(|column| column.second).max().unwrap_or(0);
                self.mixed.clear();
                let (upper, lower) = (upper.get(low..=high).unwrap_or_default(), lower.get(low..=high).unwrap_or_default());
                if row.weight == 0 {
                    self.mixed.extend_from_slice(upper);
                } else {
                    self.mixed.extend(upper.iter().zip(lower).map(|(&above, &below)| mix(above, below, row.weight)));
                }
                let mixed = &self.mixed;
                let at = |index: usize| mixed.get(index.saturating_sub(low)).copied().unwrap_or_default();
                out.extend(
                    columns.iter().map(|column| if column.weight == 0 { at(column.first) } else { mix(at(column.first), at(column.second), column.weight) }),
                );
            }
        }
    }
}

/// A linear gradient from `start` to `end`, in device pixels, through
/// opaque colour stops, its ends carried on beyond them.
pub(crate) struct GradientPaint {
    start: (f64, f64),
    /// From start to end, divided by its length squared.
    direction: (f64, f64),
    stops: Vec<(f64, [f64; 3])>,
}

impl GradientPaint {
    /// The gradient through `stops` from `start` to `end`.
    pub(crate) fn new(start: (f64, f64), end: (f64, f64), stops: &[(f32, Colour)]) -> Self {
        let (across, down) = (end.0 - start.0, end.1 - start.1);
        let length = across.mul_add(across, down * down);
        let direction = if length > 0.0 { (across / length, down / length) } else { (0.0, 0.0) };
        let stops =
            stops.iter().map(|&(offset, colour)| (f64::from(offset), [f64::from(colour.red), f64::from(colour.green), f64::from(colour.blue)])).collect();
        Self { start, direction, stops }
    }

    /// The colour at device position `x`, `y`: interpolated in the sRGB
    /// values between the stops either side.
    fn at(&self, across: f64, down: f64) -> Rgba {
        let along = ((across - self.start.0) * self.direction.0 + (down - self.start.1) * self.direction.1).clamp(0.0, 1.0);
        let mut previous: Option<&(f64, [f64; 3])> = None;
        let mut colour = self.stops.last().map_or([0.0; 3], |stop| stop.1);
        for stop in &self.stops {
            if along <= stop.0 {
                colour = match previous {
                    Some(before) if stop.0 > before.0 => {
                        let share = (along - before.0) / (stop.0 - before.0);
                        [0, 1, 2].map(|channel| {
                            let (a, b) = (before.1.get(channel).copied().unwrap_or(0.0), stop.1.get(channel).copied().unwrap_or(0.0));
                            a + (b - a) * share
                        })
                    }
                    _ => stop.1,
                };
                break;
            }
            previous = Some(stop);
        }
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a mean of bytes, rounded, between 0 and 255")]
        let [red, green, blue] = colour.map(|value| (value + HALF).clamp(0.0, f64::from(FULL)) as u8);
        [red, green, blue, FULL]
    }

    /// The colours of `length` pixels of row `y` from column `x`, each at
    /// its centre.
    pub(crate) fn row(&self, y: u16, x: u16, length: usize, out: &mut Vec<Rgba>) {
        out.clear();
        let centre_y = f64::from(y) + CENTRE;
        out.extend((x..).take(length).map(|column| self.at(f64::from(column) + CENTRE, centre_y)));
    }
}
