//! The sub-pages a channel draws in NAPLPS (the NAPLPS renderer's
//! REQ-6408 item 5, DEC-6407): for each, a NAPLPS terminal fed the
//! sub-page's records in order, and its picture, sent when it changed as
//! a ReGIS picture is.
//!
//! NAPLPS (ANSI X3.110-1983) draws on a unit screen whose visible part is
//! 1 wide by 25/32 high. NeoTeletext draws it on its teletext raster (the
//! NAPLPS renderer's REQ-5819, the owner's DEC-6667): the canvas is 480 by
//! 462 pixels, which on the raster's pixels, 16/13 as wide as they are
//! high, keep the page's shape, and the picture the browser's processes
//! carry is the whole raster, 480 by 500, the canvas on lines 19 to 480
//! and the 19 lines above and below it in the page's own background colour
//! (`Terminal::screen_colour`, DEC-6667 item 4). The 40 NAPLPS text columns
//! are the raster's 12-pixel columns, so a pointer's dot of the picture is
//! a dot of the teletext page and its cell the teletext page's cell.
//!
//! Objects and a rule table's pointer and rectangles are in the pixels of
//! a 640 by 480 picture (animation.md ANIM-11, page-rules.md RULE-13, the
//! engine's `naplps_objects::PICTURE_WIDTH`), the shape of the display at
//! square pixels. Each object is given to the renderer scaled to the
//! canvas, and each rectangle the renderer gives back is scaled to that
//! picture again; a pointer's dot of the raster is that picture's pixel
//! under it ([`design_point`]).
//!
//! The decoder gives every terminal the glyphs of the engine's 55 teletext
//! character sets, from the Noto Teletext glyphs it has compiled in, so
//! that a page's private designation of any of them draws (REQ-6668; the
//! owner's DEC-6669: only the provided sets).
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

use std::sync::OnceLock;

use interface_core::canvas::{NAPLPS_DISPLAY_LINES, NAPLPS_TOP_LINE, PAGE_HEIGHT, PAGE_WIDTH, PictureLanguage};
use naplps::limits::{MAX_ANIMATED_OBJECTS, MAX_ANIMATION_PIXELS_PER_TICK, MAX_CYCLING_OBJECTS, OBJECT_PICTURE_HEIGHT, OBJECT_PICTURE_WIDTH};
use naplps::{Animation, BitmapSet, Canvas, Look, Object, Player, StrokeFont, TELETEXT_SET_COUNT, Terminal};
use neoteletext_core::charset::CharRef;
use neoteletext_core::consts::{CELL_HEIGHT, CELL_WIDTH};
use neoteletext_core::glyphs::conformance::{FONT_SET_COUNT, FONT_SETS};
use neoteletext_rules::animate::{AnimationStore, Applied, Operation};
use neoteletext_rules::evaluator::{Bounds, TargetState};
use neoteletext_rules::limits::MAX_POINTER_OBJECTS;
use neoteletext_rules::table::{Target, TargetKind};
use neoteletext_web::records::naplps_objects::{PICTURE_HEIGHT, PICTURE_WIDTH};
use neoteletext_web::records::regis::RegisRecord;
use protocol::regis::{REGIS_COLOURS, RegisPick};

use crate::animation::{Counters, next_cycle_tick};
use crate::parts::{Figures, Sent, parts_to_send};
use crate::regis::Drawn;

/// The colour map's address bits: sixteen entries, the picture's colours.
const PALETTE_BITS: u32 = 4;
/// The canvas: the raster's full width and 462 of its lines (REQ-5819).
/// The renderer draws at this size, not a multiple of it (DEC-6667 item 2
/// leaves the choice to the implementer): one canvas pixel is one dot of
/// the raster, so the teletext sets' 12 by 20 glyphs draw dot for dot in
/// the default character field and the picture message stays the raster's.
pub(crate) const WIDTH: usize = PAGE_WIDTH as usize;
/// See [`WIDTH`].
pub(crate) const HEIGHT: usize = NAPLPS_DISPLAY_LINES as usize;
/// The picture carried: the teletext raster.
const PICTURE: PictureLanguage = PictureLanguage::Naplps;
/// The picture's lines above the canvas, and as many below it.
const TOP: usize = NAPLPS_TOP_LINE as usize;
/// The picture objects and a rule table's pointer are given in (ANIM-11,
/// RULE-13): 640 by 480.
const DESIGN_WIDTH: i64 = PICTURE_WIDTH as i64;
/// See [`DESIGN_WIDTH`].
const DESIGN_HEIGHT: i64 = PICTURE_HEIGHT as i64;
/// [`WIDTH`] and [`HEIGHT`] for arithmetic with places, which may be
/// negative.
const CANVAS_WIDTH: i64 = PAGE_WIDTH as i64;
/// See [`CANVAS_WIDTH`].
const CANVAS_HEIGHT: i64 = NAPLPS_DISPLAY_LINES as i64;
/// The most sub-pages whose NAPLPS is kept, as for ReGIS: one more, and
/// the one fed longest ago is forgotten.
pub const MAX_NAPLPS_PAGES: usize = crate::regis::MAX_REGIS_PAGES;
/// The most pages whose sheet, and sub-pages whose objects for a stream
/// not yet begun, are kept.
const MAX_WAITING: usize = MAX_NAPLPS_PAGES;

const _: () = assert!((1 << PALETTE_BITS) == REGIS_COLOURS);
const _: () = assert!(WIDTH == PICTURE.width() as usize && HEIGHT + 2 * TOP == PICTURE.height() as usize && PAGE_HEIGHT as usize == HEIGHT + 2 * TOP);
// ANIM-11: the record's picture and the renderer's are one, 640 by 480,
// and the canvas lies within it, so a scaled object's size is one the
// renderer takes.
const _: () = assert!(
    DESIGN_WIDTH == OBJECT_PICTURE_WIDTH as i64
        && DESIGN_HEIGHT == OBJECT_PICTURE_HEIGHT as i64
        && CANVAS_WIDTH <= DESIGN_WIDTH
        && CANVAS_HEIGHT <= DESIGN_HEIGHT
);
// REQ-6668: the engine's sets are the renderer's, one for one, and each
// glyph is the engine's cell.
const _: () = assert!(FONT_SET_COUNT == TELETEXT_SET_COUNT);

/// `value`, a length or place along a side of `from` pixels, along the
/// same side of `to` pixels, to the nearest pixel (halves away from the
/// top left).
fn rescale(value: i64, from: i64, to: i64) -> i64 {
    value.saturating_mul(to).saturating_mul(2).saturating_add(from).div_euclid(from.saturating_mul(2).max(1))
}

/// A place of the 640 by 480 picture on the canvas.
fn canvas_x(x: i16) -> i16 {
    i16::try_from(rescale(i64::from(x), DESIGN_WIDTH, CANVAS_WIDTH)).unwrap_or(i16::MAX)
}

/// See [`canvas_x`].
fn canvas_y(y: i16) -> i16 {
    i16::try_from(rescale(i64::from(y), DESIGN_HEIGHT, CANVAS_HEIGHT)).unwrap_or(i16::MAX)
}

/// A size of the 640 by 480 picture on the canvas: at least a pixel.
fn canvas_size(size: u16, from: i64, to: i64) -> u16 {
    u16::try_from(rescale(i64::from(size), from, to).max(1)).unwrap_or(u16::MAX)
}

/// An object of the 640 by 480 picture as the renderer draws it on the
/// canvas: its rectangle and its origin in the macro's drawing scaled, the
/// macro being drawn at the canvas's scale.
fn on_canvas(object: &Object) -> Object {
    Object {
        x: canvas_x(object.x),
        y: canvas_y(object.y),
        width: canvas_size(object.width, DESIGN_WIDTH, CANVAS_WIDTH),
        height: canvas_size(object.height, DESIGN_HEIGHT, CANVAS_HEIGHT),
        origin_x: canvas_x(object.origin_x),
        origin_y: canvas_y(object.origin_y),
        ..object.clone()
    }
}

/// A rectangle of the canvas in the 640 by 480 picture's pixels: left,
/// top, right and bottom.
fn design_rectangle(bounds: naplps::Rect) -> (i32, i32, i32, i32) {
    let back = |value: u32, to: i64, from: i64| i32::try_from(rescale(i64::from(value), from, to)).unwrap_or(i32::MAX);
    (
        back(bounds.x, DESIGN_WIDTH, CANVAS_WIDTH),
        back(bounds.y, DESIGN_HEIGHT, CANVAS_HEIGHT),
        back(bounds.x.saturating_add(bounds.width), DESIGN_WIDTH, CANVAS_WIDTH),
        back(bounds.y.saturating_add(bounds.height), DESIGN_HEIGHT, CANVAS_HEIGHT),
    )
}

/// The pixel of the 640 by 480 picture (RULE-13) under the dot (`x`, `y`)
/// of the raster a NAPLPS page is drawn on. A dot of the border lines
/// above or below the display area is given as the first row past the
/// picture's bottom, which no rectangle (cut to the picture, RULE-13) and
/// no object holds: the press, release or move is still followed
/// (RULE-8), on nothing.
#[must_use]
pub(crate) fn design_point(x: u16, y: u16) -> (u16, u16) {
    let across = i64::from(x).saturating_mul(DESIGN_WIDTH).checked_div(CANVAS_WIDTH).unwrap_or(0).min(DESIGN_WIDTH);
    let down = match i64::from(y).checked_sub(i64::from(NAPLPS_TOP_LINE)) {
        Some(line) if (0..CANVAS_HEIGHT).contains(&line) => line.saturating_mul(DESIGN_HEIGHT).checked_div(CANVAS_HEIGHT).unwrap_or(0),
        _ => DESIGN_HEIGHT,
    };
    (u16::try_from(across).unwrap_or(u16::MAX), u16::try_from(down).unwrap_or(u16::MAX))
}

/// The engine's 55 teletext character sets as the renderer takes them
/// (REQ-6668): for each, in the order of the engine's `FONT_SETS`, which is
/// the renderer's `TELETEXT_SETS`, the glyph the teletext renderer draws
/// for each code 2/0 to 7/15 from the Noto Teletext glyphs compiled in
/// (DEC-6669: the provided sets only), a G2 set's marks as their spacing
/// forms and the letters of the Arabic-script sets in their isolated forms.
/// Made once, and copied into each terminal.
fn teletext_sets() -> &'static [BitmapSet] {
    static SETS: OnceLock<Vec<BitmapSet>> = OnceLock::new();
    SETS.get_or_init(|| {
        let (Ok(width), Ok(height)) = (u8::try_from(CELL_WIDTH), u8::try_from(CELL_HEIGHT)) else { return Vec::new() };
        FONT_SETS
            .iter()
            .filter_map(|(_, set)| {
                let mut bitmap = BitmapSet::new(width, height).ok()?;
                for code in FIRST_SET_CODE..=LAST_SET_CODE {
                    // A glyph of the engine's own cell always fits; one that
                    // did not would leave its code blank.
                    let _ = bitmap.set_glyph(code, &crate::engine::GLYPHS.glyph(CharRef::new(*set, code)));
                }
                Some(bitmap)
            })
            .collect()
    })
}

/// The first and last codes of a set: 2/0 and 7/15.
const FIRST_SET_CODE: u8 = 0x20;
/// See [`FIRST_SET_CODE`].
const LAST_SET_CODE: u8 = 0x7F;
const _: () = assert!((LAST_SET_CODE - FIRST_SET_CODE + 1) as usize == naplps::bitmap::BITMAP_SET_CODES);

/// One sub-page's NAPLPS.
struct Page {
    /// Which stream of the sub-page is being drawn.
    generation: u16,
    /// The offset the next record's bytes begin at.
    next: u32,
    /// A record was lost: nothing more of this generation is drawn.
    lost: bool,
    terminal: Terminal,
    /// Its objects as the channel defined them, in the 640 by 480
    /// picture's pixels (ANIM-11), with their labels; the terminal holds
    /// each scaled to the canvas.
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
        // REQ-6668: every provided set, by its number.
        for (number, set) in (0_u8..).zip(teletext_sets()) {
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
        })
    }

    /// The picture as the browser's processes carry it, one byte a pixel:
    /// the teletext raster, the picture presented on its lines 19 to 480
    /// and the lines above and below in the colour the page last cleared
    /// its display to, its own background (REQ-5819, DEC-6667 item 4).
    fn pixels(&self) -> Vec<u8> {
        let mut pixels = vec![self.terminal.screen_colour(); PICTURE.pixels()];
        let start = TOP.saturating_mul(WIDTH);
        if let Some(display) = pixels.get_mut(start..start.saturating_add(WIDTH.saturating_mul(HEIGHT))) {
            display.copy_from_slice(self.presented.pixels());
        }
        pixels
    }

    /// Gives the terminal all the sub-page's objects, defined in the 640 by
    /// 480 picture's pixels, scaled to the canvas; false, changing nothing,
    /// when it refuses them.
    fn set_objects(&mut self, objects: Vec<Object>) -> bool {
        let scaled: Vec<Object> = objects.iter().map(on_canvas).collect();
        if self.terminal.set_objects(&scaled).is_err() {
            return false;
        }
        self.objects = objects;
        true
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

    /// The pick record (ANIM-11, "Picking"): for each dot of the raster
    /// carried, the object drawn last of those that are shown, detectable
    /// and hold the dot in their rectangle on the canvas, or 0; the place
    /// within an object is in the raster's dots. Nothing else of a NAPLPS
    /// picture can be picked.
    fn pick(&self) -> RegisPick {
        let mut objects: Vec<&Object> = self.terminal.objects().iter().filter(|object| object.detectable).collect();
        if objects.is_empty() {
            return RegisPick::default();
        }
        objects.sort_by_key(|object| self.drawn_by(object));
        let mut record = vec![0_u16; PICTURE.pixels()];
        let mut origins = Vec::new();
        for object in objects {
            // No rectangle while it is not shown.
            let Some(bounds) = self.terminal.bounds(object.number) else { continue };
            let place = |value: u32| usize::try_from(value).unwrap_or(usize::MAX);
            let top = TOP.saturating_add(place(bounds.y));
            for row in record.as_chunks_mut::<WIDTH>().0.iter_mut().skip(top).take(place(bounds.height)) {
                for pixel in row.iter_mut().skip(place(bounds.x)).take(place(bounds.width)) {
                    *pixel = object.number;
                }
            }
            origins.push((object.number, i32::try_from(bounds.x).unwrap_or(i32::MAX), i32::try_from(top).unwrap_or(i32::MAX)));
        }
        RegisPick::of(&record, origins.into_iter(), PICTURE)
    }

    /// Gives the renderer the animation state held for an object, or
    /// takes it away when none is held.
    fn give(&mut self, number: u16) {
        let before = self.terminal.animation(number);
        match self.animation.get(Target::object(number)).copied() {
            Some(state) => {
                let given = Animation {
                    shown: state.shown,
                    position: state.position.map(|(x, y)| (canvas_x(x), canvas_y(y))),
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
    /// ANIM-13), in the 640 by 480 picture's pixels, or `None` when the
    /// sub-page has no such object.
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
            bounds: self.terminal.bounds(number).map(|bounds| {
                let (left, top, right, bottom) = design_rectangle(bounds);
                Bounds { left, top, right, bottom }
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
    /// Each sub-page's sheet (ANIM-17): its generation and bytes, and
    /// whether a record of it was lost. A sub-page is a page (page-rules.md
    /// RULE-112): no sheet is fed to another sub-code.
    sheets: BTreeMap<(u16, u16), (u16, Vec<u8>, bool)>,
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
            let sheet = self.sheets.get(&key).map(|(_, bytes, _)| bytes.as_slice());
            let Some(mut page) = Page::new(record.generation, sheet, self.at_once) else { return };
            // The objects sent ahead of this generation are its own; those
            // of any other are dropped (ANIM-11).
            if let Some((_, objects)) = self.waiting.remove(&key).filter(|(generation, _)| *generation == record.generation) {
                let _ = page.set_objects(objects);
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

    /// Gives a sub-page all its objects, in the 640 by 480 picture's
    /// pixels, replacing any it had (ANIM-11), and with them their
    /// animation state (ANIM-6). Objects for a generation of the stream
    /// that is not the one shown are kept until that generation begins. A
    /// set the renderer refuses changes nothing.
    pub fn set_objects(&mut self, page: u16, sub: u16, generation: u16, objects: Vec<Object>) {
        let key = (page, sub);
        if let Some(held) = self.pages.get_mut(&key).filter(|held| held.generation == generation) {
            if held.set_objects(objects) {
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
    /// ANIM-11): shown, detectable, the pixel in its rectangle, which the
    /// renderer holds on the canvas. The count written to the front of
    /// `objects`.
    #[must_use]
    pub fn objects_at(&self, page: u16, sub: Option<u16>, x: u16, y: u16, objects: &mut [u16; MAX_POINTER_OBJECTS]) -> usize {
        let Some(held) = self.shown(page, sub).and_then(|key| self.pages.get(&key)) else { return 0 };
        // A pixel past the picture is on nothing (RULE-13).
        if i64::from(x) >= DESIGN_WIDTH || i64::from(y) >= DESIGN_HEIGHT {
            return 0;
        }
        let on = |value: u16, from: i64, to: i64| u32::try_from(i64::from(value).saturating_mul(to).checked_div(from).unwrap_or(0)).unwrap_or(u32::MAX);
        let found = held.terminal.objects_at(on(x, DESIGN_WIDTH, CANVAS_WIDTH), on(y, DESIGN_HEIGHT, CANVAS_HEIGHT));
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

    /// REQ-5819, DEC-6667 (REQ-6408 item 5): a sub-page's records draw a
    /// picture of the teletext raster, 480 by 500, whose lines 19 to 480
    /// are the 480 by 462 NAPLPS canvas, with text drawn in the font built
    /// in, and whose 19 lines above and below are the colour the page
    /// cleared its display to; it is NAPLPS's, with no pick record's
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
        assert!((0..19).chain(481..500).all(|y| line(y).iter().all(|&pixel| pixel == screen)), "the border lines are the page's background colour");
        // The text is drawn at the cursor's place after RESET, the
        // display's bottom left (section 5.3.2.9): in its last 23.1 lines,
        // which on the raster end at line 480.
        let inked_lines: Vec<usize> = (0..500).filter(|&y| line(y).iter().any(|&pixel| pixel != screen)).collect();
        assert!(inked_lines.iter().all(|&y| (481 - 24..481).contains(&y)), "the canvas ends at line 480: {inked_lines:?}");
        let inked = pixels[19 * 480..481 * 480].iter().filter(|&&pixel| pixel != screen).count();
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

    /// RULE-13 on the raster (REQ-5819): a dot of the display area is the
    /// pixel of the 640 by 480 picture under it, with no 80-pixel offset;
    /// a dot of the border lines is past the picture's bottom.
    #[test]
    fn a_dot_of_the_raster_is_the_640_by_480_pixel_under_it() {
        assert_eq!(design_point(0, 19), (0, 0));
        assert_eq!(design_point(12, 19), (16, 0), "column 1 of the raster is the picture's 16th pixel");
        assert_eq!(design_point(479, 480), (638, 478));
        assert_eq!(design_point(240, 250), (320, 240), "the middle");
        assert_eq!(design_point(100, 18), (133, 480), "above the display area: on nothing");
        assert_eq!(design_point(100, 481), (133, 480), "below it: on nothing");
    }

    /// An object of the 640 by 480 picture, 64 by 48 at (320, 240).
    fn square(number: u16, label: &str) -> Object {
        Object {
            number,
            kind: naplps::ObjectKind::Region,
            shown: true,
            detectable: true,
            layer: 1,
            order: 0,
            x: 320,
            y: 240,
            width: 64,
            height: 48,
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

    /// ANIM-11 on the raster (REQ-5819): an object placed in the 640 by
    /// 480 picture is drawn and picked where that place lies on the
    /// raster, its rectangle read back in the 640 by 480 picture's pixels,
    /// and the objects a pixel of that picture is inside are found.
    #[test]
    fn an_object_of_the_640_by_480_picture_lies_where_that_place_is_on_the_raster() {
        let mut pages = NaplpsPages::default();
        pages.read(&record(0x100, 1, 0, b"A"));
        pages.set_objects(0x100, 0, 1, vec![Object { shown: false, ..square(7, "") }]);
        let drawn = pages.take(0x100, Some(0)).unwrap();
        let pixels = unpack(&drawn.picture.packed);
        // (320, 240) is the canvas's (240, 231), the raster's (240, 250);
        // 64 by 48 is 48 by 46.
        assert_eq!(pixels[250 * 480 + 240], 3, "the hidden region's cover, at its place");
        assert_eq!(pixels[250 * 480 + 287], 3);
        assert_eq!(pixels[250 * 480 + 288], 0, "past its right edge");
        assert_eq!(pixels[249 * 480 + 240], 0, "above its top");
        assert_eq!(pixels[295 * 480 + 240], 3);
        assert_eq!(pixels[296 * 480 + 240], 0, "past its bottom");
        pages.set_objects(0x100, 0, 1, vec![square(7, "")]);
        let pick = pages.take(0x100, Some(0)).unwrap().pick.unwrap();
        assert_eq!(pick.at(PictureLanguage::Naplps, 240, 250).drawing, 7, "picked at its dot of the raster");
        assert_eq!(pick.at(PictureLanguage::Naplps, 239, 250).drawing, 0);
        let state = pages.target_state(0x100, Some(0), 7).unwrap();
        assert_eq!((state.x, state.y), (320, 240));
        assert_eq!(state.bounds, Some(Bounds { left: 320, top: 240, right: 384, bottom: 288 }), "its rectangle in the 640 by 480 picture");
        let mut found = [0; MAX_POINTER_OBJECTS];
        assert_eq!(pages.objects_at(0x100, Some(0), 330, 250, &mut found), 1);
        assert_eq!(found[0], 7);
        assert_eq!(pages.objects_at(0x100, Some(0), 310, 250, &mut found), 0);
        assert_eq!(pages.objects_at(0x100, Some(0), 330, 480, &mut found), 0, "past the picture's bottom: the border lines");
    }

    /// REQ-6668, DEC-6669: every NAPLPS terminal has the engine's 55 sets,
    /// in the engine's order, which is the renderer's; a page that
    /// designates Greek (set 17, ESC 2/8 2/1 3/1 into G0) draws the
    /// engine's Noto Teletext Greek glyph for its code 4/4, capital delta,
    /// exactly as a terminal given that glyph alone draws it, and not
    /// nothing, nor the primary set's D.
    #[test]
    fn a_page_designating_greek_draws_the_engines_greek_glyphs() {
        assert_eq!(teletext_sets().len(), TELETEXT_SET_COUNT);
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
        let mut canvas = Canvas::new(480, 462).unwrap();
        let _ = terminal.feed(&mut canvas, b"\x1b\x28\x21\x31D");
        let _ = terminal.finish(&mut canvas);
        let display = &greek[19 * 480..481 * 480];
        assert!(canvas.pixels().iter().any(|&pixel| pixel != 0), "the delta draws");
        assert_eq!(display, canvas.pixels(), "the engine's Greek glyph, as the renderer draws it");
        assert_ne!(greek, drawn(b"D"), "not the primary set's D");
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
