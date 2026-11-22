//! Teletext sprites (work package T2 of script-api.md; section 5.3,
//! operations 112 to 115 and 120 of section 10.2; DEC-7247 item 2), as
//! teletext has them and not as a drawing surface would:
//!
//! - **a sprite** is one of the page's at most [`MAX_SPRITES`] numbered
//!   placements of 1 to [`MAX_OBJECT_FRAMES`] frames, declared by the
//!   engine's `SPRITES` record (ANIM-7) and drawn by the receiver, which
//!   invokes the frame's object at the sprite's origin (ANIM-8);
//! - **a frame** is a Passive object ([`object`]) on the Object page the
//!   decoder writes for the script, where frames that are the same are
//!   one object;
//! - **moving, showing, ordering and cycling** are the receiver's, by the
//!   `ANIMATE` operations 5, 6 and 15 to 19 (ANIM-6): the host sends the
//!   operation and the receiver runs it, a frame cycle included.
//!
//! The host keeps the sprites as the script has left them and, for each
//! step, tells the decoder what differs from what it was last told
//! ([`Sprites::fill`]): the batch's sections `TELETEXT_OBJECTS`,
//! `SPRITES` and `ANIMATE`. The engine's rules shape that:
//!
//! - a `SPRITES` record gives all of a page's sprites and **forgets every
//!   sprite's animation state** (ANIM-6, "last writer wins"). So when a
//!   sprite is made or removed the host declares every sprite as it now
//!   stands, its place, visibility, layer and order in the record itself,
//!   and follows it with the frame or the cycle of each sprite that has
//!   one. A cycle is therefore started again when any sprite of the page
//!   is made or removed: one that runs once from the frame it had
//!   reached, one that repeats from its first frame;
//! - a page's sprites and controls share [`MAX_ANIMATED_OBJECTS`]
//!   animation states, and the engine never gives one back but by a
//!   `SPRITES` record. The host counts them, and declares the sprites
//!   again, which frees every sprite's, before an operation would be the
//!   one the receiver has no state for;
//! - at most [`MAX_CYCLING_OBJECTS`] sprites cycle at once, and one
//!   `ANIMATE` record holds at most [`MAX_ANIMATE_OPERATIONS`].
//!
//! Sprites take none of the page's local triplets: their objects are on
//! the Object page, which the page names by its packet X/27/4, so the
//! page's 208 are the overlay's and the controls' alone. Everything here
//! is bounded by the Object page's real size, so a script cannot make the
//! host hold more for sprites than that page (section 10.1 rule 7).

pub mod limits;
pub mod object;

use std::fmt::Write as _;

use neoteletext_rules::animate::{Cycle, Operation, OperationKind, code};
use neoteletext_rules::table::Target;
use protocol::script::batch::{MAX_TELETEXT_OBJECTS, MAX_WHEEL_THINGS, Sprite as Declared, Triplet};
use protocol::script::{Batch, ContextId};

use self::limits::{
    CELL_BYTES, CHANNEL_SHARE_SPRITES, DEFAULT_INK, DEFAULT_PAPER, DEFAULT_SPRITE_LAYER, FIRST_SPRITE_LAYER, KEEP_LAYER, LAST_SIXELS, LAST_SPRITE_LAYER,
    MAX_ANIMATE_OPERATIONS, MAX_ANIMATED_OBJECTS, MAX_BLOCK_COLUMNS, MAX_BLOCK_ROWS, MAX_CYCLE_PERIOD, MAX_CYCLING_OBJECTS, MAX_FRAMES, MAX_OBJECT_FRAMES,
    MAX_SPRITE_NUMBER, MAX_SPRITES, NO_SETS, OBJECT_PAGE_TRIPLETS, PAGE_SHARE_SPRITES, SPRITE_COLUMN_REACH, SPRITE_ROW_REACH, TEXT_BYTES,
};
use self::object::ObjectPage;
use super::text::Languages;
use super::{bytes_of, exactly, range, type_error, within};
use crate::host::{Payload, Reply};
use crate::ops::Op;

/// A bit of operation `TT_SPRITE`'s `flags` (this session's packing;
/// section 10.2 says only "flags"): the sprite is shown.
pub const FLAG_SHOWN: i32 = 1;
/// The sprite is detectable: the pointer's events see it.
pub const FLAG_DETECTABLE: i32 = 2;
/// The sprite takes the wheel.
pub const FLAG_WHEEL: i32 = 4;
/// One past the greatest `flags`.
const FLAGS_END: i32 = 8;
/// `move`'s first whole number: the place is given, or added to the
/// sprite's own.
const MOVE_TO: i32 = 0;
const MOVE_BY: i32 = 1;

/// Where and how a sprite is drawn: what the `SPRITES` record declares of
/// it and the `ANIMATE` operations show, hide, move and order change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Placing {
    row: i8,
    column: i8,
    shown: bool,
    layer: u8,
    order: i8,
}

/// What the decoder is yet to be told of a sprite's frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrameNews {
    /// Nothing.
    None,
    /// The frame it shows: operation `frame`, which also stops a cycle.
    Frame,
    /// The cycle it runs: operation `cycle`.
    Cycle,
    /// That its cycle stops at the frame shown: operation `stop_cycle`.
    Stop,
}

/// One sprite, as the script has left it.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Sprite {
    /// Its number, 1 to 65,535, which is its handle.
    number: u16,
    /// The context that made it, which alone may change it.
    owner: ContextId,
    /// The scope it belongs to, or 0.
    scope: i32,
    /// Its frames' size in cells.
    columns: u8,
    rows: u8,
    /// Its frames, as object numbers.
    objects: Vec<u16>,
    detectable: bool,
    wheel: bool,
    /// Where and how it is drawn now, and what the decoder was last told.
    now: Placing,
    told: Placing,
    /// The frame it shows, or showed when its cycle began.
    frame: u8,
    /// The cycle it runs, started at the tick after the call's.
    cycle: Option<Cycle>,
    news: FrameNews,
    /// Whether the receiver holds an animation state for it: it has been
    /// sent an operation since the sprites were last declared.
    stated: bool,
}

impl Sprite {
    /// Whether the receiver must hold an animation state for it even
    /// after the sprites are declared again: the record carries a
    /// sprite's place and look, but no frame and no cycle.
    const fn lasting(&self) -> bool {
        self.frame != 0 || self.cycle.is_some()
    }

    /// The operations that tell the decoder how it differs from what it
    /// was told.
    fn news(&self) -> Vec<OperationKind> {
        let mut kinds = Vec::new();
        if self.now.shown != self.told.shown {
            kinds.push(if self.now.shown { OperationKind::Show } else { OperationKind::Hide });
        }
        if (self.now.row, self.now.column) != (self.told.row, self.told.column) {
            kinds.push(OperationKind::Move { relative: false, x: i16::from(self.now.column), y: i16::from(self.now.row) });
        }
        if (self.now.layer, self.now.order) != (self.told.layer, self.told.order) {
            let layer = if self.now.layer == self.told.layer { KEEP_LAYER } else { self.now.layer };
            kinds.push(OperationKind::Order { layer, order: self.now.order });
        }
        match (self.news, self.cycle) {
            (FrameNews::None, _) => {}
            (FrameNews::Cycle, Some(cycle)) => {
                kinds.push(OperationKind::Cycle { first: cycle.first, last: cycle.last, period: cycle.period, once: cycle.once });
            }
            (FrameNews::Stop, _) => kinds.push(OperationKind::StopCycle),
            (FrameNews::Frame | FrameNews::Cycle, _) => kinds.push(OperationKind::Frame(self.frame)),
        }
        kinds
    }

    /// The sprite as a `SPRITES` record declares it.
    fn declared(&self) -> Declared {
        Declared {
            number: self.number,
            shown: self.now.shown,
            detectable: self.detectable,
            layer: self.now.layer,
            order: self.now.order,
            column: self.now.column,
            row: self.now.row,
            width: self.columns,
            height: self.rows,
            objects: self.objects.clone(),
        }
    }
}

/// A tick as the engine's cycles count it.
fn tick32(tick: u64) -> u32 {
    u32::try_from(tick).unwrap_or(u32::MAX)
}

/// The sprites of the page shown and the Object page their frames are on.
pub struct Sprites {
    /// The sprites, in the order they were made.
    list: Vec<Sprite>,
    /// The Object page.
    page: ObjectPage,
    /// The number the next sprite is first tried at: numbers rise, so
    /// that a number is not given again at once and, of two sprites of
    /// one layer and order, the later made lies on top (ANIM-8).
    next: u16,
    /// Whether a sprite was made or removed since the decoder was told.
    declare: bool,
    /// The things that take the wheel, as the decoder was last told.
    wheel_told: Vec<i32>,
}

impl Default for Sprites {
    fn default() -> Self {
        Self { list: Vec::with_capacity(MAX_SPRITES), page: ObjectPage::default(), next: 1, declare: false, wheel_told: Vec::new() }
    }
}

impl Sprites {
    /// The bytes held: fixed, a page's sprites and one Object page at
    /// most, whatever a script asks.
    #[must_use]
    pub const fn state_bytes() -> usize {
        size_of::<Self>()
            .saturating_add(MAX_SPRITES.saturating_mul(size_of::<Sprite>().saturating_add(MAX_OBJECT_FRAMES.saturating_mul(size_of::<u16>()))))
            .saturating_add(OBJECT_PAGE_TRIPLETS.saturating_mul(size_of::<Triplet>()))
            .saturating_add(MAX_TELETEXT_OBJECTS.saturating_mul(size_of::<(u16, Vec<Triplet>, u16)>()))
    }

    /// The bytes of the step's output that grow with what a script does:
    /// the objects and the declarations, in a step that carries them.
    #[must_use]
    pub fn batch_bytes(&self) -> usize {
        let one = |sprite: &Sprite| size_of::<Declared>().saturating_add(sprite.objects.len().saturating_mul(size_of::<u16>()));
        let declared = if self.declare { self.list.iter().fold(0_usize, |sum, sprite| sum.saturating_add(one(sprite))) } else { 0 };
        self.page.batch_bytes().saturating_add(declared)
    }

    /// The place among the sprites of the one `handle` names, when it is
    /// `context`'s: a stale or forged handle, and another context's
    /// sprite, name nothing (section 10.1 rule 4).
    fn place(&self, context: ContextId, handle: i32) -> Option<usize> {
        let number = u16::try_from(handle).ok().filter(|number| *number != 0)?;
        self.list.iter().position(|sprite| sprite.number == number && sprite.owner == context)
    }

    /// Whether `handle` names a sprite of `context`.
    #[must_use]
    pub fn owns(&self, context: ContextId, handle: i32) -> bool {
        self.place(context, handle).is_some()
    }

    /// The sprites that would hold an animation state at the receiver
    /// though the sprites were declared again: with the controls that
    /// hold one, at most [`MAX_ANIMATED_OBJECTS`].
    #[must_use]
    pub fn lasting_states(&self) -> usize {
        self.list.iter().filter(|sprite| sprite.lasting()).count()
    }

    /// A number no sprite has, from where the last was given.
    fn free_number(&mut self) -> u16 {
        loop {
            let number = self.next;
            self.next = if number == MAX_SPRITE_NUMBER { 1 } else { number.saturating_add(1) };
            if !self.list.iter().any(|sprite| sprite.number == number) {
                return number;
            }
        }
    }

    /// `sprite(...)` (section 5.3; operation `TT_SPRITE`): every frame is
    /// compiled and given its object, then the sprite is made; a refusal
    /// leaves nothing behind (section 10.1 rule 3).
    ///
    /// # Errors
    /// The `RangeError` or `TypeError` the script is thrown; nothing has
    /// then changed.
    pub fn create(&mut self, op: Op, ints: &[i32], payload: Payload<'_>, context: ContextId, scope: i32) -> Result<Reply, Reply> {
        let [columns, rows, frames, layer, order, row, column, flags] = exactly(op, ints)?;
        let cells = bytes_of(op, payload)?;
        let width = within(columns, 1, MAX_BLOCK_COLUMNS, "COLUMNS", "a block's columns")?;
        let height = within(rows, 1, MAX_BLOCK_ROWS, "TELETEXT_ROWS", "a block's rows")?;
        let count = usize::from(within(frames, 1, MAX_FRAMES, "MAX_OBJECT_FRAMES", "a sprite's frames")?);
        let frame_bytes = usize::from(width).saturating_mul(usize::from(height)).saturating_mul(CELL_BYTES);
        if cells.len() != frame_bytes.saturating_mul(count) {
            return Err(type_error("every frame of a sprite is a block of the one size, and every cell six bytes"));
        }
        let now = Placing {
            row: placed(row, SPRITE_ROW_REACH, "MAX_SPRITE_ROW", "a sprite's row")?,
            column: placed(column, SPRITE_COLUMN_REACH, "MAX_SPRITE_COLUMN", "a sprite's column")?,
            shown: flags & FLAG_SHOWN != 0,
            layer: layer_of(context, layer)?,
            order: order_of(order)?,
        };
        if !(0..FLAGS_END).contains(&flags) {
            return Err(type_error("the sprite holds a member this host does not know"));
        }
        // Each level has its share of the page's sprites, so that neither
        // can leave the other none (section 3.1).
        let own = self.list.iter().filter(|sprite| sprite.owner == context).count();
        match context {
            ContextId::Channel if own >= CHANNEL_SHARE_SPRITES => {
                return Err(range(
                    "CHANNEL_SHARE_SPRITES",
                    &format!("the channel context has at most {CHANNEL_SHARE_SPRITES} of a page's {MAX_SPRITES} sprites"),
                ));
            }
            ContextId::Page if own >= PAGE_SHARE_SPRITES => {
                return Err(range(
                    "MAX_SPRITES",
                    &format!("a page has at most {MAX_SPRITES} sprites, of which {CHANNEL_SHARE_SPRITES} are kept for the channel context"),
                ));
            }
            _ => {}
        }
        // The decoder watches the wheel for a bounded list of things.
        if flags & FLAG_WHEEL != 0 && self.list.iter().filter(|sprite| sprite.wheel).count() >= MAX_WHEEL_THINGS {
            return Err(range("MAX_WHEEL_THINGS", &format!("at most {MAX_WHEEL_THINGS} things of a page take the wheel")));
        }
        let mut bodies = Vec::with_capacity(count);
        for frame in cells.chunks(frame_bytes.max(1)) {
            bodies.push(object::compile(width, height, frame)?);
        }
        let mut objects = Vec::with_capacity(count);
        for body in &bodies {
            let Some(number) = self.page.take(context, body) else {
                for taken in objects {
                    self.page.release(taken);
                }
                return Err(range(
                    "OBJECT_PAGE_TRIPLETS",
                    &format!(
                        "the frames' objects do not fit the sub-pages of the Object page this context's sprites have; the page holds {} of its {OBJECT_PAGE_TRIPLETS} triplets",
                        self.page.triplets_used()
                    ),
                ));
            };
            objects.push(number);
        }
        let number = self.free_number();
        self.list.push(Sprite {
            number,
            owner: context,
            scope,
            columns: width,
            rows: height,
            objects,
            detectable: flags & FLAG_DETECTABLE != 0,
            wheel: flags & FLAG_WHEEL != 0,
            now,
            told: now,
            frame: 0,
            cycle: None,
            news: FrameNews::None,
            stated: false,
        });
        self.declare = true;
        Ok(Reply::Int(i32::from(number)))
    }

    /// One animation operation on a sprite (section 5.3; operation
    /// `TT_ANIMATE`): `show`, `hide`, `move`, `order`, `frame`, `cycle`
    /// or `stop_cycle`, with its whole numbers as ANIM-6 gives its body.
    /// `control_states` is the animation states the page's controls hold
    /// at the receiver, and `tick` the presentation tick of the call.
    ///
    /// # Errors
    /// The `RangeError` or `TypeError` the script is thrown; nothing has
    /// then changed.
    pub fn animate(&mut self, context: ContextId, handle: i32, operation: [i32; 5], control_states: usize, tick: u64) -> Result<(), Reply> {
        let place = self.place(context, handle).ok_or_else(|| type_error("no such thing"))?;
        let cycling = self.list.iter().filter(|sprite| sprite.cycle.is_some()).count();
        let lasting = self.lasting_states();
        let sprite = self.list.get_mut(place).ok_or_else(|| type_error("no such thing"))?;
        let [code, first, second, third, fourth] = operation;
        let code = u8::try_from(code).map_err(|_| type_error("no such animation operation"))?;
        let frames = i32::try_from(sprite.objects.len()).unwrap_or(MAX_FRAMES);
        let frame_of = |frame: i32| within(frame, 0, frames.saturating_sub(1), "MAX_OBJECT_FRAMES", "a frame of this sprite");
        let unused =
            |values: &[i32]| if values.iter().all(|value| *value == 0) { Ok(()) } else { Err(type_error("the animation operation takes fewer whole numbers")) };
        // A state the receiver must keep whatever is declared again is
        // one of the page's few (ANIM-6).
        let lasting_room = |sprite: &Sprite| {
            if !sprite.lasting() && lasting.saturating_add(control_states) >= MAX_ANIMATED_OBJECTS {
                return Err(range(
                    "MAX_ANIMATED_OBJECTS",
                    &format!("a page's sprites and controls hold at most {MAX_ANIMATED_OBJECTS} animation states, and a frame or a cycle is one"),
                ));
            }
            Ok(())
        };
        match code {
            code::SHOW | code::HIDE => {
                unused(&[first, second, third, fourth])?;
                sprite.now.shown = code == code::SHOW;
            }
            code::MOVE => {
                unused(&[fourth])?;
                let (by_column, by_row) = match first {
                    MOVE_TO => (0, 0),
                    MOVE_BY => (i32::from(sprite.now.column), i32::from(sprite.now.row)),
                    _ => return Err(type_error("a move is to a place or by an amount")),
                };
                let column = placed(by_column.saturating_add(second), SPRITE_COLUMN_REACH, "MAX_SPRITE_COLUMN", "a sprite's column")?;
                let row = placed(by_row.saturating_add(third), SPRITE_ROW_REACH, "MAX_SPRITE_ROW", "a sprite's row")?;
                (sprite.now.column, sprite.now.row) = (column, row);
            }
            code::ORDER => {
                unused(&[third, fourth])?;
                let layer = if first == i32::from(KEEP_LAYER) { sprite.now.layer } else { layer_of(context, first)? };
                let order = order_of(second)?;
                (sprite.now.layer, sprite.now.order) = (layer, order);
            }
            code::FRAME => {
                unused(&[second, third, fourth])?;
                let frame = frame_of(first)?;
                if frame != 0 {
                    lasting_room(sprite)?;
                }
                (sprite.frame, sprite.cycle, sprite.news) = (frame, None, FrameNews::Frame);
            }
            code::CYCLE => {
                let (from, to) = (frame_of(first)?, frame_of(second)?);
                if from > to {
                    return Err(range("MAX_OBJECT_FRAMES", &format!("a cycle's first frame, {from}, is after its last, {to}")));
                }
                let period = u16::try_from(third)
                    .ok()
                    .filter(|period| (1..=MAX_CYCLE_PERIOD).contains(period))
                    .ok_or_else(|| range("MAX_CYCLE_PERIOD", &format!("a cycle's period is 1 to {MAX_CYCLE_PERIOD} ticks a frame, not {third}")))?;
                if !(0..=1).contains(&fourth) {
                    return Err(type_error("once is true or false"));
                }
                if sprite.cycle.is_none() && cycling >= MAX_CYCLING_OBJECTS {
                    return Err(range("MAX_CYCLING_OBJECTS", &format!("at most {MAX_CYCLING_OBJECTS} sprites of a page cycle at once")));
                }
                lasting_room(sprite)?;
                // The receiver starts a cycle at the tick after its
                // operation is applied (ANIM-14).
                let cycle = Cycle { first: from, last: to, period, once: fourth == 1, start: tick32(tick).saturating_add(1) };
                (sprite.frame, sprite.cycle, sprite.news) = (from, Some(cycle), FrameNews::Cycle);
            }
            code::STOP_CYCLE => {
                unused(&[first, second, third, fourth])?;
                if let Some(cycle) = sprite.cycle.take() {
                    // The receiver stops at the frame it shows; the host
                    // follows by the cycle's own rule. A cycle the
                    // decoder was never told of is its first frame.
                    sprite.frame = cycle.frame_at(tick32(tick)).0;
                    sprite.news = if sprite.news == FrameNews::Cycle { FrameNews::Frame } else { FrameNews::Stop };
                }
            }
            _ => return Err(type_error("a sprite takes the animation operations show, hide, move, order, frame, cycle and stop_cycle")),
        }
        Ok(())
    }

    /// Forgets the sprites `gone` chooses: their objects are released,
    /// and their handles given.
    fn forget(&mut self, gone: impl Fn(&Sprite) -> bool) -> Vec<i32> {
        let mut handles = Vec::new();
        let mut kept = Vec::with_capacity(self.list.len());
        for sprite in core::mem::take(&mut self.list) {
            if gone(&sprite) {
                for object in &sprite.objects {
                    self.page.release(*object);
                }
                handles.push(i32::from(sprite.number));
            } else {
                kept.push(sprite);
            }
        }
        self.list = kept;
        self.declare |= !handles.is_empty();
        handles
    }

    /// `sprite.remove()` (operation `TT_REMOVE`).
    ///
    /// # Errors
    /// The `RangeError` or `TypeError` the script is thrown; nothing has
    /// then changed.
    pub fn remove(&mut self, context: ContextId, handle: i32) -> Result<(), Reply> {
        let place = self.place(context, handle).ok_or_else(|| type_error("no such thing"))?;
        let number = self.list.get(place).map_or(0, |sprite| sprite.number);
        self.forget(|sprite| sprite.number == number);
        Ok(())
    }

    /// The scope `scope` of `context` ends: its sprites are removed.
    pub fn remove_scope(&mut self, context: ContextId, scope: i32) -> Vec<i32> {
        self.forget(|sprite| sprite.owner == context && sprite.scope == scope)
    }

    /// Everything `context` drew goes: the page is left, or hidden from
    /// the channel context (section 3.1).
    pub fn remove_context(&mut self, context: ContextId) -> Vec<i32> {
        self.forget(|sprite| sprite.owner == context)
    }

    /// A step begins at the tick `tick`: a cycle that ran once and has
    /// reached its last frame is over, as the receiver ended it (ANIM-14).
    pub fn begin_step(&mut self, tick: u64) {
        for sprite in &mut self.list {
            if let Some(cycle) = sprite.cycle
                && let (frame, true) = cycle.frame_at(tick32(tick))
                && sprite.news != FrameNews::Cycle
            {
                (sprite.frame, sprite.cycle) = (frame, None);
            }
        }
    }

    /// The step's changes, as the batch's sections; what the decoder has
    /// been told is then what the sprites are. `control_states` is the
    /// animation states the page's controls hold at the receiver.
    pub fn fill(&mut self, batch: &mut Batch, control_states: usize, tick: u64) {
        if let Some(objects) = self.page.section() {
            batch.teletext_objects = Some(objects);
        }
        let news: Vec<Vec<OperationKind>> = self.list.iter().map(Sprite::news).collect();
        let operations = news.iter().fold(0_usize, |sum, kinds| sum.saturating_add(kinds.len()));
        let states = self.list.iter().zip(&news).filter(|(sprite, kinds)| sprite.stated || !kinds.is_empty()).count();
        let staged = batch.animate.as_ref().map_or(0, Vec::len);
        // The sprites are declared again when one was made or removed,
        // when the receiver would have no animation state left for an
        // operation, and when a record could not hold the operations.
        let declare = core::mem::take(&mut self.declare)
            || states.saturating_add(control_states) > MAX_ANIMATED_OBJECTS
            || staged.saturating_add(operations) > MAX_ANIMATE_OPERATIONS;
        let mut out = Vec::new();
        if declare {
            batch.sprites = Some(self.list.iter().map(Sprite::declared).collect());
            let now = tick32(tick);
            for sprite in &mut self.list {
                sprite.told = sprite.now;
                let target = Target::object(sprite.number);
                if let Some(cycle) = sprite.cycle {
                    // A cycle that runs once goes on from the frame it
                    // had reached; one that repeats starts again.
                    let first = if cycle.once { cycle.frame_at(now).0 } else { cycle.first };
                    let again = Cycle { first, start: now.saturating_add(1), ..cycle };
                    sprite.cycle = Some(again);
                    out.push(Operation { target, kind: OperationKind::Cycle { first, last: again.last, period: again.period, once: again.once } });
                } else if sprite.frame != 0 {
                    out.push(Operation { target, kind: OperationKind::Frame(sprite.frame) });
                }
                sprite.stated = sprite.lasting();
                sprite.news = FrameNews::None;
            }
        } else {
            for (sprite, kinds) in self.list.iter_mut().zip(news) {
                sprite.stated |= !kinds.is_empty();
                sprite.told = sprite.now;
                sprite.news = FrameNews::None;
                out.extend(kinds.into_iter().map(|kind| Operation { target: Target::object(sprite.number), kind }));
            }
        }
        if !out.is_empty() {
            batch.animate.get_or_insert_default().append(&mut out);
        }
        let wheel: Vec<i32> = self.list.iter().filter(|sprite| sprite.wheel).map(|sprite| i32::from(sprite.number)).collect();
        if wheel != self.wheel_told {
            self.wheel_told.clone_from(&wheel);
            batch.watch.get_or_insert_default().wheel = Some(wheel);
        }
    }

    /// The sprites' entries of the budget report (section 3.7), each
    /// written after a comma.
    pub fn budget(&self, out: &mut String) {
        let _ = write!(
            out,
            ",\"sprites\":{{\"used\":{},\"limit\":{MAX_SPRITES}}},\"spriteFrames\":{{\"used\":{},\"limit\":{MAX_TELETEXT_OBJECTS}}},\"objectPageTriplets\":{{\"used\":{},\"limit\":{OBJECT_PAGE_TRIPLETS}}}",
            self.list.len(),
            self.page.len(),
            self.page.triplets_used(),
        );
    }
}

/// The numbers the prelude's sprite part shares with this file, written
/// in front of the prelude in the table `TT` (section 10.1: defined once,
/// in Rust).
#[must_use]
pub fn prelude_numbers() -> Vec<(&'static str, i64)> {
    let byte = |value: u8| i64::from(value);
    vec![
        ("CELL_BYTES", i64::try_from(CELL_BYTES).unwrap_or(i64::MAX)),
        ("CELL_NONE", byte(object::CELL_NONE)),
        ("CELL_CONTIGUOUS", byte(object::CELL_CONTIGUOUS)),
        ("CELL_SEPARATED", byte(object::CELL_SEPARATED)),
        ("CELL_CHARACTER", byte(object::CELL_CHARACTER)),
        ("TEXT_BYTES", i64::try_from(TEXT_BYTES).unwrap_or(i64::MAX)),
        ("SPACE_MODE", byte(neoteletext_core::consts::MODE_G0_LEVEL_25)),
        ("SPACE", byte(neoteletext_core::consts::SPACE)),
        ("NO_SETS", byte(NO_SETS)),
        ("DEFAULT_INK", byte(DEFAULT_INK)),
        ("DEFAULT_PAPER", byte(DEFAULT_PAPER)),
        ("LAST_SIXELS", byte(LAST_SIXELS)),
        ("SPRITE_SHOWN", i64::from(FLAG_SHOWN)),
        ("SPRITE_DETECTABLE", i64::from(FLAG_DETECTABLE)),
        ("SPRITE_WHEEL", i64::from(FLAG_WHEEL)),
        ("MOVE_TO", i64::from(MOVE_TO)),
        ("MOVE_BY", i64::from(MOVE_BY)),
        ("KEEP_LAYER", byte(KEEP_LAYER)),
        ("SHOW", byte(code::SHOW)),
        ("HIDE", byte(code::HIDE)),
        ("MOVE", byte(code::MOVE)),
        ("ORDER", byte(code::ORDER)),
        ("FRAME", byte(code::FRAME)),
        ("CYCLE", byte(code::CYCLE)),
        ("STOP_CYCLE", byte(code::STOP_CYCLE)),
        ("MAX_OBJECT_FRAMES", i64::from(MAX_FRAMES)),
        ("MAX_BLOCK_COLUMNS", i64::from(MAX_BLOCK_COLUMNS)),
        ("MAX_BLOCK_ROWS", i64::from(MAX_BLOCK_ROWS)),
        ("SPRITE_LAYER", i64::from(DEFAULT_SPRITE_LAYER)),
        ("CHANNEL_LAYER", i64::from(LAST_SPRITE_LAYER)),
        ("PAGE_COLUMNS", i64::from(MAX_BLOCK_COLUMNS)),
        ("PAGE_ROWS", i64::try_from(neoteletext_core::consts::ROWS).unwrap_or(i64::MAX)),
    ]
}

/// A text as the cells of a block (operation `TT_BLOCK_TEXT`): for each
/// cell the engine's encoder gives the text in `language`, the triplet
/// that writes its character, [`TEXT_BYTES`] bytes a cell: the triplet's
/// mode, its data, and the designation of the sets it is read in. A
/// sprite's frame is an object, whose characters are X/26 characters,
/// so the text is encoded as the overlay's is (section 5.6).
///
/// # Errors
/// The `RangeError` or `TypeError` the script is thrown; nothing has
/// then changed.
pub fn block_text(languages: &Languages, text: &str, language: i32) -> Result<Reply, Reply> {
    let characters = languages.overlay(neoteletext_core::consts::PACKET_FIRST_ROW, 0, text, language)?;
    Ok(Reply::Bytes(characters.iter().flat_map(|character| [character.mode, character.data, character.sets]).collect()))
}

/// A sprite's row or column: within `reach` either way of the page's
/// first (the `SPRITES` record, ANIM-7).
fn placed(value: i32, reach: i32, bound: &str, what: &str) -> Result<i8, Reply> {
    i8::try_from(value)
        .ok()
        .filter(|_| (reach.saturating_neg()..=reach).contains(&value))
        .ok_or_else(|| range(bound, &format!("{what} is -{reach} to {reach}, not {value}")))
}

/// A sprite's layer: 1 to 3, and for the channel context the overlay's
/// alone, which is all of the page it may draw on (section 3.1).
fn layer_of(context: ContextId, layer: i32) -> Result<u8, Reply> {
    let layer = within(layer, FIRST_SPRITE_LAYER, LAST_SPRITE_LAYER, "LAST_LAYER", "a sprite's layer")?;
    if context == ContextId::Channel && i32::from(layer) != LAST_SPRITE_LAYER {
        return Err(type_error("the channel context draws in the overlay's layer, 3, alone"));
    }
    Ok(layer)
}

/// A sprite's order within its layer: a signed byte (ANIM-12).
fn order_of(order: i32) -> Result<i8, Reply> {
    i8::try_from(order).map_err(|_| range("SPRITE_ORDER", &format!("a sprite's order is {} to {}, not {order}", i8::MIN, i8::MAX)))
}

#[cfg(test)]
mod tests;
