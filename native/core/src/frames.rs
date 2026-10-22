//! Frame handling in the trusted core (`architecture.md` section 10, steps
//! 2, 3 and 6): only frames of pages the interface shows are passed on, a
//! frame identical to the last one sent for its page is never resent, and
//! each frame sent is encoded in `protocol::frame`'s smallest form, against
//! the last one sent for its page, which the interface holds (decision 61).

use std::collections::BTreeMap;

use interface_core::canvas::PageKey;
use protocol::frame;

/// A frame sent: its version, and the palette, screen colour and indices
/// it showed.
struct Sent {
    version: u32,
    palette: Vec<[u8; 3]>,
    screen: u8,
    indices: Vec<u8>,
}

/// The frames sent of each visible page.
#[derive(Default)]
pub struct Frames {
    visible: Vec<PageKey>,
    /// The frame last sent for each visible page.
    sent: BTreeMap<PageKey, Sent>,
}

impl Frames {
    /// Records the pages now visible.
    pub fn set_visible(&mut self, visible: Vec<PageKey>) {
        // The interface drops images of pages it stops showing, so a page
        // shown again must be sent again, whole.
        self.sent.retain(|key, _| visible.contains(key));
        self.visible = visible;
    }

    /// The encoded image and its version, when the frame should be sent.
    /// The decoder has already checked shape and indices; the protocol
    /// reader checked them again on arrival. The frame is kept as given,
    /// to compare the next with and to encode it against; an unchanged
    /// frame costs the comparison and nothing else.
    pub fn accept(&mut self, key: PageKey, palette: Vec<[u8; 3]>, screen: u8, indices: Vec<u8>) -> Option<(u32, Vec<u8>)> {
        if !self.visible.contains(&key) {
            return None;
        }
        let last = self.sent.get(&key);
        if last.is_some_and(|last| last.palette == palette && last.screen == screen && last.indices == indices) {
            return None;
        }
        let version = last.map_or(0, |last| last.version.wrapping_add(1));
        let image = frame::encode(&palette, screen, &indices, last.map(|last| (last.version, last.indices.as_slice())))?;
        self.sent.insert(key, Sent { version, palette, screen, indices });
        Some((version, image))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};

    const KEY: PageKey = PageKey { channel: 1, page: 0x100 };
    const DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;

    fn page(fill: u8) -> (Vec<[u8; 3]>, Vec<u8>) {
        (vec![[0, 0, 0], [255, 255, 255]], vec![fill; DOTS])
    }

    /// The interface's side: reads each image against the one it holds.
    fn read(image: &[u8], held: Option<(u32, &[u8])>) -> Vec<u8> {
        frame::decode(image, held).unwrap().indices
    }

    #[test]
    fn only_visible_and_changed_frames_are_sent() {
        let mut frames = Frames::default();
        let (palette, black) = page(0);
        assert!(frames.accept(KEY, palette.clone(), 0, black.clone()).is_none(), "not visible");
        frames.set_visible(vec![KEY]);
        assert_eq!(frames.accept(KEY, palette.clone(), 0, black.clone()).map(|(version, _)| version), Some(0));
        assert!(frames.accept(KEY, palette.clone(), 0, black).is_none(), "unchanged");
        let (_, white) = page(1);
        assert_eq!(frames.accept(KEY, palette.clone(), 0, white.clone()).map(|(version, _)| version), Some(1), "flash changed it");
        let other = vec![[0, 0, 0], [255, 0, 0]];
        assert_eq!(frames.accept(KEY, other, 0, white).map(|(version, _)| version), Some(2), "a new palette changed it");
    }

    #[test]
    fn a_page_shown_again_is_sent_again() {
        let mut frames = Frames::default();
        let (palette, black) = page(0);
        frames.set_visible(vec![KEY]);
        assert!(frames.accept(KEY, palette.clone(), 0, black.clone()).is_some());
        frames.set_visible(Vec::new());
        frames.set_visible(vec![KEY]);
        let (_, image) = frames.accept(KEY, palette, 0, black).unwrap();
        assert_eq!(frame::base(&image), None, "whole, as the interface dropped it");
    }

    /// Decision 61: the first frame of a page stands alone; the next is
    /// sent as its changes against the first, which the interface holds,
    /// and a new screen colour is a new frame.
    #[test]
    fn a_frame_is_sent_as_its_changes_against_the_last() {
        let mut frames = Frames::default();
        frames.set_visible(vec![KEY]);
        let (palette, black) = page(0);
        let (version, first) = frames.accept(KEY, palette.clone(), 0, black.clone()).unwrap();
        assert_eq!(frame::base(&first), None);
        assert_eq!(read(&first, None), black);
        let mut flashed = black.clone();
        flashed[100..140].fill(1);
        let (next, image) = frames.accept(KEY, palette.clone(), 0, flashed.clone()).unwrap();
        assert_eq!(frame::base(&image), Some(version));
        assert!(image.len() < 128, "forty changed dots: {} bytes", image.len());
        assert_eq!(read(&image, Some((version, &black))), flashed);
        let (_, screen) = frames.accept(KEY, palette, 1, flashed.clone()).unwrap();
        assert_eq!(frame::decode(&screen, Some((next, &flashed))).unwrap().screen, 1);
    }
}
