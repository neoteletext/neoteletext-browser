//! The ReGIS API's host half (script-api.md sections 6.1, 6.2 and 10.2,
//! operations 200 to 214; work package R1): ReGIS v2's numbered drawings
//! themselves (`neoteletext-renderer-regis/docs/regis-v2.md`), with their
//! fourteen attributes, their looks, frame lists and paint rates, the
//! colour map, the receiver's animation state, the painted report, tracked
//! bounds and picking.
//!
//! The host allocates every drawing number. A property change stages the
//! format's own attribute as `D(S n(…))` text, a definition as
//! `D(B n) … D(E)`, a removal as `D(K n)`. At the end of a step that
//! staged anything the host appends one `D(C)` and feeds the step's bytes
//! to the renderer's own [`Validator`]: if the commit would fail at the
//! receiver it fails here, nothing is sent, the tables are put back as
//! they stood before the step, and the contexts are told
//! ([`EventKind::RG_COMMIT_FAILED`]). One stream lasts for the life of a
//! page; the host keeps no copy of what it sent, only the validator's
//! scene, each drawing's attributes and the length of its contents.
//!
//! Every byte held is counted (section 10.1 rule 7; TODO-7272): a call
//! that would pass [`MAX_HOST_STATE_BYTES`](crate::limits::MAX_HOST_STATE_BYTES)
//! or [`MAX_BATCH_BYTES`](crate::limits::MAX_BATCH_BYTES) throws and
//! changes nothing.

use std::io::Write as _;

use neoteletext_rules::animate::{Operation, OperationKind, code};
use neoteletext_rules::table::{Target, TargetKind};
use protocol::script::batch::Watch;
use protocol::script::{Batch, Bounds, ContextId, EventKind as Carried, Renderer, Step};
use regis::{CommitError, Event as RegisEvent, Validator};

use super::{Host, Ints, LEVEL_WARN, Payload, Reply, cost, has_control, no_payload, range, renderer_name, text_of, type_error};
use crate::limits::regis::{
    CHANNEL_SHARE_CONTENTS_BYTES, CHANNEL_SHARE_DRAWINGS, DEFAULT_LAYER, FIRST_DRAWING_NUMBER, LAST_DRAWING_NUMBER, LAST_PAGE_DRAWING_NUMBER, LAYERS,
    MAX_ANIMATE_OPERATIONS, MAX_ANIMATED_DRAWINGS, MAX_COLOUR_NAME_BYTES, MAX_COLOUR_SETS_PER_SECOND, MAX_COORDINATE, MAX_CYCLE_PERIOD, MAX_CYCLING_OBJECTS,
    MAX_DRAWING_DEPTH, MAX_FRAME, MAX_HUE, MAX_LOOK, MAX_OBJECT_FRAMES, MAX_PAINT_RATE, MAX_PAINTER_WORDS, MAX_PERCENT, MAX_RGB24, MAX_TRACKED_DRAWINGS,
    MAX_WATCHED_PAINTINGS, MAX_WHEEL_THINGS, MIN_COORDINATE, OVERLAY_LAYER, PAINT_RATE_UNSET, REGIS_COLOURS, REGIS_MAX_DRAWINGS,
    REGIS_MAX_TOTAL_CONTENTS_BYTES,
};
use crate::limits::{CONTEXTS, MAX_LABEL_BYTES, TICKS_PER_SECOND};
use crate::link::Context as _;
use crate::ops::{EventKind, Op, Property, RegisField};

mod colour;
pub(super) mod painter;
#[cfg(test)]
mod tests;

/// The drawing numbers one generation of handles tells apart: a handle is
/// a generation above this many numbers, so that the number is its low
/// sixteen bits and the handle a positive whole number.
const HANDLE_NUMBERS: i32 = 65_536;
/// The generations of handles: fifteen bits, so that a handle is never
/// negative. A page's drawings have the generation of their page; a
/// handle of a page that was left is stale whatever number it names.
const GENERATIONS: u16 = 0x8000;
/// The text that commits a step's operations (`regis-v2.md`, "D operations").
const COMMIT: &[u8] = b"D(C)";
/// ReGIS's resynchronisation character (the manual's chapter 1), and a
/// commit.
const RESYNCHRONISE_AND_COMMIT: &[u8] = b";D(C)";
/// The most bytes that wrap one attribute change: `D(S65535(` and `))`.
const SET_WRAPPER_BYTES: usize = 11;
/// The most bytes that wrap a definition: `D(B65535)` and `D(E)`.
const DEFINE_WRAPPER_BYTES: usize = 13;
/// The most bytes one colour's setting takes: `S(M15(H360L100S100))`.
const COLOUR_SET_BYTES: usize = 20;
/// What the host counts as held for each byte staged, beside the byte
/// itself in the step's output: the validator's copy of the staged
/// operations, the staged scene's copy of a definition, and the
/// definition while it is being received. Counted until the commit, after
/// which the validator's own figure says what it holds.
const HELD_FOR_STAGED_BYTE: usize = 3;
/// What the host counts as held for a drawing made in this step, until
/// the commit: its record in the validator's committed scene and in its
/// staged scene, each with a label and a frame list at their longest.
const HELD_FOR_NEW_DRAWING: usize =
    2 * (regis::limits::DRAWING_RECORD_BYTES + regis::limits::DRAWING_EXTRAS_BYTES + MAX_LABEL_BYTES + MAX_OBJECT_FRAMES * size_of::<u16>());
/// What the host's own table holds for a drawing: its record now and its
/// record as the step began (to put back if the commit fails), each with
/// a label at its longest (two bytes of UTF-8 for each of ISO 8859-1's
/// characters) and a frame list at its longest.
const TABLE_BYTES_A_DRAWING: usize = 2 * (size_of::<Drawing>() + 2 * MAX_LABEL_BYTES + MAX_OBJECT_FRAMES * size_of::<u16>());
/// The flags of operation `RG_DRAWING`: shown, detectable, takes the
/// wheel, tracked (this work package's packing; section 10.2 says only
/// "flags").
const FLAG_VISIBLE: i32 = 1;
const FLAG_DETECTABLE: i32 = 2;
const FLAG_WHEEL: i32 = 4;
const FLAG_TRACK: i32 = 8;
const FLAGS: i32 = FLAG_VISIBLE | FLAG_DETECTABLE | FLAG_WHEEL | FLAG_TRACK;

/// The bounds the ReGIS part of the prelude needs, by name.
pub(crate) fn prelude_bounds() -> Vec<(&'static str, i64)> {
    vec![
        ("MAX_PAINTER_WORDS", i64::try_from(MAX_PAINTER_WORDS).unwrap_or(i64::MAX)),
        ("DEFAULT_LAYER", i64::from(DEFAULT_LAYER)),
        ("OVERLAY_LAYER", i64::from(OVERLAY_LAYER)),
        ("PAINT_RATE_UNSET", i64::from(PAINT_RATE_UNSET)),
        ("HANDLE_NUMBERS", i64::from(HANDLE_NUMBERS)),
        ("FLAG_VISIBLE", i64::from(FLAG_VISIBLE)),
        ("FLAG_DETECTABLE", i64::from(FLAG_DETECTABLE)),
        ("FLAG_WHEEL", i64::from(FLAG_WHEEL)),
        ("FLAG_TRACK", i64::from(FLAG_TRACK)),
        ("ANIMATE_LOOK", i64::from(code::LOOK)),
        ("ANIMATE_FRAME", i64::from(code::FRAME)),
        ("ANIMATE_CYCLE", i64::from(code::CYCLE)),
        ("ANIMATE_STOP_CYCLE", i64::from(code::STOP_CYCLE)),
    ]
}

/// The name the API gives a commit error (`regis-v2.md`, "Failed
/// commits"), and the bound a `RangeError` for it begins with.
pub(crate) const fn commit_error_name(error: u8) -> (&'static str, &'static str) {
    match error {
        1 => ("contents", "MAX_DRAWING_CONTENT_BYTES"),
        2 => ("staged", "MAX_STAGED_BYTES"),
        3 => ("depth", "MAX_DRAWING_DEPTH"),
        4 => ("cycle", "REGIS_COMMIT_CYCLE"),
        5 => ("parent", "REGIS_COMMIT_PARENT"),
        6 => ("work", "MAX_REBUILD_WORK"),
        8 => ("drawings", "MAX_DRAWING_RECORD_BYTES"),
        _ => ("malformed", "REGIS_COMMIT_MALFORMED"),
    }
}

/// One numbered drawing as the host holds it: its owner, its fourteen
/// attributes as staged, the length of its contents, and what the
/// receiver's animation state was last asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "each is one attribute or one fact of section 6.2's table, read where it is named")]
struct Drawing {
    number: u16,
    owner: ContextId,
    parent: u16,
    source: u16,
    hover: u16,
    pressed: u16,
    alternative: u16,
    x: i16,
    y: i16,
    depth: i16,
    layer: u8,
    /// None while the stream has given it no `T` (DEC-7031 item 2).
    paint_rate: Option<u16>,
    visible: bool,
    detectable: bool,
    wheel: bool,
    track: bool,
    /// Whether the receiver holds animation state for it.
    animated: bool,
    cycling: bool,
    frame: u8,
    look: u8,
    clip: Option<[i16; 4]>,
    /// The bytes of its contents.
    contents: usize,
    label: Option<Box<str>>,
    frames: Option<Box<[u16]>>,
}

/// One entry of the colour map as the host holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Colour {
    /// The name `colours.define` gave it.
    name: Option<String>,
    /// The value set and not yet written, which the rate holds back.
    pending: Option<(i32, i32, i32)>,
    /// The ticks at which it was written in the last second.
    written: Vec<u64>,
}

/// The tables as a step began, to put back if its commit fails.
struct Snapshot {
    drawings: Vec<Drawing>,
    next: [u16; CONTEXTS],
    contents: [usize; CONTEXTS],
    colours: [Colour; REGIS_COLOURS],
    /// The drawings each context listened to, and for what.
    listens: Vec<(ContextId, i32, EventKind)>,
}

/// A failed commit, for one context to be told of.
struct Failure {
    code: u8,
    drawing: u16,
    /// The context's drawings made in the step, which no longer exist.
    gone: Vec<u16>,
    /// Its drawings removed in the step, which exist again.
    back: Vec<u16>,
}

/// The ReGIS state the host keeps for one page.
pub(super) struct Regis {
    /// The generation of the page's handles.
    generation: u16,
    /// The drawings, in order of number.
    drawings: Vec<Drawing>,
    /// Where each context's next number is looked for.
    next: [u16; CONTEXTS],
    /// The bytes of contents each context's drawings hold.
    contents: [usize; CONTEXTS],
    /// The step's `D` operations so far.
    out: Vec<u8>,
    /// The drawing whose `D(S n(…))` ends `out`, to which a further
    /// attribute is added.
    open_set: Option<u16>,
    /// What is counted as held for the step's operations until its commit.
    pending: usize,
    /// The step's animation operations.
    animate: Vec<Operation>,
    colours: [Colour; REGIS_COLOURS],
    /// The tracked drawings' rectangles as of the last step.
    bounds: Vec<Bounds>,
    snapshot: Option<Box<Snapshot>>,
    validator: Box<Validator>,
    /// What the validator held after the last commit.
    validator_held: usize,
    /// How many steps' commits were made or refused: what tells the
    /// removals of one step from another's.
    epoch: u32,
    /// The step's bytes, checked, for the batch.
    ready: Option<Vec<u8>>,
    /// Whether the step's commit was already made.
    committed: bool,
    failures: [Option<Failure>; CONTEXTS],
    /// Whether the tracked or wheel lists may differ from those told.
    lists_changed: bool,
    tracked_told: Vec<i32>,
    wheel_told: Vec<i32>,
}

/// Writes text to a vector, which cannot fail.
fn put(out: &mut Vec<u8>, text: core::fmt::Arguments<'_>) {
    let _ = out.write_fmt(text);
}

/// The context's place in the per-context tables.
fn slot(context: ContextId) -> usize {
    context.index()
}

impl Regis {
    pub(super) fn new() -> Self {
        let validator = Box::new(Validator::new());
        let validator_held = validator.held_bytes();
        Self {
            generation: 0,
            drawings: Vec::new(),
            next: Self::first_numbers(),
            contents: [0; CONTEXTS],
            out: Vec::new(),
            open_set: None,
            pending: 0,
            animate: Vec::new(),
            colours: Default::default(),
            bounds: Vec::new(),
            snapshot: None,
            validator,
            validator_held,
            epoch: 0,
            ready: None,
            committed: false,
            failures: [None, None],
            lists_changed: false,
            tracked_told: Vec::new(),
            wheel_told: Vec::new(),
        }
    }

    /// Where each context's numbers begin.
    fn first_numbers() -> [u16; CONTEXTS] {
        let mut first = [FIRST_DRAWING_NUMBER; CONTEXTS];
        if let Some(channel) = first.get_mut(slot(ContextId::Channel)) {
            *channel = LAST_PAGE_DRAWING_NUMBER.saturating_add(1);
        }
        first
    }

    /// The first and last number of `context`'s drawings.
    const fn numbers_of(context: ContextId) -> (u16, u16) {
        match context {
            ContextId::Page => (FIRST_DRAWING_NUMBER, LAST_PAGE_DRAWING_NUMBER),
            ContextId::Channel => (LAST_PAGE_DRAWING_NUMBER.saturating_add(1), LAST_DRAWING_NUMBER),
        }
    }

    /// Whether nothing is held: no drawing, nothing staged, no colour named.
    fn is_empty(&self) -> bool {
        self.drawings.is_empty() && self.out.is_empty() && self.animate.is_empty() && self.colours.iter().all(|colour| *colour == Colour::default())
    }

    fn get(&self, number: u16) -> Option<&Drawing> {
        self.drawings.binary_search_by_key(&number, |drawing| drawing.number).ok().and_then(|index| self.drawings.get(index))
    }

    fn get_mut(&mut self, number: u16) -> Option<&mut Drawing> {
        self.drawings.binary_search_by_key(&number, |drawing| drawing.number).ok().and_then(|index| self.drawings.get_mut(index))
    }

    /// The handle a script holds for drawing `number`.
    fn handle(&self, number: u16) -> i32 {
        i32::from(self.generation).saturating_mul(HANDLE_NUMBERS).saturating_add(i32::from(number))
    }

    /// The drawing of `context` that `handle` names: a checked index of
    /// the context's own table (section 10.1 rule 4).
    fn resolve(&self, context: ContextId, handle: i32) -> Result<u16, Reply> {
        let number = handle.checked_rem(HANDLE_NUMBERS).and_then(|number| u16::try_from(number).ok());
        let generation = handle.checked_div(HANDLE_NUMBERS).and_then(|generation| u16::try_from(generation).ok());
        let (Some(number), Some(generation)) = (number, generation) else { return Err(type_error("not a drawing")) };
        if handle <= 0 || number == 0 {
            return Err(type_error("not a drawing"));
        }
        match self.get(number) {
            Some(drawing) if generation == self.generation && drawing.owner == context => Ok(number),
            Some(_) if generation == self.generation => Err(type_error("the drawing belongs to the other context")),
            _ => Err(type_error("the drawing was removed: its handle is stale")),
        }
    }

    /// As [`Self::resolve`], with 0 for no drawing.
    fn resolve_or_none(&self, context: ContextId, handle: i32) -> Result<u16, Reply> {
        if handle == 0 { Ok(0) } else { self.resolve(context, handle) }
    }

    /// How many drawings `context` holds.
    fn count(&self, context: ContextId) -> usize {
        self.drawings.iter().filter(|drawing| drawing.owner == context).count()
    }

    /// The most drawings and bytes of contents `context` may hold.
    const fn share(context: ContextId) -> (usize, usize) {
        match context {
            ContextId::Page => (REGIS_MAX_DRAWINGS - CHANNEL_SHARE_DRAWINGS, REGIS_MAX_TOTAL_CONTENTS_BYTES - CHANNEL_SHARE_CONTENTS_BYTES),
            ContextId::Channel => (CHANNEL_SHARE_DRAWINGS, CHANNEL_SHARE_CONTENTS_BYTES),
        }
    }

    /// How many drawings are in the chain from `number` up to its
    /// top-level ancestor, `number` among them; none when the chain is
    /// longer than a chain may be.
    fn chain(&self, number: u16) -> Option<usize> {
        let mut length = 0_usize;
        let mut at = number;
        while at != 0 {
            length = length.saturating_add(1);
            if length > MAX_DRAWING_DEPTH {
                return None;
            }
            at = self.get(at).map_or(0, |drawing| drawing.parent);
        }
        Some(length)
    }

    /// Whether `ancestor` is `number` or above it.
    fn is_at_or_above(&self, ancestor: u16, number: u16) -> bool {
        let mut at = number;
        // A chain is never longer than the table.
        for _ in 0..=self.drawings.len() {
            if at == ancestor {
                return true;
            }
            if at == 0 {
                return false;
            }
            at = self.get(at).map_or(0, |drawing| drawing.parent);
        }
        false
    }

    /// A number for a new drawing of `context`: the next from where the
    /// last was found that no drawing has and none had as the step began,
    /// so that a number freed in a step names nothing else until the
    /// step's commit has been made or refused.
    fn allocate(&mut self, context: ContextId) -> Option<u16> {
        let (first, last) = Self::numbers_of(context);
        let mut number = self.next.get(slot(context)).copied().unwrap_or(first).clamp(first, last);
        let taken = |regis: &Self, number: u16| {
            regis.get(number).is_some()
                || regis.snapshot.as_ref().is_some_and(|snapshot| snapshot.drawings.binary_search_by_key(&number, |drawing| drawing.number).is_ok())
        };
        // The range holds more numbers than can be taken (asserted in the
        // limits), so a free one is found within this many looks.
        for _ in 0..=REGIS_MAX_DRAWINGS.saturating_mul(2) {
            let after = if number >= last { first } else { number.saturating_add(1) };
            if !taken(self, number) {
                if let Some(next) = self.next.get_mut(slot(context)) {
                    *next = after;
                }
                return Some(number);
            }
            number = after;
        }
        None
    }

    /// Adds attribute text to the step's operations for drawing `number`:
    /// to the `D(S n(…))` that ends them when that is this drawing's, or
    /// in a new one.
    fn stage_set(&mut self, number: u16, attributes: &[u8]) {
        if self.open_set == Some(number) && self.out.ends_with(b"))") {
            self.out.truncate(self.out.len().saturating_sub(2));
            self.out.push(b',');
        } else {
            put(&mut self.out, format_args!("D(S{number}("));
        }
        self.out.extend_from_slice(attributes);
        self.out.extend_from_slice(b"))");
        self.open_set = Some(number);
    }

    /// Adds an operation that is not an attribute change.
    fn stage(&mut self, operation: &[u8]) {
        self.out.extend_from_slice(operation);
        self.open_set = None;
    }

    /// The bytes of the step's output so far, with what its end will
    /// add: the commit, once anything is staged, and each colour that
    /// waits to be written.
    pub(super) fn batch_bytes(&self) -> usize {
        let commit = if self.out.is_empty() { 0 } else { COMMIT.len() };
        let colours = self.colours.iter().filter(|colour| colour.pending.is_some()).count().saturating_mul(COLOUR_SET_BYTES);
        self.out.len().saturating_add(commit).saturating_add(colours).saturating_add(self.animate.len().saturating_mul(size_of::<Operation>()))
    }

    /// The bytes held beside the step's output: the table, twice; the
    /// validator; what the step's operations will add to it; the colour
    /// names; the bounds reported; a failure not yet told; and the bytes
    /// checked and waiting for the batch.
    pub(super) fn held_bytes(&self) -> usize {
        let mut bytes = self.drawings.len().saturating_mul(TABLE_BYTES_A_DRAWING);
        bytes = bytes.saturating_add(self.snapshot.as_ref().map_or(0, |snapshot| {
            // Drawings removed in the step are still held as they began it.
            snapshot.drawings.len().saturating_sub(self.drawings.len()).saturating_mul(TABLE_BYTES_A_DRAWING).saturating_add(
                snapshot
                    .listens
                    .len()
                    .saturating_mul(size_of::<(ContextId, i32, EventKind)>())
                    .saturating_add(REGIS_COLOURS.saturating_mul(size_of::<Colour>())),
            )
        }));
        bytes = bytes.saturating_add(self.validator_held).saturating_add(self.pending);
        bytes = self.colours.iter().fold(bytes, |sum, colour| {
            sum.saturating_add(colour.name.as_deref().map_or(0, cost)).saturating_add(colour.written.len().saturating_mul(size_of::<u64>()))
        });
        bytes = bytes.saturating_add(self.bounds.len().saturating_mul(size_of::<Bounds>()));
        bytes = self
            .failures
            .iter()
            .flatten()
            .fold(bytes, |sum, failure| sum.saturating_add(failure.gone.len().saturating_add(failure.back.len()).saturating_mul(size_of::<u16>())));
        bytes = bytes.saturating_add(self.tracked_told.len().saturating_add(self.wheel_told.len()).saturating_mul(size_of::<i32>()));
        bytes.saturating_add(self.ready.as_ref().map_or(0, Vec::len))
    }

    /// What the validator holds now, as it counts it: for the tests of
    /// the accounting.
    #[cfg(test)]
    fn validator_holds(&self) -> usize {
        self.validator.held_bytes()
    }

    /// The numbers of `number` and every drawing below it, in order.
    fn with_descendants(&self, number: u16) -> Vec<u16> {
        let mut found = vec![number];
        // Each pass finds the children of those found; a chain is at most
        // as long as the table, and usually a few drawings.
        loop {
            let before = found.len();
            for drawing in &self.drawings {
                if drawing.parent != 0 && found.binary_search(&drawing.parent).is_ok() && found.binary_search(&drawing.number).is_err() {
                    let at = found.binary_search(&drawing.number).unwrap_or_else(|at| at);
                    found.insert(at, drawing.number);
                }
            }
            if found.len() == before {
                return found;
            }
        }
    }

    /// Forgets the drawings `numbers` (in order): their records, their
    /// contents' bytes and their reported bounds.
    fn forget(&mut self, numbers: &[u16]) {
        for drawing in self.drawings.iter().filter(|drawing| numbers.binary_search(&drawing.number).is_ok()) {
            if let Some(held) = self.contents.get_mut(slot(drawing.owner)) {
                *held = held.saturating_sub(drawing.contents);
            }
        }
        self.drawings.retain(|drawing| numbers.binary_search(&drawing.number).is_err());
        self.bounds.retain(|bounds| u16::try_from(bounds.handle).map_or(true, |number| numbers.binary_search(&number).is_err()));
        // An animation operation of this step on a drawing that is gone
        // has nothing to act on.
        self.animate.retain(|operation| numbers.binary_search(&operation.target.number).is_err());
        self.lists_changed = true;
    }

    /// The tracked drawings and those that take the wheel, as the decoder
    /// is told them: by number.
    fn lists(&self) -> (Vec<i32>, Vec<i32>) {
        let numbers = |wanted: fn(&Drawing) -> bool| self.drawings.iter().filter(|drawing| wanted(drawing)).map(|drawing| i32::from(drawing.number)).collect();
        (numbers(|drawing| drawing.track), numbers(|drawing| drawing.wheel))
    }
}

/// `value` as a signed 16-bit attribute value, or the `RangeError`.
fn attribute(value: i32, what: &str) -> Result<i16, Reply> {
    i16::try_from(value).map_err(|_| Reply::Range(format!("MAX_COORDINATE: {what} is {MIN_COORDINATE} to {MAX_COORDINATE}, not {value}")))
}

/// `value` as a flag, 0 or 1.
fn flag(value: i32, what: &str) -> Result<bool, Reply> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Reply::Type(format!("{what} is 0 or 1"))),
    }
}

/// `value` as a layer.
fn layer(value: i32) -> Result<u8, Reply> {
    u8::try_from(value).ok().filter(|layer| i32::from(*layer) < LAYERS).ok_or_else(|| range("LAYERS", "a layer is 0 to 3"))
}

/// `value` as a paint rate: none for unset.
fn paint_rate(value: i32) -> Result<Option<u16>, Reply> {
    if value == PAINT_RATE_UNSET {
        return Ok(None);
    }
    u16::try_from(value)
        .ok()
        .filter(|rate| i32::from(*rate) <= MAX_PAINT_RATE)
        .map(Some)
        .ok_or_else(|| range("MAX_PAINT_RATE", "a paint rate is 0 to 32,767, or unset"))
}

/// A hue, lightness and saturation from an operation's whole numbers:
/// the three themselves, or one colour of 24 bits, red first, which the
/// host converts to the nearest whole values (section 6.1).
fn colour_of(values: &[i32]) -> Result<(i32, i32, i32), Reply> {
    match *values {
        [rgb] => {
            u32::try_from(rgb).ok().filter(|_| rgb <= MAX_RGB24).map(colour::hls_of_rgb).ok_or_else(|| type_error("a colour as red, green and blue is 24 bits"))
        }
        [hue, lightness, saturation] => {
            if !(0..=MAX_HUE).contains(&hue) || !(0..=MAX_PERCENT).contains(&lightness) || !(0..=MAX_PERCENT).contains(&saturation) {
                return Err(type_error("a colour's hue is 0 to 360 and its lightness and saturation 0 to 100"));
            }
            Ok((hue, lightness, saturation))
        }
        _ => Err(type_error("a colour is a hue, a lightness and a saturation, or one number of red, green and blue")),
    }
}

/// An `Int32Array` payload's numbers (four bytes each, little-endian, as
/// the shim copies them), or none for no payload.
fn ints_of(op: Op, payload: Payload<'_>) -> Result<Vec<i32>, Reply> {
    match payload {
        Payload::None => Ok(Vec::new()),
        Payload::Ints(bytes) => Ok(bytes.as_chunks::<{ size_of::<i32>() }>().0.iter().copied().map(i32::from_le_bytes).collect()),
        _ => Err(Reply::Type(format!("operation {op:?} takes an Int32Array"))),
    }
}

impl Host {
    /// Whether the ReGIS API is `context`'s to call: the page shown is a
    /// ReGIS page (section 3.1: the channel context is given the renderer
    /// API of the page shown).
    fn regis_available(&self, context: ContextId) -> Result<(), Reply> {
        let held = self.context(context).ok_or_else(|| type_error("the context is not open"))?;
        let renderer = match (context, &held.shown) {
            (ContextId::Channel, Some(shown)) => shown.renderer,
            _ => renderer_name(held.open.renderer),
        };
        if renderer == renderer_name(Renderer::Regis) { Ok(()) } else { Err(type_error("the page shown is not a ReGIS page: nt.regis draws only on one")) }
    }

    /// Room for `output` more bytes of ReGIS in the step's output and
    /// `drawings` more drawings, or the `RangeError`; then the step's
    /// first change keeps the tables as they stood.
    fn regis_room(&mut self, output: usize, drawings: usize) -> Result<(), Reply> {
        let until_commit = output.saturating_mul(HELD_FOR_STAGED_BYTE).saturating_add(drawings.saturating_mul(HELD_FOR_NEW_DRAWING));
        let table = drawings.saturating_mul(TABLE_BYTES_A_DRAWING);
        // Before the snapshot, its cost is the cost of what it would hold.
        let snapshot = if self.regis.snapshot.is_none() { self.regis_listens().len().saturating_mul(size_of::<(ContextId, i32, EventKind)>()) } else { 0 };
        // The first operation staged brings the step's commit with it.
        let output = if output > 0 && self.regis.out.is_empty() { output.saturating_add(COMMIT.len()) } else { output };
        self.room(until_commit.saturating_add(output).saturating_add(table).saturating_add(snapshot), output)?;
        self.regis_begin_change();
        self.regis.pending = self.regis.pending.saturating_add(until_commit);
        Ok(())
    }

    /// The drawings the contexts listen to, and for what.
    fn regis_listens(&self) -> Vec<(ContextId, i32, EventKind)> {
        [ContextId::Channel, ContextId::Page]
            .into_iter()
            .filter_map(|context| self.context(context).map(|held| (context, held)))
            .flat_map(|(context, held)| held.listens.iter().filter(|(handle, _)| *handle > 0).map(move |(handle, kind)| (context, *handle, *kind)))
            .collect()
    }

    /// Keeps the tables as they stand, before the step's first change.
    fn regis_begin_change(&mut self) {
        if self.regis.snapshot.is_none() {
            let listens = self.regis_listens();
            let regis = &self.regis;
            let snapshot = Snapshot { drawings: regis.drawings.clone(), next: regis.next, contents: regis.contents, colours: regis.colours.clone(), listens };
            self.regis.snapshot = Some(Box::new(snapshot));
        }
    }

    /// Drops what the contexts listen for on the drawings `numbers` (in order).
    fn regis_unlisten(&mut self, numbers: &[u16]) {
        for held in self.contexts.iter_mut().flatten() {
            let before = held.listens.len();
            held.listens.retain(|(handle, _)| u16::try_from(*handle).map_or(true, |number| numbers.binary_search(&number).is_err()));
            if held.listens.len() != before {
                self.watch_changed = true;
            }
        }
    }

    /// Forgets the page's ReGIS state: the page context ended or the page
    /// shown was left, and with it the page's one stream. Handles of the
    /// page left are stale from here on.
    pub(super) fn regis_reset(&mut self) {
        if self.regis.is_empty() && self.regis.failures.iter().all(Option::is_none) {
            return;
        }
        let numbers: Vec<u16> = self.regis.drawings.iter().map(|drawing| drawing.number).collect();
        self.regis_unlisten(&numbers);
        let generation = self.regis.generation.checked_add(1).and_then(|next| next.checked_rem(GENERATIONS)).unwrap_or(0);
        self.regis = Regis::new();
        self.regis.generation = generation;
    }

    /// Takes in what a step reports of the page's drawings before its
    /// events are given: the page shown was left (the stream ends with
    /// it); a `once` cycle ended; and the tracked drawings' rectangles.
    pub fn regis_reports(&mut self, step: &Step) {
        if step.events.iter().any(|event| event.kind == Carried::PageHide) {
            self.regis_reset();
        }
        self.regis.committed = false;
        for handle in &step.cycle_ended {
            if let Some(drawing) = u16::try_from(*handle).ok().and_then(|number| self.regis.get_mut(number)) {
                drawing.cycling = false;
            }
        }
        // A tracked drawing the report leaves out is hidden or reaches
        // no pixel of the screen (`Terminal::bounds`): it has no rectangle.
        let regis = &mut self.regis;
        regis.bounds.clear();
        for bounds in step.bounds.iter().take(MAX_TRACKED_DRAWINGS) {
            let tracked = u16::try_from(bounds.handle).ok().and_then(|number| regis.get(number)).is_some_and(|drawing| drawing.track);
            if tracked && !regis.bounds.iter().any(|held| held.handle == bounds.handle) {
                regis.bounds.push(*bounds);
            }
        }
    }

    /// Whether `context` may listen, or stop listening, for events of
    /// `kind` on the thing `handle`, a drawing's number (operation
    /// `LISTEN`; section 6.2: adding a `painted` listener watches the
    /// drawing, at most [`MAX_WATCHED_PAINTINGS`]).
    pub(super) fn regis_listen(&self, context: ContextId, handle: i32, kind: EventKind, on: bool) -> Result<(), Reply> {
        let owned = u16::try_from(handle).ok().and_then(|number| self.regis.get(number)).is_some_and(|drawing| drawing.owner == context);
        if !owned {
            return Err(type_error("no such thing"));
        }
        if on && kind == EventKind::PAINTED && !self.listens_to(context, handle, kind) {
            let mut watched: Vec<i32> = self
                .contexts
                .iter()
                .flatten()
                .flat_map(|held| &held.listens)
                .filter(|(_, kind)| *kind == EventKind::PAINTED)
                .map(|(handle, _)| *handle)
                .collect();
            watched.sort_unstable();
            watched.dedup();
            if watched.len() >= MAX_WATCHED_PAINTINGS {
                return Err(range("MAX_WATCHED_PAINTINGS", "as many drawings' painting is watched as may be"));
            }
        }
        Ok(())
    }

    /// The step at which a context must be woken for ReGIS's sake: the
    /// next, while a failed commit waits to be told or a colour waits for
    /// its rate.
    pub(super) fn regis_wake(&self, context: ContextId) -> Option<u64> {
        let waiting = self.regis.failures.get(slot(context)).is_some_and(Option::is_some)
            || (context == ContextId::Page && self.regis.colours.iter().any(|colour| colour.pending.is_some()));
        waiting.then(|| self.tick.saturating_add(1))
    }

    /// The ReGIS entries of the budget report (section 3.7), added to `out`.
    pub(super) fn regis_budget(&self, context: ContextId, out: &mut String) {
        use std::fmt::Write as _;
        if self.regis_available(context).is_err() {
            return;
        }
        let regis = &self.regis;
        let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        let (drawings, contents) = Regis::share(context);
        let watched = self.watched_paintings().len();
        let entries: [(&str, usize, usize); 9] = [
            ("drawings", regis.count(context), drawings),
            ("contentBytes", regis.contents.get(slot(context)).copied().unwrap_or(0), contents),
            ("stagedBytes", regis.out.len(), self.batch_limit),
            ("animated", regis.drawings.iter().filter(|drawing| drawing.animated).count(), MAX_ANIMATED_DRAWINGS),
            ("cycling", regis.drawings.iter().filter(|drawing| drawing.cycling).count(), MAX_CYCLING_OBJECTS),
            ("watched", watched, MAX_WATCHED_PAINTINGS),
            // Macrographs are work package R2's.
            ("macrographBytes", 0, regis::limits::MAX_MACROGRAPH_BYTES),
            ("colours", regis.colours.iter().filter(|colour| colour.name.is_some()).count(), REGIS_COLOURS.saturating_sub(1)),
            // The validator does not say what a commit's rebuild costs
            // (the renderer's to-do); the limit is the renderer's.
            ("rebuildWork", 0, usize::try_from(regis::limits::MAX_REBUILD_WORK).unwrap_or(usize::MAX)),
        ];
        for (name, used, limit) in entries {
            let _ = write!(out, ",\"{name}\":{{\"used\":{},\"limit\":{}}}", count(used), count(limit));
        }
    }

    /// One ReGIS operation of `context` (section 10.2, 200 to 214).
    #[expect(clippy::too_many_lines, reason = "one arm for each operation of section 10.2's ReGIS rows, in the table's order")]
    pub(super) fn regis_operate(&mut self, context: ContextId, op: Op, ints: &Ints<'_>, payload: Payload<'_>) -> Result<Reply, Reply> {
        self.regis_available(context)?;
        match op {
            Op::RG_COLOUR_DEFINE => {
                let name = text_of(op, payload)?;
                let colour = colour_of(ints.ints)?;
                self.regis_colour_define(context, name, colour)
            }
            Op::RG_COLOUR_SET => {
                no_payload(op, payload)?;
                let (entry, values) = ints.ints.split_first().ok_or_else(|| type_error("a colour entry and a colour"))?;
                let colour = colour_of(values)?;
                let entry =
                    usize::try_from(*entry).ok().filter(|entry| *entry < REGIS_COLOURS).ok_or_else(|| range("REGIS_COLOURS", "a colour entry is 0 to 15"))?;
                self.regis_colour_set(context, entry, colour)?;
                Ok(Reply::Undefined)
            }
            Op::RG_DRAWING => {
                no_payload(op, payload)?;
                self.regis_drawing(context, ints.exactly()?)
            }
            Op::RG_SET => {
                let [handle, property, value] = ints.exactly()?;
                no_payload(op, payload)?;
                let number = self.regis.resolve(context, handle)?;
                let property = Property::from_number(property).ok_or_else(|| type_error("no such property"))?;
                self.regis_set(context, number, property, value)?;
                Ok(Reply::Undefined)
            }
            Op::RG_CLIP => {
                let [handle, on, x, y, width, height] = ints.exactly()?;
                no_payload(op, payload)?;
                let number = self.regis.resolve(context, handle)?;
                let clip = if flag(on, "whether a drawing is clipped")? {
                    let clip = [
                        attribute(x, "a clip rectangle's x")?,
                        attribute(y, "a clip rectangle's y")?,
                        attribute(width, "a clip rectangle's width")?,
                        attribute(height, "a clip rectangle's height")?,
                    ];
                    if width < 0 || height < 0 {
                        return Err(type_error("a clip rectangle's width and height are not negative"));
                    }
                    Some(clip)
                } else {
                    None
                };
                let mut text = Vec::new();
                match clip {
                    Some([x, y, width, height]) => put(&mut text, format_args!("C(X{x},Y{y},W{width},H{height})")),
                    None => text.extend_from_slice(b"C0"),
                }
                self.regis_room(text.len().saturating_add(SET_WRAPPER_BYTES), 0)?;
                if let Some(drawing) = self.regis.get_mut(number) {
                    drawing.clip = clip;
                }
                self.regis.stage_set(number, &text);
                Ok(Reply::Undefined)
            }
            Op::RG_FRAMES => {
                let [handle] = ints.exactly()?;
                let number = self.regis.resolve(context, handle)?;
                let handles = ints_of(op, payload)?;
                if handles.len() > MAX_OBJECT_FRAMES {
                    return Err(range("MAX_OBJECT_FRAMES", "a frame list has at most 16 frames"));
                }
                let frames = handles.iter().map(|frame| self.regis.resolve(context, *frame)).collect::<Result<Vec<u16>, Reply>>()?;
                let mut text = Vec::new();
                if frames.is_empty() {
                    text.extend_from_slice(b"F0");
                } else {
                    text.extend_from_slice(b"F(");
                    for (index, frame) in frames.iter().enumerate() {
                        put(&mut text, format_args!("{}{frame}", if index > 0 { "," } else { "" }));
                    }
                    text.push(b')');
                }
                self.regis_room(text.len().saturating_add(SET_WRAPPER_BYTES), 0)?;
                if let Some(drawing) = self.regis.get_mut(number) {
                    drawing.frames = (!frames.is_empty()).then(|| frames.into_boxed_slice());
                }
                self.regis.stage_set(number, &text);
                Ok(Reply::Undefined)
            }
            Op::RG_LABEL => {
                let [handle] = ints.exactly()?;
                let number = self.regis.resolve(context, handle)?;
                let label = if payload == Payload::None { "" } else { text_of(op, payload)? };
                // Plain text as a v1 quoted string keeps it: ISO 8859-1's
                // printable characters (`regis-v2.md`, "Attributes").
                let mut text = Vec::from(*b"U'");
                let mut length = 0_usize;
                for character in label.chars() {
                    let byte = u8::try_from(u32::from(character)).ok().filter(|byte| matches!(byte, 0x20..=0x7E | 0xA0..=0xFF));
                    let byte = byte.ok_or_else(|| type_error("a label holds only ISO 8859-1's printable characters"))?;
                    length = length.saturating_add(1);
                    if length > MAX_LABEL_BYTES {
                        return Err(range("MAX_LABEL_BYTES", "a label is at most 64 bytes"));
                    }
                    // A quote is doubled inside a quoted string (chapter 7).
                    if byte == b'\'' {
                        text.push(byte);
                    }
                    text.push(byte);
                }
                text.push(b'\'');
                self.regis_room(text.len().saturating_add(SET_WRAPPER_BYTES), 0)?;
                if let Some(drawing) = self.regis.get_mut(number) {
                    drawing.label = (!label.is_empty()).then(|| label.into());
                }
                self.regis.stage_set(number, &text);
                Ok(Reply::Undefined)
            }
            Op::RG_DEFINE => {
                let [handle] = ints.exactly()?;
                let number = self.regis.resolve(context, handle)?;
                let program = ints_of(op, payload)?;
                if program.len() > MAX_PAINTER_WORDS {
                    return Err(range("MAX_PAINTER_WORDS", "a painter program is at most 65,536 whole numbers"));
                }
                let contents = painter::encode(&program)?;
                let before = self.regis.get(number).map_or(0, |drawing| drawing.contents);
                let held = self.regis.contents.get(slot(context)).copied().unwrap_or(0);
                let after = held.saturating_sub(before).saturating_add(contents.len());
                if after > Regis::share(context).1 {
                    return Err(range("REGIS_MAX_TOTAL_CONTENTS_BYTES", "the page's drawings hold as many bytes of contents as they may"));
                }
                self.regis_room(contents.len().saturating_add(DEFINE_WRAPPER_BYTES), 0)?;
                if let Some(drawing) = self.regis.get_mut(number) {
                    drawing.contents = contents.len();
                }
                if let Some(held) = self.regis.contents.get_mut(slot(context)) {
                    *held = after;
                }
                let mut begin = Vec::new();
                put(&mut begin, format_args!("D(B{number})"));
                self.regis.stage(&begin);
                self.regis.stage(&contents);
                self.regis.stage(b"D(E)");
                Ok(Reply::Undefined)
            }
            Op::RG_REMOVE => {
                let [handle] = ints.exactly()?;
                no_payload(op, payload)?;
                let number = self.regis.resolve(context, handle)?;
                let mut text = Vec::new();
                put(&mut text, format_args!("D(K{number})"));
                self.regis_room(text.len(), 0)?;
                let removed = self.regis.with_descendants(number);
                self.regis.stage(&text);
                Ok(self.regis_removed(&removed))
            }
            Op::RG_CLEAR => {
                let [] = ints.exactly()?;
                no_payload(op, payload)?;
                let regis = &self.regis;
                let own: Vec<u16> = regis.drawings.iter().filter(|drawing| drawing.owner == context).map(|drawing| drawing.number).collect();
                // `D(Z)` deletes every numbered drawing: it is written
                // when the other context has none, and otherwise each of
                // this context's top-level drawings is deleted, with its
                // children.
                let mut text = Vec::new();
                if own.len() == regis.drawings.len() {
                    text.extend_from_slice(b"D(Z)");
                } else {
                    for drawing in regis.drawings.iter().filter(|drawing| drawing.owner == context && drawing.parent == 0) {
                        put(&mut text, format_args!("D(K{})", drawing.number));
                    }
                }
                if own.is_empty() {
                    return Ok(self.regis_removed(&own));
                }
                self.regis_room(text.len(), 0)?;
                self.regis.stage(&text);
                Ok(self.regis_removed(&own))
            }
            Op::RG_ANIMATE => {
                let [handle, operation, first, second, third, fourth] = ints.exactly()?;
                no_payload(op, payload)?;
                let number = self.regis.resolve(context, handle)?;
                self.regis_animate(number, operation, [first, second, third, fourth])?;
                Ok(Reply::Undefined)
            }
            Op::RG_MACROGRAPH | Op::RG_MEASURE => Err(type_error("macrographs and measuring are not built yet (work package R2)")),
            Op::RG_BOUNDS => {
                let [handle] = ints.exactly()?;
                no_payload(op, payload)?;
                let number = self.regis.resolve(context, handle)?;
                let bounds = self.regis.bounds.iter().find(|bounds| bounds.handle == i32::from(number));
                Ok(bounds.map_or(Reply::Undefined, |bounds| Reply::Text(format!("{},{},{},{}", bounds.x, bounds.y, bounds.width, bounds.height))))
            }
            Op::RG_GET => {
                let [handle, field] = ints.exactly()?;
                no_payload(op, payload)?;
                let number = self.regis.resolve(context, handle)?;
                self.regis_get(number, field)
            }
            _ => Err(type_error("not a ReGIS operation")),
        }
    }

    /// Forgets the drawings `removed` (in order) and what the contexts
    /// listen for on them, and answers with the step's epoch and their
    /// numbers: text `epoch:n,n,…`.
    fn regis_removed(&mut self, removed: &[u16]) -> Reply {
        self.regis.forget(removed);
        self.regis_unlisten(removed);
        let mut text = format!("{}:", self.regis.epoch);
        for (index, number) in removed.iter().enumerate() {
            use std::fmt::Write as _;
            let _ = write!(text, "{}{number}", if index > 0 { "," } else { "" });
        }
        Reply::Text(text)
    }

    fn regis_colour_define(&mut self, context: ContextId, name: &str, colour: (i32, i32, i32)) -> Result<Reply, Reply> {
        if name.len() > MAX_COLOUR_NAME_BYTES {
            return Err(range("MAX_COLOUR_NAME_BYTES", "a colour's name is at most 32 bytes"));
        }
        if has_control(name) {
            return Err(type_error("a colour's name holds no control character"));
        }
        // A name defined before names the same entry again; a new name
        // takes the next free entry from 1 (entry 0 is the background).
        let named = self.regis.colours.iter().position(|entry| entry.name.as_deref() == Some(name));
        let entry = named
            .or_else(|| self.regis.colours.iter().enumerate().skip(1).find(|(_, entry)| entry.name.is_none()).map(|(index, _)| index))
            .ok_or_else(|| range("REGIS_COLOURS", "every colour entry from 1 to 15 is named"))?;
        if named.is_none() {
            self.room(cost(name), 0)?;
        }
        self.regis_colour_set(context, entry, colour)?;
        if let Some(held) = self.regis.colours.get_mut(entry) {
            held.name = Some(name.to_owned());
        }
        Ok(Reply::Int(i32::try_from(entry).unwrap_or(0)))
    }

    /// Sets a colour entry: the value is written with the step's commit,
    /// or at the next tick the entry's rate allows (section 10.5).
    fn regis_colour_set(&mut self, context: ContextId, entry: usize, colour: (i32, i32, i32)) -> Result<(), Reply> {
        // The colour map is the page's: one for the whole screen
        // (`regis-v2.md`, "Rules for contents", rule 4). The channel
        // context draws only in the overlay layer and changes no colour
        // under the page's drawings.
        if context != ContextId::Page {
            return Err(type_error("only the page context names and sets colours"));
        }
        let waits = self.regis.colours.get(entry).is_some_and(|held| held.pending.is_some());
        self.regis_room(if waits { 0 } else { COLOUR_SET_BYTES }, 0)?;
        if let Some(held) = self.regis.colours.get_mut(entry) {
            held.pending = Some(colour);
        }
        Ok(())
    }

    fn regis_drawing(&mut self, context: ContextId, ints: [i32; 8]) -> Result<Reply, Reply> {
        let [parent, source, in_layer, x, y, depth, rate, flags] = ints;
        let parent = self.regis.resolve_or_none(context, parent)?;
        let source = self.regis.resolve_or_none(context, source)?;
        let in_layer = layer(in_layer)?;
        let (x, y, depth) = (attribute(x, "a drawing's x")?, attribute(y, "a drawing's y")?, attribute(depth, "a drawing's depth")?);
        let rate = paint_rate(rate)?;
        if flags & !FLAGS != 0 {
            return Err(type_error("a drawing's flags are shown, detectable, wheel and track"));
        }
        let (visible, detectable, wheel, track) = (flags & FLAG_VISIBLE != 0, flags & FLAG_DETECTABLE != 0, flags & FLAG_WHEEL != 0, flags & FLAG_TRACK != 0);
        let regis = &self.regis;
        if context == ContextId::Channel && parent == 0 && i32::from(in_layer) != OVERLAY_LAYER {
            return Err(type_error("the channel context draws only in the overlay layer, 3"));
        }
        if regis.count(context) >= Regis::share(context).0 {
            return Err(match context {
                ContextId::Page => range("REGIS_MAX_DRAWINGS", "the page holds as many drawings as it may"),
                ContextId::Channel => range("CHANNEL_SHARE_DRAWINGS", "the channel context holds as many drawings as it may"),
            });
        }
        if parent != 0 && regis.chain(parent).is_none_or(|length| length >= MAX_DRAWING_DEPTH) {
            return Err(range("MAX_DRAWING_DEPTH", "a chain of parents is at most 16 drawings"));
        }
        if wheel && regis.drawings.iter().filter(|drawing| drawing.wheel).count() >= MAX_WHEEL_THINGS {
            return Err(range("MAX_WHEEL_THINGS", "as many drawings take the wheel as may"));
        }
        if track && regis.drawings.iter().filter(|drawing| drawing.track).count() >= MAX_TRACKED_DRAWINGS {
            return Err(range("MAX_TRACKED_DRAWINGS", "as many drawings are tracked as may be"));
        }
        // Only what differs from a new drawing's defaults is written
        // (`regis-v2.md`, "Attributes"); `X0` alone makes the drawing.
        let mut text = Vec::new();
        let add = |text: &mut Vec<u8>, attribute: core::fmt::Arguments<'_>| {
            if !text.is_empty() {
                text.push(b',');
            }
            put(text, attribute);
        };
        if parent != 0 {
            add(&mut text, format_args!("P{parent}"));
        }
        if source != 0 {
            add(&mut text, format_args!("R{source}"));
        }
        if i32::from(in_layer) != DEFAULT_LAYER {
            add(&mut text, format_args!("G{in_layer}"));
        }
        if x != 0 || (text.is_empty() && y == 0 && depth == 0 && rate.is_none() && visible && !detectable) {
            add(&mut text, format_args!("X{x}"));
        }
        if y != 0 {
            add(&mut text, format_args!("Y{y}"));
        }
        if depth != 0 {
            add(&mut text, format_args!("L{depth}"));
        }
        if let Some(rate) = rate {
            add(&mut text, format_args!("T{rate}"));
        }
        if !visible {
            add(&mut text, format_args!("V0"));
        }
        if detectable {
            add(&mut text, format_args!("I1"));
        }
        self.regis_room(text.len().saturating_add(SET_WRAPPER_BYTES), 1)?;
        let Some(number) = self.regis.allocate(context) else { return Err(Reply::Error("no drawing number is free".to_owned())) };
        let drawing = Drawing {
            number,
            owner: context,
            parent,
            source,
            hover: 0,
            pressed: 0,
            alternative: 0,
            x,
            y,
            depth,
            layer: in_layer,
            paint_rate: rate,
            visible,
            detectable,
            wheel,
            track,
            animated: false,
            cycling: false,
            frame: 0,
            look: 0,
            clip: None,
            contents: 0,
            label: None,
            frames: None,
        };
        let regis = &mut self.regis;
        let at = regis.drawings.binary_search_by_key(&number, |drawing| drawing.number).unwrap_or_else(|at| at);
        regis.drawings.insert(at, drawing);
        regis.stage_set(number, &text);
        regis.lists_changed |= wheel || track;
        Ok(Reply::Int(regis.handle(number)))
    }

    #[expect(clippy::too_many_lines, reason = "one arm for each property of section 10.4, in its order")]
    fn regis_set(&mut self, context: ContextId, number: u16, property: Property, value: i32) -> Result<(), Reply> {
        let Some(now) = self.regis.get(number).cloned() else { return Err(type_error("not a drawing")) };
        // The attribute's text, and the change to the record. A value the
        // drawing already has stages nothing.
        let mut text = Vec::new();
        let mut change: Box<dyn FnOnce(&mut Drawing)> = Box::new(|_| {});
        let regis = &self.regis;
        match property {
            Property::X | Property::Y | Property::DEPTH => {
                let (letter, held, what) = match property {
                    Property::X => ('X', now.x, "a drawing's x"),
                    Property::Y => ('Y', now.y, "a drawing's y"),
                    _ => ('L', now.depth, "a drawing's depth"),
                };
                let value = attribute(value, what)?;
                if value != held {
                    put(&mut text, format_args!("{letter}{value}"));
                    change = Box::new(move |drawing| match property {
                        Property::X => drawing.x = value,
                        Property::Y => drawing.y = value,
                        _ => drawing.depth = value,
                    });
                }
            }
            Property::VISIBLE | Property::DETECTABLE => {
                let on = flag(value, "a flag")?;
                let (letter, held) = if property == Property::VISIBLE { ('V', now.visible) } else { ('I', now.detectable) };
                if on != held {
                    put(&mut text, format_args!("{letter}{}", u8::from(on)));
                    change = Box::new(move |drawing| if property == Property::VISIBLE { drawing.visible = on } else { drawing.detectable = on });
                }
            }
            Property::LAYER => {
                let in_layer = layer(value)?;
                if context == ContextId::Channel && now.parent == 0 && i32::from(in_layer) != OVERLAY_LAYER {
                    return Err(type_error("the channel context draws only in the overlay layer, 3"));
                }
                if in_layer != now.layer {
                    put(&mut text, format_args!("G{in_layer}"));
                    change = Box::new(move |drawing| drawing.layer = in_layer);
                }
            }
            Property::PARENT => {
                let parent = regis.resolve_or_none(context, value)?;
                if parent != 0 {
                    // A drawing is never its own ancestor (`regis-v2.md`,
                    // "References"). A chain made too long by what hangs
                    // below this drawing is the commit's to refuse.
                    if regis.is_at_or_above(number, parent) {
                        return Err(type_error("a drawing is not its own parent, nor the parent of a drawing above it"));
                    }
                    if regis.chain(parent).is_none_or(|length| length >= MAX_DRAWING_DEPTH) {
                        return Err(range("MAX_DRAWING_DEPTH", "a chain of parents is at most 16 drawings"));
                    }
                } else if context == ContextId::Channel && i32::from(now.layer) != OVERLAY_LAYER {
                    return Err(type_error("the channel context draws only in the overlay layer, 3"));
                }
                if parent != now.parent {
                    put(&mut text, format_args!("P{parent}"));
                    change = Box::new(move |drawing| drawing.parent = parent);
                }
            }
            Property::SOURCE | Property::HOVER | Property::PRESSED | Property::ALTERNATIVE => {
                let named = regis.resolve_or_none(context, value)?;
                let (letter, held) = match property {
                    Property::SOURCE => ('R', now.source),
                    Property::HOVER => ('H', now.hover),
                    Property::PRESSED => ('A', now.pressed),
                    _ => ('N', now.alternative),
                };
                if named != held {
                    put(&mut text, format_args!("{letter}{named}"));
                    change = Box::new(move |drawing| match property {
                        Property::SOURCE => drawing.source = named,
                        Property::HOVER => drawing.hover = named,
                        Property::PRESSED => drawing.pressed = named,
                        _ => drawing.alternative = named,
                    });
                }
            }
            Property::PAINT_RATE => {
                let rate = paint_rate(value)?;
                // No operation of the stream unsets a rate once given
                // (`regis-v2.md`, "Paint rate"; DEC-7031 item 2).
                if rate.is_none() && now.paint_rate.is_some() {
                    return Err(type_error("a paint rate once given cannot be unset: ReGIS has no operation for it; 0 paints at once"));
                }
                if let Some(given) = rate
                    && rate != now.paint_rate
                {
                    put(&mut text, format_args!("T{given}"));
                    change = Box::new(move |drawing| drawing.paint_rate = rate);
                }
            }
            Property::WHEEL | Property::TRACK => {
                let on = flag(value, "a flag")?;
                let wheel = property == Property::WHEEL;
                let (held, most, bound) =
                    if wheel { (now.wheel, MAX_WHEEL_THINGS, "MAX_WHEEL_THINGS") } else { (now.track, MAX_TRACKED_DRAWINGS, "MAX_TRACKED_DRAWINGS") };
                if on && !held && regis.drawings.iter().filter(|drawing| if wheel { drawing.wheel } else { drawing.track }).count() >= most {
                    return Err(range(bound, "as many drawings as may be"));
                }
                // The host's own, never written to the stream; kept with
                // the step all the same, to be put back if its commit fails.
                self.regis_room(0, 0)?;
                let regis = &mut self.regis;
                if let Some(drawing) = regis.get_mut(number) {
                    if wheel {
                        drawing.wheel = on;
                    } else {
                        drawing.track = on;
                    }
                }
                if !on && !wheel {
                    regis.bounds.retain(|bounds| bounds.handle != i32::from(number));
                }
                regis.lists_changed = true;
                return Ok(());
            }
        }
        if text.is_empty() {
            return Ok(());
        }
        self.regis_room(text.len().saturating_add(SET_WRAPPER_BYTES), 0)?;
        if let Some(drawing) = self.regis.get_mut(number) {
            change(drawing);
        }
        self.regis.stage_set(number, &text);
        Ok(())
    }

    /// An animation operation on the receiver's animation state (section
    /// 6.2; `ANIMATE` codes 7, 17, 18 and 19).
    fn regis_animate(&mut self, number: u16, operation: i32, operands: [i32; 4]) -> Result<(), Reply> {
        let [first, second, third, fourth] = operands;
        let frame_of = |value: i32| {
            u8::try_from(value).ok().filter(|frame| i32::from(*frame) <= MAX_FRAME).ok_or_else(|| range("MAX_OBJECT_FRAMES", "a frame is 0 to 15"))
        };
        let kind = match u8::try_from(operation).unwrap_or(0) {
            code::LOOK => OperationKind::Look(
                u8::try_from(first)
                    .ok()
                    .filter(|look| i32::from(*look) <= MAX_LOOK)
                    .ok_or_else(|| type_error("a look is 0 (none), 1 (hover), 2 (pressed) or 3 (alternative)"))?,
            ),
            code::FRAME => OperationKind::Frame(frame_of(first)?),
            code::CYCLE => {
                let (from, to) = (frame_of(first)?, frame_of(second)?);
                if from > to {
                    return Err(type_error("a cycle's first frame is not after its last"));
                }
                let period = u16::try_from(third)
                    .ok()
                    .filter(|period| (1..=MAX_CYCLE_PERIOD).contains(&i32::from(*period)))
                    .ok_or_else(|| range("MAX_CYCLE_PERIOD", "a cycle's period is 1 to 3,600 ticks"))?;
                OperationKind::Cycle { first: from, last: to, period, once: flag(fourth, "whether a cycle runs once")? }
            }
            code::STOP_CYCLE => OperationKind::StopCycle,
            _ => return Err(type_error("a drawing's animation operations are look, frame, cycle and stop_cycle")),
        };
        let regis = &self.regis;
        let Some(now) = regis.get(number) else { return Err(type_error("not a drawing")) };
        if !now.animated && regis.drawings.iter().filter(|drawing| drawing.animated).count() >= MAX_ANIMATED_DRAWINGS {
            return Err(range("MAX_ANIMATED_DRAWINGS", "as many drawings hold animation state as may"));
        }
        let cycles = matches!(kind, OperationKind::Cycle { .. });
        if cycles && !now.cycling && regis.drawings.iter().filter(|drawing| drawing.cycling).count() >= MAX_CYCLING_OBJECTS {
            return Err(range("MAX_CYCLING_OBJECTS", "as many drawings cycle their frames as may"));
        }
        if regis.animate.len() >= MAX_ANIMATE_OPERATIONS {
            return Err(range("MAX_ANIMATE_OPERATIONS", "the step asked for as many animation operations as it may"));
        }
        let operation = Operation { target: Target { kind: TargetKind::Object, number }, kind };
        if operation.check().is_err() {
            return Err(type_error("not an animation operation a drawing takes"));
        }
        self.room(size_of::<Operation>(), size_of::<Operation>())?;
        self.regis_begin_change();
        let regis = &mut self.regis;
        if let Some(drawing) = regis.get_mut(number) {
            drawing.animated = true;
            match kind {
                OperationKind::Look(look) => drawing.look = look,
                OperationKind::Frame(frame) => {
                    drawing.frame = frame;
                    drawing.cycling = false;
                }
                OperationKind::Cycle { first, .. } => {
                    drawing.frame = first;
                    drawing.cycling = true;
                }
                _ => drawing.cycling = false,
            }
        }
        regis.animate.push(operation);
        Ok(())
    }

    /// A property or field of a drawing as the host holds it.
    fn regis_get(&self, number: u16, field: i32) -> Result<Reply, Reply> {
        let regis = &self.regis;
        let Some(drawing) = regis.get(number) else { return Err(type_error("not a drawing")) };
        let named = |number: u16| Reply::Int(if number == 0 { 0 } else { regis.handle(number) });
        if let Some(property) = Property::from_number(field) {
            return Ok(match property {
                Property::X => Reply::Int(i32::from(drawing.x)),
                Property::Y => Reply::Int(i32::from(drawing.y)),
                Property::VISIBLE => Reply::Int(i32::from(drawing.visible)),
                Property::DEPTH => Reply::Int(i32::from(drawing.depth)),
                Property::LAYER => Reply::Int(i32::from(drawing.layer)),
                Property::PARENT => named(drawing.parent),
                Property::SOURCE => named(drawing.source),
                Property::DETECTABLE => Reply::Int(i32::from(drawing.detectable)),
                Property::PAINT_RATE => Reply::Int(drawing.paint_rate.map_or(PAINT_RATE_UNSET, i32::from)),
                Property::HOVER => named(drawing.hover),
                Property::PRESSED => named(drawing.pressed),
                Property::ALTERNATIVE => named(drawing.alternative),
                Property::WHEEL => Reply::Int(i32::from(drawing.wheel)),
                Property::TRACK => Reply::Int(i32::from(drawing.track)),
            });
        }
        let list = |numbers: &mut dyn Iterator<Item = i32>| numbers.map(|number| number.to_string()).collect::<Vec<String>>().join(",");
        Ok(match RegisField::from_number(field).ok_or_else(|| type_error("no such field"))? {
            RegisField::CLIP => drawing.clip.map_or(Reply::Undefined, |clip| Reply::Text(list(&mut clip.iter().map(|part| i32::from(*part))))),
            RegisField::FRAMES => {
                drawing.frames.as_ref().map_or(Reply::Undefined, |frames| Reply::Text(list(&mut frames.iter().map(|frame| regis.handle(*frame)))))
            }
            RegisField::LABEL => drawing.label.as_ref().map_or(Reply::Undefined, |label| Reply::Text(label.to_string())),
            RegisField::FRAME => Reply::Int(i32::from(drawing.frame)),
            RegisField::LOOK => Reply::Int(i32::from(drawing.look)),
        })
    }

    /// Ends the step for ReGIS: writes the colours whose rate allows it,
    /// appends the one commit, and feeds the step's bytes to the
    /// validator. A commit that would fail is not sent: the tables are
    /// put back and each context that held drawings is told at the next
    /// step. Done once a step; [`Self::regis_take`] does it if the
    /// container has not.
    pub fn regis_commit(&mut self) {
        if core::mem::replace(&mut self.regis.committed, true) {
            return;
        }
        // A container stopped during the step commits nothing of it: the
        // last committed picture stays (section 3.1), and the validator's
        // time is not spent on what a stopped script staged.
        if self.contexts.iter().flatten().all(|held| held.stopped.is_some()) && self.regis.snapshot.is_some() {
            self.regis.out.clear();
            self.regis.open_set = None;
            self.regis.pending = 0;
            if let Some(snapshot) = self.regis.snapshot.take() {
                self.regis_put_back(*snapshot, 0, 0);
            }
            self.regis.failures = [None, None];
            return;
        }
        let tick = self.tick;
        let regis = &mut self.regis;
        let mut bytes = core::mem::take(&mut regis.out);
        regis.open_set = None;
        // Inside the step's batch a colour is held with the commit and
        // discarded with it; with no batch open it takes effect at once
        // (`regis-v2.md`, "Rules for contents", rule 4).
        let batch = !bytes.is_empty();
        for (entry, colour) in regis.colours.iter_mut().enumerate() {
            colour.written.retain(|written| written.saturating_add(TICKS_PER_SECOND) > tick);
            let allowed = colour.written.len() < usize::try_from(MAX_COLOUR_SETS_PER_SECOND).unwrap_or(0);
            if allowed && let Some((hue, lightness, saturation)) = colour.pending.take() {
                put(&mut bytes, format_args!("S(M{entry}(H{hue}L{lightness}S{saturation}))"));
                colour.written.push(tick);
            }
        }
        if batch {
            bytes.extend_from_slice(COMMIT);
        }
        let mut failure = None;
        if !bytes.is_empty() {
            let events = regis.validator.feed_regis(&bytes);
            failure = events.iter().find_map(|event| match event {
                RegisEvent::CommitFailed { error, drawing } => Some((error.code(), *drawing)),
                _ => None,
            });
            // A batch that neither committed nor failed is one the
            // validator did not read to its commit: after a malformed
            // operation the renderer reads nothing until the next
            // semicolon (`regis-v2.md`, "D operations"). The receiver is
            // sent none of it; the validator is given the semicolon and
            // the commit, which fails and drops the batch.
            if batch && failure.is_none() && !events.iter().any(|event| matches!(event, RegisEvent::Committed(_))) {
                let events = regis.validator.feed_regis(RESYNCHRONISE_AND_COMMIT);
                failure = events
                    .iter()
                    .find_map(|event| match event {
                        RegisEvent::CommitFailed { error, drawing } => Some((error.code(), *drawing)),
                        _ => None,
                    })
                    .or(Some((CommitError::Malformed.code(), 0)));
            }
        }
        regis.validator_held = regis.validator.held_bytes();
        regis.pending = 0;
        regis.epoch = regis.epoch.wrapping_add(1);
        let snapshot = regis.snapshot.take();
        match failure {
            None => regis.ready = (!bytes.is_empty()).then_some(bytes),
            Some((error, drawing)) => {
                if let Some(snapshot) = snapshot {
                    self.regis_put_back(*snapshot, error, drawing);
                }
                let (name, _) = commit_error_name(error);
                self.log_line(
                    LEVEL_WARN,
                    None,
                    &format!("the step's ReGIS commit would fail ({name}, code {error}, drawing {drawing}): its changes were discarded and nothing was sent"),
                );
            }
        }
    }

    /// Puts the tables back as `snapshot` kept them and notes, for each
    /// context, which of its drawings are gone and which are back.
    fn regis_put_back(&mut self, snapshot: Snapshot, error: u8, drawing: u16) {
        let regis = &mut self.regis;
        let had = |drawings: &[Drawing], number: u16| drawings.binary_search_by_key(&number, |drawing| drawing.number).is_ok();
        let mut gone_from_all = Vec::new();
        for context in [ContextId::Channel, ContextId::Page] {
            let gone: Vec<u16> = regis
                .drawings
                .iter()
                .filter(|drawing| drawing.owner == context && !had(&snapshot.drawings, drawing.number))
                .map(|drawing| drawing.number)
                .collect();
            let back: Vec<u16> = snapshot
                .drawings
                .iter()
                .filter(|drawing| drawing.owner == context && !had(&regis.drawings, drawing.number))
                .map(|drawing| drawing.number)
                .collect();
            gone_from_all.extend_from_slice(&gone);
            if let Some(failure) = regis.failures.get_mut(slot(context)) {
                *failure = Some(Failure { code: error, drawing, gone, back });
            }
        }
        gone_from_all.sort_unstable();
        regis.drawings = snapshot.drawings;
        // The numbers go on from where they were: one given out in the
        // step is not given out again at once.
        regis.contents = snapshot.contents;
        regis.colours = snapshot.colours;
        regis.animate.clear();
        regis.bounds.retain(|bounds| u16::try_from(bounds.handle).is_ok_and(|number| had(&regis.drawings, number)));
        regis.lists_changed = true;
        let _ = snapshot.next;
        // What was listened for on a drawing that is back is listened for
        // again; on one that is gone, no longer.
        self.regis_unlisten(&gone_from_all);
        for (context, handle, kind) in snapshot.listens {
            let exists = u16::try_from(handle).is_ok_and(|number| self.regis.get(number).is_some());
            if exists
                && let Some(held) = self.context_mut(context)
                && !held.listens.contains(&(handle, kind))
            {
                held.listens.push((handle, kind));
                self.watch_changed = true;
            }
        }
    }

    /// A failed commit `context` has not been told of: the event's whole
    /// numbers (the commit error's code and the drawing's number) and its
    /// payload (how many drawings are gone, their numbers, then the
    /// numbers of those that are back).
    pub fn regis_take_failure(&mut self, context: ContextId) -> Option<([i32; 4], Vec<i32>)> {
        let failure = self.regis.failures.get_mut(slot(context))?.take()?;
        let mut payload = vec![i32::try_from(failure.gone.len()).unwrap_or(0)];
        payload.extend(failure.gone.iter().chain(&failure.back).map(|number| i32::from(*number)));
        Some(([i32::from(failure.code), i32::from(failure.drawing), 0, 0], payload))
    }

    /// Gives the step's ReGIS to its batch: the checked bytes, the
    /// animation operations, and the tracked and wheel lists when they
    /// changed.
    pub(super) fn regis_take(&mut self, batch: &mut Batch) {
        self.regis_commit();
        let regis = &mut self.regis;
        regis.committed = false;
        batch.regis = regis.ready.take();
        if !regis.animate.is_empty() {
            batch.animate = Some(core::mem::take(&mut regis.animate));
        }
        if core::mem::take(&mut regis.lists_changed) {
            let (tracked, wheel) = regis.lists();
            if tracked != regis.tracked_told {
                regis.tracked_told.clone_from(&tracked);
                batch.watch.get_or_insert_with(Watch::default).tracked = Some(tracked);
            }
            if wheel != regis.wheel_told {
                regis.wheel_told.clone_from(&wheel);
                batch.watch.get_or_insert_with(Watch::default).wheel = Some(wheel);
            }
        }
    }
}
