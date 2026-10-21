//! Frame handling in the trusted core (`architecture.md` section 10, steps
//! 2, 3 and 6): only frames of pages the interface shows are passed on, a
//! frame identical to the last one sent for its page is never resent, and
//! each frame sent is encoded as an indexed image by the engine's writer.

use std::collections::BTreeMap;

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH, PageKey};
use neoteletext_image::palette::Rgba;
use neoteletext_image::png;

/// The engine's writer needs two rows of scratch, plus a filter byte each.
const SCRATCH: usize = 2 * (PAGE_WIDTH as usize + 1);

/// A frame sent: its version, and the palette and indices it showed.
struct Sent {
    version: u32,
    palette: Vec<[u8; 3]>,
    indices: Vec<u8>,
}

/// The frames sent of each visible page.
#[derive(Default)]
pub struct Frames {
    visible: Vec<PageKey>,
    /// The frame last sent for each visible page.
    sent: BTreeMap<PageKey, Sent>,
    /// How long the last image encoded was: the next is about as long.
    encoded_len: usize,
}

impl Frames {
    /// Records the pages now visible.
    pub fn set_visible(&mut self, visible: Vec<PageKey>) {
        // The interface drops images of pages it stops showing, so a page
        // shown again must be sent again.
        self.sent.retain(|key, _| visible.contains(key));
        self.visible = visible;
    }

    /// The encoded image and its version, when the frame should be sent.
    /// The decoder has already checked shape and indices; the protocol
    /// reader checked them again on arrival. The frame is kept as given,
    /// to compare the next with; an unchanged frame costs the comparison
    /// and nothing else.
    pub fn accept(&mut self, key: PageKey, palette: Vec<[u8; 3]>, indices: Vec<u8>) -> Option<(u32, Vec<u8>)> {
        if !self.visible.contains(&key) {
            return None;
        }
        let version = match self.sent.get(&key) {
            Some(last) if last.palette == palette && last.indices == indices => return None,
            Some(last) => last.version.wrapping_add(1),
            None => 0,
        };
        let png = encode(&palette, &indices, self.encoded_len)?;
        self.encoded_len = png.len();
        self.sent.insert(key, Sent { version, palette, indices });
        Some((version, png))
    }
}

/// An indexed image of a page, by the engine's writer, in room for about
/// `expected` bytes.
fn encode(palette: &[[u8; 3]], indices: &[u8], expected: usize) -> Option<Vec<u8>> {
    let palette: Vec<Rgba> = palette.iter().map(|&[red, green, blue]| Rgba::opaque(red, green, blue)).collect();
    let width = usize::from(PAGE_WIDTH);
    let mut out = Vec::with_capacity(expected);
    let mut scratch = [0_u8; SCRATCH];
    let mut row = |y: usize, line: &mut [u8]| {
        if let Some(source) = indices.chunks_exact(width).nth(y) {
            line.copy_from_slice(source);
        }
    };
    png::write(&mut out, width, usize::from(PAGE_HEIGHT), &palette, &mut row, &mut scratch).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: PageKey = PageKey { channel: 1, page: 0x100 };
    const DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;

    fn page(fill: u8) -> (Vec<[u8; 3]>, Vec<u8>) {
        (vec![[0, 0, 0], [255, 255, 255]], vec![fill; DOTS])
    }

    #[test]
    fn only_visible_and_changed_frames_are_sent() {
        let mut frames = Frames::default();
        let (palette, black) = page(0);
        assert!(frames.accept(KEY, palette.clone(), black.clone()).is_none(), "not visible");
        frames.set_visible(vec![KEY]);
        assert_eq!(frames.accept(KEY, palette.clone(), black.clone()).map(|(version, _)| version), Some(0));
        assert!(frames.accept(KEY, palette.clone(), black).is_none(), "unchanged");
        let (_, white) = page(1);
        assert_eq!(frames.accept(KEY, palette.clone(), white.clone()).map(|(version, _)| version), Some(1), "flash changed it");
        let other = vec![[0, 0, 0], [255, 0, 0]];
        assert_eq!(frames.accept(KEY, other, white).map(|(version, _)| version), Some(2), "a new palette changed it");
    }

    #[test]
    fn a_page_shown_again_is_sent_again() {
        let mut frames = Frames::default();
        let (palette, black) = page(0);
        frames.set_visible(vec![KEY]);
        assert!(frames.accept(KEY, palette.clone(), black.clone()).is_some());
        frames.set_visible(Vec::new());
        frames.set_visible(vec![KEY]);
        assert!(frames.accept(KEY, palette, black).is_some());
    }

    #[test]
    fn frames_are_encoded_as_indexed_images() {
        let (palette, indices) = page(1);
        let png = encode(&palette, &indices, 0).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR: 480 by 500, 8 bits, colour type 3 (indexed).
        assert_eq!(&png[16..26], &[0, 0, 1, 0xE0, 0, 0, 1, 0xF4, 8, 3]);
    }
}
