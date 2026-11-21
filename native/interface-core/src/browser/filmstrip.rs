//! The tab filmstrip (REQ-6640, the owner's DEC-6637): a rest on the tab
//! strip slides out, from under it, one row of still pictures of every tab
//! of the window, each the tab's page as the viewer last saw it, captioned
//! with its title and page number. The wheel slides the row with easing,
//! as far as bringing its end picture all the way across the window; a
//! press on a picture chooses its tab, and the strip slides to show it. It
//! replaces the overflow panel of decision 102.
//!
//! Everything it does is one small state, [`Filmstrip`]: whether it is
//! shut, waiting out the rest that opens it, open, or waiting out the
//! pointer's absence that closes it; how far it has slid out; where the
//! row is slid to, eased; the picture the pointer rests on; and the tab
//! strip's own slide. The pointer's moves and presses, the wheel, the
//! timers and the drawing all read and change only it (REQ-6640 item 11).

use alloc::vec::Vec;

use crate::browser::Browser;
use crate::browser::output::Output;
use crate::browser::tab_strip::STRIP_HEIGHT;
use crate::canvas::{Canvas, MAX_STILLS, PageKey};
use crate::geometry::{Point, Rect};
use crate::tween::{EASE_OUT, STANDARD, Tween};

mod draw;

/// How long the pointer rests on the tabs before the filmstrip opens
/// (DEC-6637 item 1): the overflow panel's wait (decision 102), for its
/// reason, that a hand crossing the strip to the toolbar takes about a
/// tenth of a second and must not open anything in its way.
pub(super) const FILMSTRIP_DELAY_MILLISECONDS: u64 = 300;
/// How long the pointer is away from the strip and the filmstrip before
/// it closes (DEC-6637 item 6): long enough for a hand that slips off its
/// edge and comes back.
pub(super) const FILMSTRIP_CLOSE_MILLISECONDS: u64 = 300;
/// How long the rested tab's picture shows alone, directly under its tab,
/// before the rest of the row appears (REQ-6921, the owner's "500ms
/// delay", DEC-6920).
pub(super) const FILMSTRIP_ALONE_MILLISECONDS: u64 = 500;
/// How long it takes to slide out from under the strip, and back: about
/// as quick as a menu opens, so it never keeps the viewer waiting.
const FILMSTRIP_OPEN_MILLISECONDS: u64 = 200;
/// How long a slide by the wheel, and the strip's slide to a tab, take to
/// settle: the page area's slide between pages.
const FILMSTRIP_SLIDE_MILLISECONDS: u64 = crate::wall::SLIDE_MILLISECONDS;

/// Where the filmstrip is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Not shown.
    Shut,
    /// The pointer has rested on the tabs since this moment.
    Resting(u64),
    /// Shown, the pointer on the strip or the filmstrip.
    Open,
    /// Shown, the pointer away from both since this moment.
    Leaving(u64),
    /// Put away by Escape or the count: not shown, and not opened again
    /// by a rest until the pointer has left the strip, as the overflow
    /// panel did (decision 102).
    Dismissed,
}

/// The first stage of a filmstrip opened by a rest on a tab (REQ-6921):
/// that tab's picture shows alone, directly under it, until the rest of
/// the row appears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Alone {
    /// The tab whose picture shows: the one the pointer is over.
    pub(super) tab: usize,
    /// When the picture appeared: the rest appear
    /// [`FILMSTRIP_ALONE_MILLISECONDS`] later, the wait not restarted by a
    /// move to another tab (DEC-6920 item 5).
    since: u64,
}

/// A picture as last drawn, and its buttons.
#[derive(Clone, Copy, Debug)]
pub(super) struct Picture {
    /// The tab's place.
    pub(super) index: usize,
    /// The picture with its caption: what a press chooses.
    pub(super) rect: Rect,
    /// The close button, shown while the pointer rests on the picture.
    pub(super) close: Rect,
    /// The mute button beside it.
    pub(super) speaker: Rect,
}

/// A tab's still, as the interface knows it (REQ-6640 item 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Still {
    /// The tab's channel, which numbers its still in the canvas.
    pub(super) channel: u32,
    /// The page it shows.
    pub(super) page: u16,
    /// The page is drawn as a ReGIS picture: its stand-in is black, in the
    /// picture's shape. A NAPLPS picture is the teletext raster (REQ-5819),
    /// so its page stands in as a teletext page does, in the page's shape.
    pub(super) picture: bool,
    /// The canvas made it; else the stand-in shows.
    pub(super) kept: bool,
    /// Made from a snapshot the trusted core sent (REQ-6883), not as seen
    /// this run: a still as seen replaces it, never the other way.
    pub(super) snapshot: bool,
}

/// The filmstrip's one state (REQ-6640 item 11).
pub(super) struct Filmstrip {
    phase: Phase,
    /// How far it has slid out, from 0 (under the strip) to 1.
    reveal: Tween,
    /// How far the row is slid from its place at the window's left
    /// margin, in points: negative to the left.
    slide: Tween,
    /// How far the tab strip's row of tabs is slid to the left, in points
    /// (REQ-6640 item 8).
    pub(super) strip: Tween,
    /// The active tab when the strip was last drawn: the strip slides when
    /// it changes, and is placed at once otherwise, as when the window
    /// opens or is resized.
    pub(super) strip_active: Option<usize>,
    /// The picture the pointer rests on.
    pub(super) hovered: Option<usize>,
    /// The tab whose picture shows alone, while the filmstrip opened by a
    /// rest on it is in its first stage (REQ-6921).
    pub(super) alone: Option<Alone>,
    /// The rest that opened the filmstrip was on the overflow count, and
    /// the pointer has stayed on the count since. A press there is then
    /// the press the viewer was making when the rest opened it first, and
    /// keeps it open rather than shutting it (REQ-6640 item 1; TODO-6758).
    rested_on_count: bool,
    /// The pictures as last drawn, and the panel they are on.
    pub(super) pictures: Vec<Picture>,
    pub(super) panel: Rect,
    /// The window's width, and the slide's least and most, as last laid
    /// out.
    width: f64,
    bounds: (f64, f64),
    /// One picture and its gap, as last laid out: a notch of the wheel
    /// slides by one.
    step: f64,
    /// The stills, the one taken longest ago first, at most
    /// [`MAX_STILLS`].
    pub(super) stills: Vec<Still>,
    /// Stills to take at the next frame, from what the canvas holds then:
    /// the tab's channel, the page, and whether it is a picture.
    pending: Vec<(u32, PageKey, bool)>,
    /// Stills for the canvas to forget at the next frame.
    forget: Vec<u32>,
    /// The tabs last asked a thumbnail for (REQ-6883), while any were.
    pub(super) asked: Vec<u32>,
}

impl Default for Filmstrip {
    fn default() -> Self {
        Self {
            phase: Phase::Shut,
            reveal: Tween::new(0.0, FILMSTRIP_OPEN_MILLISECONDS, EASE_OUT),
            slide: Tween::new(0.0, FILMSTRIP_SLIDE_MILLISECONDS, STANDARD),
            strip: Tween::new(0.0, FILMSTRIP_SLIDE_MILLISECONDS, STANDARD),
            strip_active: None,
            hovered: None,
            alone: None,
            rested_on_count: false,
            pictures: Vec::new(),
            panel: Rect::ZERO,
            width: 0.0,
            bounds: (0.0, 0.0),
            step: 0.0,
            stills: Vec::new(),
            pending: Vec::new(),
            forget: Vec::new(),
            asked: Vec::new(),
        }
    }
}

impl Filmstrip {
    /// Shown, or waiting out the pointer's absence.
    const fn shown(&self) -> bool {
        matches!(self.phase, Phase::Open | Phase::Leaving(_))
    }

    /// A tab's still, if one was taken.
    pub(super) fn still(&self, channel: u32) -> Option<&Still> {
        self.stills.iter().find(|still| still.channel == channel)
    }
}

/// A number held within `f32`, for the tweens.
#[expect(clippy::cast_possible_truncation, reason = "points across a window, far within f32")]
fn narrow(value: f64) -> f32 {
    value as f32
}

impl Browser {
    /// Whether the filmstrip is open: shown, or waiting out the pointer's
    /// absence before it closes.
    #[must_use]
    pub fn filmstrip_open(&self) -> bool {
        self.filmstrip.shown()
    }

    /// Whether the filmstrip may open: not while the address field has the
    /// keys (TODO-7014, the owner's "if the url bar has focus then
    /// disable the tabs dropdown"), so that a pointer passing over the
    /// tabs while an address is typed lays no row of pictures over the
    /// field and its suggestions.
    pub(super) const fn filmstrip_available(&self) -> bool {
        !self.editing
    }

    /// The address field took the keys: an open filmstrip slides away, and
    /// a rest waiting to open it opens nothing.
    pub(super) fn filmstrip_withdraw(&mut self) {
        match self.filmstrip.phase {
            Phase::Open | Phase::Leaving(_) => self.filmstrip_hide(self.drawn_at),
            Phase::Resting(_) => self.filmstrip.phase = Phase::Shut,
            Phase::Shut | Phase::Dismissed => {}
        }
    }

    /// Every move takes its time, or none with reduced motion (REQ-754).
    fn filmstrip_motion(&mut self) {
        let reduced = self.wall.reduced_motion();
        let (open, slide) = if reduced { (0, 0) } else { (FILMSTRIP_OPEN_MILLISECONDS, FILMSTRIP_SLIDE_MILLISECONDS) };
        self.filmstrip.reveal.set_duration(open);
        self.filmstrip.slide.set_duration(slide);
        self.filmstrip.strip.set_duration(slide);
    }

    /// The page a tab's still shows: the page open in the page area, else
    /// the one chosen in the list, else the first the wall shows.
    fn still_page(&self) -> Option<PageKey> {
        self.wall.zoomed().or_else(|| self.selected.and_then(|index| self.page_key(index))).or_else(|| self.wall.shown().next().map(|(key, _)| key))
    }

    /// A still of the tab of `channel`, the window's channel now, is taken
    /// at the next frame.
    fn queue_still(&mut self, channel: u32) {
        if self.channel.channel != channel {
            return;
        }
        let Some(key) = self.still_page() else { return };
        let picture = self.picture_language(key) == Some(crate::canvas::PictureLanguage::Regis);
        self.filmstrip.pending.retain(|&(queued, _, _)| queued != channel);
        self.filmstrip.pending.push((channel, key, picture));
    }

    /// The tabs changed: the tab left, if the active tab changed, has its
    /// still taken (REQ-6640 item 3), and the stills of tabs that have
    /// closed are forgotten.
    pub(super) fn filmstrip_tabs_changed(&mut self, left: Option<u32>) {
        let open: Vec<u32> = self.tabs.iter().map(|tab| tab.channel).collect();
        // A tab left because it closed keeps nothing.
        if let Some(channel) = left.filter(|channel| open.contains(channel)) {
            self.queue_still(channel);
        }
        let gone: Vec<u32> = self.filmstrip.stills.iter().map(|still| still.channel).filter(|channel| !open.contains(channel)).collect();
        self.filmstrip.stills.retain(|still| open.contains(&still.channel));
        self.filmstrip.pending.retain(|(channel, _, _)| open.contains(channel));
        self.filmstrip.forget.extend(gone);
        if self.filmstrip.hovered.is_some_and(|index| index >= self.tabs.len()) {
            self.filmstrip.hovered = None;
        }
        // The tab shown alone has closed: the whole row shows.
        if self.filmstrip.alone.is_some_and(|alone| alone.tab >= self.tabs.len()) {
            self.filmstrip.alone = None;
        }
    }

    /// The stills waiting are taken, and those let go forgotten, from
    /// what the canvas holds now: before anything of the frame replaces
    /// the pages left.
    pub(super) fn take_stills<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        for channel in core::mem::take(&mut self.filmstrip.forget) {
            canvas.forget_still(channel);
        }
        for (channel, key, picture) in core::mem::take(&mut self.filmstrip.pending) {
            let kept = canvas.keep_still(channel, key);
            self.filmstrip.stills.retain(|still| still.channel != channel);
            self.filmstrip.stills.push(Still { channel, page: key.page, picture, kept, snapshot: false });
            // Past the cap the still taken longest ago goes.
            while self.filmstrip.stills.len() > MAX_STILLS {
                let oldest = self.filmstrip.stills.remove(0);
                canvas.forget_still(oldest.channel);
            }
        }
    }

    /// Whether a thumbnail of `page`'s `sub` is still wanted for the tab of
    /// `channel` (REQ-6883): the tab is the window's, its address names
    /// that place or none, and it has no still as seen this run.
    #[must_use]
    pub fn thumbnail_wanted(&self, channel: u32, page: u16, sub: u16) -> bool {
        let Some(tab) = self.tabs.iter().find(|tab| tab.channel == channel) else { return false };
        let place = tab.page.is_none_or(|named| named == page) && tab.subpage.is_none_or(|named| named == sub);
        place && !self.filmstrip.still(channel).is_some_and(|still| still.kept && !still.snapshot)
    }

    /// The host made a still of the tab of `channel` from a thumbnail of
    /// `page` (REQ-6883), kept by the canvas when `kept`; a ReGIS picture
    /// when `picture`. It counts within `MAX_STILLS`, the one taken longest
    /// ago going first.
    pub fn thumbnail_kept(&mut self, channel: u32, page: u16, picture: bool, kept: bool) {
        self.filmstrip.stills.retain(|still| still.channel != channel);
        self.filmstrip.stills.push(Still { channel, page, picture, kept, snapshot: true });
        while self.filmstrip.stills.len() > MAX_STILLS {
            let oldest = self.filmstrip.stills.remove(0);
            self.filmstrip.forget.push(oldest.channel);
        }
    }

    /// Asks the trusted core for pictures of the tabs in view without
    /// stills (REQ-6883): when the filmstrip has opened, its slide has
    /// settled, or its tabs changed while open, each time the tabs asked
    /// for change; and once, empty, when it closes.
    pub(super) fn ask_thumbnails(&mut self, now: u64) {
        let open = self.filmstrip.shown();
        if !open {
            if !self.filmstrip.asked.is_empty() {
                self.filmstrip.asked.clear();
                self.outputs.push(Output::Thumbnails(Vec::new()));
            }
            return;
        }
        if self.filmstrip.slide.moving(now) || self.filmstrip.reveal.moving(now) {
            return;
        }
        let wanted: Vec<u32> = self
            .filmstrip
            .pictures
            .iter()
            .filter_map(|picture| self.tabs.get(picture.index))
            .map(|tab| tab.channel)
            // The window's tabs, which the trusted core sends at most the
            // protocol's MAX_TABS of, bound the request.
            .filter(|&channel| !self.filmstrip.still(channel).is_some_and(|still| still.kept))
            .collect();
        if wanted != self.filmstrip.asked {
            self.filmstrip.asked.clone_from(&wanted);
            self.outputs.push(Output::Thumbnails(wanted));
        }
    }

    /// Opens the filmstrip at `now`: the active tab's still is taken, and
    /// the row placed with the active tab's picture fully in view.
    fn filmstrip_show(&mut self, now: u64) {
        self.filmstrip_motion();
        if let Some(channel) = self.tabs.get(self.active).map(|tab| tab.channel) {
            self.queue_still(channel);
        }
        self.filmstrip.phase = Phase::Open;
        self.filmstrip.rested_on_count = false;
        self.filmstrip.alone = None;
        self.filmstrip.reveal.set(1.0, now);
        let place = self.filmstrip_place();
        self.filmstrip.slide = Tween::new(narrow(place), self.filmstrip_slide_duration(), STANDARD);
        self.filmstrip.hovered = None;
    }

    pub(super) fn filmstrip_slide_duration(&self) -> u64 {
        if self.wall.reduced_motion() { 0 } else { FILMSTRIP_SLIDE_MILLISECONDS }
    }

    /// Where the row is placed as it opens (REQ-6640 item 6): at its start
    /// when the active tab's picture is fully in view there, else with
    /// that picture centred, within the slide's bounds.
    fn filmstrip_place(&self) -> f64 {
        let layout = draw::Layout::of(self.filmstrip.width, self.tabs.len(), self.wall.shape().height_per_width());
        #[allow(clippy::cast_precision_loss, reason = "a tab's place, below a hundred")]
        let left = draw::MARGIN + self.active as f64 * layout.step;
        if left + layout.picture <= self.filmstrip.width - draw::MARGIN {
            return 0.0;
        }
        let centred = self.filmstrip.width / 2.0 - (left + layout.picture / 2.0);
        centred.clamp(layout.bounds.0, layout.bounds.1)
    }

    /// The tab under `point` on the strip, if any.
    fn tab_under(&self, point: Point) -> Option<usize> {
        self.regions.tabs.iter().find(|region| region.tab.contains(point)).map(|region| region.index)
    }

    /// The rest of the row appears (REQ-6921 items 3 and 4): the row is
    /// laid out around the picture shown alone, slid so that it stays
    /// exactly where it is, and the others appear in place beside it. It
    /// is not eased to the row's usual place: the owner saw the picture
    /// already shown moved when the others appeared (DEC-6920 item 6). A
    /// row that runs off the window's edge stays so, for the wheel to
    /// slide (REQ-6640 item 6); the slide is always within its bounds,
    /// since the picture alone is kept within the window's margins.
    fn filmstrip_join(&mut self) {
        let Some(alone) = self.filmstrip.alone.take() else { return };
        self.filmstrip_motion();
        let layout = draw::Layout::of(self.filmstrip.width, self.tabs.len(), self.wall.shape().height_per_width());
        let around = self.alone_slide(&layout, alone.tab).unwrap_or_else(|| narrow(self.filmstrip_place()));
        self.filmstrip.slide = Tween::new(around, self.filmstrip_slide_duration(), STANDARD);
    }

    /// Closes the filmstrip at `now`: it slides back under the strip.
    fn filmstrip_hide(&mut self, now: u64) {
        self.filmstrip_motion();
        self.filmstrip.phase = Phase::Shut;
        self.filmstrip.rested_on_count = false;
        self.filmstrip.alone = None;
        self.filmstrip.reveal.set(0.0, now);
        self.filmstrip.hovered = None;
    }

    /// The strip is not on the screen: the filmstrip shuts at once, as it
    /// was never shown.
    pub(super) fn filmstrip_forget(&mut self) {
        self.filmstrip.phase = Phase::Shut;
        self.filmstrip.reveal = Tween::new(0.0, FILMSTRIP_OPEN_MILLISECONDS, EASE_OUT);
        self.filmstrip.hovered = None;
        self.filmstrip.alone = None;
        self.filmstrip.rested_on_count = false;
        self.filmstrip.pictures.clear();
        self.filmstrip.panel = Rect::ZERO;
    }

    /// The pointer moved to `point` at `now` (REQ-6640 items 1 and 9): a
    /// rest on the tabs starts the wait that opens the filmstrip, leaving
    /// the strip and the filmstrip the wait that closes it, and coming
    /// back ends that wait. A tab pressed or dragged opens nothing.
    /// Returns whether anything changed.
    pub(super) fn filmstrip_moved(&mut self, point: Point, now: u64) -> bool {
        let before = (self.filmstrip.phase, self.filmstrip.hovered, self.filmstrip.alone);
        let on_tabs = self.tab_strip.tabs_area.contains(point);
        let on_strip = Rect::new(0.0, 0.0, self.regions.strip.x1, STRIP_HEIGHT).contains(point);
        let on_filmstrip = self.filmstrip.panel.contains(point);
        // Unavailable, it rests as a press on a tab does: it never opens.
        let pressing = self.tab_strip_pressed() || !self.filmstrip_available();
        if !self.tab_strip.count.contains(point) {
            self.filmstrip.rested_on_count = false;
        }
        self.filmstrip.phase = match self.filmstrip.phase {
            Phase::Shut if on_tabs && !pressing => Phase::Resting(now),
            Phase::Resting(_) if !on_tabs || pressing => Phase::Shut,
            Phase::Open if !on_strip && !on_filmstrip => Phase::Leaving(now),
            Phase::Leaving(_) if on_strip || on_filmstrip => Phase::Open,
            Phase::Dismissed if !on_strip => Phase::Shut,
            unchanged => unchanged,
        };
        // REQ-6921 item 2: the picture shown alone follows the pointer to
        // another tab.
        if self.filmstrip.shown()
            && let Some(tab) = self.tab_under(point)
            && let Some(alone) = self.filmstrip.alone.as_mut()
        {
            alone.tab = tab;
        }
        self.filmstrip.hovered = if self.filmstrip.shown() {
            self.filmstrip.pictures.iter().find(|picture| picture.rect.contains(point)).map(|picture| picture.index)
        } else {
            None
        };
        before != (self.filmstrip.phase, self.filmstrip.hovered, self.filmstrip.alone)
    }

    /// How long until the filmstrip opens or closes, while it waits.
    pub(super) fn filmstrip_deadline(&self, now: u64) -> Option<u64> {
        let due = match self.filmstrip.phase {
            Phase::Resting(since) => since.saturating_add(FILMSTRIP_DELAY_MILLISECONDS),
            Phase::Leaving(since) => since.saturating_add(FILMSTRIP_CLOSE_MILLISECONDS),
            // The rest of the row appears only with the pointer present.
            Phase::Open => self.filmstrip.alone?.since.saturating_add(FILMSTRIP_ALONE_MILLISECONDS),
            Phase::Shut | Phase::Dismissed => return None,
        };
        Some(due.saturating_sub(now).max(1))
    }

    /// The filmstrip opens once the pointer has rested on the tabs, with
    /// the rested tab's picture alone when the rest was on a tab
    /// (REQ-6921), shows the rest of the row once that picture has shown
    /// long enough with the pointer present, and closes once the pointer
    /// has been away; whether any happened now.
    pub(super) fn filmstrip_settle(&mut self, now: u64) -> bool {
        match self.filmstrip.phase {
            Phase::Resting(since) if now.saturating_sub(since) >= FILMSTRIP_DELAY_MILLISECONDS => {
                self.filmstrip_show(now);
                self.filmstrip.rested_on_count = self.tab_strip.count.contains(self.pointer);
                self.filmstrip.alone = self.tab_under(self.pointer).map(|tab| Alone { tab, since: now });
            }
            Phase::Open if self.filmstrip.alone.is_some_and(|alone| now.saturating_sub(alone.since) >= FILMSTRIP_ALONE_MILLISECONDS) => {
                self.filmstrip_join();
            }
            Phase::Leaving(since) if now.saturating_sub(since) >= FILMSTRIP_CLOSE_MILLISECONDS => self.filmstrip_hide(now),
            Phase::Shut | Phase::Resting(_) | Phase::Open | Phase::Leaving(_) | Phase::Dismissed => return false,
        }
        true
    }

    /// Whether the filmstrip, its row or the strip's row is moving at `now`.
    pub(super) fn filmstrip_animating(&self, now: u64) -> bool {
        self.filmstrip.reveal.moving(now) || self.filmstrip.slide.moving(now) || self.filmstrip.strip.moving(now)
    }

    /// A press at `point` at `now` (REQ-6640 items 1 and 7): the overflow
    /// count opens the filmstrip at once, or closes it, except that a press
    /// on the count the rest has just opened it from keeps it open: a hand
    /// that comes to rest on the count to click it rests there longer than
    /// the delay, so the rest opens the filmstrip under it first, and the
    /// click would otherwise shut it again and keep it shut while the
    /// pointer stays on the strip, leaving only the count's tip (TODO-6758).
    /// A press on a tab ends the rest, which then opens nothing. On the open
    /// filmstrip a picture's close or mute button acts on its tab, and the
    /// picture chooses its tab and closes the filmstrip. Returns whether
    /// the press was the filmstrip's.
    pub(super) fn filmstrip_press(&mut self, point: Point, now: u64) -> bool {
        if self.tab_strip.count.contains(point) {
            // The count drawn unavailable opens nothing; the press takes the
            // keys from the address field, as a press anywhere off it does.
            if !self.filmstrip_available() {
                self.stop_editing();
                return true;
            }
            if self.filmstrip.shown() && self.filmstrip.rested_on_count {
                self.filmstrip.rested_on_count = false;
            } else if self.filmstrip.shown() {
                self.filmstrip_hide(now);
                self.filmstrip.phase = Phase::Dismissed;
            } else {
                self.filmstrip_show(now);
            }
            return true;
        }
        if !self.filmstrip.shown() || !self.filmstrip.panel.contains(point) {
            // REQ-6640 item 1: the filmstrip opens with no tab pressed.
            if matches!(self.filmstrip.phase, Phase::Resting(_)) && self.tab_strip.tabs_area.contains(point) {
                self.filmstrip.phase = Phase::Shut;
            }
            return false;
        }
        let Some(picture) = self.filmstrip.pictures.iter().find(|picture| picture.rect.contains(point)).copied() else { return true };
        self.stop_editing();
        let buttons = self.filmstrip.hovered == Some(picture.index);
        if buttons && picture.close.contains(point) {
            self.outputs.push(Output::CloseTab(picture.index));
        } else if (buttons || self.filmstrip_speaker_shown(picture.index)) && picture.speaker.contains(point) {
            self.outputs.push(Output::MuteTab(picture.index));
        } else {
            self.outputs.push(Output::SelectTab(picture.index));
            self.filmstrip_hide(now);
        }
        true
    }

    /// Whether a picture shows its mute button without the pointer on
    /// it: a tab muted or playing sound, as the strip's tab shows it.
    fn filmstrip_speaker_shown(&self, index: usize) -> bool {
        self.tabs.get(index).is_some_and(|tab| tab.muted || self.tab_sounding(tab))
    }

    /// Escape closes the open filmstrip (REQ-6640 item 9); whether it did.
    pub(super) fn filmstrip_escape(&mut self, now: u64) -> bool {
        if !self.filmstrip.shown() {
            return false;
        }
        self.filmstrip_hide(now);
        self.filmstrip.phase = Phase::Dismissed;
        true
    }

    /// Whether the pointer is on the open filmstrip.
    pub(super) fn over_filmstrip(&self) -> bool {
        self.filmstrip.shown() && self.filmstrip.panel.contains(self.pointer)
    }

    /// Whether the pointer is on a picture of the open filmstrip, which a
    /// press would choose.
    pub(super) fn over_filmstrip_picture(&self) -> bool {
        self.over_filmstrip() && self.filmstrip.hovered.is_some()
    }

    /// The wheel, which is the filmstrip's over it (`wheel_owner`), slides
    /// its row by `distance` points at `now`, the wheel towards the viewer
    /// to the left, within the slide's bounds (REQ-6640 item 6).
    pub(super) fn filmstrip_wheel(&mut self, distance: f64, now: u64) {
        self.filmstrip_motion();
        let (least, most) = self.filmstrip.bounds;
        let target = (f64::from(self.filmstrip.slide.target()) - distance).clamp(least, most);
        self.filmstrip.slide.set(narrow(target), now);
    }

    /// A notch of the wheel over the open filmstrip slides it by one
    /// picture.
    pub(super) fn filmstrip_wheel_lines(&mut self, lines: f64, now: u64) {
        let step = self.filmstrip.step;
        self.filmstrip_wheel(lines * step, now);
    }

    /// Where the row is slid to, eased, at `now`.
    #[cfg(test)]
    pub(super) fn filmstrip_slide_at(&self, now: u64) -> (f64, f64) {
        (f64::from(self.filmstrip.slide.value(now)), f64::from(self.filmstrip.slide.target()))
    }

    /// The slide's least and most, as last laid out.
    #[cfg(test)]
    pub(super) const fn filmstrip_bounds(&self) -> (f64, f64) {
        self.filmstrip.bounds
    }
}

#[cfg(test)]
mod tests;
