//! Frame handling in the trusted core (`architecture.md` section 10, steps
//! 2, 3 and 6): only frames of pages the interface shows are passed on, a
//! frame identical to the last one sent for its page is never resent, a
//! page's frames are sent at most once a field on each of its clocks, and
//! each frame sent is encoded in `protocol::frame`'s smallest form,
//! against the last one sent for its page, which the interface holds
//! (decision 61).
//!
//! A page has three clocks (finding F-11, requirement 18.12, REQ-RCH-002
//! of `~/devel/neoteletext-common/RETURN-CHANNEL.md`): its content, the
//! look under the pointer, and the first frame after the viewer's input to
//! it. With one clock the channel's answer waited behind the frame of a
//! pressed look, or of the header's clock, sent a few milliseconds before
//! it; with three, a frame of one kind never waits for a frame of another,
//! and each kind still goes at most once a field.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use interface_core::canvas::{CELL_HEIGHT, CELL_WIDTH, COLUMNS, PAGE_WIDTH, PageKey, ROWS};
use neoteletext_core::consts::FIELD_PERIOD_US;
use protocol::frame;

/// The least time between two frames of one page on each of its clocks
/// (finding F-11): one field of the 50 a second a teletext display shows
/// (the engine's `FIELD_PERIOD_US`). No rendering legitimately changes
/// faster: the fastest flash phase lasts a twelfth of a second (the
/// engine's decision E.48), and a page answering keys is pushed twenty
/// times a second (`interaction.md` section 2). A decoder sending faster
/// has its frames held, the newest kept, so it cannot fill the interface's
/// queue: at most [`CLOCKS`] frames of a page a field.
pub const MIN_FRAME_INTERVAL: Duration = Duration::from_micros(FIELD_PERIOD_US);

/// The clocks of a page, each letting one frame go a field: its content,
/// the look under the pointer, and the answer to an input.
pub const CLOCKS: u32 = 3;

/// The cells of a page whose look under the pointer was asked for, a bit
/// a column in a word a row.
type Cells = [u64; ROWS as usize];
// A row's columns fit its word.
const _: () = assert!(COLUMNS as u32 <= u64::BITS);

/// A frame: its palette, screen colour and indices.
struct Image {
    palette: Vec<[u8; 3]>,
    screen: u8,
    indices: Vec<u8>,
}

/// What a frame changed against the last one sent for its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// The page itself: its content, its flash, its clock.
    Content,
    /// Only cells whose look under the pointer was asked for: the look,
    /// hovered or pressed, which counts nothing towards the content's
    /// interval (REQ-RCH-002).
    Look,
}

/// A frame sent: its version, the image it showed, and when each clock
/// last let a frame go.
struct Sent {
    version: u32,
    image: Image,
    /// The last frame of the page's content.
    at: Instant,
    /// The last frame of only the look under the pointer.
    look_at: Option<Instant>,
    /// The last frame the input's clock let go.
    input_at: Option<Instant>,
}

impl Sent {
    /// When the clock of frames of `kind` last let one go; never, for a
    /// page's first look.
    fn clock(&self, kind: Kind) -> Option<Instant> {
        match kind {
            Kind::Content => Some(self.at),
            Kind::Look => self.look_at,
        }
    }
}

/// A frame held until a clock lets it go.
struct Held {
    image: Image,
    kind: Kind,
    /// It came after an input to its page that no frame has answered, so
    /// the input's clock may let it go.
    after_input: bool,
}

/// The frames sent of each visible page, and the newest held of each that
/// came too soon after the last.
#[derive(Default)]
pub struct Frames {
    visible: Vec<PageKey>,
    /// The frame last sent for each visible page.
    sent: BTreeMap<PageKey, Sent>,
    /// The newest frame of a page that came too soon on its clocks.
    held: BTreeMap<PageKey, Held>,
    /// The visible pages the viewer gave input to that no frame has
    /// answered yet.
    inputs: BTreeSet<PageKey>,
    /// The cells of each visible page whose look was asked for: a frame
    /// that changes nothing else is the look's.
    looks: BTreeMap<PageKey, Cells>,
}

/// Whether a clock that last let a frame go at `at`, or never, lets one go
/// at `now`.
fn open(at: Option<Instant>, now: Instant) -> bool {
    at.is_none_or(|at| now.saturating_duration_since(at) >= MIN_FRAME_INTERVAL)
}

/// When a clock that last let a frame go at `at` lets the next go; at
/// `since`, already past, when it never has.
fn opens(at: Option<Instant>, since: Instant) -> Option<Instant> {
    at.map_or(Some(since), |at| at.checked_add(MIN_FRAME_INTERVAL))
}

impl Frames {
    /// Records the pages now visible.
    pub fn set_visible(&mut self, visible: Vec<PageKey>) {
        // The interface drops images of pages it stops showing, so a page
        // shown again must be sent again, whole.
        self.sent.retain(|key, _| visible.contains(key));
        self.held.retain(|key, _| visible.contains(key));
        self.inputs.retain(|key| visible.contains(key));
        self.looks.retain(|key, _| visible.contains(key));
        self.visible = visible;
    }

    /// The viewer's input went to the channel of `key`, the page open: the
    /// next frame of the page is its answer, which the content's clock
    /// does not hold (REQ-RCH-002). A frame held before the input is not
    /// an answer and keeps its clock.
    pub fn input(&mut self, key: PageKey) {
        if self.visible.contains(&key) {
            self.inputs.insert(key);
        }
    }

    /// The decoder of `key`'s channel was told to draw the look under the
    /// pointer on `area`, or on none: a frame that changes only cells of
    /// the looks asked for is the look's. The cells are kept while the
    /// page is visible, so a look's frame that comes after the next look
    /// is asked for is still the look's; a change of content that falls
    /// only within them is then paced as a look, still once a field.
    #[cfg(feature = "links")]
    pub fn looked(&mut self, key: PageKey, area: Option<interface_core::links::Area>) {
        let (Some(area), true) = (area, self.visible.contains(&key)) else { return };
        let cells = self.looks.entry(key).or_insert([0; ROWS as usize]);
        for (row, word) in (0..u8::MAX).zip(cells.iter_mut()) {
            for column in (0..u8::MAX).take(usize::from(COLUMNS)) {
                if area.contains(column, row) {
                    *word |= 1_u64.checked_shl(u32::from(column)).unwrap_or(0);
                }
            }
        }
    }

    /// The frame to send now, when one should be: a visible page's frame
    /// that changed, unless its clock let one go within
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
        self.held.remove(&key);
        let Some(last) = self.sent.get(&key) else {
            return self.send(key, image, Kind::Content, false, now);
        };
        if last.image.palette == image.palette && last.image.screen == image.screen && last.image.indices == image.indices {
            // The page is back to what was sent: nothing held is wanted.
            return None;
        }
        let kind = self.kind(key, &last.image, &image);
        let after_input = self.inputs.contains(&key);
        let by_own = open(last.clock(kind), now);
        let by_input = !by_own && after_input && open(last.input_at, now);
        if !by_own && !by_input {
            self.held.insert(key, Held { image, kind, after_input });
            return None;
        }
        self.send(key, image, kind, by_input, now)
    }

    /// What `image` changed against `last`, the last frame sent for `key`.
    fn kind(&self, key: PageKey, last: &Image, image: &Image) -> Kind {
        let Some(cells) = self.looks.get(&key) else { return Kind::Content };
        if last.palette != image.palette || last.screen != image.screen || last.indices.len() != image.indices.len() {
            return Kind::Content;
        }
        let (width, cell_width, cell_height) = (usize::from(PAGE_WIDTH), usize::from(CELL_WIDTH), usize::from(CELL_HEIGHT));
        let looked = |dot: usize| {
            let row = dot.checked_div(width).and_then(|line| line.checked_div(cell_height));
            let column = dot.checked_rem(width).and_then(|x| x.checked_div(cell_width));
            let bit = column.and_then(|column| u32::try_from(column).ok()).and_then(|column| 1_u64.checked_shl(column)).unwrap_or(0);
            row.and_then(|row| cells.get(row)).is_some_and(|word| word & bit != 0)
        };
        let only_looks = last.indices.iter().zip(&image.indices).enumerate().all(|(dot, (before, after))| before == after || looked(dot));
        if only_looks { Kind::Look } else { Kind::Content }
    }

    /// When the first held frame falls due.
    #[must_use]
    pub fn next_due(&self) -> Option<Instant> {
        self.held.iter().filter_map(|(key, held)| self.due_at(*key, held)).min()
    }

    /// When the frame held for `key` may go: on its kind's clock, or on the
    /// input's when it answers one.
    fn due_at(&self, key: PageKey, held: &Held) -> Option<Instant> {
        let sent = self.sent.get(&key)?;
        let own = opens(sent.clock(held.kind), sent.at);
        let by_input = (held.after_input && self.inputs.contains(&key)).then(|| opens(sent.input_at, sent.at)).flatten();
        own.into_iter().chain(by_input).min()
    }

    /// The held frames whose interval has passed, each with its page and
    /// version, encoded against the last frame sent for its page.
    pub fn due(&mut self, now: Instant) -> Vec<(PageKey, u32, Vec<u8>)> {
        let due: Vec<PageKey> = self.held.iter().filter(|(key, held)| self.due_at(**key, held).is_none_or(|at| at <= now)).map(|(key, _)| *key).collect();
        let mut sent = Vec::new();
        for key in due {
            let Some(held) = self.held.remove(&key) else { continue };
            let by_input = self.sent.get(&key).is_some_and(|last| !open(last.clock(held.kind), now));
            if let Some((version, image)) = self.send(key, held.image, held.kind, by_input, now) {
                sent.push((key, version, image));
            }
        }
        sent
    }

    /// Sends `image` as `key`'s next frame, of `kind`, let go by the
    /// input's clock when `by_input`.
    fn send(&mut self, key: PageKey, image: Image, kind: Kind, by_input: bool, now: Instant) -> Option<(u32, Vec<u8>)> {
        let last = self.sent.get(&key);
        let version = last.map_or(0, |last| last.version.wrapping_add(1));
        let encoded = frame::encode(&image.palette, image.screen, &image.indices, last.map(|last| (last.version, last.image.indices.as_slice())))?;
        let (mut at, mut look_at, mut input_at) = last.map_or((now, None, None), |last| (last.at, last.look_at, last.input_at));
        if by_input {
            input_at = Some(now);
        } else {
            match kind {
                Kind::Content => at = now,
                Kind::Look => look_at = Some(now),
            }
        }
        // A look's frame is not the answer: the input stays unanswered
        // until a frame of content goes.
        if kind == Kind::Content || by_input {
            self.inputs.remove(&key);
        }
        self.sent.insert(key, Sent { version, image, at, look_at, input_at });
        Some((version, encoded))
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, reason = "a test's clock and counts")]
mod tests {
    use super::*;
    use interface_core::canvas::PAGE_HEIGHT;

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

    /// A millisecond: how soon the channel's answer follows an input on
    /// the local server (`RETURN-CHANNEL.md` section 2.1).
    const MILLISECOND: Duration = Duration::from_millis(1);

    /// `image` with the cell at `column`, `row` filled with `index`.
    fn painted(image: &[u8], column: usize, row: usize, index: u8) -> Vec<u8> {
        let mut painted = image.to_vec();
        let (width, cell_width, cell_height) = (usize::from(PAGE_WIDTH), usize::from(CELL_WIDTH), usize::from(CELL_HEIGHT));
        for line in row * cell_height..(row + 1) * cell_height {
            painted[line * width + column * cell_width..line * width + (column + 1) * cell_width].fill(index);
        }
        painted
    }

    /// A button of four cells on row 20, as a control would be.
    #[cfg(feature = "links")]
    const BUTTON: interface_core::links::Area = interface_core::links::Area { row: 20, column: 2, width: 4, height: 1 };

    /// REQ-RCH-002: a click on a control, its pressed look drawn, then the
    /// channel's answer within a millisecond: the look and the answer are
    /// each sent at once. With one clock the look's frame held the answer
    /// for a field.
    #[cfg(feature = "links")]
    #[test]
    fn a_click_on_a_control_its_look_then_the_answer_goes_at_once() {
        let mut frames = Frames::default();
        frames.set_visible(vec![KEY]);
        let (palette, black) = page(0);
        let start = Instant::now();
        assert!(frames.accept(KEY, palette.clone(), 0, black.clone(), start).is_some());
        // The press: its look asked for, and the click sent to the channel.
        let pressed_at = start + LATER;
        frames.looked(KEY, Some(BUTTON));
        frames.input(KEY);
        let pressed = painted(&black, 3, 20, 1);
        assert!(frames.accept(KEY, palette.clone(), 0, pressed.clone(), pressed_at).is_some(), "the pressed look goes at once");
        // The channel's answer, a line of text elsewhere on the page.
        let answer = painted(&pressed, 10, 5, 1);
        assert!(frames.accept(KEY, palette, 0, answer, pressed_at + MILLISECOND).is_some(), "the answer goes at once, not a field after the look");
        assert_eq!(frames.next_due(), None, "nothing held");
    }

    /// REQ-RCH-002: the first frame after an input is not held by the
    /// content's clock, here the header's clock drawn five milliseconds
    /// before the key; the frame after it is.
    #[test]
    fn the_first_frame_after_an_input_is_not_held() {
        let mut frames = Frames::default();
        frames.set_visible(vec![KEY]);
        let (palette, black) = page(0);
        let start = Instant::now();
        assert!(frames.accept(KEY, palette.clone(), 0, black.clone(), start).is_some());
        let ticked_at = start + LATER;
        let ticked = painted(&black, 39, 0, 1);
        assert!(frames.accept(KEY, palette.clone(), 0, ticked.clone(), ticked_at).is_some(), "the header's clock");
        let typed_at = ticked_at + 5 * MILLISECOND;
        frames.input(KEY);
        let answer = painted(&ticked, 1, 5, 1);
        assert!(frames.accept(KEY, palette.clone(), 0, answer.clone(), typed_at + MILLISECOND).is_some(), "the answer goes at once");
        // A frame no input asked for is held by the content's clock.
        let flashed = painted(&answer, 20, 10, 1);
        assert!(frames.accept(KEY, palette, 0, flashed, typed_at + 2 * MILLISECOND).is_none(), "held");
        assert_eq!(frames.next_due(), Some(ticked_at + MIN_FRAME_INTERVAL), "on the content's clock");
        assert_eq!(frames.due(ticked_at + MIN_FRAME_INTERVAL).len(), 1);
    }

    /// A frame held before an input is not its answer: the input does not
    /// let it go early, and the answer that replaces it goes at once.
    #[test]
    fn a_frame_held_before_an_input_keeps_its_clock() {
        let mut frames = Frames::default();
        frames.set_visible(vec![KEY]);
        let (palette, black) = page(0);
        let start = Instant::now();
        assert!(frames.accept(KEY, palette.clone(), 0, black.clone(), start).is_some());
        let ticked = painted(&black, 39, 0, 1);
        assert!(frames.accept(KEY, palette.clone(), 0, ticked.clone(), start + 5 * MILLISECOND).is_none(), "held");
        frames.input(KEY);
        assert_eq!(frames.next_due(), Some(start + MIN_FRAME_INTERVAL), "the input does not hurry it");
        assert_eq!(frames.due(start + 6 * MILLISECOND), Vec::new(), "not yet due");
        let answer = painted(&ticked, 1, 5, 1);
        assert!(frames.accept(KEY, palette, 0, answer, start + 7 * MILLISECOND).is_some(), "the answer, with the tick, goes at once");
        assert_eq!(frames.next_due(), None);
    }

    /// Finding F-11 with every clock in use: a decoder alternating images
    /// a thousand times a second, some changing only the looked cells and
    /// some the content, while the interface asks for a look and sends an
    /// input every millisecond, gets at most [`CLOCKS`] frames of the page
    /// through in any field, and the interface ends with the last image.
    #[cfg(feature = "links")]
    #[test]
    fn inputs_and_looks_keep_a_page_to_its_clocks() {
        const SENDS: u32 = 1_000;
        const SPACING: Duration = Duration::from_millis(1);
        let mut frames = Frames::default();
        frames.set_visible(vec![KEY]);
        let (palette, black) = page(0);
        let images = [painted(&black, 3, 20, 1), painted(&black, 10, 5, 1), painted(&black, 4, 20, 1), painted(&black, 11, 6, 1)];
        let start = Instant::now();
        let mut times = Vec::new();
        let mut last = None;
        assert!(frames.accept(KEY, palette.clone(), 0, black, start).is_some());
        for step in 1..=SENDS {
            let now = start + SPACING * step;
            frames.looked(KEY, Some(BUTTON));
            frames.input(KEY);
            let image = images[usize::try_from(step).unwrap() % images.len()].clone();
            let mut sent: Vec<u32> = frames.accept(KEY, palette.clone(), 0, image, now).map(|(version, _)| version).into_iter().collect();
            if frames.next_due().is_some_and(|due| due <= now) {
                sent.extend(frames.due(now).into_iter().map(|(_, version, _)| version));
            }
            for version in sent {
                times.push(now);
                last = Some(version);
            }
        }
        let end = start + SPACING * SENDS + MIN_FRAME_INTERVAL;
        for (_, version, _) in frames.due(end) {
            times.push(end);
            last = Some(version);
        }
        let clocks = usize::try_from(CLOCKS).unwrap();
        for (index, &at) in times.iter().enumerate() {
            let within = times[index..].iter().take_while(|&&later| later < at + MIN_FRAME_INTERVAL).count();
            assert!(within <= clocks, "{within} frames in the field from {:?}", at - start);
        }
        assert!(times.len() > clocks, "frames still go through: {}", times.len());
        assert_eq!(last, u32::try_from(times.len()).ok(), "every frame sent was the next version");
        assert_eq!(frames.next_due(), None, "the newest held frame was sent, and none waits");
    }
}
