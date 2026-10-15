//! Teletext pages from the stand-in engine as macroquad textures.

use macroquad::texture::{FilterMode, Texture2D};
use standin::page::render;

/// How many pages the wall shows, as the prototype showed twelve frames
/// twice over.
pub const PAGE_COUNT: u16 = 24;
const FIRST_PAGE: u16 = 0x100;
/// Three stand-in channels, so neighbouring pages differ in colour.
const CHANNELS: u16 = 3;
const OPAQUE: u8 = u8::MAX;

pub struct Page {
    pub channel: u32,
    pub number: u16,
    pub texture: Texture2D,
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
            let texture = Texture2D::from_rgba8(u16::try_from(frame.width).ok()?, u16::try_from(frame.height).ok()?, &rgba);
            // Nearest-neighbour scaling keeps teletext's blocks sharp.
            texture.set_filter(FilterMode::Nearest);
            Some(Page { channel, number, texture })
        })
        .collect()
}
