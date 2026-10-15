//! Teletext pages from the stand-in engine as `vello_cpu` images, each also
//! cut into horizontal strips, so the tilt's perspective can be drawn as a
//! stack of exact affine pieces.

use std::sync::Arc;

use standin::page::render;
use vello_cpu::peniko::ImageAlphaType;
use vello_cpu::{Image, PixelMetadata, Pixmap};

use crate::paint::image;

/// How many pages the wall shows, as the prototype showed twelve frames
/// twice over.
pub const PAGE_COUNT: u16 = 24;
const FIRST_PAGE: u16 = 0x100;
/// Three stand-in channels, so neighbouring pages differ in colour.
const CHANNELS: u16 = 3;
const OPAQUE: u8 = u8::MAX;
const RGBA_BYTES: usize = 4;
/// Strips per page for the tilt; 500 dots in 25 strips of 20, one text row each.
const STRIPS: usize = 25;

pub struct Page {
    pub channel: u32,
    pub number: u16,
    pub width: f64,
    pub height: f64,
    pub image: Image,
    pub strips: Vec<Image>,
    pub strip_height: f64,
}

fn pixmap(rgba: Vec<u8>, width: u16, height: u16) -> Arc<Pixmap> {
    // Every teletext pixel is opaque, which lets the renderer skip blending.
    let metadata = PixelMetadata { may_have_transparency: false, alpha_type: ImageAlphaType::Alpha };
    Arc::new(Pixmap::from_parts(rgba, width, height, metadata))
}

pub fn pages() -> Vec<Page> {
    (0..PAGE_COUNT)
        .filter_map(|offset| {
            let channel = u32::from(offset % CHANNELS + 1);
            let number = FIRST_PAGE + offset;
            let frame = render(channel, number, false);
            let rgba: Vec<u8> = frame
                .indices
                .iter()
                .flat_map(|&index| {
                    let colour = frame.palette[usize::from(index)];
                    [colour.red, colour.green, colour.blue, OPAQUE]
                })
                .collect();
            let (width, height) = (u16::try_from(frame.width).ok()?, u16::try_from(frame.height).ok()?);
            let strip_rows = frame.height / STRIPS;
            let row_bytes = frame.width * RGBA_BYTES;
            let strips = rgba
                .chunks_exact(row_bytes * strip_rows)
                .filter_map(|chunk| Some(image(pixmap(chunk.to_vec(), width, u16::try_from(strip_rows).ok()?))))
                .collect();
            Some(Page {
                channel,
                number,
                width: f64::from(width),
                height: f64::from(height),
                image: image(pixmap(rgba, width, height)),
                strips,
                strip_height: f64::from(u16::try_from(strip_rows).ok()?),
            })
        })
        .collect()
}
