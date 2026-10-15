//! Teletext pages from the stand-in engine, as egui textures.

use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions};
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
    pub texture: TextureHandle,
}

pub fn pages(context: &Context) -> Vec<Page> {
    (0..PAGE_COUNT)
        .map(|offset| {
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
            let image = ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &rgba);
            // Nearest-neighbour scaling keeps teletext's blocks sharp.
            let texture = context.load_texture(format!("page {number:03X} channel {channel}"), image, TextureOptions::NEAREST);
            Page { channel, number, texture }
        })
        .collect()
}
