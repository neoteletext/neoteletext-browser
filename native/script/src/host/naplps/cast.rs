//! NAPLPS definitions and the cast (script-api.md sections 7.4 and 7.5;
//! work package N2, DEC-7247 item 4; its own choices are DEC-7377): what the format and NeoTeletext
//! retain beside the backdrop. **Symbols** are macros the host names,
//! written entirely in relative forms; **DRCS characters** and
//! **textures** are one-bit bitmaps a painter draws; the **cast** is at
//! most 64 actors and regions, the objects of the engine's
//! `NAPLPS_OBJECTS` record (animation.md ANIM-11), moved, shown, ordered
//! and cycled by `ANIMATE` operations the receiver applies (ANIM-6,
//! ANIM-14).
//!
//! The host keeps each definition's bytes, because a new generation
//! deletes the receiver's (RESET) and the host sends them again
//! (section 7.2), and it keeps each actor as its state stands, because a
//! record of all the objects makes the receiver forget their animation
//! state. Everything kept is bounded by a named constant and counted
//! against the container's host state (section 10.1 rule 7, TODO-7272).

use std::fmt::Write as _;

use neoteletext_rules::animate::{Operation, OperationKind, code};
use neoteletext_rules::table::Target as AnimateTarget;
use protocol::script::batch::{MAX_WHEEL_THINGS, NaplpsObject, NaplpsObjectKind, NaplpsObjects};
use protocol::script::{Batch, ContextId};

use super::limits::{
    CANVAS_HEIGHT, CANVAS_TOP_LINE, CANVAS_WIDTH, CELL_DOTS, CELL_LINES, CHANNEL_SHARE_ACTORS, CHANNEL_SHARE_SYMBOLS, DEFINABLE_CODES, MAX_ALL_MACRO_BYTES,
    MAX_ANIMATE_OPERATIONS, MAX_CYCLE_PERIOD, MAX_CYCLING_OBJECTS, MAX_FRAME, MAX_MACRO_BYTES, MAX_MASK_DOTS, MAX_OBJECT_FRAMES, MAX_OBJECTS, NAPLPS_COLOURS,
    NAPLPS_MAX_PATTERN_BYTES, TEXTURE_MASKS,
};
use super::writer::{Definitions, Painter};
use super::{HANDLE_KIND_SHIFT, KIND_ACTOR, KIND_DRCS, KIND_SYMBOL, KIND_TEXTURE, program_of};
use crate::host::{Host, Ints, Payload, Reply, range, text_of, type_error};
use crate::limits::MAX_LABEL_BYTES;
use crate::ops::Op;

const ESC: u8 = 0x1B;
/// The C1 definitions as the final byte after ESC (ANSI X3.110-1983
/// section 6.2): DEF MACRO, DEF DRCS, DEF TEXTURE and END.
const DEF_MACRO: u8 = 0x40;
const DEF_DRCS: u8 = 0x43;
const DEF_TEXTURE: u8 = 0x44;
const END: u8 = 0x45;
/// The first code a macro or a DRCS character is named by, 2/0 (section 6.2.2).
const FIRST_CODE: u8 = 0x20;
/// The byte that names programmable pattern A in DEF TEXTURE, 4/1 (section 6.2.4).
const FIRST_TEXTURE: u8 = 0x41;
/// The bytes a definition adds around its contents: ESC, the definition,
/// its code, and ESC, END.
const DEFINITION_BYTES: usize = 5;
/// The most bytes written before DEF DRCS or DEF TEXTURE to give the
/// bitmap its shape: SO, DOMAIN (8), TEXT or TEXTURE with a size (9).
const BEFORE_PATTERN_BYTES: usize = 18;
/// The bits of a symbol's handle that hold its slot; those above, below
/// the kind, count the symbols the slot has held, so that the handle of a
/// symbol removed does not name the one that takes its place.
const SLOT_BITS: u32 = 8;
const SLOT_MASK: i32 = (1 << SLOT_BITS) - 1;
const COUNT_MASK: i32 = (1 << (HANDLE_KIND_SHIFT - SLOT_BITS)) - 1;
/// The kinds of object `NP_ACTOR` takes: the record's own numbers.
const KIND_MACRO_OBJECT: i32 = 1;
const KIND_REGION_OBJECT: i32 = 2;
/// The bits of `NP_ACTOR`'s flags.
const FLAG_SHOWN: i32 = 1;
const FLAG_DETECTABLE: i32 = 2;
const FLAG_WHEEL: i32 = 4;
const ALL_FLAGS: i32 = FLAG_SHOWN | FLAG_DETECTABLE | FLAG_WHEEL;
/// The looks an object has beside its frames: hover, pressed, alternative.
const LOOKS: usize = protocol::script::batch::LOOKS;
/// The forced looks: 0 normal, 1 hover, 2 pressed, 3 alternative (ANIM-6).
const LAST_LOOK: i32 = 3;
/// `order`'s layer byte for "the layer in force" (ANIM-6).
pub(super) const KEEP_LAYER: u8 = 255;
/// The first and last layers a script's object may lie in (ANIM-12:
/// layer 0 is the stream's own picture).
const FIRST_LAYER: i32 = 1;
const LAST_LAYER: i32 = 3;
/// The raster line one past the canvas's last: where a macro that an
/// object runs starts drawing is the line above it (ANIM-11).
const RASTER_FOOT: i32 = CANVAS_TOP_LINE + CANVAS_HEIGHT;

/// The most bytes of one object in the engine's `NAPLPS_OBJECTS` record:
/// its 24 fixed bytes, a code for each frame and its label (ANIM-11).
const OBJECT_RECORD_BYTES: usize = 24 + MAX_OBJECT_FRAMES + MAX_LABEL_BYTES;

/// The symbols a page context may hold; the channel context's are the
/// codes after them.
const PAGE_SYMBOLS: usize = DEFINABLE_CODES - CHANNEL_SHARE_SYMBOLS;
/// The objects a page context may hold.
const PAGE_ACTORS: usize = MAX_OBJECTS - CHANNEL_SHARE_ACTORS;

/// A symbol: a macro of the stream.
#[derive(Clone, Debug)]
struct Symbol {
    width: i32,
    height: i32,
    /// The macro's contents.
    body: Vec<u8>,
    /// The scope it was made in, or 0.
    scope: i32,
}

/// A texture: one of the programmable patterns.
#[derive(Clone, Debug)]
struct Texture {
    width: i32,
    height: i32,
    body: Vec<u8>,
}

/// A frame cycle as it was last started (ANIM-14).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cycle {
    first: u8,
    last: u8,
    period: u16,
    once: bool,
}

/// An actor or a region, as its state stands.
#[derive(Clone, Debug)]
#[expect(clippy::struct_excessive_bools, reason = "four facts of the record's object, each a field of its own there")]
struct Actor {
    number: u16,
    context: ContextId,
    scope: i32,
    region: bool,
    /// Its rectangle's top left, in canvas dots.
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    /// Where in its symbols the rectangle's top left is.
    origin: (i32, i32),
    layer: u8,
    order: i8,
    shown: bool,
    detectable: bool,
    wheel: bool,
    cover: u8,
    /// The slots of its look symbols.
    looks: [Option<usize>; LOOKS],
    /// The slots of its frames' symbols.
    frames: Vec<usize>,
    label: String,
    frame: u8,
    cycle: Option<Cycle>,
    look: u8,
}

/// The page's definitions and cast.
#[derive(Clone, Debug)]
pub struct Cast {
    /// The symbols, by slot: the macro's code is 2/0 and the slot.
    symbols: Vec<Option<Symbol>>,
    /// How many symbols each slot has held.
    counts: Vec<u16>,
    /// The DRCS characters' contents, by code from 2/0.
    drcs: Vec<Vec<u8>>,
    textures: Vec<Texture>,
    actors: Vec<Actor>,
    next_number: u16,
    /// Every object must be sent again.
    whole: bool,
    /// The objects made or redefined since the last batch.
    changed: Vec<u16>,
    /// The operations waiting to leave.
    operations: Vec<Operation>,
    wheel_changed: bool,
}

impl Default for Cast {
    fn default() -> Self {
        Self {
            symbols: vec![None; DEFINABLE_CODES],
            counts: vec![0; DEFINABLE_CODES],
            drcs: Vec::new(),
            textures: Vec::new(),
            actors: Vec::new(),
            next_number: 1,
            whole: false,
            changed: Vec::new(),
            operations: Vec::new(),
            wheel_changed: false,
        }
    }
}

/// A slot's code: 2/0 and the slot.
fn code_of(slot: usize) -> u8 {
    FIRST_CODE.saturating_add(u8::try_from(slot).unwrap_or(0))
}

/// The slots of `context`'s symbols.
const fn pool(context: ContextId) -> core::ops::Range<usize> {
    match context {
        ContextId::Page => 0..PAGE_SYMBOLS,
        ContextId::Channel => PAGE_SYMBOLS..DEFINABLE_CODES,
    }
}

fn handle(kind: i32, rest: i32) -> i32 {
    kind.checked_shl(HANDLE_KIND_SHIFT).unwrap_or(0) | rest
}

/// The kind a handle names, and the rest of it.
fn parts(handle: i32) -> (i32, i32) {
    (handle.checked_shr(HANDLE_KIND_SHIFT).unwrap_or(0), handle & ((1 << HANDLE_KIND_SHIFT) - 1))
}

/// A definition: ESC, `definition`, `code`, the contents, ESC, END.
fn definition(definition: u8, code: u8, body: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&[ESC, definition, code]);
    out.extend_from_slice(body);
    out.extend_from_slice(&[ESC, END]);
}

impl Cast {
    /// The handle of the symbol in `slot`.
    fn symbol_handle(&self, slot: usize) -> i32 {
        let count = i32::from(self.counts.get(slot).copied().unwrap_or(0)) & COUNT_MASK;
        handle(KIND_SYMBOL, count.checked_shl(SLOT_BITS).unwrap_or(0) | (i32::try_from(slot).unwrap_or(0) & SLOT_MASK))
    }

    /// The slot of the live symbol `handle` names.
    fn symbol_slot(&self, handle: i32) -> Option<usize> {
        let (kind, rest) = parts(handle);
        let slot = usize::try_from(rest & SLOT_MASK).ok()?;
        (kind == KIND_SYMBOL && self.symbols.get(slot)?.is_some() && self.symbol_handle(slot) == handle).then_some(slot)
    }

    /// The slot of `context`'s own live symbol `handle` names.
    fn own_symbol(&self, context: ContextId, handle: i32) -> Result<usize, Reply> {
        self.symbol_slot(handle).filter(|slot| pool(context).contains(slot)).ok_or_else(|| type_error("no such symbol"))
    }

    /// The place in the cast of `context`'s own actor or region `handle` names.
    fn own_actor(&self, context: ContextId, handle: i32) -> Result<usize, Reply> {
        let (kind, number) = parts(handle);
        self.actors
            .iter()
            .position(|actor| kind == KIND_ACTOR && i32::from(actor.number) == number && actor.context == context)
            .ok_or_else(|| type_error("no such actor or region"))
    }

    /// Whether `handle` names a thing of `context`'s: an actor or a region.
    pub(super) fn is_thing(&self, context: ContextId, handle: i32) -> bool {
        self.own_actor(context, handle).is_ok()
    }

    /// The bytes of every symbol's contents.
    fn symbol_bytes(&self) -> usize {
        self.symbols.iter().flatten().fold(0_usize, |sum, symbol| sum.saturating_add(symbol.body.len()))
    }

    /// The bytes of every DRCS character's and texture's contents.
    fn pattern_bytes(&self) -> usize {
        let drcs = self.drcs.iter().fold(0_usize, |sum, body| sum.saturating_add(body.len()));
        self.textures.iter().fold(drcs, |sum, texture| sum.saturating_add(texture.body.len()))
    }

    /// The bytes held.
    pub(super) fn state_bytes(&self) -> usize {
        let actors = self.actors.iter().fold(0_usize, |sum, actor| {
            sum.saturating_add(size_of::<Actor>()).saturating_add(actor.label.len()).saturating_add(actor.frames.len().saturating_mul(size_of::<usize>()))
        });
        self.symbol_bytes()
            .saturating_add(self.pattern_bytes())
            .saturating_add(self.symbols.len().saturating_mul(size_of::<Option<Symbol>>()))
            .saturating_add(self.drcs.len().saturating_mul(size_of::<Vec<u8>>()))
            .saturating_add(self.textures.len().saturating_mul(size_of::<Texture>()))
            .saturating_add(actors)
            .saturating_add(self.staged_bytes())
    }

    /// The bytes waiting for the step's batch, beside the stream's: the
    /// operations, and the objects' record when one is owed, each object
    /// at the most its record takes.
    pub(super) fn staged_bytes(&self) -> usize {
        let objects = if self.whole || self.changed.len() > 1 { self.actors.len() } else { self.changed.len() };
        self.operations.len().saturating_mul(size_of::<Operation>()).saturating_add(objects.saturating_mul(OBJECT_RECORD_BYTES))
    }

    /// Every definition, written again for a new generation (section
    /// 7.2): the receiver's RESET deleted them. The objects are then sent
    /// again too.
    pub(super) fn resend(&mut self, painter: &mut Painter, out: &mut Vec<u8>) {
        painter.end(out);
        for (slot, symbol) in self.symbols.iter().enumerate() {
            if let Some(symbol) = symbol {
                definition(DEF_MACRO, code_of(slot), &symbol.body, out);
            }
        }
        for (slot, body) in self.drcs.iter().enumerate() {
            painter.before_drcs(out);
            definition(DEF_DRCS, code_of(slot), body, out);
            painter.forget();
        }
        for (slot, texture) in self.textures.iter().enumerate() {
            painter.before_texture(texture.width, texture.height, out);
            definition(DEF_TEXTURE, FIRST_TEXTURE.saturating_add(u8::try_from(slot).unwrap_or(0)), &texture.body, out);
            painter.forget();
        }
        self.whole = !self.actors.is_empty() || self.whole;
    }

    /// An actor as the record's object: its state as it stands.
    fn record(&self, actor: &Actor) -> NaplpsObject {
        let code = |slot: &usize| code_of(*slot);
        // A macro an object runs starts at the unit screen's origin, the
        // raster's dot (0, 479), which is its symbol's lower left; the
        // dot of the symbol that is the rectangle's top left is there.
        let tall = actor.frames.first().and_then(|slot| self.symbols.get(*slot)?.as_ref()).map_or(0, |symbol| symbol.height);
        let origin = if actor.region {
            (0, 0)
        } else {
            let down = RASTER_FOOT.saturating_sub(tall).saturating_add(actor.origin.1);
            (i16::try_from(actor.origin.0).unwrap_or(0), i16::try_from(down).unwrap_or(0))
        };
        NaplpsObject {
            number: actor.number,
            kind: if actor.region { NaplpsObjectKind::Region } else { NaplpsObjectKind::Macro },
            shown: actor.shown,
            detectable: actor.detectable,
            layer: actor.layer,
            order: actor.order,
            x: actor.x,
            y: actor.y.saturating_add(i16::try_from(CANVAS_TOP_LINE).unwrap_or(0)),
            width: actor.width,
            height: actor.height,
            origin,
            cover: actor.cover,
            looks: actor.looks.map(|look| look.as_ref().map_or(0, code)),
            frames: actor.frames.iter().map(code).collect(),
            label: actor.label.clone(),
        }
    }

    /// The operations that give an object sent again its frame, cycle
    /// and forced look back: the record carries the rest of its state.
    fn restore(actor: &Actor, out: &mut Vec<Operation>) {
        let target = AnimateTarget::object(actor.number);
        match actor.cycle {
            Some(Cycle { first, last, period, once }) => out.push(Operation { target, kind: OperationKind::Cycle { first, last, period, once } }),
            None if actor.frame != 0 => out.push(Operation { target, kind: OperationKind::Frame(actor.frame) }),
            None => {}
        }
        if actor.look != 0 {
            out.push(Operation { target, kind: OperationKind::Look(actor.look) });
        }
    }

    /// Moves what the step changed into its batch: the objects, whole or
    /// the one that changed, then the animation operations, as many as a
    /// batch holds; the rest leave with the next. `title` is the page's.
    pub(super) fn take_batch(&mut self, batch: &mut Batch, title: &str) {
        self.changed.retain(|number| self.actors.iter().any(|actor| actor.number == *number));
        self.changed.dedup();
        if self.whole || self.changed.len() > 1 {
            // A record of every object makes the receiver forget each
            // one's animation state, and the record says where each is,
            // so the operations still waiting are spent and each object's
            // frame, cycle and look are given back.
            self.operations.clear();
            for actor in &self.actors {
                Self::restore(actor, &mut self.operations);
            }
            batch.naplps_objects =
                Some(NaplpsObjects { one: false, title: title.to_owned(), objects: self.actors.iter().map(|actor| self.record(actor)).collect() });
        } else if let Some(actor) = self.changed.first().and_then(|number| self.actors.iter().find(|actor| actor.number == *number)) {
            let target = AnimateTarget::object(actor.number);
            self.operations.retain(|operation| operation.target != target);
            Self::restore(actor, &mut self.operations);
            batch.naplps_objects = Some(NaplpsObjects { one: true, title: String::new(), objects: vec![self.record(actor)] });
        }
        self.whole = false;
        self.changed.clear();
        if !self.operations.is_empty() {
            let animate = batch.animate.get_or_insert_default();
            let room = MAX_ANIMATE_OPERATIONS.saturating_sub(animate.len()).min(self.operations.len());
            animate.extend(self.operations.drain(..room));
        }
        if core::mem::take(&mut self.wheel_changed) {
            let wheel = self.actors.iter().filter(|actor| actor.wheel).map(|actor| handle(KIND_ACTOR, i32::from(actor.number))).collect();
            batch.watch.get_or_insert_default().wheel = Some(wheel);
        }
    }

    /// Everything of `context` goes: its actors, regions and symbols.
    pub(super) fn drop_context(&mut self, context: ContextId) {
        let before = self.actors.len();
        self.actors.retain(|actor| actor.context != context);
        self.whole |= self.actors.len() != before;
        self.wheel_changed = true;
        for slot in pool(context) {
            self.free_symbol(slot);
        }
    }

    fn free_symbol(&mut self, slot: usize) {
        if let Some(held) = self.symbols.get_mut(slot)
            && held.take().is_some()
            && let Some(count) = self.counts.get_mut(slot)
        {
            *count = count.wrapping_add(1);
        }
    }

    /// Whether any actor draws with the symbol in `slot`.
    fn in_use(&self, slot: usize) -> bool {
        self.actors.iter().any(|actor| actor.frames.contains(&slot) || actor.looks.contains(&Some(slot)))
    }

    /// The NAPLPS entries of the budget report that are the cast's
    /// (section 3.7), for `context`'s share.
    pub(super) fn budget(&self, context: ContextId, out: &mut String) {
        let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        let symbols = pool(context).filter(|slot| self.symbols.get(*slot).is_some_and(Option::is_some)).count();
        let actors = self.actors.iter().filter(|actor| actor.context == context).count();
        let entries: [(&str, u64, u64); 5] = [
            ("symbols", count(symbols), count(pool(context).len())),
            ("symbolBytes", count(self.symbol_bytes()), count(MAX_ALL_MACRO_BYTES)),
            ("drcs", count(self.drcs.len()), count(DEFINABLE_CODES)),
            ("textures", count(self.textures.len()), count(TEXTURE_MASKS)),
            ("actors", count(actors), count(if context == ContextId::Page { PAGE_ACTORS } else { CHANNEL_SHARE_ACTORS })),
        ];
        for (name, used, limit) in entries {
            let _ = write!(out, ",\"{name}\":{{\"used\":{used},\"limit\":{limit}}}");
        }
    }
}

/// The definitions a context's painter may name: its own symbols, and
/// the page's DRCS characters and textures.
pub(super) struct Defined<'a>(pub &'a Cast, pub ContextId);

impl Definitions for Defined<'_> {
    fn symbol(&self, handle: i32) -> Option<(u8, i32)> {
        let slot = self.0.symbol_slot(handle).filter(|slot| pool(self.1).contains(slot))?;
        Some((code_of(slot), self.0.symbols.get(slot)?.as_ref()?.height))
    }

    fn drcs(&self, handle: i32) -> Option<u8> {
        let (kind, slot) = parts(handle);
        let slot = usize::try_from(slot).ok().filter(|slot| *slot < self.0.drcs.len())?;
        (kind == KIND_DRCS).then(|| code_of(slot))
    }

    fn texture(&self, handle: i32) -> Option<(u8, i32, i32)> {
        let (kind, slot) = parts(handle);
        let texture = self.0.textures.get(usize::try_from(slot).ok()?)?;
        (kind == KIND_TEXTURE).then(|| (u8::try_from(slot).unwrap_or(0), texture.width, texture.height))
    }
}

/// `value` when it is from `low` to `high`, else a `RangeError` naming `bound`.
fn within(value: i32, low: i32, high: i32, bound: &str, what: &str) -> Result<i32, Reply> {
    if (low..=high).contains(&value) { Ok(value) } else { Err(Reply::Range(format!("{bound}: {what} is {low} to {high}, not {value}"))) }
}

/// What `NP_ACTOR` asks for, checked.
struct Asked {
    region: bool,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    origin: (i32, i32),
    layer: u8,
    order: i8,
    flags: i32,
    cover: u8,
    looks: [Option<usize>; LOOKS],
    frames: Vec<usize>,
}

impl Host {
    /// The operations of definitions and the cast (section 10.2, 305 to 311).
    pub(super) fn naplps_cast_operation(&mut self, context: ContextId, op: Op, ints: &Ints<'_>, payload: Payload<'_>) -> Result<Reply, Reply> {
        match op {
            Op::NP_SYMBOL => {
                let [existing, width, height] = ints.exactly()?;
                self.naplps_symbol(context, existing, width, height, &program_of(payload)?)
            }
            Op::NP_DRCS => {
                let [] = ints.exactly()?;
                self.naplps_pattern(context, None, &program_of(payload)?)
            }
            Op::NP_TEXTURE => {
                let [width, height] = ints.exactly()?;
                let width = within(width, 1, MAX_MASK_DOTS, "MASK_SIDE", "a texture's width")?;
                let height = within(height, 1, MAX_MASK_DOTS, "MASK_SIDE", "a texture's height")?;
                self.naplps_pattern(context, Some((width, height)), &program_of(payload)?)
            }
            Op::NP_ACTOR => {
                let numbers: [i32; 15] = ints.exactly()?;
                let frames = match payload {
                    Payload::None => Vec::new(),
                    other => program_of(other)?,
                };
                self.naplps_actor(context, numbers, &frames)
            }
            Op::NP_LABEL => {
                let [thing] = ints.exactly()?;
                let label = text_of(op, payload)?;
                if label.len() > MAX_LABEL_BYTES {
                    return Err(range("MAX_LABEL_BYTES", "a label is too long"));
                }
                if super::has_control(label) {
                    return Err(type_error("a label holds no control character"));
                }
                let place = self.naplps.cast.own_actor(context, thing)?;
                self.room(label.len().saturating_add(size_of::<u16>()), size_of::<u16>())?;
                if let Some(actor) = self.naplps.cast.actors.get_mut(place) {
                    label.clone_into(&mut actor.label);
                    self.naplps.cast.changed.push(actor.number);
                }
                Ok(Reply::Undefined)
            }
            Op::NP_ANIMATE => {
                let [thing, operation, a, b, c, d] = ints.exactly()?;
                super::no_payload(op, payload)?;
                self.naplps_animate(context, thing, operation, [a, b, c, d])
            }
            _ => {
                let [thing] = ints.exactly()?;
                super::no_payload(op, payload)?;
                self.naplps_remove(context, thing)
            }
        }
    }

    /// Room for `bytes` more of NAPLPS that a definition appends, with the
    /// generation's opening if this begins it, and for `held` more bytes kept.
    fn naplps_definition_room(&mut self, bytes: usize, held: usize) -> Result<(), Reply> {
        let opening = if self.naplps.started { 0 } else { self.naplps_opening_bytes() };
        let (_, second) = self.naplps.rooms();
        if opening.saturating_add(bytes) > second {
            return Err(range("NAPLPS_MAX_BYTES_PER_SECOND", "no more NAPLPS may be appended now"));
        }
        self.room(opening.saturating_add(bytes).saturating_add(held), opening.saturating_add(bytes))
    }

    /// `symbol` and `symbol.redefine` (section 7.4): DEF MACRO.
    fn naplps_symbol(&mut self, context: ContextId, existing: i32, width: i32, height: i32, program: &[i32]) -> Result<Reply, Reply> {
        let width = within(width, 1, CANVAS_WIDTH, "CANVAS_WIDTH", "a symbol's width")?;
        let height = within(height, 1, CANVAS_HEIGHT, "CANVAS_HEIGHT", "a symbol's height")?;
        let cast = &self.naplps.cast;
        let slot = if existing == 0 {
            let (bound, what) = if context == ContextId::Page {
                ("DEFINABLE_CODES", "the page has as many symbols as it may")
            } else {
                ("CHANNEL_SHARE_SYMBOLS", "the channel context has as many symbols as it may")
            };
            pool(context).find(|slot| cast.symbols.get(*slot).is_some_and(Option::is_none)).ok_or_else(|| range(bound, what))?
        } else {
            let slot = cast.own_symbol(context, existing)?;
            // An actor's frames are symbols of one height, which is where its origin is counted from.
            if cast.in_use(slot) && cast.symbols.get(slot).and_then(Option::as_ref).is_some_and(|symbol| symbol.height != height) {
                return Err(type_error("a symbol an actor draws with keeps its height"));
            }
            slot
        };
        let mut body = Vec::new();
        let mut painter = Painter::symbol(height);
        let language = self.naplps.language.clone();
        painter.run(program, &language, &Defined(cast, context), &mut body, MAX_MACRO_BYTES, "MAX_MACRO_BYTES")?;
        painter.close(&mut body);
        if body.len() > MAX_MACRO_BYTES {
            return Err(range("MAX_MACRO_BYTES", "the symbol's contents are too long"));
        }
        let before = cast.symbols.get(slot).and_then(Option::as_ref).map_or(0, |symbol| symbol.body.len());
        if cast.symbol_bytes().saturating_sub(before).saturating_add(body.len()) > MAX_ALL_MACRO_BYTES {
            return Err(range("MAX_ALL_MACRO_BYTES", "the symbols together are too long"));
        }
        self.naplps_definition_room(body.len().saturating_add(DEFINITION_BYTES), body.len().saturating_add(size_of::<Symbol>()))?;
        // The generation's opening first, if this begins it: it sends
        // the symbols there are, and this one follows.
        let mut live = self.naplps.begin();
        let mut out = Vec::new();
        live.end(&mut out);
        definition(DEF_MACRO, code_of(slot), &body, &mut out);
        // A macro's contents are recorded, not executed: the receiver's
        // state is what it was (TODO-7367 item 3).
        self.naplps.append_owed(live, &out);
        let scope = self.context(context).and_then(|held| held.scope_stack.last().copied()).unwrap_or(0);
        let cast = &mut self.naplps.cast;
        let kept = cast.symbols.get(slot).and_then(Option::as_ref).map(|symbol| symbol.scope);
        if let Some(held) = cast.symbols.get_mut(slot) {
            *held = Some(Symbol { width, height, body, scope: kept.unwrap_or(scope) });
        }
        Ok(Reply::Int(cast.symbol_handle(slot)))
    }

    /// `drcs` (no size: DEF DRCS, a bitmap the shape of one teletext
    /// cell) and `texture` (a size: DEF TEXTURE), section 7.4.
    fn naplps_pattern(&mut self, context: ContextId, texture: Option<(i32, i32)>, program: &[i32]) -> Result<Reply, Reply> {
        if context != ContextId::Page {
            return Err(type_error("DRCS characters and textures are the page context's"));
        }
        let cast = &self.naplps.cast;
        if texture.is_some() && cast.textures.len() >= TEXTURE_MASKS {
            return Err(range("TEXTURE_MASKS", "the four programmable patterns are defined"));
        }
        if texture.is_none() && cast.drcs.len() >= DEFINABLE_CODES {
            return Err(range("DEFINABLE_CODES", "every DRCS character is defined"));
        }
        let (width, height) = texture.unwrap_or((CELL_DOTS, CELL_LINES));
        let mut body = Vec::new();
        let mut painter = Painter::pattern(width, height);
        let language = self.naplps.language.clone();
        painter.run(program, &language, &Defined(cast, context), &mut body, MAX_MACRO_BYTES, "MAX_MACRO_BYTES")?;
        painter.close(&mut body);
        if body.len() > MAX_MACRO_BYTES {
            return Err(range("MAX_MACRO_BYTES", "the definition's contents are too long"));
        }
        if cast.pattern_bytes().saturating_add(body.len()) > NAPLPS_MAX_PATTERN_BYTES {
            return Err(range("NAPLPS_MAX_PATTERN_BYTES", "the DRCS characters and textures together are too long"));
        }
        let bytes = body.len().saturating_add(DEFINITION_BYTES).saturating_add(BEFORE_PATTERN_BYTES);
        self.naplps_definition_room(bytes, body.len().saturating_add(size_of::<Texture>()))?;
        let mut live = self.naplps.begin();
        let mut out = Vec::new();
        let reply = if texture.is_some() {
            let slot = self.naplps.cast.textures.len();
            live.before_texture(width, height, &mut out);
            definition(DEF_TEXTURE, FIRST_TEXTURE.saturating_add(u8::try_from(slot).unwrap_or(0)), &body, &mut out);
            self.naplps.cast.textures.push(Texture { width, height, body });
            handle(KIND_TEXTURE, i32::try_from(slot).unwrap_or(0))
        } else {
            let slot = self.naplps.cast.drcs.len();
            live.before_drcs(&mut out);
            definition(DEF_DRCS, code_of(slot), &body, &mut out);
            self.naplps.cast.drcs.push(body);
            handle(KIND_DRCS, i32::try_from(slot).unwrap_or(0))
        };
        // The receiver drew the contents as it read them, in its own
        // state: the backdrop's painter takes nothing it sent before as
        // still so, and writes each thing again when next it is needed
        // (TODO-7367 item 3).
        live.forget();
        self.naplps.append_owed(live, &out);
        Ok(Reply::Int(reply))
    }

    /// Checks what `NP_ACTOR` asks for.
    fn naplps_asked(&self, context: ContextId, numbers: [i32; 15], frames: &[i32]) -> Result<Asked, Reply> {
        let [_, kind, x, y, width, height, origin_x, origin_y, layer, order, flags, cover, hover, pressed, alternative] = numbers;
        let cast = &self.naplps.cast;
        let region = match kind {
            KIND_MACRO_OBJECT => false,
            KIND_REGION_OBJECT => true,
            _ => return Err(type_error("an object is an actor (1) or a region (2)")),
        };
        // The record holds a place as sixteen bits, in raster lines.
        let x = i16::try_from(x).map_err(|_| range("MAX_X", "an object's place is sixteen bits across"))?;
        let y = i16::try_from(y).ok().filter(|y| y.checked_add(i16::try_from(CANVAS_TOP_LINE).unwrap_or(0)).is_some());
        let y = y.ok_or_else(|| range("MAX_Y", "an object's place is sixteen bits down"))?;
        if frames.len() > MAX_OBJECT_FRAMES {
            return Err(range("MAX_OBJECT_FRAMES", "an actor has at most 16 frames"));
        }
        let frames = frames.iter().map(|frame| cast.own_symbol(context, *frame)).collect::<Result<Vec<usize>, Reply>>()?;
        if region != frames.is_empty() {
            return Err(type_error("an actor has 1 to 16 frames and a region none"));
        }
        let look = |look: i32| if look == 0 { Ok(None) } else { cast.own_symbol(context, look).map(Some) };
        let looks = [look(hover)?, look(pressed)?, look(alternative)?];
        let size = |slot: &usize| cast.symbols.get(*slot).and_then(Option::as_ref).map_or((0, 0), |symbol| (symbol.width, symbol.height));
        let first = frames.first().map_or((0, 0), size);
        // The rectangle is the first frame's size unless it is given.
        let width = within(if width == 0 { first.0 } else { width }, 1, CANVAS_WIDTH, "CANVAS_WIDTH", "an object's width")?;
        let height = within(if height == 0 { first.1 } else { height }, 1, CANVAS_HEIGHT, "CANVAS_HEIGHT", "an object's height")?;
        if !region && frames.iter().chain(looks.iter().flatten()).any(|slot| size(slot).1 != first.1) {
            return Err(type_error("an actor's frames and looks are symbols of one height"));
        }
        let origin_x = within(origin_x, -CANVAS_WIDTH, CANVAS_WIDTH, "CANVAS_WIDTH", "an actor's origin across")?;
        let origin_y = within(origin_y, -CANVAS_HEIGHT, CANVAS_HEIGHT, "CANVAS_HEIGHT", "an actor's origin down")?;
        let layer = within(layer, FIRST_LAYER, LAST_LAYER, "LAYERS", "an object's layer")?;
        if context == ContextId::Channel && layer != LAST_LAYER {
            return Err(type_error("the channel context's objects lie in layer 3, the overlay"));
        }
        let order = i8::try_from(order).map_err(|_| range("MAX_ORDER", "an object's order is -128 to 127"))?;
        if flags & !ALL_FLAGS != 0 {
            return Err(type_error("no such flag of an object"));
        }
        let cover = within(cover, 0, NAPLPS_COLOURS.saturating_sub(1), "NAPLPS_COLOURS", "a region's cover")?;
        Ok(Asked {
            region,
            x,
            y,
            width: u16::try_from(width).unwrap_or(1),
            height: u16::try_from(height).unwrap_or(1),
            origin: (origin_x, origin_y),
            layer: u8::try_from(layer).unwrap_or(1),
            order,
            flags,
            cover: u8::try_from(cover).unwrap_or(0),
            looks,
            frames,
        })
    }

    /// `actor`, `region` and `redefine` (section 7.5): an object of the
    /// `NAPLPS_OBJECTS` record, sent alone by the record's `ONE` flag.
    fn naplps_actor(&mut self, context: ContextId, numbers: [i32; 15], frames: &[i32]) -> Result<Reply, Reply> {
        let asked = self.naplps_asked(context, numbers, frames)?;
        let existing = numbers.first().copied().unwrap_or(0);
        let cast = &self.naplps.cast;
        let place = if existing == 0 { None } else { Some(cast.own_actor(context, existing)?) };
        if place.is_none() {
            let (most, bound) = if context == ContextId::Page { (PAGE_ACTORS, "MAX_OBJECTS") } else { (CHANNEL_SHARE_ACTORS, "CHANNEL_SHARE_ACTORS") };
            if cast.actors.iter().filter(|actor| actor.context == context).count() >= most {
                return Err(range(bound, "the context has as many actors and regions as it may"));
            }
        }
        let wheel = asked.flags & FLAG_WHEEL != 0;
        let wheels = cast.actors.iter().enumerate().filter(|(index, actor)| actor.wheel && Some(*index) != place).count();
        if wheel && wheels >= MAX_WHEEL_THINGS {
            return Err(range("MAX_WHEEL_THINGS", "as many things take the wheel as may"));
        }
        self.naplps_definition_room(
            0,
            size_of::<Actor>().saturating_add(asked.frames.len().saturating_mul(size_of::<usize>())).saturating_add(MAX_LABEL_BYTES),
        )?;
        // The objects are a generation's: the page's first object begins it.
        let live = self.naplps.begin();
        self.naplps.append_owed(live, &[]);
        let scope = self.context(context).and_then(|held| held.scope_stack.last().copied()).unwrap_or(0);
        let cast = &mut self.naplps.cast;
        let (number, scope, label) = if let Some(actor) = place.and_then(|place| cast.actors.get(place)) {
            (actor.number, actor.scope, actor.label.clone())
        } else {
            // A number no object has; numbers go round after 65,535.
            let mut number = cast.next_number.max(1);
            while cast.actors.iter().any(|actor| actor.number == number) {
                number = number.checked_add(1).unwrap_or(1);
            }
            cast.next_number = number.checked_add(1).unwrap_or(1);
            (number, scope, String::new())
        };
        let actor = Actor {
            number,
            context,
            scope,
            region: asked.region,
            x: asked.x,
            y: asked.y,
            width: asked.width,
            height: asked.height,
            origin: asked.origin,
            layer: asked.layer,
            order: asked.order,
            shown: asked.flags & FLAG_SHOWN != 0,
            detectable: asked.flags & FLAG_DETECTABLE != 0,
            wheel,
            cover: asked.cover,
            looks: asked.looks,
            frames: asked.frames,
            label,
            // The object's animation state is forgotten with its definition (ANIM-11).
            frame: 0,
            cycle: None,
            look: 0,
        };
        match place.and_then(|place| cast.actors.get_mut(place)) {
            Some(held) => *held = actor,
            None => cast.actors.push(actor),
        }
        cast.changed.push(number);
        cast.wheel_changed = true;
        Ok(Reply::Int(handle(KIND_ACTOR, i32::from(number))))
    }

    /// An `ANIMATE` operation on an actor or a region (section 7.5,
    /// ANIM-6): the codes 5, 6, 7 and 15 to 19.
    fn naplps_animate(&mut self, context: ContextId, thing: i32, operation: i32, [first, second, third, fourth]: [i32; 4]) -> Result<Reply, Reply> {
        let cast = &self.naplps.cast;
        let place = cast.own_actor(context, thing)?;
        let actor = cast.actors.get(place).ok_or_else(|| type_error("no such actor or region"))?;
        let frame = |frame: i32| within(frame, 0, MAX_FRAME, "MAX_FRAME", "a frame").map(|frame| u8::try_from(frame).unwrap_or(0));
        let frames_only = |kind: OperationKind| if actor.region { Err(type_error("a region has no frames")) } else { Ok(kind) };
        let kind = match u8::try_from(operation).unwrap_or(0) {
            code::SHOW => OperationKind::Show,
            code::HIDE => OperationKind::Hide,
            code::LOOK => OperationKind::Look(u8::try_from(within(first, 0, LAST_LOOK, "LOOKS", "a look")?).unwrap_or(0)),
            code::MOVE => {
                let relative = match first {
                    0 => false,
                    1 => true,
                    _ => return Err(type_error("a move is to a place (0) or by an amount (1)")),
                };
                let x = i16::try_from(second).map_err(|_| range("MAX_X", "a move is sixteen bits across"))?;
                let y = i16::try_from(third).map_err(|_| range("MAX_Y", "a move is sixteen bits down"))?;
                OperationKind::Move { relative, x, y }
            }
            code::ORDER => {
                let layer = if first == i32::from(KEEP_LAYER) { first } else { within(first, FIRST_LAYER, LAST_LAYER, "LAYERS", "an object's layer")? };
                if context == ContextId::Channel && layer != i32::from(KEEP_LAYER) && layer != LAST_LAYER {
                    return Err(type_error("the channel context's objects lie in layer 3, the overlay"));
                }
                let order = i8::try_from(second).map_err(|_| range("MAX_ORDER", "an object's order is -128 to 127"))?;
                OperationKind::Order { layer: u8::try_from(layer).unwrap_or(KEEP_LAYER), order }
            }
            code::FRAME => frames_only(OperationKind::Frame(frame(first)?))?,
            code::CYCLE => {
                let (from, last) = (frame(first)?, frame(second)?);
                if from > last {
                    return Err(type_error("a cycle's first frame is not after its last"));
                }
                let period = u16::try_from(within(third, 1, MAX_CYCLE_PERIOD, "MAX_CYCLE_PERIOD", "a cycle's period in ticks")?).unwrap_or(1);
                let once = match fourth {
                    0 => false,
                    1 => true,
                    _ => return Err(type_error("a cycle loops (0) or runs once (1)")),
                };
                // ANIM-14: a seventeenth cycling object would do nothing.
                if actor.cycle.is_none() && cast.actors.iter().filter(|actor| actor.cycle.is_some()).count() >= MAX_CYCLING_OBJECTS {
                    return Err(range("MAX_CYCLING_OBJECTS", "as many objects cycle as may"));
                }
                frames_only(OperationKind::Cycle { first: from, last, period, once })?
            }
            code::STOP_CYCLE => frames_only(OperationKind::StopCycle)?,
            _ => return Err(type_error("no such animation operation")),
        };
        if cast.operations.len() >= MAX_ANIMATE_OPERATIONS {
            return Err(range("MAX_ANIMATE_OPERATIONS", "the step has as many animation operations as it may"));
        }
        self.room(size_of::<Operation>(), size_of::<Operation>())?;
        let cast = &mut self.naplps.cast;
        let Some(actor) = cast.actors.get_mut(place) else { return Ok(Reply::Undefined) };
        match kind {
            OperationKind::Show => actor.shown = true,
            OperationKind::Hide => actor.shown = false,
            OperationKind::Look(look) => actor.look = look,
            OperationKind::Move { relative: false, x, y } => (actor.x, actor.y) = (x, y),
            // A relative move stops at the ends of sixteen bits, as the receiver's does (ANIM-6).
            OperationKind::Move { relative: true, x, y } => (actor.x, actor.y) = (actor.x.saturating_add(x), actor.y.saturating_add(y)),
            OperationKind::Order { layer, order } => {
                if layer != KEEP_LAYER {
                    actor.layer = layer;
                }
                actor.order = order;
            }
            OperationKind::Frame(frame) => (actor.frame, actor.cycle) = (frame, None),
            OperationKind::Cycle { first, last, period, once } => (actor.frame, actor.cycle) = (first, Some(Cycle { first, last, period, once })),
            OperationKind::StopCycle => actor.cycle = None,
            OperationKind::Reset => {}
        }
        // A move to a place is written in the record's lines.
        let kind = match kind {
            OperationKind::Move { relative: false, x, y } => {
                OperationKind::Move { relative: false, x, y: y.saturating_add(i16::try_from(CANVAS_TOP_LINE).unwrap_or(0)) }
            }
            other => other,
        };
        cast.operations.push(Operation { target: AnimateTarget::object(actor.number), kind });
        Ok(Reply::Undefined)
    }

    /// `remove` of an actor, a region or a symbol.
    fn naplps_remove(&mut self, context: ContextId, thing: i32) -> Result<Reply, Reply> {
        let (kind, _) = parts(thing);
        if kind == KIND_SYMBOL {
            let slot = self.naplps.cast.own_symbol(context, thing)?;
            if self.naplps.cast.in_use(slot) {
                return Err(type_error("an actor draws with the symbol"));
            }
            // A macro defined with no contents is deleted (section 6.2.2).
            self.naplps_definition_room(DEFINITION_BYTES, 0)?;
            let mut live = self.naplps.begin();
            let mut out = Vec::new();
            live.end(&mut out);
            definition(DEF_MACRO, code_of(slot), &[], &mut out);
            self.naplps.append_owed(live, &out);
            self.naplps.cast.free_symbol(slot);
            return Ok(Reply::Undefined);
        }
        let place = self.naplps.cast.own_actor(context, thing)?;
        self.naplps.cast.actors.remove(place);
        self.naplps.cast.whole = true;
        self.naplps.cast.wheel_changed = true;
        if let Some(held) = self.context_mut(context) {
            held.listens.retain(|(listened, _)| *listened != thing);
        }
        Ok(Reply::Undefined)
    }

    /// A scope of `context` ends: the actors, regions and symbols made in
    /// it go (section 3.3). The handles of the things that went.
    pub(crate) fn naplps_scope_end(&mut self, context: ContextId, scope: i32) -> Vec<i32> {
        let cast = &mut self.naplps.cast;
        let gone: Vec<i32> = cast
            .actors
            .iter()
            .filter(|actor| actor.context == context && actor.scope == scope)
            .map(|actor| handle(KIND_ACTOR, i32::from(actor.number)))
            .collect();
        if !gone.is_empty() {
            cast.actors.retain(|actor| !(actor.context == context && actor.scope == scope));
            cast.whole = true;
            cast.wheel_changed = true;
        }
        for slot in pool(context) {
            if cast.symbols.get(slot).and_then(Option::as_ref).is_some_and(|symbol| symbol.scope == scope) && !cast.in_use(slot) {
                cast.free_symbol(slot);
            }
        }
        gone
    }
}
