//! What a fill paints with besides one colour: an image, sampled at each
//! pixel's centre, or a linear gradient.

use interface_core::theme::Colour;

use crate::raster::surface::{Rgba, word};

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

/// An image's pixels, row by row, each opaque.
#[derive(Clone)]
pub(crate) struct Pixels {
    width: u16,
    height: u16,
    data: Vec<Rgba>,
}

impl Pixels {
    /// `data` as an image `width` by `height`; `None` when it is not that
    /// many pixels.
    pub(crate) fn new(width: u16, height: u16, data: Vec<Rgba>) -> Option<Self> {
        (data.len() == usize::from(width).saturating_mul(usize::from(height))).then_some(Self { width, height, data })
    }

    pub(crate) const fn width(&self) -> u16 {
        self.width
    }

    pub(crate) const fn height(&self) -> u16 {
        self.height
    }

    pub(crate) fn data(&self) -> &[Rgba] {
        &self.data
    }

    pub(crate) fn data_mut(&mut self) -> &mut [Rgba] {
        &mut self.data
    }

    fn row(&self, row: usize) -> &[Rgba] {
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

/// Two pixels mixed, `weight` 255ths of the second, each channel's product
/// divided by 255 rounding up.
#[expect(clippy::arithmetic_side_effects, reason = "channels and weights of at most 255: each sum is at most 255 × 255 + 255")]
#[expect(clippy::cast_possible_truncation, reason = "a weighted mean of bytes is at most 255")]
fn mix(first: Rgba, second: Rgba, weight: u16) -> Rgba {
    let rest = FULL_WIDE - weight;
    let channel = |a: u8, b: u8| ((u32::from(a) * u32::from(rest) + u32::from(b) * u32::from(weight) + u32::from(FULL_WIDE)) >> 8) as u8;
    [channel(first[0], second[0]), channel(first[1], second[1]), channel(first[2], second[2]), channel(first[3], second[3])]
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
}

impl<'a> ImagePaint<'a> {
    /// `pixels` mapped to device pixels by `across` and `down`, for the
    /// columns `from..to`.
    pub(crate) fn new(pixels: &'a Pixels, sampling: Sampling, across: Axis, down: Axis, from: u16, to: u16) -> Self {
        let columns = (from..to).map(|column| tap(across, column, pixels.width, sampling)).collect();
        Self { pixels, sampling, down, from, columns, last: None }
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
            Sampling::Nearest => out.extend(columns.iter().map(|column| word(upper.get(column.first).copied().unwrap_or_default()))),
            Sampling::Smooth => out.extend(columns.iter().map(|column| {
                let at = |line: &[Rgba], index: usize| line.get(index).copied().unwrap_or_default();
                let above = mix(at(upper, column.first), at(upper, column.second), column.weight);
                let below = mix(at(lower, column.first), at(lower, column.second), column.weight);
                word(mix(above, below, row.weight))
            })),
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
