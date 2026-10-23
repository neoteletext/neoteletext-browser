//! Frame handling in the trusted core (`architecture.md` section 10, steps
//! 2, 3 and 6): only frames of pages the interface shows are passed on, a
//! frame identical to the last one sent for its page is never resent, a
//! page's frames are sent at most once a field, and each frame sent is
//! encoded in `protocol::frame`'s smallest form, against the last one sent
//! for its page, which the interface holds (decision 61).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use interface_core::canvas::PageKey;
use neoteletext_core::consts::FIELD_PERIOD_US;
use protocol::frame;

/// The least time between two frames of one page (finding F-11): one field
/// of the 50 a second a teletext display shows (the engine's
/// `FIELD_PERIOD_US`). No rendering legitimately changes faster: the
/// fastest flash phase lasts a twelfth of a second (the engine's decision
/// E.48), and a page answering keys is pushed twenty times a second
/// (`interaction.md` section 2). A decoder sending faster has its frames
/// held, the newest kept, so it cannot fill the interface's queue.
pub const MIN_FRAME_INTERVAL: Duration = Duration::from_micros(FIELD_PERIOD_US);

/// A frame: its palette, screen colour and indices.
struct Image {
    palette: Vec<[u8; 3]>,
    screen: u8,
    indices: Vec<u8>,
}

/// A frame sent: its version, the image it showed, and when.
struct Sent {
    version: u32,
    image: Image,
    at: Instant,
}

/// The frames sent of each visible page, and the newest held of each that
/// came too soon after the last.
#[derive(Default)]
pub struct Frames {
    visible: Vec<PageKey>,
    /// The frame last sent for each visible page.
    sent: BTreeMap<PageKey, Sent>,
    /// The newest frame of a page that came within [`MIN_FRAME_INTERVAL`]
    /// of the last sent.
    held: BTreeMap<PageKey, Image>,
}

impl Frames {
    /// Records the pages now visible.
    pub fn set_visible(&mut self, visible: Vec<PageKey>) {
        // The interface drops images of pages it stops showing, so a page
        // shown again must be sent again, whole.
        self.sent.retain(|key, _| visible.contains(key));
        self.held.retain(|key, _| visible.contains(key));
        self.visible = visible;
    }

    /// The frame to send now, when one should be: a visible page's frame
    /// that changed, unless the page's last was sent within
    /// [`MIN_FRAME_INTERVAL`], when it is held, replacing any held before,
    /// until [`Frames::due`]. The decoder has already checked shape and
    /// indices; the protocol reader checked them again on arrival. The
    /// frame is kept as given, to compare the next with and to encode it
    /// against; an unchanged frame costs the comparison and nothing else.
    pub fn accept(&mut self, key: PageKey, palette: Vec<[u8; 3]>, screen: u8, indices: Vec<u8>, now: Instant) -> Option<(u32, Vec<u8>)> {
        if !self.visible.contains(&key) {
            return None;
        }
        let image = Image { palette, screen, indices };
        let last = self.sent.get(&key);
        if last.is_some_and(|last| last.image.palette == image.palette && last.image.screen == image.screen && last.image.indices == image.indices) {
            // The page is back to what was sent: nothing held is wanted.
            self.held.remove(&key);
            return None;
        }
        if last.is_some_and(|last| now.saturating_duration_since(last.at) < MIN_FRAME_INTERVAL) {
            self.held.insert(key, image);
            return None;
        }
        self.held.remove(&key);
        self.send(key, image, now)
    }

    /// When the first held frame falls due.
    #[must_use]
    pub fn next_due(&self) -> Option<Instant> {
        self.held.keys().filter_map(|key| self.sent.get(key)).filter_map(|sent| sent.at.checked_add(MIN_FRAME_INTERVAL)).min()
    }

    /// The held frames whose interval has passed, each with its page and
    /// version, encoded against the last frame sent for its page.
    pub fn due(&mut self, now: Instant) -> Vec<(PageKey, u32, Vec<u8>)> {
        let due: Vec<PageKey> = self
            .held
            .keys()
            .copied()
            .filter(|key| self.sent.get(key).is_none_or(|sent| now.saturating_duration_since(sent.at) >= MIN_FRAME_INTERVAL))
            .collect();
        due.into_iter()
            .filter_map(|key| self.held.remove(&key).and_then(|image| self.send(key, image, now)).map(|(version, image)| (key, version, image)))
            .collect()
    }

    fn send(&mut self, key: PageKey, image: Image, now: Instant) -> Option<(u32, Vec<u8>)> {
        let last = self.sent.get(&key);
        let version = last.map_or(0, |last| last.version.wrapping_add(1));
        let encoded = frame::encode(&image.palette, image.screen, &image.indices, last.map(|last| (last.version, last.image.indices.as_slice())))?;
        self.sent.insert(key, Sent { version, image, at: now });
        Some((version, encoded))
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, reason = "a test's clock and counts")]
mod tests {
    use super::*;
    use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};

    const KEY: PageKey = PageKey { channel: 1, page: 0x100 };
    const DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
    /// Past every interval.
    const LATER: Duration = Duration::from_secs(1);

    fn page(fill: u8) -> (Vec<[u8; 3]>, Vec<u8>) {
        (vec![[0, 0, 0], [255, 255, 255]], vec![fill; DOTS])
    }

    /// A clock for a test: each reading a second after the one before.
    struct Clock(Instant);

    impl Clock {
        fn new() -> Self {
            Self(Instant::now())
        }

        fn next(&mut self) -> Instant {
            self.0 += LATER;
            self.0
        }
    }

    /// The interface's side: reads each image against the one it holds.
    fn read(image: &[u8], held: Option<(u32, &[u8])>) -> Vec<u8> {
        frame::decode(image, held).unwrap().indices
    }

    #[test]
    fn only_visible_and_changed_frames_are_sent() {
        let mut frames = Frames::default();
        let mut clock = Clock::new();
        let (palette, black) = page(0);
        assert!(frames.accept(KEY, palette.clone(), 0, black.clone(), clock.next()).is_none(), "not visible");
        frames.set_visible(vec![KEY]);
        assert_eq!(frames.accept(KEY, palette.clone(), 0, black.clone(), clock.next()).map(|(version, _)| version), Some(0));
        assert!(frames.accept(KEY, palette.clone(), 0, black, clock.next()).is_none(), "unchanged");
        let (_, white) = page(1);
        assert_eq!(frames.accept(KEY, palette.clone(), 0, white.clone(), clock.next()).map(|(version, _)| version), Some(1), "flash changed it");
        let other = vec![[0, 0, 0], [255, 0, 0]];
        assert_eq!(frames.accept(KEY, other, 0, white, clock.next()).map(|(version, _)| version), Some(2), "a new palette changed it");
    }

    #[test]
    fn a_page_shown_again_is_sent_again() {
        let mut frames = Frames::default();
        let mut clock = Clock::new();
        let (palette, black) = page(0);
        frames.set_visible(vec![KEY]);
        assert!(frames.accept(KEY, palette.clone(), 0, black.clone(), clock.next()).is_some());
        frames.set_visible(Vec::new());
        frames.set_visible(vec![KEY]);
        let (_, image) = frames.accept(KEY, palette, 0, black, clock.next()).unwrap();
        assert_eq!(frame::base(&image), None, "whole, as the interface dropped it");
    }

    /// Decision 61: the first frame of a page stands alone; the next is
    /// sent as its changes against the first, which the interface holds,
    /// and a new screen colour is a new frame.
    #[test]
    fn a_frame_is_sent_as_its_changes_against_the_last() {
        let mut frames = Frames::default();
        let mut clock = Clock::new();
        frames.set_visible(vec![KEY]);
        let (palette, black) = page(0);
        let (version, first) = frames.accept(KEY, palette.clone(), 0, black.clone(), clock.next()).unwrap();
        assert_eq!(frame::base(&first), None);
        assert_eq!(read(&first, None), black);
        let mut flashed = black.clone();
        flashed[100..140].fill(1);
        let (next, image) = frames.accept(KEY, palette.clone(), 0, flashed.clone(), clock.next()).unwrap();
        assert_eq!(frame::base(&image), Some(version));
        assert!(image.len() < 128, "forty changed dots: {} bytes", image.len());
        assert_eq!(read(&image, Some((version, &black))), flashed);
        let (_, screen) = frames.accept(KEY, palette, 1, flashed.clone(), clock.next()).unwrap();
        assert_eq!(frame::decode(&screen, Some((next, &flashed))).unwrap().screen, 1);
    }

    /// Finding F-11: a decoder alternating two images for one visible page
    /// a thousand times a second, for a second, gets at most one frame a
    /// field through, the newest held frame sent when its interval is up,
    /// and the last image is the one the interface ends with.
    #[test]
    fn a_page_s_frames_are_sent_at_most_once_a_field() {
        const SENDS: u32 = 1_000;
        const SPACING: Duration = Duration::from_millis(1);
        let mut frames = Frames::default();
        frames.set_visible(vec![KEY]);
        let (palette, black) = page(0);
        let (_, white) = page(1);
        let start = Instant::now();
        let mut sent = 0_u32;
        let mut last = None;
        for step in 0..SENDS {
            let now = start + SPACING * step;
            let image = if step % 2 == 0 { black.clone() } else { white.clone() };
            if let Some((version, _)) = frames.accept(KEY, palette.clone(), 0, image, now) {
                last = Some(version);
                sent += 1;
            }
            if frames.next_due().is_some_and(|due| due <= now) {
                for (key, version, _) in frames.due(now) {
                    assert_eq!(key, KEY);
                    last = Some(version);
                    sent += 1;
                }
            }
        }
        let end = start + SPACING * SENDS + MIN_FRAME_INTERVAL;
        for (_, version, _) in frames.due(end) {
            last = Some(version);
            sent += 1;
        }
        let most = (SPACING * SENDS).as_micros().div_ceil(MIN_FRAME_INTERVAL.as_micros()) + 1;
        assert!(u128::from(sent) <= most, "{sent} frames for one page in a second; at most {most}");
        assert!(sent > 1, "frames still go through");
        assert_eq!(last, Some(sent - 1), "every frame sent was the next version");
        assert_eq!(frames.next_due(), None, "the newest held frame was sent, and none waits");
    }
}
