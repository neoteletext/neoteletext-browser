//! Teletext pages from the stand-in engine as tiny-skia images, each also
//! cut into horizontal strips, so the tilt's perspective can be drawn as a
//! stack of exact affine pieces.

use standin::page::render;
use tiny_skia::{IntRect, Pixmap};

/// How many pages the wall shows, as the prototype showed twelve frames
/// twice over.
pub const PAGE_COUNT: u16 = 24;
const FIRST_PAGE: u16 = 0x100;
/// Three stand-in channels, so neighbouring pages differ in colour.
const CHANNELS: u16 = 3;
const OPAQUE: u8 = u8::MAX;
/// Strips per page for the tilt; 500 dots in 25 strips of 20, one text row each.
pub const STRIPS: u32 = 25;

pub struct Page {
    pub channel: u32,
    pub number: u16,
    pub image: Pixmap,
    pub strips: Vec<Pixmap>,
}

pub fn pages() -> Vec<Page> {
    (0..PAGE_COUNT)
        .filter_map(|offset| {
            let channel = u32::from(offset % CHANNELS + 1);
            let number = FIRST_PAGE + offset;
            let frame = render(channel, number, false);
            let mut image = Pixmap::new(u32::try_from(frame.width).ok()?, u32::try_from(frame.height).ok()?)?;
            for (pixel, &index) in image.data_mut().chunks_exact_mut(4).zip(&frame.indices) {
                let colour = frame.palette[usize::from(index)];
                pixel.copy_from_slice(&[colour.red, colour.green, colour.blue, OPAQUE]);
            }
            let strip_height = image.height() / STRIPS;
            let strips = (0..STRIPS)
                .filter_map(|strip| {
                    let top = i32::try_from(strip * strip_height).ok()?;
                    image.clone_rect(IntRect::from_xywh(0, top, image.width(), strip_height)?)
                })
                .collect();
            Some(Page { channel, number, image, strips })
        })
        .collect()
}
