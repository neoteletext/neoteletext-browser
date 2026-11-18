//! The sub-pages a channel draws in NAPLPS (the NAPLPS renderer's
//! REQ-6408 item 5, DEC-6407): for each, a NAPLPS terminal fed the
//! sub-page's records in order, and its picture, sent when it changed as
//! a ReGIS picture is.
//!
//! NAPLPS (ANSI X3.110-1983) draws on a unit screen 1 wide and 0.75 high,
//! which 640 by 480 pixels hold exactly. The picture the browser's
//! processes carry is 800 by 480, so the NAPLPS canvas lies in its middle
//! 640 columns and the columns either side take the terminal's border
//! colour.
//!
//! The decoder is the compositor of its animation (neoteletext-common's
//! `docs/animation.md`, ANIM-22). WAIT (ANSI X3.110-1983 section 5.3.2.8)
//! and BLINK (section 5.3.2.7) are played on the presentation clock by the
//! renderer's `Player` (ANIM-20); the sub-page's numbered objects
//! (ANIM-11), with their animation state (ANIM-6) and frame cycles
//! (ANIM-14), are composed over the stream's picture into the picture
//! presented, repainting only damage; and the objects that can be picked
//! are sent as the picture's pick record, as a ReGIS picture's detectable
//! drawings are (DEC-6522 item 11).

use std::collections::BTreeMap;

use naplps::limits::{MAX_ANIMATED_OBJECTS, MAX_ANIMATION_PIXELS_PER_TICK, MAX_CYCLING_OBJECTS};
use naplps::{Animation, Canvas, Look, Object, Player, StrokeFont, Terminal};
use neoteletext_rules::animate::{AnimationStore, Applied, Operation};
use neoteletext_rules::evaluator::{Bounds, TargetState};
use neoteletext_rules::limits::MAX_POINTER_OBJECTS;
use neoteletext_rules::table::{Target, TargetKind};
use neoteletext_web::records::regis::RegisRecord;
use protocol::regis::{PictureLanguage, REGIS_COLOURS, REGIS_HEIGHT, REGIS_WIDTH, RegisPick};

use crate::animation::{Counters, next_cycle_tick};
use crate::parts::{Figures, Sent, parts_to_send};
use crate::regis::Drawn;

/// The colour map's address bits: sixteen entries, the picture's colours.
const PALETTE_BITS: u32 = 4;
/// The canvas: the unit screen, 1 by 0.75, with square pixels.
pub(crate) const WIDTH: usize = 640;
pub(crate) const HEIGHT: usize = REGIS_HEIGHT;
/// The picture's columns to the left of the canvas, which lies in its middle.
const LEFT: usize = (REGIS_WIDTH - WIDTH) / 2;
/// The most sub-pages whose NAPLPS is kept, as for ReGIS: one more, and
/// the one fed longest ago is forgotten.
pub const MAX_NAPLPS_PAGES: usize = crate::regis::MAX_REGIS_PAGES;
/// The most pages whose sheet, and sub-pages whose objects for a stream
/// not yet begun, are kept.
const MAX_WAITING: usize = MAX_NAPLPS_PAGES;

const _: () = assert!(WIDTH <= REGIS_WIDTH && (1 << PALETTE_BITS) == REGIS_COLOURS);
// ANIM-11: the objects' picture is the canvas, 640 by 480.
const _: () = assert!(
    WIDTH == neoteletext_web::records::naplps_objects::PICTURE_WIDTH as usize && HEIGHT == neoteletext_web::records::naplps_objects::PICTURE_HEIGHT as usize
);

/// One sub-page's NAPLPS.
struct Page {
    /// Which stream of the sub-page is being drawn.
    generation: u16,
    /// The offset the next record's bytes begin at.
    next: u32,
    /// A record was lost: nothing more of this generation is drawn.
    lost: bool,
    terminal: Terminal,
    /// The stream's own picture: the background its objects lie over
    /// (ANIM-12).
    canvas: Canvas,
    /// The picture presented: the stream's with the shown objects
    /// composed over it.
    presented: Canvas,
    /// Plays the stream's WAIT and BLINK on the clock (ANIM-20).
    player: Player,
    /// Bytes were drawn, or the objects or the colours changed, since the
    /// picture was last sent.
    changed: bool,
    /// When it was last fed, by the count of records read.
    fed: u64,
    /// The records and the bytes drawn of this stream, which the browser
    /// shows above the picture.
    records: u32,
    bytes: u64,
    /// The picture last sent, which the next is sent as parts of.
    sent: Option<Sent>,
    /// The animation state of its objects (ANIM-6), with the cycles the
    /// renderer has no clock to run (ANIM-14).
    animation: Box<AnimationStore<MAX_ANIMATED_OBJECTS>>,
    /// The presentation tick it was last brought to.
    told: Option<u32>,
    /// The pixels repainted for its animation in the tick `told`
    /// (ANIM-16).
    painted: u64,
    /// Whether it is owed a repaint for its animation.
    motion: Motion,
    counters: Counters,
}

/// What a sub-page's animation is owed.
#[derive(Clone, Copy, Default)]
struct Motion {
    /// Its animation state changed since its picture was last taken.
    animated: bool,
    /// The repaint its animation is owed was held back for the budget,
    /// and counted (ANIM-16).
    slowed: bool,
}

impl Page {
    /// A sub-page's NAPLPS begun, the page's `sheet` fed before its
    /// stream (ANIM-17); `None` only if a canvas cannot be made.
    fn new(generation: u16, sheet: Option<&[u8]>, at_once: bool) -> Option<Self> {
        let mut terminal = Terminal::new(PALETTE_BITS);
        // The font is this program's own data: if it did not parse, text
        // would move the cursor and draw nothing.
        if let Ok(font) = StrokeFont::parse(naplps_fonts::NEOTELETEXT) {
            terminal.set_font(font);
        }
        let (width, height) = (u32::try_from(WIDTH).ok()?, u32::try_from(HEIGHT).ok()?);
        let mut canvas = Canvas::new(width, height).ok()?;
        // A sheet defines macros, DRCS characters and textures; it draws
        // nothing, and a WAIT in it delays nothing.
        if let Some(sheet) = sheet {
            let _ = terminal.feed(&mut canvas, sheet);
        }
        let mut player = Player::new();
        player.set_delays(!at_once);
        Some(Self {
            generation,
            next: 0,
            lost: false,
            terminal,
            canvas,
            presented: Canvas::new(width, height).ok()?,
            player,
            changed: true,
            fed: 0,
            records: 0,
            bytes: 0,
            sent: None,
            animation: Box::new(AnimationStore::new()),
            told: None,
            painted: 0,
            motion: Motion::default(),
            counters: Counters::default(),
        })
    }

    /// The picture as the browser's processes carry it, one byte a pixel:
    /// the picture presented in the middle columns, the border's colour
    /// either side.
    fn pixels(&self) -> Vec<u8> {
        let mut pixels = vec![self.terminal.border(); REGIS_WIDTH.saturating_mul(REGIS_HEIGHT)];
        for (row, line) in pixels.as_chunks_mut::<REGIS_WIDTH>().0.iter_mut().zip(self.presented.pixels().as_chunks::<WIDTH>().0) {
            if let Some(middle) = row.get_mut(LEFT..LEFT.saturating_add(WIDTH)) {
                middle.copy_from_slice(line);
            }
        }
        pixels
    }

    /// The colour map's sixteen colours at the tick reached, with each
    /// BLINK process in its phase (ANIM-20): a blink is shown by the
    /// colours alone.
    fn palette(&self) -> [[u8; 3]; REGIS_COLOURS] {
        let mut palette = [[0_u8; 3]; REGIS_COLOURS];
        for (colour, entry) in palette.iter_mut().zip(self.player.colours(&self.terminal)) {
            *colour = entry;
        }
        palette
    }

    /// The layer, the order and the number an object is drawn by (ANIM-12).
    fn drawn_by(&self, object: &Object) -> (u8, i8, u16) {
        let state = self.animation.get(Target::object(object.number));
        (state.and_then(|state| state.layer).unwrap_or(object.layer), state.and_then(|state| state.order).unwrap_or(object.order), object.number)
    }

    /// The pick record (ANIM-11, "Picking"): for each pixel of the
    /// picture carried, the object drawn last of those that are shown,
    /// detectable and hold the pixel in their rectangle, or 0. Nothing
    /// else of a NAPLPS picture can be picked.
    fn pick(&self) -> RegisPick {
        let mut objects: Vec<&Object> = self.terminal.objects().iter().filter(|object| object.detectable).collect();
        if objects.is_empty() {
            return RegisPick::default();
        }
        objects.sort_by_key(|object| self.drawn_by(object));
        let mut record = vec![0_u16; REGIS_WIDTH.saturating_mul(REGIS_HEIGHT)];
        let mut origins = Vec::new();
        for object in objects {
            // No rectangle while it is not shown.
            let Some(bounds) = self.terminal.bounds(object.number) else { continue };
            let place = |value: u32| usize::try_from(value).unwrap_or(usize::MAX);
            let left = LEFT.saturating_add(place(bounds.x));
            for row in record.as_chunks_mut::<REGIS_WIDTH>().0.iter_mut().skip(place(bounds.y)).take(place(bounds.height)) {
                for pixel in row.iter_mut().skip(left).take(place(bounds.width)) {
                    *pixel = object.number;
                }
            }
            origins.push((object.number, i32::try_from(left).unwrap_or(i32::MAX), i32::try_from(bounds.y).unwrap_or(i32::MAX)));
        }
        RegisPick::of(&record, origins.into_iter())
    }

    /// Gives the renderer the animation state held for an object, or
    /// takes it away when none is held.
    fn give(&mut self, number: u16) {
        let before = self.terminal.animation(number);
        match self.animation.get(Target::object(number)).copied() {
            Some(state) => {
                let given = Animation {
                    shown: state.shown,
                    position: state.position,
                    layer: state.layer,
                    order: state.order,
                    frame: Some(state.frame),
                    look: Look::from_code(state.look).unwrap_or(Look::Normal),
                };
                let _ = self.terminal.set_animation(number, given);
            }
            None => {
                let _ = self.terminal.clear_animation(number);
            }
        }
        if self.terminal.animation(number) != before {
            self.changed = true;
            self.motion.animated = true;
        }
    }

    /// Carries out one animation operation at presentation tick `tick`
    /// (ANIM-6); false when it did nothing, which is counted.
    fn animate(&mut self, operation: &Operation, tick: u32) -> bool {
        let number = operation.target.number;
        let own = (operation.target.kind == TargetKind::Object).then(|| self.terminal.object(number).map(|object| (object.x, object.y))).flatten();
        let applied = own.is_some_and(|own| self.animation.apply(operation, own, tick, MAX_CYCLING_OBJECTS) == Applied::Done);
        if applied {
            self.give(number);
        } else {
            self.counters.skipped = self.counters.skipped.saturating_add(1);
        }
        applied
    }

    /// Brings the sub-page to presentation tick `tick`: a WAIT being
    /// played runs down and what it held is drawn, the BLINK processes
    /// reach their phase (ANIM-20), and each cycling object whose frame
    /// is due is given it (ANIM-14).
    fn advance(&mut self, tick: u32) {
        let passed = self.told.map_or(0, |told| tick.saturating_sub(told));
        if self.told != Some(tick) {
            self.painted = 0;
        }
        self.told = Some(tick);
        if passed > 0 {
            let colours = self.player.colours(&self.terminal);
            let held = self.player.held_bytes();
            let _ = self.player.advance(&mut self.terminal, &mut self.canvas, passed);
            self.changed |= self.player.held_bytes() != held || self.player.colours(&self.terminal) != colours;
        }
        let mut due = Vec::new();
        self.animation.advance(tick, &mut |target| due.push(target.number));
        for number in due {
            self.give(number);
        }
    }

    /// The next presentation tick after `tick` this sub-page needs the
    /// loop at: a WAIT's end, a blink's change, a cycle's next frame, or
    /// the next tick while a repaint of its animation waits for budget.
    fn next_tick(&self, tick: u32) -> Option<u32> {
        let player = self.player.ticks_to_next(&self.terminal).map(|ticks| tick.saturating_add(ticks.max(1)));
        let waiting = self.motion.slowed.then(|| tick.saturating_add(1));
        [player, next_cycle_tick(self.animation.entries(), tick), waiting].into_iter().flatten().min()
    }

    /// The state of an object for a rule table's conditions (RULE-26,
    /// ANIM-13), or `None` when the sub-page has no such object.
    fn target_state(&self, number: u16) -> Option<TargetState> {
        let object = self.terminal.object(number)?;
        let state = self.animation.get(Target::object(number)).copied().unwrap_or_default();
        let (x, y) = state.position.unwrap_or((object.x, object.y));
        let cut = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
        Some(TargetState {
            shown: state.shown.unwrap_or(object.shown),
            cycling: state.cycle.is_some(),
            frame: state.frame,
            look: state.look,
            layer: state.layer.unwrap_or(object.layer),
            order: state.order.unwrap_or(object.order),
            x,
            y,
            bounds: self.terminal.bounds(number).map(|bounds| Bounds {
                left: cut(bounds.x),
                top: cut(bounds.y),
                right: cut(bounds.x.saturating_add(bounds.width)),
                bottom: cut(bounds.y.saturating_add(bounds.height)),
            }),
        })
    }
}

/// Every sub-page's NAPLPS a channel has sent.
#[derive(Default)]
pub struct NaplpsPages {
    pages: BTreeMap<(u16, u16), Page>,
    /// Records read.
    records: u64,
    /// The sub-page whose picture was last sent for each page.
    sent_sub: BTreeMap<u16, u16>,
    /// No WAIT delays anything: the viewer's setting "Paint drawings
    /// gradually" is off (ANIM-15, ANIM-20).
    at_once: bool,
    /// Each page's sheet (ANIM-17): its generation and bytes, and whether
    /// a record of it was lost.
    sheets: BTreeMap<u16, (u16, Vec<u8>, bool)>,
    /// Objects received for a generation of a sub-page's stream that has
    /// not begun: kept until it does, and dropped when another begins
    /// (ANIM-11).
    waiting: BTreeMap<(u16, u16), (u16, Vec<Object>)>,
}

impl NaplpsPages {
    /// Reads one NAPLPS record: its bytes go to its sub-page's terminal.
    pub fn read(&mut self, record: &RegisRecord<'_>) {
        let key = (record.page.mpp(), record.subcode.value());
        self.records = self.records.saturating_add(1);
        let fresh = self.pages.get(&key).is_none_or(|page| page.generation != record.generation);
        if fresh {
            // A stream is followed from its first byte: one joined part
            // way through waits for the next generation.
            if record.offset != 0 {
                if let Some(page) = self.pages.get_mut(&key) {
                    page.lost = true;
                }
                return;
            }
            if !self.pages.contains_key(&key) && self.pages.len() >= MAX_NAPLPS_PAGES {
                let oldest = self.pages.iter().min_by_key(|(_, page)| page.fed).map(|(key, _)| *key);
                if let Some(oldest) = oldest {
                    self.pages.remove(&oldest);
                }
            }
            let sheet = self.sheets.get(&key.0).map(|(_, bytes, _)| bytes.as_slice());
            let Some(mut page) = Page::new(record.generation, sheet, self.at_once) else { return };
            // The objects sent ahead of this generation are its own; those
            // of any other are dropped (ANIM-11).
            if let Some((_, objects)) = self.waiting.remove(&key).filter(|(generation, _)| *generation == record.generation) {
                let _ = page.terminal.set_objects(&objects);
            }
            self.pages.insert(key, page);
        }
        let Some(page) = self.pages.get_mut(&key) else { return };
        page.fed = self.records;
        if page.lost || record.offset != page.next {
            page.lost = true;
            return;
        }
        page.next = page.next.saturating_add(u32::try_from(record.data.len()).unwrap_or(u32::MAX));
        // What is drawn at once is drawn; what follows a WAIT is held by
        // the player until the clock has played it (ANIM-20). A bell is
        // not rung (DEC-6407 item 4).
        let _ = page.player.push(&mut page.terminal, &mut page.canvas, record.data);
        page.records = page.records.saturating_add(1);
        page.bytes = page.bytes.saturating_add(u64::try_from(record.data.len()).unwrap_or(u64::MAX));
        page.changed = true;
    }

    /// Gives a sub-page all its objects, replacing any it had (ANIM-11),
    /// and with them their animation state (ANIM-6). Objects for a
    /// generation of the stream that is not the one shown are kept until
    /// that generation begins. A set the renderer refuses changes nothing.
    pub fn set_objects(&mut self, page: u16, sub: u16, generation: u16, objects: Vec<Object>) {
        let key = (page, sub);
        if let Some(held) = self.pages.get_mut(&key).filter(|held| held.generation == generation) {
            if held.terminal.set_objects(&objects).is_ok() {
                held.animation.clear();
                held.changed = true;
            }
            return;
        }
        if !self.waiting.contains_key(&key) && self.waiting.len() >= MAX_WAITING {
            self.waiting.pop_first();
        }
        self.waiting.insert(key, (generation, objects));
    }

    /// Reads one SHEET record of NAPLPS (ANIM-17): the bytes are kept, in
    /// order, as the page's sheet, and fed to the renderer of each of the
    /// page's sub-pages before its stream. A new generation starts the
    /// sheet again and forgets the page's sub-pages, whose streams its
    /// channel sends again after it.
    pub fn read_sheet(&mut self, page: u16, generation: u16, offset: u32, data: &[u8]) {
        if self.sheets.get(&page).is_none_or(|(held, _, _)| *held != generation) {
            if !self.sheets.contains_key(&page) && self.sheets.len() >= MAX_WAITING {
                self.sheets.pop_first();
            }
            if self.sheets.insert(page, (generation, Vec::new(), offset != 0)).is_some() {
                self.pages.retain(|(held, _), _| *held != page);
                self.sent_sub.remove(&page);
            }
        }
        let Some((_, bytes, lost)) = self.sheets.get_mut(&page) else { return };
        let end = usize::try_from(offset).ok().and_then(|offset| offset.checked_add(data.len()));
        if *lost || usize::try_from(offset).ok() != Some(bytes.len()) || end.is_none_or(|end| end > neoteletext_web::consts::MAX_SHEET_BYTES) {
            *lost = true;
            return;
        }
        bytes.extend_from_slice(data);
    }

    /// The viewer's setting "Paint drawings gradually" (ANIM-15, ANIM-20):
    /// off, no WAIT delays anything, and a WAIT being played ends.
    pub fn set_gradual(&mut self, gradual: bool) {
        self.at_once = !gradual;
        for held in self.pages.values_mut() {
            held.player.set_delays(gradual);
            if !gradual {
                // What the wait held is drawn at once.
                let _ = held.player.advance(&mut held.terminal, &mut held.canvas, 0);
                held.changed = true;
            }
        }
    }

    /// Whether the sub-page of `page` on screen is drawn in NAPLPS.
    #[must_use]
    pub fn holds(&self, page: u16, sub: Option<u16>) -> bool {
        self.shown(page, sub).is_some_and(|key| self.pages.contains_key(&key))
    }

    /// Whether a page has a sub-page drawn in NAPLPS.
    #[must_use]
    pub fn has(&self, page: u16) -> bool {
        self.pages.keys().any(|(held, _)| *held == page)
    }

    /// Marks a page's pictures as not yet sent: the page came on screen.
    pub fn show_again(&mut self, page: u16) {
        for (_, held) in self.pages.iter_mut().filter(|((held, _), _)| *held == page) {
            held.changed = true;
            held.sent = None;
        }
        self.sent_sub.remove(&page);
    }

    /// The sub-page of `page` on screen: `sub`, or with none the page's
    /// first with any NAPLPS.
    fn shown(&self, page: u16, sub: Option<u16>) -> Option<(u16, u16)> {
        match sub {
            Some(sub) if sub != neoteletext_core::limits::SUBCODE_ANY => Some((page, sub)),
            _ => self.pages.keys().find(|(held, _)| *held == page).copied(),
        }
    }

    /// Carries out one animation operation on the sub-page of `page` on
    /// screen at presentation tick `tick` (ANIM-6); false when it did
    /// nothing.
    pub fn animate(&mut self, page: u16, sub: Option<u16>, operation: &Operation, tick: u32) -> bool {
        self.shown(page, sub).and_then(|key| self.pages.get_mut(&key)).is_some_and(|held| held.animate(operation, tick))
    }

    /// Brings the sub-page of `page` on screen to presentation tick
    /// `tick`: its WAIT, its BLINK and its cycles (ANIM-14, ANIM-20).
    pub fn advance(&mut self, page: u16, sub: Option<u16>, tick: u32) {
        if let Some(held) = self.shown(page, sub).and_then(|key| self.pages.get_mut(&key)) {
            held.advance(tick);
        }
    }

    /// The next presentation tick after `tick` the sub-page of `page` on
    /// screen needs the loop at, if any (ANIM-1).
    #[must_use]
    pub fn next_tick(&self, page: u16, sub: Option<u16>, tick: u32) -> Option<u32> {
        self.shown(page, sub).and_then(|key| self.pages.get(&key))?.next_tick(tick)
    }

    /// The state of an object of the sub-page of `page` on screen, as a
    /// rule table's conditions read it (RULE-26, ANIM-13).
    #[must_use]
    pub fn target_state(&self, page: u16, sub: Option<u16>, number: u16) -> Option<TargetState> {
        self.shown(page, sub).and_then(|key| self.pages.get(&key))?.target_state(number)
    }

    /// The objects a pixel of the 640 by 480 picture is inside (RULE-12,
    /// ANIM-11): shown, detectable, the pixel in its rectangle. The count
    /// written to the front of `objects`.
    #[must_use]
    pub fn objects_at(&self, page: u16, sub: Option<u16>, x: u16, y: u16, objects: &mut [u16; MAX_POINTER_OBJECTS]) -> usize {
        let Some(held) = self.shown(page, sub).and_then(|key| self.pages.get(&key)) else { return 0 };
        let found = held.terminal.objects_at(u32::from(x), u32::from(y));
        let mut count = 0_usize;
        for (slot, number) in objects.iter_mut().zip(found) {
            *slot = number;
            count = count.saturating_add(1);
        }
        count
    }

    /// What the animation of the sub-page of `page` on screen has cost
    /// since it was last asked (ANIM-16, RULE-48).
    pub fn take_counters(&mut self, page: u16, sub: Option<u16>) -> Counters {
        self.shown(page, sub).and_then(|key| self.pages.get_mut(&key)).map(|held| core::mem::take(&mut held.counters)).unwrap_or_default()
    }

    /// The picture of a page's sub-page on screen, if it changed since it
    /// was last taken: `sub`, or with none the page's first with any
    /// NAPLPS. Whole at first, and afterwards the areas that changed
    /// (REQ-6399), with the objects that can be picked as its pick record.
    ///
    /// The stream's picture and the shown objects are composed here, only
    /// the damage repainted (ANIM-4). A repaint for animation that would
    /// take the tick's pixels past the animation budget waits for the
    /// next tick, its damage merged with what follows (ANIM-16): the
    /// animation slows, and nothing else.
    pub fn take(&mut self, page: u16, sub: Option<u16>) -> Option<Drawn> {
        let key = self.shown(page, sub)?;
        let same_sub = self.sent_sub.get(&key.0) == Some(&key.1);
        let held = self.pages.get_mut(&key)?;
        if !held.changed {
            return None;
        }
        // Only a repaint the animation state asked for is held to the
        // budget: the stream's own drawing is shown as it arrives.
        if held.motion.animated {
            let pending = held.terminal.pending(&held.presented).map_or(0, |damage| damage.area());
            if held.painted > 0 && held.painted.saturating_add(pending) > MAX_ANIMATION_PIXELS_PER_TICK {
                if !core::mem::replace(&mut held.motion.slowed, true) {
                    held.counters.slowed = held.counters.slowed.saturating_add(1);
                }
                return None;
            }
            held.painted = held.painted.saturating_add(pending);
        }
        held.changed = false;
        held.motion.animated = false;
        held.motion.slowed = false;
        // A picture the renderer cannot compose is sent as it stands.
        let _ = held.terminal.compose(&held.canvas, &mut held.presented);
        let _ = held.terminal.take_damage();
        let (pixels, palette) = (held.pixels(), held.palette());
        let figures = Figures { commits: held.records, work: held.bytes, language: PictureLanguage::Naplps };
        let pick = held.pick();
        let (mut parts, pick_changed) = parts_to_send(&mut held.sent, same_sub, &pixels, palette, &pick, figures)?;
        self.sent_sub.insert(key.0, key.1);
        let more = parts.split_off(1);
        Some(Drawn { page: key.0, sub: key.1, picture: parts.pop()?, pick: pick_changed.then_some(pick), more })
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        reason = "a test reports a failure by panicking, and indexes and counts what it made"
    )]

    use neoteletext_core::types::{PageAddress, SubCode};
    use protocol::regis::{RegisArea, unpack};

    use super::*;

    fn record(page: u16, generation: u16, offset: u32, data: &[u8]) -> RegisRecord<'_> {
        RegisRecord { page: PageAddress::from_mpp(page).unwrap(), subcode: SubCode::new(0), generation, offset, data }
    }

    /// REQ-6408 item 5: a sub-page's records draw a picture whose middle
    /// 640 columns are the NAPLPS canvas, with text drawn in the font
    /// built in, and whose other columns are the border's colour; it is
    /// NAPLPS's, with no pick record's drawings; more bytes are sent as
    /// the parts that changed, and a new generation starts again.
    #[test]
    fn records_draw_a_picture_in_the_middle_of_the_wider_one() {
        let mut pages = NaplpsPages::default();
        assert!(!pages.has(0x100));
        // Text in the primary set is its ASCII bytes, drawn from the home position.
        pages.read(&record(0x100, 1, 0, b"Hello"));
        assert!(pages.has(0x100));
        let drawn = pages.take(0x100, Some(0)).unwrap();
        assert!(drawn.picture.whole() && drawn.picture.language == PictureLanguage::Naplps);
        assert_eq!(drawn.pick, Some(RegisPick::default()), "no drawing is named anywhere");
        assert_eq!((drawn.picture.commits, drawn.picture.work), (1, 5), "a record and five bytes");
        let pixels = unpack(&drawn.picture.packed);
        let border = pixels[0];
        let column = |x: usize| (0..REGIS_HEIGHT).map(|y| pixels[y * REGIS_WIDTH + x]).collect::<Vec<u8>>();
        assert!((0..LEFT).chain(LEFT + WIDTH..REGIS_WIDTH).all(|x| column(x).iter().all(|&pixel| pixel == border)), "the columns either side are the border's");
        let inked = (0..REGIS_HEIGHT).flat_map(|y| (LEFT..LEFT + WIDTH).map(move |x| (x, y))).filter(|&(x, y)| pixels[y * REGIS_WIDTH + x] != border).count();
        assert!(inked > 50, "the text is drawn, in the font built in: {inked} pixels");
        assert!(pages.take(0x100, Some(0)).is_none(), "nothing changed since");

        pages.read(&record(0x100, 1, 5, b" there"));
        let more = pages.take(0x100, Some(0)).unwrap();
        assert!(!more.picture.whole() && more.pick.is_none(), "a part, and the pick record as before");
        assert!(more.picture.area != RegisArea::default() && more.picture.area.pixels() < REGIS_WIDTH * REGIS_HEIGHT / 10, "{:?}", more.picture.area);
        assert_eq!((more.picture.commits, more.picture.work), (2, 11));

        pages.read(&record(0x100, 1, 99, b"lost"));
        assert!(pages.take(0x100, Some(0)).is_none(), "a record after a gap is not drawn");
        pages.read(&record(0x100, 2, 0, b"Again"));
        let again = pages.take(0x100, Some(0)).unwrap();
        assert_eq!((again.picture.commits, again.picture.work), (1, 5), "a new generation starts the picture again");
    }

    /// As for ReGIS: one sub-page more than are kept, and the one fed
    /// longest ago is forgotten; a page that comes on screen again is
    /// sent whole.
    #[test]
    fn the_sub_page_fed_longest_ago_is_forgotten_and_a_page_shown_again_is_sent_whole() {
        let mut pages = NaplpsPages::default();
        for page in 0..=u16::try_from(MAX_NAPLPS_PAGES).unwrap() {
            pages.read(&record(0x100 + page, 1, 0, b"A"));
        }
        assert!(!pages.has(0x100), "the first fed is forgotten");
        assert!(pages.has(0x101));
        assert!(pages.take(0x101, None).unwrap().picture.whole());
        pages.read(&record(0x101, 1, 1, b"B"));
        pages.show_again(0x101);
        assert!(pages.take(0x101, None).unwrap().picture.whole(), "the page came on screen again");
    }
}
