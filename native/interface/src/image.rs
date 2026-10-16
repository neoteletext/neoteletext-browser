//! Reads the page images the trusted core writes: indexed, 8 bits a dot,
//! exactly 480 by 500 and at most 32 colours (requirement 9.3). Anything
//! else is refused, and the decoder may allocate no more than one image
//! needs.

use std::io::Cursor;

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};
use png::{BitDepth, ColorType, Decoder, Limits, Transformations};

/// The Colour Map of Presentation Level 2.5: four tables of eight.
const MAX_COLOURS: usize = 32;
const RGB_BYTES: usize = 3;
const PAGE_DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
/// The decoder's own working memory beyond the image: its buffers for one
/// compressed row and the inflate window are well under this.
const DECODER_OVERHEAD: usize = 256 * 1024;

/// A page image as read: its colours and one index into them per dot.
pub struct Indexed {
    /// Red, green and blue of each colour.
    pub palette: Vec<[u8; 3]>,
    /// One palette index per dot, row by row.
    pub indices: Vec<u8>,
}

/// # Errors
/// A description of the first thing wrong with the image.
pub fn decode(bytes: &[u8]) -> Result<Indexed, String> {
    let mut decoder = Decoder::new_with_limits(Cursor::new(bytes), Limits { bytes: PAGE_DOTS + DECODER_OVERHEAD });
    // Palette indices as they are, not expanded to colours.
    decoder.set_transformations(Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let info = reader.info();
    if (info.width, info.height) != (u32::from(PAGE_WIDTH), u32::from(PAGE_HEIGHT)) {
        return Err(format!("image is {} by {}", info.width, info.height));
    }
    if info.color_type != ColorType::Indexed || info.bit_depth != BitDepth::Eight {
        return Err("image is not indexed at 8 bits".into());
    }
    let palette: Vec<[u8; 3]> = info.palette.as_ref().ok_or("image has no palette")?.as_chunks::<RGB_BYTES>().0.to_vec();
    if palette.is_empty() || palette.len() > MAX_COLOURS {
        return Err(format!("palette of {} colours", palette.len()));
    }
    let mut indices = vec![0; PAGE_DOTS];
    reader.next_frame(&mut indices).map_err(|error| error.to_string())?;
    if indices.iter().any(|&index| usize::from(index) >= palette.len()) {
        return Err("index outside the palette".into());
    }
    Ok(Indexed { palette, indices })
}

#[cfg(test)]
mod tests {
    use super::*;
    use decoder::Engine;
    use neoteletext_image::palette::Rgba;

    /// A page rendered by the engine, written as the trusted core writes it.
    #[allow(clippy::arithmetic_side_effects, reason = "the PNG writer's scratch size for a width of at most 480")]
    fn engine_page(width: usize) -> (Vec<u8>, Vec<u8>, usize) {
        let mut engine = Engine::new();
        engine.feed(&decoder::sample::tti(&[0x100], &["Headlines"]));
        engine.finish();
        let page = engine.render(0x100).unwrap();
        let palette: Vec<Rgba> = page.palette.iter().map(|&[red, green, blue]| Rgba::opaque(red, green, blue)).collect();
        let mut out = Vec::new();
        let mut scratch = vec![0; 2 * (width + 1)];
        let mut row = |y: usize, line: &mut [u8]| line.copy_from_slice(&page.indices.chunks_exact(usize::from(PAGE_WIDTH)).nth(y).unwrap()[..width]);
        neoteletext_image::png::write(&mut out, width, usize::from(PAGE_HEIGHT), &palette, &mut row, &mut scratch).unwrap();
        (out, page.indices, page.palette.len())
    }

    #[test]
    fn reads_what_the_trusted_core_writes() {
        let (png, indices, colours) = engine_page(usize::from(PAGE_WIDTH));
        let image = decode(&png).unwrap();
        assert_eq!(image.indices, indices);
        assert_eq!(image.palette.len(), colours);
    }

    #[test]
    fn refuses_other_shapes_and_damage() {
        let (narrow, _, _) = engine_page(usize::from(PAGE_WIDTH) - 1);
        assert!(decode(&narrow).is_err());
        let (mut bytes, _, _) = engine_page(usize::from(PAGE_WIDTH));
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xFF;
        assert!(decode(&bytes).is_err(), "a damaged image fails its checksum");
        assert!(decode(b"not an image").is_err());
    }
}
