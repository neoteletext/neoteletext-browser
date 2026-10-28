//! NeoTeletext's icon (decision 124, requirement 12.19): the owner's pixel
//! art, `neoteletext-icon.png` beside this crate, is the one source every
//! platform's icon is derived from, by build scripts and the release tool.
//! No program reads it at run time, so the confined interface needs no file
//! for it (requirement 3.6).
//!
//! The art is 128 pixels square in 2 by 2 blocks: an exact double of a 64
//! by 64 picture, whose letters stand on a grid of 4 by 4 pixels of the
//! 128 offset 2 pixels across (measured on 28 October 2026). Every size is
//! made by nearest neighbour, never smoothed ([`Image::scaled`]): 64 and
//! every multiple of 128 are exact, 32 holds each cell of the letters' grid
//! once, and smaller or uneven sizes drop or repeat rows and columns.

// No panicking constructs: the release tool reads PNG files `iconutil`
// wrote. The same list is in each such crate; scripts/check.sh checks
// that they agree.
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

pub mod macos;
pub mod png;
pub mod windows;

/// The owner's art, as the owner gave it on 28 October 2026.
pub const SOURCE: &[u8] = include_bytes!("../neoteletext-icon.png");
/// Its width and height, in pixels.
pub const SOURCE_SIDE: u32 = 128;
/// Red, green, blue and alpha, a byte each: PNG's colour type 6 at bit
/// depth 8 (the PNG specification, section 11.2.2), the only form read or
/// written.
pub const BYTES_PER_PIXEL: usize = 4;
/// The largest side read or made: the macOS iconset's
/// `icon_512x512@2x.png`.
pub const MAX_SIDE: u32 = 1024;

/// The images of the macOS bundle's icon file, by their names in an
/// iconset as `iconutil` reads them (its manual page), each with its side
/// in pixels: every size at 1x and 2x from 16 to 512 points.
pub const MACOS_ICONSET: [(&str, u32); 10] = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

/// The side of the picture the macOS Dock is given for a program run
/// outside a bundle: 8 times the art, so the Dock's own scaling to its
/// tile's power-of-two sizes averages blocks of one colour and keeps the
/// pixels sharp.
pub const DOCK_SIDE: u32 = 1024;

/// Why an image was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused(pub &'static str);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Refused {}

/// A picture of RGBA pixels, a row at a time from the top, each row from
/// the left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// The bytes of a picture `width` by `height`, if neither side is 0 or
/// above [`MAX_SIDE`].
fn image_bytes(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    usize::try_from(width).ok()?.checked_mul(usize::try_from(height).ok()?)?.checked_mul(BYTES_PER_PIXEL)
}

/// The pixel of a side of `from` pixels whose centre is nearest the centre
/// of pixel `at` of a side of `to`: `floor((at + 1/2) × from / to)`, in
/// whole numbers.
fn nearest(at: u32, to: u32, from: u32) -> Option<u32> {
    let centre = u64::from(at).checked_mul(2)?.checked_add(1)?;
    let source = centre.checked_mul(u64::from(from))?.checked_div(u64::from(to).checked_mul(2)?)?;
    u32::try_from(source).ok()
}

impl Image {
    /// `rgba` as a picture `width` by `height`.
    ///
    /// # Errors
    /// When a side is 0 or above [`MAX_SIDE`], or `rgba` is not exactly
    /// that many pixels.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, Refused> {
        if image_bytes(width, height) != Some(rgba.len()) {
            return Err(Refused("the pixels are not the picture's size"));
        }
        Ok(Self { width, height, rgba })
    }

    /// Its width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Its height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Its pixels, red, green, blue and alpha.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The pixel `x` from the left and `y` from the top, if there is one.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; BYTES_PER_PIXEL]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::try_from(y).ok()?.checked_mul(usize::try_from(self.width).ok()?)?.checked_add(usize::try_from(x).ok()?)?;
        let start = index.checked_mul(BYTES_PER_PIXEL)?;
        self.rgba.get(start..start.checked_add(BYTES_PER_PIXEL)?)?.try_into().ok()
    }

    /// This picture `side` pixels square by nearest neighbour: each pixel
    /// takes the colour of the pixel of this one under its centre, so no
    /// colour is made that the art does not have. A side that divides
    /// this one's, or is a multiple of it, is exact: every pixel of the
    /// art becomes a square block of one colour, or every block of the art
    /// one pixel.
    ///
    /// # Errors
    /// When `side` is 0 or above [`MAX_SIDE`].
    pub fn scaled(&self, side: u32) -> Result<Self, Refused> {
        let bytes = image_bytes(side, side).ok_or(Refused("no icon is that size"))?;
        let mut rgba = Vec::with_capacity(bytes);
        for y in 0..side {
            let from_y = nearest(y, side, self.height).ok_or(Refused("a size too large"))?;
            for x in 0..side {
                let from_x = nearest(x, side, self.width).ok_or(Refused("a size too large"))?;
                rgba.extend_from_slice(&self.pixel(from_x, from_y).ok_or(Refused("a pixel outside the picture"))?);
            }
        }
        Self::new(side, side, rgba)
    }
}

/// The owner's art, decoded.
///
/// # Errors
/// When [`SOURCE`] is not an RGBA PNG [`SOURCE_SIDE`] pixels square: the
/// art was replaced by something the build cannot read.
pub fn source() -> Result<Image, Refused> {
    let image = png::decode(SOURCE)?;
    if image.width() != SOURCE_SIDE || image.height() != SOURCE_SIDE {
        return Err(Refused("the art is not the side it is said to be"));
    }
    Ok(image)
}

#[cfg(test)]
mod tests;
