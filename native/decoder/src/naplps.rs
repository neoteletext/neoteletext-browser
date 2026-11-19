//! The sub-pages a channel draws in NAPLPS (the NAPLPS renderer's
//! REQ-6408 item 5, DEC-6407): for each, a NAPLPS terminal fed the
//! sub-page's records in order, and its picture, sent when it changed as
//! a ReGIS picture is.
//!
//! NAPLPS (ANSI X3.110-1983) draws on a unit screen whose visible part is
//! 1 wide by 25/32 high. NeoTeletext draws it on its teletext raster (the
//! owner's DEC-6853 and DEC-6857; neoteletext-common's `naplps-raster.md`
//! RAS-11): the canvas is the renderer's receiver canvas, 480 by 460
//! dots, placed on the raster's lines 20 to 479, exactly teletext rows 1
//! to 23, and the picture the browser's processes carry is the whole
//! raster, 480 by 500, rows 0 and 24 in the page's own background colour
//! (`Terminal::screen_colour`) where the teletext page beneath draws
//! nothing (DEC-6512). The 40 NAPLPS text columns are the raster's
//! 12-dot columns, so a pointer's dot of the picture is a dot of the
//! teletext page and its cell the teletext page's cell.
//!
//! Every place NeoTeletext defines on a NAPLPS page is a dot of that
//! raster (DEC-6853, RAS-4, RAS-5): an object's rectangle, its moves, its
//! bounds, a rule table's rectangles and the pointer's place. The renderer
//! takes objects in dots and gives their bounds in dots, so the decoder
//! passes both through unchanged, and the pick record holds each object's
//! bounds dot for dot.
//!
//! The decoder gives every terminal the glyphs of the engine's 55 teletext
//! character sets, from the Noto Teletext glyphs it has compiled in, so
//! that a page's private designation of any of them draws (REQ-6668; the
//! owner's DEC-6669: only the provided sets), with the joining data of the
//! plain Arabic-script sets (renderer parity PAR-14, [`crate::provided`]).
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

use interface_core::canvas::{NAPLPS_DISPLAY_LINES, NAPLPS_TOP_LINE, PAGE_HEIGHT, PAGE_WIDTH, PictureLanguage};
use naplps::limits::{
    DISPLAY_FIRST_LINE, DISPLAY_LINES, MAX_ANIMATED_OBJECTS, MAX_ANIMATION_PIXELS_PER_TICK, MAX_CYCLING_OBJECTS, RASTER_HEIGHT, RASTER_WIDTH,
    RECEIVER_CANVAS_HEIGHT, RECEIVER_CANVAS_WIDTH,
};
use naplps::{Animation, Canvas, Look, Object, Player, StrokeFont, Terminal};
use neoteletext_rules::animate::{AnimationStore, Applied, Operation};
use neoteletext_rules::evaluator::{Bounds, TargetState};
use neoteletext_rules::limits::MAX_POINTER_OBJECTS;
use neoteletext_rules::table::{Target, TargetKind};
use neoteletext_web::records::regis::RegisRecord;
use protocol::regis::{REGIS_COLOURS, RegisPick, RegisViewer};

use crate::animation::{Counters, next_cycle_tick};
use crate::parts::{Figures, Sent, parts_to_send};
use crate::regis::Drawn;

/// The colour map's address bits: sixteen entries, the picture's colours.
const PALETTE_BITS: u32 = 4;
/// The canvas: the renderer's receiver canvas, the raster's full width
/// and its 460 lines of rows 1 to 23 (RAS-11). The renderer draws at this
/// size: one canvas pixel is one dot of the raster, so the teletext sets'
/// 12 by 20 glyphs draw dot for dot and the picture message stays the
/// raster's.
pub(crate) const WIDTH: usize = RECEIVER_CANVAS_WIDTH as usize;
/// See [`WIDTH`].
pub(crate) const HEIGHT: usize = RECEIVER_CANVAS_HEIGHT as usize;
/// The picture carried: the teletext raster.
const PICTURE: PictureLanguage = PictureLanguage::Naplps;
/// The raster's line the canvas's first line is drawn on: row 0's 20
/// lines lie above it.
const TOP: usize = DISPLAY_FIRST_LINE as usize;
/// The raster places are dots of: 480 by 500 (RAS-4).
const RASTER: (u16, u16) = (PAGE_WIDTH, PAGE_HEIGHT);
/// The most sub-pages whose NAPLPS is kept, as for ReGIS: one more, and
/// the one fed longest ago is forgotten.
pub const MAX_NAPLPS_PAGES: usize = crate::regis::MAX_REGIS_PAGES;
/// The most pages whose sheet, and sub-pages whose objects for a stream
/// not yet begun, are kept.
const MAX_WAITING: usize = MAX_NAPLPS_PAGES;

const _: () = assert!((1 << PALETTE_BITS) == REGIS_COLOURS);
// RAS-11: the interface's figures, which it has without the renderer, are
// the renderer's own, and the canvas with rows 0 and 24 is the raster.
const _: () = assert!(
    NAPLPS_DISPLAY_LINES as u32 == DISPLAY_LINES
        && NAPLPS_TOP_LINE as u32 == DISPLAY_FIRST_LINE
        && RECEIVER_CANVAS_WIDTH == RASTER_WIDTH
        && PAGE_WIDTH as u32 == RASTER_WIDTH
        && PAGE_HEIGHT as u32 == RASTER_HEIGHT
        && WIDTH == PICTURE.width() as usize
        && RASTER_HEIGHT as usize == PICTURE.height() as usize
        && TOP + HEIGHT + TOP == PICTURE.height() as usize
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
    /// Its objects as the channel defined them, in raster dots (ANIM-11,
    /// RAS-4), with their labels.
    objects: Vec<Object>,
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
    /// What the viewer's pointer is doing to its objects, as the trusted
    /// core said (REQ-6374, DEC-6638 item 2): the hovered object is drawn
    /// with its hover look and the pressed one with its pressed look.
    viewer: RegisViewer,
    /// The object that is its cursor, and its blink (renderer parity
    /// PAR-27).
    cursor: Cursor,
    /// Its stream's drawing stopped at the renderer's pixel work
    /// allowance, and its composing of objects at its bound, and whether
    /// each was told (DEC-6851): once a generation each.
    stopped: Stopped,
}

/// Which of the NAPLPS renderer's work bounds (DEC-6851) a sub-page's
/// picture has met and been told of.
#[derive(Clone, Copy, Debug, Default)]
struct Stopped {
    /// The stream's allowance, `WORK_CAP` and `WORK_PER_BYTE` a byte: its
    /// drawing stopped for good, the rest of its stream passed over.
    stream: bool,
    /// One composing's bound, `MAX_COMPOSE_WORK`: its objects were shown
    /// only as far as it paid for.
    composing: bool,
}

/// The most notes of stopped drawing held between two takes: one of each
/// kind for every sub-page held.
const MAX_NOTES_HELD: usize = 2 * MAX_NAPLPS_PAGES;

/// A sub-page whose NAPLPS drawing met one of its renderer's work bounds
/// (DEC-6851): its page as `0xMPP`, its sub-code, and what stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoppedNote {
    /// The page.
    pub page: u16,
    /// The sub-code.
    pub sub: u16,
    /// What stopped, in words for the developer console.
    pub text: String,
}

/// The object a `REGIS_CURSOR` record names as a NAPLPS sub-page's cursor
/// (renderer parity PAR-2, PAR-27), and where its blink stands: shown and
/// hidden in turn, each for the half period the ReGIS cursor drawing
/// blinks at (REQ-6378), on the presentation clock (ANIM-1), while keys
/// go to the channel.
#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    /// The object's number, or 0 for none.
    number: u16,
    /// The tick the blink began at; `None` while it does not blink.
    from: Option<u32>,
    /// The blink is to begin again, shown, at the next tick it is brought
    /// to: the cursor changed, or the stream drew more, as a key typed
    /// makes it draw.
    again: bool,
    /// The renderer is holding it hidden for the blink.
    hidden: bool,
    /// Motion is reduced (PAR-33): it shows steadily.
    steady: bool,
}

/// The half period of a cursor's blink in presentation ticks: the ReGIS
/// cursor drawing's (REQ-6378), as PAR-27 asks.
const BLINK_HALF_TICKS: u32 = interface_core::regis::BLINK_HALF_TICKS;

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
    /// `font` is the stroke font the viewer chose (renderer parity
    /// PAR-35), or `None` for the one compiled in.
    fn new(generation: u16, sheet: Option<&[u8]>, at_once: bool, font: Option<&StrokeFont>) -> Option<Self> {
        let mut terminal = Terminal::new(PALETTE_BITS);
        if let Some(font) = font {
            terminal.set_font(font.clone());
        // The font compiled in is this program's own data: if it did not
        // parse, text would move the cursor and draw nothing.
        } else if let Ok(font) = StrokeFont::parse(naplps_fonts::NEOTELETEXT) {
            terminal.set_font(font);
        }
        // REQ-6668: every provided set, by its number.
        for (number, set) in (0_u8..).zip(crate::provided::naplps_sets()) {
            let _ = terminal.set_teletext_set(number, set.clone());
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
            objects: Vec::new(),
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
            viewer: RegisViewer::default(),
            cursor: Cursor::default(),
            stopped: Stopped::default(),
        })
    }

    /// Whether its stream's drawing has stopped at the renderer's work
    /// allowance and that was not yet told: true once (DEC-6851). Drawing
    /// stops for good, so every later byte is passed over.
    fn newly_stopped(&mut self) -> bool {
        let stopped = self.terminal.diagnostics().count(naplps::Refusal::WorkLimit) > 0;
        stopped && !core::mem::replace(&mut self.stopped.stream, true)
    }

    /// Whether its last composing stopped at the renderer's bound and that
    /// was not yet told this generation: true once (DEC-6851).
    fn newly_stopped_composing(&mut self) -> bool {
        let (_, stopped) = self.terminal.compose_work();
        stopped && !core::mem::replace(&mut self.stopped.composing, true)
    }

    /// The picture as the browser's processes carry it, one byte a pixel:
    /// the teletext raster, the picture presented on its lines 20 to 479
    /// and rows 0 and 24 in the colour the page last cleared its display
    /// to, its own background (RAS-11, DEC-6667 item 4), which the
    /// teletext page's own rows are drawn over (PAR-31).
    fn pixels(&self) -> Vec<u8> {
        let mut pixels = vec![self.terminal.screen_colour(); PICTURE.pixels()];
        let start = TOP.saturating_mul(WIDTH);
        if let Some(display) = pixels.get_mut(start..start.saturating_add(WIDTH.saturating_mul(HEIGHT))) {
            display.copy_from_slice(self.presented.pixels());
        }
        pixels
    }

    /// Gives the terminal all the sub-page's objects, defined in raster
    /// dots, which the renderer places on the canvas itself (ANIM-11,
    /// RAS-4); false, changing nothing, when it refuses them.
    fn set_objects(&mut self, objects: Vec<Object>) -> bool {
        if self.terminal.set_objects(&objects).is_err() {
            return false;
        }
        self.objects = objects;
        // The renderer forgets every pointer look with the objects it had,
        // and every animation state: the cursor is shown, and its blink
        // begins again if the object is still there.
        self.point_again();
        self.cursor.hidden = false;
        self.cursor.again = true;
        true
    }

    /// Redefines one object, or adds it (DEC-6638 item 3), keeping every
    /// other; false, changing nothing, when the terminal refuses it.
    fn replace_object(&mut self, object: Object) -> bool {
        if self.terminal.replace_object(object.clone()).is_err() {
            return false;
        }
        let number = object.number;
        match self.objects.iter_mut().find(|held| held.number == number) {
            Some(held) => *held = object,
            None => self.objects.push(object),
        }
        // The renderer forgets the redefined object's pointer look, and its
        // animation state: a cursor redefined is shown, and blinks afresh.
        self.point_again();
        if number == self.cursor.number {
            self.cursor.hidden = false;
            self.cursor.again = true;
        }
        true
    }

    /// The look the viewer's pointer gives object `number`: pressed while
    /// a button is held on it, else hover while the pointer is over it.
    const fn look_of(viewer: RegisViewer, number: u16) -> Look {
        if number == 0 {
            Look::Normal
        } else if viewer.pressed == number {
            Look::Pressed
        } else if viewer.hovered == number {
            Look::Hover
        } else {
            Look::Normal
        }
    }

    /// Takes what the viewer's pointer is doing to the objects (REQ-6374):
    /// each object whose look it changes is given its new look, and the
    /// picture is repainted where it changed. A viewer that changes no
    /// object's look changes nothing.
    fn point(&mut self, viewer: RegisViewer) {
        let before = core::mem::replace(&mut self.viewer, viewer);
        for number in [before.hovered, before.pressed, viewer.hovered, viewer.pressed] {
            let look = Self::look_of(viewer, number);
            if look != Self::look_of(before, number) && self.terminal.set_pointer_look(number, look) {
                self.changed = true;
            }
        }
    }

    /// Gives the hovered and pressed objects their looks again, after the
    /// renderer forgot them with the objects it was given.
    fn point_again(&mut self) {
        let viewer = core::mem::take(&mut self.viewer);
        self.point(viewer);
    }

    /// The object of `number` as the channel defined it.
    fn object(&self, number: u16) -> Option<&Object> {
        self.objects.iter().find(|object| object.number == number)
    }

    /// The colour map's sixteen colours at the tick reached, with each
    /// BLINK process in its phase (ANIM-20): a blink is shown by the
    /// colours alone.
    fn palette(&self) -> [[u8; 3]; REGIS_COLOURS] {
        let mut palette = [[0_u8; 3]; REGIS_COLOURS];
        // Under reduced motion a BLINK shows its first colour steadily
        // (PAR-33): the colour map as it stands before any blink.
        let colours = if self.cursor.steady { self.terminal.palette().at(0) } else { self.player.colours(&self.terminal) };
        for (colour, entry) in palette.iter_mut().zip(colours) {
            *colour = entry;
        }
        palette
    }

    /// The layer, the order and the number an object is drawn by (ANIM-12).
    fn drawn_by(&self, object: &Object) -> (u8, i8, u16) {
        let state = self.animation.get(Target::object(object.number));
        (state.and_then(|state| state.layer).unwrap_or(object.layer), state.and_then(|state| state.order).unwrap_or(object.order), object.number)
    }

    /// The pick record (ANIM-11, "Picking"; RAS-4): for each dot of the
    /// raster carried, the object drawn last of those that are shown,
    /// detectable and hold the dot in their bounds, or 0; the place within
    /// an object is in the raster's dots. Nothing else of a NAPLPS picture
    /// can be picked.
    fn pick(&self) -> RegisPick {
        let mut objects: Vec<&Object> = self.terminal.objects().iter().filter(|object| object.detectable).collect();
        if objects.is_empty() {
            return RegisPick::default();
        }
        objects.sort_by_key(|object| self.drawn_by(object));
        let mut record = vec![0_u16; PICTURE.pixels()];
        let mut origins = Vec::new();
        for object in objects {
            // No rectangle while it is not shown. The bounds are raster
            // dots cut to the raster, as the record's are: they are held dot
            // for dot, with no conversion and no offset (RAS-4), so a dot of
            // rows 0 and 24 picks an object whose bounds reach it (DEC-6856
            // item 3) though nothing of it is drawn there.
            let Some(bounds) = self.terminal.bounds(object.number) else { continue };
            let place = |value: u32| usize::try_from(value).unwrap_or(usize::MAX);
            for row in record.as_chunks_mut::<WIDTH>().0.iter_mut().skip(place(bounds.y)).take(place(bounds.height)) {
                for pixel in row.iter_mut().skip(place(bounds.x)).take(place(bounds.width)) {
                    *pixel = object.number;
                }
            }
            origins.push((object.number, i32::try_from(bounds.x).unwrap_or(i32::MAX), i32::try_from(bounds.y).unwrap_or(i32::MAX)));
        }
        RegisPick::of(&record, origins.into_iter(), PICTURE)
    }

    /// Gives the renderer the animation state held for an object, or
    /// takes it away when none is held.
    fn give(&mut self, number: u16) {
        let before = self.terminal.animation(number);
        // The cursor in the hidden half of its blink is hidden over whatever
        // state the rules gave it (PAR-27), which it keeps for when it shows.
        let blinked_off = (number != 0 && number == self.cursor.number && self.cursor.hidden).then_some(false);
        match self.animation.get(Target::object(number)).copied() {
            Some(state) => {
                let given = Animation {
                    shown: blinked_off.or(state.shown),
                    position: state.position,
                    layer: state.layer,
                    order: state.order,
                    frame: Some(state.frame),
                    look: Look::from_code(state.look).unwrap_or(Look::Normal),
                };
                let _ = self.terminal.set_animation(number, given);
            }
            None if blinked_off.is_some() => {
                let _ = self.terminal.set_animation(number, Animation { shown: blinked_off, ..Animation::default() });
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
        let own = (operation.target.kind == TargetKind::Object).then(|| self.object(number).map(|object| (object.x, object.y))).flatten();
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
        self.blink(tick);
        let passed = self.told.map_or(0, |told| tick.saturating_sub(told));
        if self.told != Some(tick) {
            self.painted = 0;
        }
        self.told = Some(tick);
        if passed > 0 {
            // The colours as shown, which reduced motion keeps steady.
            let colours = self.palette();
            let held = self.player.held_bytes();
            let _ = self.player.advance(&mut self.terminal, &mut self.canvas, passed);
            self.changed |= self.player.held_bytes() != held || self.palette() != colours;
        }
        let mut due = Vec::new();
        // Under reduced motion a frame cycle shows its first frame, the
        // frame in force before it starts, and does not advance (PAR-33).
        self.animation.advance(if self.cursor.steady { 0 } else { tick }, &mut |target| due.push(target.number));
        for number in due {
            self.give(number);
        }
    }

    /// The next presentation tick after `tick` this sub-page needs the
    /// loop at: a WAIT's end, a blink's change, a cycle's next frame, the
    /// cursor's next showing or hiding, or the next tick while a repaint
    /// of its animation waits for budget.
    fn next_tick(&self, tick: u32) -> Option<u32> {
        let player = self.player.ticks_to_next(&self.terminal).map(|ticks| tick.saturating_add(ticks.max(1)));
        let waiting = self.motion.slowed.then(|| tick.saturating_add(1));
        // Under reduced motion nothing waits and nothing blinks or cycles:
        // only a repaint held for the budget needs a tick (PAR-33).
        if self.cursor.steady {
            return waiting;
        }
        // A blink begun, or begun again, waits for the tick that begins it.
        let cursor = if !self.blinks() {
            None
        } else if self.cursor.again || self.cursor.from.is_none() {
            Some(tick.saturating_add(1))
        } else {
            self.next_blink_tick(tick)
        };
        [player, next_cycle_tick(self.animation.entries(), tick), waiting, cursor].into_iter().flatten().min()
    }

    /// Whether the cursor blinks (PAR-27): keys go to the channel, the
    /// sub-page names a cursor object that exists, and motion is not
    /// reduced (PAR-33: the cursor then shows steadily).
    fn blinks(&self) -> bool {
        !self.cursor.steady && self.viewer.typing && self.cursor.number != 0 && self.object(self.cursor.number).is_some()
    }

    /// Brings the cursor's blink to presentation tick `tick`: hidden
    /// through every second half period from the tick the blink began
    /// at, shown otherwise and whenever it does not blink. The renderer
    /// is told only when that changes.
    fn blink(&mut self, tick: u32) {
        let hidden = if self.blinks() {
            let from = match self.cursor.from {
                Some(from) if !self.cursor.again => from,
                _ => tick,
            };
            self.cursor.from = Some(from);
            !tick.saturating_sub(from).checked_div(BLINK_HALF_TICKS).unwrap_or(0).is_multiple_of(2)
        } else {
            self.cursor.from = None;
            false
        };
        self.cursor.again = false;
        if hidden != self.cursor.hidden {
            self.cursor.hidden = hidden;
            self.give(self.cursor.number);
        }
    }

    /// The tick the cursor next shows or hides at, if it blinks.
    fn next_blink_tick(&self, tick: u32) -> Option<u32> {
        let from = self.cursor.from?;
        let halves = tick.saturating_sub(from).checked_div(BLINK_HALF_TICKS).unwrap_or(0);
        Some(from.saturating_add(halves.saturating_add(1).saturating_mul(BLINK_HALF_TICKS)))
    }

    /// Makes `number` the cursor, or none for 0: an old cursor hidden for
    /// its blink is shown, and the new one's blink begins.
    fn set_cursor(&mut self, number: u16) {
        if number == self.cursor.number {
            return;
        }
        let steady = self.cursor.steady;
        let old = core::mem::replace(&mut self.cursor, Cursor { number, from: None, again: true, hidden: false, steady });
        if old.hidden {
            self.give(old.number);
        }
    }

    /// The state of an object for a rule table's conditions (RULE-26,
    /// ANIM-13), in raster dots (RAS-5), or `None` when the sub-page has
    /// no such object.
    fn target_state(&self, number: u16) -> Option<TargetState> {
        let object = self.object(number)?;
        let state = self.animation.get(Target::object(number)).copied().unwrap_or_default();
        let (x, y) = state.position.unwrap_or((object.x, object.y));
        Some(TargetState {
            shown: state.shown.unwrap_or(object.shown),
            cycling: state.cycle.is_some(),
            frame: state.frame,
            look: state.look,
            layer: state.layer.unwrap_or(object.layer),
            order: state.order.unwrap_or(object.order),
            x,
            y,
            // The renderer's bounds are raster dots, as the rules' are.
            bounds: self.terminal.bounds(number).map(|bounds| {
                let edge = |value: u32| i32::try_from(value).unwrap_or(i32::MAX);
                let (left, top) = (edge(bounds.x), edge(bounds.y));
                Bounds { left, top, right: edge(bounds.x.saturating_add(bounds.width)), bottom: edge(bounds.y.saturating_add(bounds.height)) }
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
    /// The platform asks for reduced motion (REQ-754, renderer parity
    /// PAR-33): what only the receiver moves stays still.
    reduced: bool,
    /// Each sub-page's sheet (ANIM-17): its generation and bytes, and
    /// whether a record of it was lost. A sub-page is a page (page-rules.md
    /// RULE-112): no sheet is fed to another sub-code.
    sheets: BTreeMap<(u16, u16), (u16, Vec<u8>, bool)>,
    /// Objects received for a generation of a sub-page's stream that has
    /// not begun: kept until it does, and dropped when another begins
    /// (ANIM-11).
    waiting: BTreeMap<(u16, u16), Waiting>,
    /// The stroke font the viewer chose for every new terminal, or none
    /// for the one compiled in (renderer parity PAR-35).
    font: Option<StrokeFont>,
    /// The sub-pages whose drawing met a work bound since the last take,
    /// for the developer console (DEC-6851).
    notes: Vec<StoppedNote>,
}

/// What a `NAPLPS_OBJECTS` record does to its sub-page's objects (the
/// engine's `naplps_objects::Scope`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Its objects are all the sub-page's, and its title, or none, the
    /// picture's (ANIM-11).
    Whole,
    /// Its one object redefines the sub-page's object of that number, or
    /// is added; every other object, with its animation state, and the
    /// title are kept (DEC-6638 item 3).
    One,
}

/// The objects, and the title, kept for a generation of a sub-page's
/// stream not yet begun.
struct Waiting {
    generation: u16,
    objects: Vec<Object>,
    /// The title, as the records kept say it.
    title: Title,
}

/// What the records kept for a stream not yet begun say of its title.
enum Title {
    /// Only single objects have come, which leave the title as it is.
    Kept,
    /// A whole record gave this title, or none.
    Given(Option<String>),
}

impl Page {
    /// Gives the picture its title, or takes it away; a title the
    /// renderer refuses (the engine's reader has checked it as the
    /// renderer does) leaves none.
    fn set_title(&mut self, title: Option<&str>) {
        if self.terminal.set_title(title.unwrap_or_default()).is_err() {
            let _ = self.terminal.set_title("");
        }
    }
}

impl NaplpsPages {
    /// Notes that the sub-page `key`'s drawing met a work bound.
    fn note(notes: &mut Vec<StoppedNote>, key: (u16, u16), text: String) {
        if notes.len() < MAX_NOTES_HELD {
            notes.push(StoppedNote { page: key.0, sub: key.1, text });
        }
    }

    /// The words for a stream's drawing stopped at its work allowance.
    fn stream_stopped_text() -> String {
        format!(
            "NAPLPS: the picture stopped drawing at its pixel work allowance (WORK_CAP {} and WORK_PER_BYTE {} a byte); the rest of its stream is not drawn",
            naplps::limits::WORK_CAP,
            naplps::limits::WORK_PER_BYTE
        )
    }

    /// The sub-pages whose drawing met a work bound since the last call
    /// (DEC-6851), for the developer console.
    pub fn take_notes(&mut self) -> Vec<StoppedNote> {
        core::mem::take(&mut self.notes)
    }

    /// Reads one NAPLPS record: its bytes go to its sub-page's terminal.
    pub fn read(&mut self, record: &RegisRecord<'_>) {
        let key = (record.page.mpp(), record.subcode.value());
        self.records = self.records.saturating_add(1);
        // A first byte again, of the same generation, is the stream sent
        // again whole, as a channel does when the page is selected again
        // after it was drawn from its kept source (PAR-34): it starts over.
        let fresh = self.pages.get(&key).is_none_or(|page| page.generation != record.generation || (record.offset == 0 && page.next != 0));
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
            let sheet = self.sheets.get(&key).map(|(_, bytes, _)| bytes.as_slice());
            // What the pointer is doing stays with the sub-page, as for ReGIS.
            // So does the object that is its cursor (PAR-27), as for ReGIS:
            // it is not the stream's to change.
            let (viewer, cursor) = self.pages.get(&key).map(|page| (page.viewer, page.cursor.number)).unwrap_or_default();
            let Some(mut page) = Page::new(record.generation, sheet, self.at_once || self.reduced, self.font.as_ref()) else { return };
            page.viewer = viewer;
            page.cursor.number = cursor;
            page.cursor.steady = self.reduced;
            // The objects sent ahead of this generation are its own; those
            // of any other are dropped (ANIM-11).
            if let Some(waiting) = self.waiting.remove(&key).filter(|waiting| waiting.generation == record.generation) {
                let _ = page.set_objects(waiting.objects);
                if let Title::Given(title) = waiting.title {
                    page.set_title(title.as_deref());
                }
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
        if page.newly_stopped() {
            Self::note(&mut self.notes, key, Self::stream_stopped_text());
        }
        page.records = page.records.saturating_add(1);
        page.bytes = page.bytes.saturating_add(u64::try_from(record.data.len()).unwrap_or(u64::MAX));
        page.changed = true;
        // The channel draws after each key typed: the cursor is shown again
        // at once, never hidden where the viewer looks for it (REQ-6378).
        page.cursor.again = true;
    }

    /// The channel named the object that is a NAPLPS sub-page's cursor,
    /// or 0 for none (a `REGIS_CURSOR` record, renderer parity PAR-2 and
    /// PAR-27). A sub-page with no NAPLPS yet has nothing to blink; its
    /// channel names the cursor again with its NAPLPS, as for ReGIS.
    pub fn set_cursor(&mut self, page: u16, sub: u16, object: u16) {
        if let Some(held) = self.pages.get_mut(&(page, sub)) {
            held.set_cursor(object);
        }
    }

    /// Takes a `NAPLPS_OBJECTS` record for a sub-page. A whole record
    /// gives it all its objects, replacing any it had (ANIM-11), and with
    /// them their animation state (ANIM-6), and its title, or none; a
    /// record of one object redefines or adds that object alone, its own
    /// animation state forgotten and every other object's, and the
    /// title, kept (DEC-6638 item 3). Objects are in raster dots
    /// (RAS-4) and carry their labels, which the renderer keeps
    /// for the screen-reader tree. Objects for a generation of the stream
    /// that is not the one shown are kept until that generation begins.
    /// What the renderer refuses changes nothing.
    pub fn set_objects(&mut self, page: u16, sub: u16, generation: u16, scope: Scope, title: Option<String>, objects: Vec<Object>) {
        let key = (page, sub);
        if let Some(held) = self.pages.get_mut(&key).filter(|held| held.generation == generation) {
            match scope {
                Scope::Whole => {
                    if held.set_objects(objects) {
                        held.animation.clear();
                        held.set_title(title.as_deref());
                        held.changed = true;
                    }
                }
                Scope::One => {
                    let Some(object) = objects.into_iter().next() else { return };
                    let number = object.number;
                    if held.replace_object(object) {
                        held.animation.forget(Target::object(number));
                        held.changed = true;
                    }
                }
            }
            return;
        }
        if !self.waiting.contains_key(&key) && self.waiting.len() >= MAX_WAITING {
            self.waiting.pop_first();
        }
        match scope {
            Scope::Whole => {
                self.waiting.insert(key, Waiting { generation, objects, title: Title::Given(title) });
            }
            Scope::One => {
                let Some(object) = objects.into_iter().next() else { return };
                let waiting = self.waiting.entry(key).or_insert(Waiting { generation, objects: Vec::new(), title: Title::Kept });
                if waiting.generation != generation {
                    *waiting = Waiting { generation, objects: Vec::new(), title: Title::Kept };
                }
                match waiting.objects.iter_mut().find(|held| held.number == object.number) {
                    Some(held) => *held = object,
                    None => waiting.objects.push(object),
                }
            }
        }
    }

    /// The object that is a sub-page's cursor, or 0.
    #[cfg(test)]
    pub(crate) fn cursor(&self, page: u16, sub: u16) -> u16 {
        self.pages.get(&(page, sub)).map_or(0, |held| held.cursor.number)
    }

    /// The trusted core said what the viewer's pointer is doing to a
    /// sub-page's picture (REQ-6374; DEC-6638 item 2: NAPLPS objects are
    /// controls as ReGIS drawings are): the hovered object is drawn with
    /// its hover look, the pressed one with its pressed look, and one no
    /// longer either with its frame again. A sub-page with no NAPLPS has
    /// no objects for it to change.
    pub fn set_viewer(&mut self, page: u16, sub: u16, viewer: RegisViewer) {
        if let Some(held) = self.pages.get_mut(&(page, sub)) {
            held.point(viewer);
            // Keys stopped going to the channel: the cursor is shown at
            // once, not at the next tick.
            if !viewer.typing && held.cursor.hidden {
                held.cursor.hidden = false;
                held.give(held.cursor.number);
            }
        }
    }

    /// The title of a NAPLPS picture of `page`, the first of its sub-pages
    /// with one: the name a page list shows for it when the program's
    /// index gives none (DEC-6638 item 3).
    #[must_use]
    pub fn title(&self, page: u16) -> Option<&str> {
        self.pages.iter().filter(|((held, _), _)| *held == page).find_map(|(_, held)| held.terminal.title())
    }

    /// The labels of the objects of `page`'s sub-page `sub`, by number:
    /// what the screen-reader tree is to say for each (TODO-6658).
    #[must_use]
    pub fn labels(&self, page: u16, sub: u16) -> Vec<(u16, &str)> {
        self.pages
            .get(&(page, sub))
            .map(|held| held.objects.iter().filter(|object| !object.label.is_empty()).map(|object| (object.number, object.label.as_str())).collect())
            .unwrap_or_default()
    }

    /// Reads one SHEET record of NAPLPS (ANIM-17): the bytes are kept, in
    /// order, as the sub-page's sheet (RULE-112), and fed to its renderer
    /// before its stream. A new generation starts the sheet again and
    /// forgets the sub-page, whose stream its channel sends again after it.
    pub fn read_sheet(&mut self, page: u16, sub: u16, generation: u16, offset: u32, data: &[u8]) {
        let key = (page, sub);
        if self.sheets.get(&key).is_none_or(|(held, _, _)| *held != generation) {
            if !self.sheets.contains_key(&key) && self.sheets.len() >= MAX_WAITING {
                self.sheets.pop_first();
            }
            if self.sheets.insert(key, (generation, Vec::new(), offset != 0)).is_some() {
                self.pages.remove(&key);
                if self.sent_sub.get(&page) == Some(&sub) {
                    self.sent_sub.remove(&page);
                }
            }
        }
        let Some((_, bytes, lost)) = self.sheets.get_mut(&key) else { return };
        let end = usize::try_from(offset).ok().and_then(|offset| offset.checked_add(data.len()));
        if *lost || usize::try_from(offset).ok() != Some(bytes.len()) || end.is_none_or(|end| end > neoteletext_web::consts::MAX_SHEET_BYTES) {
            *lost = true;
            return;
        }
        bytes.extend_from_slice(data);
    }

    /// Forgets everything held of one sub-page: its renderer, its sheet,
    /// its objects waiting and its animation state (page-rules.md
    /// RULE-111: a page closed drops its local state).
    pub fn forget(&mut self, page: u16, sub: u16) {
        self.pages.remove(&(page, sub));
        self.sheets.remove(&(page, sub));
        self.waiting.remove(&(page, sub));
        if self.sent_sub.get(&page) == Some(&sub) {
            self.sent_sub.remove(&page);
        }
    }

    /// The stroke font the viewer chose, or `None` for the one compiled in
    /// (renderer parity PAR-35): every terminal begun after this draws its
    /// text in it.
    pub fn set_font(&mut self, font: Option<StrokeFont>) {
        self.font = font;
    }

    /// The platform's preference for reduced motion (REQ-754, renderer
    /// parity PAR-33): on, no WAIT is waited for, a BLINK shows its first
    /// colour, a frame cycle its first frame, and the cursor object shows
    /// steadily; the channel's and the rules' state changes still apply.
    pub fn set_reduced(&mut self, reduced: bool) {
        self.reduced = reduced;
        let gradual = !self.at_once;
        for held in self.pages.values_mut() {
            held.cursor.steady = reduced;
            if reduced && held.cursor.hidden {
                held.cursor.hidden = false;
                held.give(held.cursor.number);
            }
            held.player.set_delays(gradual && !reduced);
            // What a WAIT held is drawn at once; the colours and frames
            // change to their steady ones.
            let _ = held.player.advance(&mut held.terminal, &mut held.canvas, 0);
            held.changed = true;
        }
    }

    /// The viewer's setting "Paint drawings gradually" (ANIM-15, ANIM-20):
    /// off, no WAIT delays anything, and a WAIT being played ends.
    pub fn set_gradual(&mut self, gradual: bool) {
        self.at_once = !gradual;
        let reduced = self.reduced;
        for held in self.pages.values_mut() {
            held.player.set_delays(gradual && !reduced);
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
        let Some(key) = self.shown(page, sub) else { return };
        if let Some(held) = self.pages.get_mut(&key) {
            held.advance(tick);
            // What a WAIT held is drawn as the clock plays it, and may meet
            // the allowance then.
            if held.newly_stopped() {
                Self::note(&mut self.notes, key, Self::stream_stopped_text());
            }
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

    /// The objects a dot of the raster is inside (RULE-12, ANIM-11, RAS-4):
    /// shown, detectable, the dot in its bounds. The count written to the
    /// front of `objects`.
    #[must_use]
    pub fn objects_at(&self, page: u16, sub: Option<u16>, x: u16, y: u16, objects: &mut [u16; MAX_POINTER_OBJECTS]) -> usize {
        let Some(held) = self.shown(page, sub).and_then(|key| self.pages.get(&key)) else { return 0 };
        // A dot past the raster is on nothing (RULE-13).
        if x >= RASTER.0 || y >= RASTER.1 {
            return 0;
        }
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
        if held.newly_stopped_composing() {
            let text = format!(
                "NAPLPS: drawing the picture's objects stopped at the composing work bound (MAX_COMPOSE_WORK {}); only part of them is shown",
                naplps::limits::MAX_COMPOSE_WORK
            );
            Self::note(&mut self.notes, key, text);
        }
        let _ = held.terminal.take_damage();
        let (pixels, palette) = (held.pixels(), held.palette());
        let figures = Figures { commits: held.records, work: held.bytes, language: PICTURE };
        let pick = held.pick();
        let (mut parts, pick_changed) = parts_to_send(&mut held.sent, same_sub, &pixels, palette, &pick, crate::parts::Changed::Unknown, figures)?;
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

    /// SO, then POINT SET at the origin and RECTANGLE (FILLED) over the
    /// whole unit screen (ANSI X3.110-1983 sections 5.3.3.1 and 5.3.3.4),
    /// `fills` times: each costs about a whole canvas of pixel work.
    fn whole_screen_fills(fills: usize) -> Vec<u8> {
        let mut stream = vec![0x0e];
        for _ in 0..fills {
            // The origin, then 255 by 191 256ths, three bytes a pair
            // (section 5.3.2.2.3): x and y three bits at a time.
            stream.extend_from_slice(&[0x24, 0x40, 0x40, 0x40, 0x31, 0x5a, 0x7f, 0x7f]);
        }
        stream
    }

    /// DEC-6851: a stream whose drawing passes the NAPLPS renderer's pixel
    /// work allowance (128 screens and 16 units a byte) stops drawing for
    /// good, and the developer console is told once, naming the
    /// allowance; a picture within it says nothing. Before the allowance
    /// a megabyte of such fills took the decoder five minutes to draw.
    #[test]
    fn a_stream_past_the_work_allowance_stops_drawing_and_says_so_once() {
        let mut pages = NaplpsPages::default();
        pages.read(&record(0x100, 1, 0, &whole_screen_fills(2)));
        assert_eq!(pages.take_notes(), [], "two fills are within the allowance");
        let mut pages = NaplpsPages::default();
        let stream = whole_screen_fills(200);
        pages.read(&record(0x100, 1, 0, &stream));
        let notes = pages.take_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!((notes[0].page, notes[0].sub), (0x100, 0));
        assert!(notes[0].text.contains("pixel work allowance (WORK_CAP 28262400 and WORK_PER_BYTE 16 a byte)"), "{}", notes[0].text);
        let spent = pages.pages.get(&(0x100, 0)).unwrap().terminal.work_spent();
        let offset = u32::try_from(stream.len()).unwrap();
        pages.read(&record(0x100, 1, offset, &whole_screen_fills(10)[1..]));
        assert_eq!(pages.take_notes(), [], "told once");
        assert_eq!(pages.pages.get(&(0x100, 0)).unwrap().terminal.work_spent(), spent, "nothing more is drawn");
        assert!(pages.take(0x100, Some(0)).is_some(), "what was drawn is shown");
    }

    /// Renderer parity PAR-35: a stroke font the viewer chose is the font
    /// of every terminal begun after it: one whose H is a single dot draws
    /// far less than the H compiled in.
    #[test]
    fn a_chosen_stroke_font_changes_the_glyphs_drawn() {
        let inked = |font: Option<StrokeFont>| {
            let mut pages = NaplpsPages::default();
            pages.set_font(font);
            pages.read(&record(0x100, 1, 0, b"\x0e\x20\x50\x40\x3e\x48\x0fHHHH"));
            let drawn = pages.take(0x100, Some(0)).unwrap();
            let screen = pages.pages.get(&(0x100, 0)).unwrap().terminal.screen_colour();
            unpack(&drawn.picture.packed).iter().filter(|&&pixel| pixel != screen).count()
        };
        let compiled_in = inked(None);
        let dot = StrokeFont::parse("[primary]\n48 0 6 3,4,3,4\n").unwrap();
        let chosen = inked(Some(dot));
        assert!(chosen < compiled_in, "the chosen font's H, a dot, is drawn: {chosen} against {compiled_in}");
    }

    /// RAS-11, DEC-6857 (REQ-6408 item 5): a sub-page's records draw a
    /// picture of the teletext raster, 480 by 500, whose lines 20 to 479
    /// are the 480 by 460 NAPLPS canvas, with text drawn in the font built
    /// in, and whose rows 0 and 24 are the colour the page cleared its
    /// display to; it is NAPLPS's, with no pick record's
    /// objects; more bytes are sent as the parts that changed, and a new
    /// generation starts again.
    #[test]
    fn records_draw_a_picture_of_the_teletext_raster() {
        let mut pages = NaplpsPages::default();
        assert!(!pages.has(0x100));
        // RESET clearing the display to the drawing colour (010 in bits 4
        // to 6, ANSI X3.110-1983 Table 15), SELECT COLOR entry 2, then text
        // in the primary set, its ASCII bytes, drawn from the home position.
        pages.read(&record(0x100, 1, 0, b"\x0e\x20\x50\x40\x3e\x48\x0fHello"));
        assert!(pages.has(0x100));
        let drawn = pages.take(0x100, Some(0)).unwrap();
        assert!(drawn.picture.whole() && drawn.picture.language == PictureLanguage::Naplps);
        assert_eq!(drawn.picture.area, RegisArea { x: 0, y: 0, width: 480, height: 500 }, "the teletext raster");
        assert_eq!(drawn.pick, Some(RegisPick::default()), "no object is named anywhere");
        let pixels = unpack(&drawn.picture.packed);
        let line = |y: usize| &pixels[y * 480..(y + 1) * 480];
        let screen = pages.pages.get(&(0x100, 0)).unwrap().terminal.screen_colour();
        assert_ne!(screen, 0, "the page cleared its display to another colour than nominal black");
        assert!((0..20).chain(480..500).all(|y| line(y).iter().all(|&pixel| pixel == screen)), "rows 0 and 24 are the page's background colour");
        // The text is drawn at the cursor's place after RESET, the
        // display's bottom left (section 5.3.2.9): in its last 23.1 lines,
        // which on the raster end at line 479.
        let inked_lines: Vec<usize> = (0..500).filter(|&y| line(y).iter().any(|&pixel| pixel != screen)).collect();
        assert!(inked_lines.iter().all(|&y| (480 - 24..480).contains(&y)), "the canvas ends at line 479: {inked_lines:?}");
        let inked = pixels[20 * 480..480 * 480].iter().filter(|&&pixel| pixel != screen).count();
        assert!(inked > 50, "the text is drawn, in the font built in: {inked} pixels");
        assert!(pages.take(0x100, Some(0)).is_none(), "nothing changed since");

        pages.read(&record(0x100, 1, 12, b" there"));
        let more = pages.take(0x100, Some(0)).unwrap();
        assert!(!more.picture.whole() && more.pick.is_none(), "a part, and the pick record as before");
        assert!(more.picture.area != RegisArea::default() && more.picture.area.pixels() < 480 * 500 / 10, "{:?}", more.picture.area);
        assert_eq!((more.picture.commits, more.picture.work), (2, 18));

        pages.read(&record(0x100, 1, 99, b"lost"));
        assert!(pages.take(0x100, Some(0)).is_none(), "a record after a gap is not drawn");
        pages.read(&record(0x100, 2, 0, b"Again"));
        let again = pages.take(0x100, Some(0)).unwrap();
        assert_eq!((again.picture.commits, again.picture.work), (1, 5), "a new generation starts the picture again");
    }

    /// RAS-11 (DEC-6857): a fill of the unit screen past its top draws
    /// exactly the raster's lines 20 to 479, rows 1 to 23, each across;
    /// rows 0 and 24 are left to the teletext page.
    #[test]
    fn a_naplps_picture_is_drawn_on_lines_20_to_479_and_no_other() {
        let mut pages = NaplpsPages::default();
        // SELECT COLOR entry 2, POINT SET at the origin, RECTANGLE filled by
        // (255, 255) 256ths, three bytes (sections 5.3.2.5, 5.3.3.1, 5.3.3.4);
        // a command is drawn when the next begins, so SELECT COLOR again.
        pages.read(&record(0x100, 1, 0, &[0x0e, 0x3e, 0x48, 0x24, 0x40, 0x40, 0x40, 0x31, 0x5b, 0x7f, 0x7f, 0x3e, 0x40]));
        let pixels = unpack(&pages.take(0x100, Some(0)).unwrap().picture.packed);
        let screen = pages.pages.get(&(0x100, 0)).unwrap().terminal.screen_colour();
        let line = |y: usize| &pixels[y * 480..(y + 1) * 480];
        let inked: Vec<usize> = (0..500).filter(|&y| line(y).iter().any(|&pixel| pixel != screen)).collect();
        assert_eq!(inked, (20..480).collect::<Vec<_>>(), "the first drawn line is 20 and the last 479");
        // The far corner, 255/256 across, is dot 478 (478.1 of 480).
        assert!((20..480).all(|y| line(y)[..=478].iter().all(|&pixel| pixel != screen)), "every line of rows 1 to 23 is drawn across");
    }

    /// An object of 48 by 40 raster dots at the dot (240, 250).
    fn square(number: u16, label: &str) -> Object {
        Object {
            number,
            kind: naplps::ObjectKind::Region,
            shown: true,
            detectable: true,
            layer: 1,
            order: 0,
            x: 240,
            y: 250,
            width: 48,
            height: 40,
            origin_x: 0,
            origin_y: 0,
            cover: 3,
            hover: 0,
            pressed: 0,
            alternative: 0,
            frames: Vec::new(),
            label: label.into(),
        }
    }

    /// RAS-4 (DEC-6853): an object placed at raster dots is drawn on
    /// exactly those dots, picked on exactly those dots, its bounds read
    /// back as the same dots, and `objects_at` takes the same dots: nothing
    /// is converted anywhere.
    #[test]
    fn an_object_in_raster_dots_is_drawn_picked_and_bounded_on_those_dots() {
        let mut pages = NaplpsPages::default();
        pages.read(&record(0x100, 1, 0, b"A"));
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![Object { shown: false, ..square(7, "") }]);
        let pixels = unpack(&pages.take(0x100, Some(0)).unwrap().picture.packed);
        let covered: Vec<(usize, usize)> = (0..500).flat_map(|y| (0..480).map(move |x| (x, y))).filter(|&(x, y)| pixels[y * 480 + x] == 3).collect();
        let expected: Vec<(usize, usize)> = (250..290).flat_map(|y| (240..288).map(move |x| (x, y))).collect();
        assert_eq!(covered, expected, "the hidden region's cover lies on dots 240 to 287 of lines 250 to 289");
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![square(7, "")]);
        let pick = pages.take(0x100, Some(0)).unwrap().pick.unwrap();
        let picked: Vec<(usize, usize)> =
            (0..500).flat_map(|y| (0..480).map(move |x| (x, y))).filter(|&(x, y)| pick.at(PictureLanguage::Naplps, x, y).drawing == 7).collect();
        assert_eq!(picked, expected, "picked on exactly its dots");
        let state = pages.target_state(0x100, Some(0), 7).unwrap();
        assert_eq!((state.x, state.y), (240, 250));
        assert_eq!(state.bounds, Some(Bounds { left: 240, top: 250, right: 288, bottom: 290 }), "its bounds in the same dots");
        let mut found = [0; MAX_POINTER_OBJECTS];
        assert_eq!(pages.objects_at(0x100, Some(0), 240, 250, &mut found), 1);
        assert_eq!(found[0], 7);
        assert_eq!(pages.objects_at(0x100, Some(0), 287, 289, &mut found), 1, "its last dot");
        assert_eq!(pages.objects_at(0x100, Some(0), 288, 289, &mut found), 0, "past its right edge");
        assert_eq!(pages.objects_at(0x100, Some(0), 240, 290, &mut found), 0, "past its bottom");
        assert_eq!(pages.objects_at(0x100, Some(0), 480, 250, &mut found), 0, "past the raster");
    }

    /// RAS-4 with DEC-6856 item 3: an object reaching into row 0 is drawn
    /// only on the display area, but its bounds are cut to the raster, so
    /// a dot of row 0 within them picks it and a rule there finds it.
    #[test]
    fn an_object_reaching_row_0_is_drawn_below_it_and_picked_on_it() {
        let mut pages = NaplpsPages::default();
        pages.read(&record(0x100, 1, 0, b"A"));
        let top = Object { x: 120, y: 0, width: 24, height: 40, ..square(7, "") };
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![Object { shown: false, ..top.clone() }]);
        let pixels = unpack(&pages.take(0x100, Some(0)).unwrap().picture.packed);
        let lines: Vec<usize> = (0..500).filter(|&line| pixels[line * 480 + 130] == 3).collect();
        assert_eq!(lines, (20..40).collect::<Vec<_>>(), "drawn on row 1 only: nothing over row 0");
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![top]);
        let pick = pages.take(0x100, Some(0)).unwrap().pick.unwrap();
        let picked: Vec<usize> = (0..500).filter(|&line| pick.at(PictureLanguage::Naplps, 130, line).drawing == 7).collect();
        assert_eq!(picked, (0..40).collect::<Vec<_>>(), "picked on its bounds, row 0 included");
        let mut found = [0; MAX_POINTER_OBJECTS];
        assert_eq!(pages.objects_at(0x100, Some(0), 130, 10, &mut found), 1, "a dot of row 0");
        assert_eq!(pages.target_state(0x100, Some(0), 7).unwrap().bounds, Some(Bounds { left: 120, top: 0, right: 144, bottom: 40 }));
    }

    /// RAS-7's cell on the raster: an object of a component-library page's
    /// row 7, lines 140 to 159, is drawn whole on exactly those lines and
    /// the pick record names it on exactly those lines.
    #[test]
    fn an_object_on_a_row_of_cells_is_drawn_whole_and_picked_where_drawn() {
        let mut pages = NaplpsPages::default();
        pages.read(&record(0x100, 1, 0, b"A"));
        let cell_row = Object { x: 0, y: 140, width: 64, height: 20, ..square(7, "") };
        // Hidden, a region shows its cover; shown, it is picked.
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![Object { shown: false, ..cell_row.clone() }]);
        let pixels = unpack(&pages.take(0x100, Some(0)).unwrap().picture.packed);
        let lines: Vec<usize> = (0..500).filter(|&line| pixels[line * 480 + 10] == 3).collect();
        assert_eq!(lines, (140..160).collect::<Vec<_>>(), "drawn on raster lines 140 to 159");
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![cell_row]);
        let pick = pages.take(0x100, Some(0)).unwrap().pick.unwrap();
        let picked: Vec<usize> = (0..500).filter(|&line| pick.at(PictureLanguage::Naplps, 10, line).drawing == 7).collect();
        assert_eq!(picked, lines, "picked on exactly the lines drawn");
        let state = pages.target_state(0x100, Some(0), 7).unwrap();
        assert_eq!(state.bounds, Some(Bounds { left: 0, top: 140, right: 64, bottom: 160 }), "its bounds in raster dots");
    }

    /// A filled rectangle of colour-map entry `entry` over the bottom left
    /// eighth of the unit screen across and up (32/256 on each axis, three
    /// bytes of the default domain, 4/0 6/4 4/0), as a macro's drawing:
    /// SELECT COLOR, POINT SET absolute at the origin, RECTANGLE filled
    /// (ANSI X3.110-1983 sections 5.3.2.5, 5.3.3.1 and 5.3.3.4), each
    /// coordinate three bytes of the default domain.
    fn filled(entry: u8) -> Vec<u8> {
        let mut bytes = vec![0x0E, 0x3E, 0x40 | (entry << 2), 0x24, 0x40, 0x40, 0x40, 0x31, 0x40, 0x64, 0x40];
        bytes.push(0x0F);
        bytes
    }

    /// Lays the picture of page 100 sub-page 0, if it changed, as a
    /// receiver holds it: a whole one, then each part laid over it where
    /// it changed; false when nothing changed.
    fn lay(pages: &mut NaplpsPages, picture: &mut Vec<u8>) -> bool {
        let Some(drawn) = pages.take(0x100, Some(0)) else { return false };
        for part in core::iter::once(&drawn.picture).chain(&drawn.more) {
            let pixels = unpack(&part.packed);
            if part.whole() {
                *picture = pixels;
                continue;
            }
            let (left, across) = (usize::from(part.area.x), usize::from(part.area.width));
            for (row, line) in pixels.chunks_exact(across.max(1)).enumerate().take(usize::from(part.area.height)) {
                let start = (usize::from(part.area.y) + row) * 480 + left;
                picture[start..start + across].copy_from_slice(line);
            }
        }
        true
    }

    /// TODO-6747, REQ-6374, DEC-6638 item 2: the trusted core's viewer
    /// draws a NAPLPS object's hover look while the pointer is over it and
    /// its pressed look while it is pressed, exactly as the same object
    /// with that look's macro as its frame is drawn; and the object's
    /// frame again when neither. The look is given again after the
    /// object is redefined, and to a new stream of the sub-page.
    #[test]
    fn the_viewer_draws_a_naplps_objects_hover_and_pressed_looks() {
        // Macros 2/1, 2/2 and 2/3 fill in entries 3, 6 and 9 (DEF MACRO
        // 8/0 and END 8/5 in their eight-bit forms, section 6.2).
        let mut stream = Vec::new();
        for (name, entry) in [(0x21, 3), (0x22, 6), (0x23, 9)] {
            stream.extend([0x80, name]);
            stream.extend(filled(entry));
            stream.push(0x85);
        }
        let button = |frame: u8| Object {
            kind: naplps::ObjectKind::Macro,
            layer: 2,
            origin_y: 420,
            frames: vec![frame],
            cover: 0,
            hover: 0x22,
            pressed: 0x23,
            ..square(5, "Hide")
        };
        let drawn_with = |frame: u8| {
            let mut pages = NaplpsPages::default();
            pages.read(&record(0x100, 1, 0, &stream));
            pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![button(frame)]);
            unpack(&pages.take(0x100, Some(0)).unwrap().picture.packed)
        };
        let (normal, hover, pressed) = (drawn_with(0x21), drawn_with(0x22), drawn_with(0x23));
        assert!(normal != hover && hover != pressed && normal != pressed, "the three looks differ");

        let mut picture = Vec::new();
        let mut pages = NaplpsPages::default();
        pages.read(&record(0x100, 1, 0, &stream));
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![button(0x21)]);
        assert!(lay(&mut pages, &mut picture));
        let mut follow = |pages: &mut NaplpsPages, viewer: RegisViewer| -> Vec<u8> {
            pages.set_viewer(0x100, 0, viewer);
            lay(pages, &mut picture);
            picture.clone()
        };
        let hovered = RegisViewer { hovered: 5, pressed: 0, typing: true };
        assert!(follow(&mut pages, RegisViewer::default()) == normal, "its frame");
        assert!(follow(&mut pages, hovered) == hover, "hovered");
        pages.set_viewer(0x100, 0, hovered);
        assert!(pages.take(0x100, Some(0)).is_none(), "the same viewer again changes nothing");
        assert!(follow(&mut pages, RegisViewer { hovered: 5, pressed: 5, typing: true }) == pressed, "pressed over hovered");
        assert!(follow(&mut pages, RegisViewer { hovered: 0, pressed: 5, typing: true }) == pressed, "pressed, the pointer off it");
        assert!(follow(&mut pages, RegisViewer::default()) == normal, "neither: its frame again");
        assert!(follow(&mut pages, hovered) == hover, "hovered again");
        pages.set_objects(0x100, 0, 1, Scope::One, None, vec![button(0x21)]);
        assert!(follow(&mut pages, hovered) == hover, "kept over the object's redefinition");
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![button(0x21)]);
        assert!(follow(&mut pages, hovered) == hover, "kept over the objects given again");
        pages.read(&record(0x100, 2, 0, &stream));
        pages.set_objects(0x100, 0, 2, Scope::Whole, None, vec![button(0x21)]);
        assert!(lay(&mut pages, &mut picture) && picture == hover, "kept for a new stream of the sub-page");
    }

    /// REQ-6668, DEC-6669: every NAPLPS terminal has the engine's 55 sets,
    /// in the engine's order, which is the renderer's; a page that
    /// designates Greek (set 17, ESC 2/8 2/1 3/1 into G0) draws the
    /// engine's Noto Teletext Greek glyph for its code 4/4, capital delta,
    /// exactly as a terminal given that glyph alone draws it, and not
    /// nothing, nor the primary set's D.
    #[test]
    fn a_page_designating_greek_draws_the_engines_greek_glyphs() {
        use naplps::BitmapSet;
        use neoteletext_core::glyphs::conformance::FONT_SETS;
        let teletext_sets = crate::provided::naplps_sets;
        assert_eq!(teletext_sets().len(), naplps::TELETEXT_SET_COUNT);
        for (number, (engine, renderer)) in FONT_SETS.iter().zip(naplps::TELETEXT_SETS).enumerate() {
            assert_eq!(engine.0, renderer, "set {number}");
        }
        let delta = *crate::engine::GLYPHS.lookup('\u{0394}').unwrap();
        assert_eq!(teletext_sets()[17].glyph(0x44).unwrap(), delta.as_slice(), "the Greek set's 4/4 is the engine's capital delta");
        let drawn = |bytes: &[u8]| {
            let mut pages = NaplpsPages::default();
            pages.read(&record(0x100, 1, 0, bytes));
            unpack(&pages.take(0x100, Some(0)).unwrap().picture.packed)
        };
        let greek = drawn(b"\x1b\x28\x21\x31D");
        // The same bytes on a terminal given the engine's delta alone.
        let mut terminal = Terminal::new(PALETTE_BITS);
        let mut set = BitmapSet::new(12, 20).unwrap();
        set.set_glyph(0x44, &delta).unwrap();
        terminal.set_teletext_set(17, set).unwrap();
        let mut canvas = Canvas::new(480, 460).unwrap();
        let _ = terminal.feed(&mut canvas, b"\x1b\x28\x21\x31D");
        let _ = terminal.finish(&mut canvas);
        let display = &greek[20 * 480..480 * 480];
        assert!(canvas.pixels().iter().any(|&pixel| pixel != 0), "the delta draws");
        assert_eq!(display, canvas.pixels(), "the engine's Greek glyph, as the renderer draws it");
        assert_ne!(greek, drawn(b"D"), "not the primary set's D");
    }

    /// DEC-6638 item 3, TODO-6694 step 3: a whole record's labels and
    /// title reach the renderer, and the title names the page; a record of
    /// one object redefines that object alone, at its new definition,
    /// forgetting its own animation state and keeping every other object,
    /// its state, and the title; and one for a stream not yet begun waits
    /// for it with the rest.
    #[test]
    fn labels_a_title_and_one_object_redefined_reach_the_renderer() {
        use neoteletext_rules::animate::OperationKind;
        let mut pages = NaplpsPages::default();
        pages.read(&record(0x100, 1, 0, b"A"));
        let other = Object { number: 8, x: 0, y: 0, ..square(8, "Stop") };
        pages.set_objects(0x100, 0, 1, Scope::Whole, Some("Weather map".into()), vec![square(7, "Play"), other.clone()]);
        assert_eq!(pages.title(0x100), Some("Weather map"));
        assert_eq!(pages.labels(0x100, 0), [(7, "Play"), (8, "Stop")]);
        let held = pages.pages.get(&(0x100, 0)).unwrap();
        assert_eq!((held.terminal.object(7).unwrap().label.as_str(), held.terminal.title()), ("Play", Some("Weather map")), "the renderer has them");
        // Object 8 is hidden by its animation state, then object 7 is redefined.
        let hide = Operation { target: Target::object(8), kind: OperationKind::Hide };
        assert!(pages.animate(0x100, Some(0), &hide, 0));
        let hide = Operation { target: Target::object(7), kind: OperationKind::Hide };
        assert!(pages.animate(0x100, Some(0), &hide, 0));
        pages.set_objects(0x100, 0, 1, Scope::One, None, vec![Object { x: 100, y: 50, ..square(7, "Pause") }]);
        let state = pages.target_state(0x100, Some(0), 7).unwrap();
        assert_eq!((state.x, state.y, state.shown), (100, 50, true), "at its new definition, its own state forgotten");
        assert!(!pages.target_state(0x100, Some(0), 8).unwrap().shown, "the other keeps its state");
        assert_eq!(pages.labels(0x100, 0), [(7, "Pause"), (8, "Stop")]);
        assert_eq!(pages.title(0x100), Some("Weather map"), "the title kept");
        let canvas_object = pages.pages.get(&(0x100, 0)).unwrap().terminal.object(7).unwrap().clone();
        assert_eq!((canvas_object.x, canvas_object.y), (100, 50), "in raster dots, as given (RAS-4)");
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![other]);
        assert_eq!(pages.title(0x100), None, "a whole record without a title takes it away");
        // For generation 2, not yet begun: a whole set and one object more.
        pages.set_objects(0x100, 0, 2, Scope::Whole, Some("Later".into()), vec![square(7, "")]);
        pages.set_objects(0x100, 0, 2, Scope::One, None, vec![Object { x: 0, ..square(9, "Nine") }]);
        pages.read(&record(0x100, 2, 0, b"B"));
        assert_eq!((pages.title(0x100), pages.labels(0x100, 0)), (Some("Later"), vec![(9, "Nine")]));
        assert!(pages.target_state(0x100, Some(0), 7).is_some());
    }

    /// Renderer parity PAR-27: the object a `REGIS_CURSOR` record names is
    /// a NAPLPS sub-page's cursor, shown and hidden in turn at the ReGIS
    /// cursor's half period while keys go to the channel; it does not
    /// blink without keys, number 0 stops it, and so do objects given again
    /// without it; the stream drawing more shows it again at once.
    #[test]
    fn the_cursor_object_blinks_at_the_regis_cursors_period_while_keys_go_to_the_channel() {
        const HALF: u32 = BLINK_HALF_TICKS;
        let mut pages = NaplpsPages::default();
        pages.read(&record(0x100, 1, 0, b"A"));
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![square(7, ""), Object { x: 0, y: 0, ..square(8, "") }]);
        let shown = |pages: &NaplpsPages, number: u16| pages.pages.get(&(0x100, 0)).unwrap().terminal.bounds(number).is_some();
        pages.set_cursor(0x100, 0, 7);
        pages.advance(0x100, Some(0), 10);
        pages.advance(0x100, Some(0), 10 + HALF);
        assert!(shown(&pages, 7), "keys do not go to the channel: it does not blink");
        let typing = RegisViewer { hovered: 0, pressed: 0, typing: true };
        pages.set_viewer(0x100, 0, typing);
        let start = 100;
        pages.advance(0x100, Some(0), start);
        assert!(shown(&pages, 7), "shown for the first half period");
        assert_eq!(pages.next_tick(0x100, Some(0), start), Some(start + HALF), "the loop wakes when it is to hide");
        pages.advance(0x100, Some(0), start + HALF - 1);
        assert!(shown(&pages, 7));
        pages.advance(0x100, Some(0), start + HALF);
        assert!(!shown(&pages, 7), "hidden for the second");
        assert!(shown(&pages, 8), "no other object blinks");
        pages.advance(0x100, Some(0), start + 2 * HALF);
        assert!(shown(&pages, 7), "and shown again");
        pages.advance(0x100, Some(0), start + 3 * HALF);
        assert!(!shown(&pages, 7));
        // The channel draws after a key: shown at once, for a full half period.
        pages.read(&record(0x100, 1, 1, b"B"));
        let typed = start + 3 * HALF + 5;
        pages.advance(0x100, Some(0), typed);
        assert!(shown(&pages, 7), "shown again when the stream draws");
        pages.advance(0x100, Some(0), typed + HALF - 1);
        assert!(shown(&pages, 7));
        pages.advance(0x100, Some(0), typed + HALF);
        assert!(!shown(&pages, 7));
        // Number 0 stops it, shown.
        pages.set_cursor(0x100, 0, 0);
        assert!(shown(&pages, 7), "no cursor: shown at once");
        pages.advance(0x100, Some(0), typed + 3 * HALF);
        assert!(shown(&pages, 7) && pages.next_tick(0x100, Some(0), typed + 3 * HALF).is_none());
        // Objects given again without it: nothing blinks.
        pages.set_cursor(0x100, 0, 7);
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![Object { x: 0, y: 0, ..square(8, "") }]);
        pages.advance(0x100, Some(0), 1000);
        pages.advance(0x100, Some(0), 1000 + HALF);
        assert!(shown(&pages, 8) && pages.next_tick(0x100, Some(0), 1000 + HALF).is_none(), "the cursor object is gone: nothing blinks");
        // Keys stopping while it is hidden show it at once.
        pages.set_objects(0x100, 0, 1, Scope::Whole, None, vec![square(7, "")]);
        pages.advance(0x100, Some(0), 2000);
        pages.advance(0x100, Some(0), 2000 + HALF);
        assert!(!shown(&pages, 7));
        pages.set_viewer(0x100, 0, RegisViewer::default());
        assert!(shown(&pages, 7), "keys stopped: shown");
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
