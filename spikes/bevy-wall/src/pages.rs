//! Teletext pages from the stand-in engine, as images the interface can draw.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use standin::page::{Frame, render};

/// How many pages the wall shows, as the prototype showed twelve frames
/// twice over.
pub const PAGE_COUNT: u16 = 24;
const FIRST_PAGE: u16 = 0x100;
/// Three stand-in channels, so neighbouring pages differ in colour.
const CHANNELS: u16 = 3;
const RGBA_BYTES: usize = 4;
const OPAQUE: u8 = u8::MAX;

pub struct Page {
    pub channel: u32,
    pub number: u16,
    pub image: Image,
}

fn to_image(frame: &Frame) -> Image {
    let mut data = Vec::with_capacity(frame.indices.len() * RGBA_BYTES);
    for &index in &frame.indices {
        let colour = frame.palette[usize::from(index)];
        data.extend_from_slice(&[colour.red, colour.green, colour.blue, OPAQUE]);
    }
    let size = Extent3d {
        width: u32::try_from(frame.width).unwrap_or(0),
        height: u32::try_from(frame.height).unwrap_or(0),
        depth_or_array_layers: 1,
    };
    let mut image =
        Image::new(size, TextureDimension::D2, data, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
    // Nearest-neighbour scaling keeps teletext's blocks sharp at every size.
    image.sampler = ImageSampler::nearest();
    image
}

pub fn pages() -> Vec<Page> {
    (0..PAGE_COUNT)
        .map(|offset| {
            let channel = u32::from(offset % CHANNELS + 1);
            let number = FIRST_PAGE + offset;
            Page { channel, number, image: to_image(&render(channel, number, false)) }
        })
        .collect()
}
