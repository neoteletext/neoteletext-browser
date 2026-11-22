//! The NAPLPS API's host half (script-api.md sections 7.1 to 7.3; work
//! package N1, DEC-7247 item 4; this package's own choices are DEC-7364):
//! NAPLPS's own model, as it is. A stream
//! paints a picture and is forgotten, so a script holds what the format
//! keeps: the **backdrop**, appended to by a typed painter that owns
//! every piece of hidden presentation state ([`writer`]), with WAITs the
//! receiver plays and a rate-held repaint that starts a new generation;
//! and the **colour map**, the first animation channel, whose entries 8
//! to 15 a script defines, redefines and has BLINK. Scripts write no
//! NAPLPS: a painter's calls arrive as a painter program ([`program`])
//! and the bytes are the host's.
//!
//! Operations 300 to 304 and 312 are here. Definitions (symbols, DRCS
//! characters, textures) and the cast, operations 305 to 311 (work
//! package N2), are in the module `cast`, on the same painter and stream.
//!
//! Every bound is in [`limits`]; every byte held is counted against the
//! container's host state (section 10.1 rule 7, TODO-7272) and every
//! byte staged against one step's output.

mod cast;
pub mod limits;
pub mod program;
pub mod writer;

use std::collections::VecDeque;
use std::fmt::Write as _;

use neoteletext_rules::animate;
use protocol::script::batch::NaplpsAppend;
use protocol::script::{Batch, ContextId, ContextOpen, Renderer};

use self::limits::{
    FIRST_DEFINABLE_COLOUR, MAX_BLINK_TENTHS, MAX_BLINKS, MAX_COLOUR_NAME_BYTES, MAX_COLOUR_SETS_PER_SECOND, MAX_PAINTER_WORDS, MAX_TEXT_CHARS, NAPLPS_COLOURS,
    NAPLPS_MAX_BYTES_PER_CALLBACK, NAPLPS_MAX_BYTES_PER_SECOND, NAPLPS_REPAINT_TICKS, WAIT_BUDGET,
};
use self::writer::Painter;
use super::{Host, Ints, Payload, Reply, has_control, no_payload, range, text_of, type_error};
use crate::limits::{MAX_LANGUAGE_BYTES, TICKS_PER_SECOND};
use crate::ops::Op;

/// Why a painter program or a call was refused: the two errors a script
/// is thrown (section 3.8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A bound was passed; the message begins with the bound's name.
    Range(String),
    /// A wrong type, a stale handle, or a call the place does not allow.
    Type(String),
}

impl From<Fault> for Reply {
    fn from(fault: Fault) -> Self {
        match fault {
            Fault::Range(message) => Self::Range(message),
            Fault::Type(message) => Self::Type(message),
        }
    }
}

/// The bits of a handle that say what kind of thing it names; the rest
/// are the thing's place in its table (section 10.1 rule 4).
pub const HANDLE_KIND_SHIFT: u32 = 24;
/// The kinds of NAPLPS things. The backdrop is the one thing of this
/// work package; the others are named here so that the two packages'
/// handles cannot meet.
pub const KIND_BACKDROP: i32 = 1;
/// A symbol (work package N2).
pub const KIND_SYMBOL: i32 = 2;
/// A DRCS character (work package N2).
pub const KIND_DRCS: i32 = 3;
/// A texture (work package N2).
pub const KIND_TEXTURE: i32 = 4;
/// An actor or a region (work package N2).
pub const KIND_ACTOR: i32 = 5;
/// The backdrop's handle: what `painted` is listened for on and reported by.
pub const BACKDROP_HANDLE: i32 = KIND_BACKDROP << HANDLE_KIND_SHIFT;

/// The colours a script defines: entries 8 to 15.
const DEFINABLE: usize = (NAPLPS_COLOURS - FIRST_DEFINABLE_COLOUR) as usize;
/// The names of teletext's eight colours, entries 0 to 7, which a script
/// cannot define again.
pub const TELETEXT_COLOUR_NAMES: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
/// The largest colour: 24 bits of red, green and blue.
const MAX_RGB24: i32 = 0x00FF_FFFF;
/// What `NP_BLINK`'s `to` is for `steady`: no entry.
const NO_ENTRY: i32 = -1;
/// The widths of character spacing `NP_MEASURE` takes, as the numerator
/// over four of the field's width: 1, 5/4 and 3/2 (section 5.3.2.3.1);
/// proportional spacing is measured as 1, its upper bound.
const SPACING_QUARTERS: [i64; 4] = [QUARTERS, 5, 6, QUARTERS];
/// The quarters of a whole.
const QUARTERS: i64 = 4;

/// A defined colour.
#[derive(Clone, Debug)]
struct Colour {
    name: String,
    rgb: [u8; 3],
    /// A later value the rate has not yet let through (section 10.5).
    waiting: Option<[u8; 3]>,
    /// The ticks of its changes in the last second.
    changes: VecDeque<u64>,
}

/// A blink process, as the receiver keeps it (section 5.3.2.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Blink {
    from: u8,
    to: u8,
    /// On, off and delay, in tenths of a second.
    times: [u8; 3],
}

/// What the host keeps for the open page's NAPLPS.
#[derive(Clone, Debug)]
pub struct Naplps {
    /// The page context is open and its page is drawn in NAPLPS.
    active: bool,
    /// The language text is written in where a call names none.
    language: String,
    /// The generation's opening bytes were written.
    started: bool,
    painter: Painter,
    colours: [Option<Colour>; DEFINABLE],
    blinks: Vec<Blink>,
    /// The bytes to append at the step's end.
    staged: Vec<u8>,
    /// Whether they begin a new generation.
    new_generation: bool,
    /// The bytes the callback running has appended.
    callback_bytes: usize,
    /// The bytes appended in the last second: tick and count.
    second: VecDeque<(u64, usize)>,
    /// The tick of the last generation's start.
    generation_tick: Option<u64>,
    /// The tenths of a second of WAIT written in this generation.
    wait_tenths: u32,
    tick: u64,
    /// The definitions and the cast (work package N2).
    cast: cast::Cast,
}

impl Default for Naplps {
    fn default() -> Self {
        Self {
            active: false,
            language: String::new(),
            started: false,
            painter: Painter::backdrop(),
            colours: [const { None }; DEFINABLE],
            blinks: Vec::new(),
            staged: Vec::new(),
            new_generation: false,
            callback_bytes: 0,
            second: VecDeque::new(),
            generation_tick: None,
            wait_tenths: 0,
            tick: 0,
            cast: cast::Cast::default(),
        }
    }
}

/// The index of `entry` among the definable colours.
fn definable(entry: i32) -> Option<usize> {
    usize::try_from(entry.checked_sub(FIRST_DEFINABLE_COLOUR)?).ok().filter(|index| *index < DEFINABLE)
}

/// The entry of the definable colour at `index`.
fn entry_at(index: usize) -> u8 {
    u8::try_from(index).unwrap_or(0).saturating_add(u8::try_from(FIRST_DEFINABLE_COLOUR).unwrap_or(0))
}

/// 24 bits as red, green and blue.
fn rgb_of(rgb24: i32) -> Result<[u8; 3], Reply> {
    if !(0..=MAX_RGB24).contains(&rgb24) {
        return Err(type_error("a colour is 24 bits of red, green and blue"));
    }
    let [_, red, green, blue] = rgb24.to_be_bytes();
    Ok([red, green, blue])
}

impl Naplps {
    /// A context opens: the page context of a NAPLPS page begins with an
    /// empty backdrop and colour map.
    pub(super) fn open(&mut self, open: &ContextOpen) {
        if open.context == ContextId::Page {
            *self = Self { active: open.renderer == Renderer::Naplps, language: open.language.clone(), tick: self.tick, ..Self::default() };
        }
    }

    /// A context closes: the page's NAPLPS goes with the page context.
    pub(super) fn close(&mut self, context: ContextId) {
        if context == ContextId::Page {
            *self = Self { tick: self.tick, ..Self::default() };
        } else {
            // The channel context's actors and symbols go with it.
            self.cast.drop_context(context);
        }
    }

    /// A callback begins: its allowance of bytes starts again.
    pub(super) const fn begin_callback(&mut self) {
        self.callback_bytes = 0;
    }

    /// Whether `handle` names a thing of the page's NAPLPS.
    pub(super) fn is_thing(&self, context: ContextId, handle: i32) -> bool {
        self.active && ((context == ContextId::Page && handle == BACKDROP_HANDLE) || self.cast.is_thing(context, handle))
    }

    /// The bytes held, beside those staged.
    pub(super) fn state_bytes(&self) -> usize {
        let colours = self.colours.iter().flatten().fold(0_usize, |sum, colour| {
            sum.saturating_add(colour.name.len()).saturating_add(colour.changes.len().saturating_mul(size_of::<u64>())).saturating_add(size_of::<Colour>())
        });
        size_of::<Self>()
            .saturating_add(self.language.len())
            .saturating_add(colours)
            .saturating_add(self.blinks.len().saturating_mul(size_of::<Blink>()))
            .saturating_add(self.second.len().saturating_mul(size_of::<(u64, usize)>()))
            .saturating_add(self.cast.state_bytes())
    }

    /// The bytes staged for the step's output.
    pub(super) fn staged_bytes(&self) -> usize {
        self.staged.len().saturating_add(self.cast.staged_bytes())
    }

    /// Moves what is staged into the step's batch.
    /// `title` is the page's, which the objects' record carries.
    fn take_batch(&mut self, batch: &mut Batch, title: &str) {
        if !self.staged.is_empty() {
            batch.naplps = Some(NaplpsAppend { new_generation: core::mem::take(&mut self.new_generation), bytes: core::mem::take(&mut self.staged) });
        }
        if self.active {
            self.cast.take_batch(batch, title);
        }
    }

    /// The bytes that may still be appended in this callback, and in this second.
    fn rooms(&mut self) -> (usize, usize) {
        (NAPLPS_MAX_BYTES_PER_CALLBACK.saturating_sub(self.callback_bytes), NAPLPS_MAX_BYTES_PER_SECOND.saturating_sub(self.second_bytes()))
    }

    /// Stages bytes the host owes the stream for something a script did
    /// that is bounded on its own account (a generation's opening, held
    /// to its rate; a definition, held to its table): they count against
    /// the second's bytes and the step's output, and not against the
    /// callback's allowance, which is the painter programs' own.
    fn stage_owed(&mut self, bytes: &[u8]) {
        let callback = self.callback_bytes;
        self.stage(bytes);
        self.callback_bytes = callback;
    }

    /// As [`Naplps::append`], for bytes the host owes.
    fn append_owed(&mut self, painter: Painter, bytes: &[u8]) {
        self.painter = painter;
        self.stage_owed(bytes);
    }

    /// The bytes appended in the second ending now.
    fn second_bytes(&mut self) -> usize {
        let tick = self.tick;
        while self.second.front().is_some_and(|(at, _)| at.saturating_add(TICKS_PER_SECOND) <= tick) {
            self.second.pop_front();
        }
        self.second.iter().fold(0_usize, |sum, (_, count)| sum.saturating_add(*count))
    }

    /// The bytes a callback may still append, and the bound that holds it.
    fn allowance(&mut self) -> (usize, &'static str) {
        let callback = NAPLPS_MAX_BYTES_PER_CALLBACK.saturating_sub(self.callback_bytes);
        let second = NAPLPS_MAX_BYTES_PER_SECOND.saturating_sub(self.second_bytes());
        if second < callback { (second, "NAPLPS_MAX_BYTES_PER_SECOND") } else { (callback, "NAPLPS_MAX_BYTES_PER_CALLBACK") }
    }

    /// Counts `bytes` appended now and stages them.
    fn stage(&mut self, bytes: &[u8]) {
        self.callback_bytes = self.callback_bytes.saturating_add(bytes.len());
        match self.second.back_mut() {
            Some((at, count)) if *at == self.tick => *count = count.saturating_add(bytes.len()),
            _ => self.second.push_back((self.tick, bytes.len())),
        }
        self.staged.extend_from_slice(bytes);
    }

    /// What opens a generation: the renderer's reset, the domain,
    /// teletext's colours, then the script's colours and blinks as they
    /// stand, so that a script sees nothing of a new generation but its
    /// new backdrop (section 7.2). Work package N2 adds its definitions
    /// and cast after these.
    fn opening(&mut self, out: &mut Vec<u8>) -> Painter {
        let mut painter = Painter::backdrop();
        Painter::start_generation(out);
        for (index, colour) in self.colours.iter_mut().enumerate() {
            let Some(colour) = colour else { continue };
            // A value the rate was holding back is the colour's value now.
            if let Some(waiting) = colour.waiting.take() {
                colour.rgb = waiting;
            }
            painter.set_colour(entry_at(index), colour.rgb, out);
        }
        for blink in &self.blinks {
            painter.blink(blink.from, blink.to, blink.times, out);
        }
        self.cast.resend(&mut painter, out);
        painter.end(out);
        painter
    }

    /// Appends `bytes`, written by `painter` from the live painter's
    /// state, to the backdrop's stream: what work package N2's
    /// definitions call. The bytes are counted against the callback's and
    /// the second's allowances, which the caller has checked with
    /// [`Naplps::room`].
    pub fn append(&mut self, painter: Painter, bytes: &[u8]) {
        self.painter = painter;
        self.stage(bytes);
    }

    /// The live painter and the bytes that must go before anything else
    /// is appended: none once the generation has begun, else its opening,
    /// which is then staged as a new generation.
    pub fn begin(&mut self) -> Painter {
        if !self.started {
            let mut out = Vec::new();
            self.painter = self.opening(&mut out);
            self.started = true;
            self.new_generation = true;
            self.generation_tick = Some(self.tick);
            self.wait_tenths = 0;
            self.staged.clear();
            self.stage_owed(&out);
        }
        self.painter.clone()
    }

    /// The bytes that may still be appended now, and the bound's name.
    pub fn room(&mut self) -> (usize, &'static str) {
        self.allowance()
    }

    /// Whether colour `index`'s rate lets a change through now.
    fn may_change(&mut self, index: usize) -> bool {
        let tick = self.tick;
        let Some(Some(colour)) = self.colours.get_mut(index) else { return false };
        while colour.changes.front().is_some_and(|at| at.saturating_add(TICKS_PER_SECOND) <= tick) {
            colour.changes.pop_front();
        }
        colour.changes.len() < MAX_COLOUR_SETS_PER_SECOND
    }

    /// Counts a change of colour `index` now.
    fn count_change(&mut self, index: usize) {
        let tick = self.tick;
        if let Some(Some(colour)) = self.colours.get_mut(index) {
            colour.changes.push_back(tick);
        }
    }

    /// Writes colour `index`'s value to the stream.
    fn send_colour(&mut self, index: usize) {
        let Some(Some(colour)) = self.colours.get(index) else { return };
        let rgb = colour.rgb;
        let mut painter = self.begin();
        let mut out = Vec::new();
        painter.set_colour(entry_at(index), rgb, &mut out);
        painter.end(&mut out);
        self.append(painter, &out);
    }

    /// A step begins: the colours whose latest value the rate held back
    /// are set as soon as it allows (section 10.5: the latest value is
    /// applied at the next tick the rate allows, and nothing throws).
    pub(super) fn begin_step(&mut self, tick: u64) {
        self.tick = tick;
        for index in 0..DEFINABLE {
            let waiting = self.colours.get(index).and_then(Option::as_ref).and_then(|colour| colour.waiting);
            let Some(rgb) = waiting else { continue };
            if !self.may_change(index) {
                continue;
            }
            if let Some(Some(colour)) = self.colours.get_mut(index) {
                colour.rgb = rgb;
                colour.waiting = None;
            }
            self.count_change(index);
            self.send_colour(index);
        }
    }

    /// The NAPLPS entries of the budget report (section 3.7), appended to
    /// the report's JSON object.
    pub(super) fn budget(&self, context: ContextId, out: &mut String) {
        if !self.active {
            return;
        }
        self.cast.budget(context, out);
        let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        let second =
            self.second.iter().filter(|(at, _)| at.saturating_add(TICKS_PER_SECOND) > self.tick).fold(0_usize, |sum, (_, bytes)| sum.saturating_add(*bytes));
        let entries: [(&str, u64, u64); 4] = [
            ("blinks", count(self.blinks.len()), count(MAX_BLINKS)),
            ("appendedBytesThisSecond", count(second), count(NAPLPS_MAX_BYTES_PER_SECOND)),
            ("waitTicks", u64::from(self.wait_tenths.saturating_mul(naplps::limits::TENTH_TICKS)), u64::from(WAIT_BUDGET)),
            ("colours", count(self.colours.iter().flatten().count()), count(DEFINABLE)),
        ];
        for (name, used, limit) in entries {
            let _ = write!(out, ",\"{name}\":{{\"used\":{used},\"limit\":{limit}}}");
        }
    }
}

/// The numbers the prelude's NAPLPS part needs, written in front of it
/// as `NP_BOUND` so that the two halves cannot disagree.
#[must_use]
pub fn prelude_bounds() -> [(&'static str, i64); 15] {
    [
        ("BACKDROP_HANDLE", i64::from(BACKDROP_HANDLE)),
        ("MAX_PAINTER_WORDS", i64::try_from(MAX_PAINTER_WORDS).unwrap_or(i64::MAX)),
        ("CELL_DOTS", i64::from(limits::CELL_DOTS)),
        ("CELL_LINES", i64::from(limits::CELL_LINES)),
        ("FILL_TEXTURE", i64::from(writer::FIRST_PROGRAMMABLE_FILL)),
        ("STEADY", i64::from(NO_ENTRY)),
        // The `ANIMATE` operations' codes (animation.md ANIM-6), by the rule table's names.
        ("ANIMATE_SHOW", i64::from(animate::code::SHOW)),
        ("ANIMATE_HIDE", i64::from(animate::code::HIDE)),
        ("ANIMATE_LOOK", i64::from(animate::code::LOOK)),
        ("ANIMATE_MOVE", i64::from(animate::code::MOVE)),
        ("ANIMATE_ORDER", i64::from(animate::code::ORDER)),
        ("ANIMATE_FRAME", i64::from(animate::code::FRAME)),
        ("ANIMATE_CYCLE", i64::from(animate::code::CYCLE)),
        ("ANIMATE_STOP_CYCLE", i64::from(animate::code::STOP_CYCLE)),
        ("KEEP_LAYER", i64::from(cast::KEEP_LAYER)),
    ]
}

/// A painter program's words, from a host call's payload.
fn program_of(payload: Payload<'_>) -> Result<Vec<i32>, Reply> {
    let Payload::Ints(bytes) = payload else { return Err(type_error("a painter program is an Int32Array")) };
    if bytes.len() > MAX_PAINTER_WORDS.saturating_mul(size_of::<i32>()) {
        return Err(range("MAX_PAINTER_WORDS", "the painter program is too long"));
    }
    Ok(bytes.as_chunks::<{ size_of::<i32>() }>().0.iter().copied().map(i32::from_le_bytes).collect())
}

impl Host {
    /// The NAPLPS operations (section 10.2, 300 to 304 and 312).
    pub(super) fn naplps_operation(&mut self, context: ContextId, op: Op, ints: &Ints<'_>, payload: Payload<'_>) -> Result<Reply, Reply> {
        if !self.naplps.active {
            return Err(type_error("the page is not drawn in NAPLPS by a script of its own"));
        }
        // Definitions and the cast: the page context's, and the channel
        // context's share of symbols and overlay objects (section 3.1).
        if matches!(op, Op::NP_SYMBOL | Op::NP_DRCS | Op::NP_TEXTURE | Op::NP_ACTOR | Op::NP_LABEL | Op::NP_ANIMATE | Op::NP_REMOVE) {
            return self.naplps_cast_operation(context, op, ints, payload);
        }
        // The backdrop and the colour map are the page's (section 3.1:
        // renderer state belongs to the page).
        if context != ContextId::Page {
            return Err(type_error("the backdrop and the colour map are the page context's"));
        }
        match op {
            Op::NP_COLOUR_DEFINE => {
                let [rgb24] = ints.exactly()?;
                let name = text_of(op, payload)?;
                self.naplps_define(name, rgb_of(rgb24)?)
            }
            Op::NP_COLOUR_SET => {
                let [entry, rgb24] = ints.exactly()?;
                no_payload(op, payload)?;
                let rgb = rgb_of(rgb24)?;
                let index = self.naplps_defined(entry)?;
                self.naplps_set(index, rgb)?;
                Ok(Reply::Undefined)
            }
            Op::NP_BLINK => {
                let [entry, to, on, off, delay] = ints.exactly()?;
                no_payload(op, payload)?;
                self.naplps_blink(entry, to, [on, off, delay])
            }
            Op::NP_PAINT | Op::NP_REPAINT => {
                let [] = ints.exactly()?;
                let program = program_of(payload)?;
                self.naplps_paint(op == Op::NP_REPAINT, &program)
            }
            _ => {
                let [width, height, spacing, language_bytes] = ints.exactly()?;
                let whole = text_of(op, payload)?;
                self.naplps_measure(width, height, spacing, language_bytes, whole)
            }
        }
    }

    /// The index of the defined colour `entry`.
    fn naplps_defined(&self, entry: i32) -> Result<usize, Reply> {
        let index = definable(entry).ok_or_else(|| range("NAPLPS_COLOURS", "a script's colours are entries 8 to 15"))?;
        if self.naplps.colours.get(index).is_some_and(Option::is_some) { Ok(index) } else { Err(type_error("the entry is not a defined colour")) }
    }

    /// Room in the step's output and the host's state for `bytes` more of
    /// NAPLPS, and in the callback's and the second's allowances.
    fn naplps_room(&mut self, bytes: usize) -> Result<(), Reply> {
        let (room, bound) = self.naplps.room();
        if bytes > room {
            return Err(range(bound, "no more NAPLPS may be appended now"));
        }
        self.room(bytes, bytes)
    }

    fn naplps_define(&mut self, name: &str, rgb: [u8; 3]) -> Result<Reply, Reply> {
        if name.is_empty() || name.len() > MAX_COLOUR_NAME_BYTES {
            return Err(range("MAX_COLOUR_NAME_BYTES", "a colour's name is 1 to 32 bytes"));
        }
        if has_control(name) {
            return Err(type_error("a colour's name holds no control character"));
        }
        if TELETEXT_COLOUR_NAMES.contains(&name) {
            return Err(type_error("teletext's eight colours are entries 0 to 7 and are not defined again"));
        }
        // A name defined before keeps its entry and is set, at the rate a
        // set is held to.
        if let Some(index) = self.naplps.colours.iter().position(|colour| colour.as_ref().is_some_and(|colour| colour.name == name)) {
            self.naplps_set(index, rgb)?;
            return Ok(Reply::Int(i32::from(entry_at(index))));
        }
        let index = self.naplps.colours.iter().position(Option::is_none).ok_or_else(|| range("NAPLPS_COLOURS", "every entry from 8 to 15 is defined"))?;
        // The generation's opening, if this begins it, and the colour.
        let opening = if self.naplps.started { 0 } else { self.naplps_opening_bytes() };
        self.naplps_room(opening.saturating_add(COLOUR_BYTES).saturating_add(name.len()).saturating_add(size_of::<Colour>()))?;
        if let Some(slot) = self.naplps.colours.get_mut(index) {
            *slot = Some(Colour { name: name.to_owned(), rgb, waiting: None, changes: VecDeque::new() });
        }
        if self.naplps.started {
            self.naplps.send_colour(index);
        } else {
            // The opening writes every defined colour, this one among them.
            self.naplps.begin();
        }
        Ok(Reply::Int(i32::from(entry_at(index))))
    }

    /// The length of a generation's opening as it would be written now.
    fn naplps_opening_bytes(&self) -> usize {
        let mut out = Vec::new();
        self.naplps.clone().opening(&mut out);
        out.len()
    }

    fn naplps_set(&mut self, index: usize, rgb: [u8; 3]) -> Result<(), Reply> {
        if !self.naplps.may_change(index) {
            // Held back, never thrown (section 10.5): the latest value
            // waits for the next tick the rate allows.
            if let Some(Some(colour)) = self.naplps.colours.get_mut(index) {
                colour.waiting = Some(rgb);
            }
            return Ok(());
        }
        let opening = if self.naplps.started { 0 } else { self.naplps_opening_bytes() };
        self.naplps_room(opening.saturating_add(COLOUR_BYTES))?;
        // Counted before the work and never given back (section 10.1 rule 6).
        self.naplps.count_change(index);
        if let Some(Some(colour)) = self.naplps.colours.get_mut(index) {
            colour.rgb = rgb;
            colour.waiting = None;
        }
        if self.naplps.started {
            self.naplps.send_colour(index);
        } else {
            self.naplps.begin();
        }
        Ok(())
    }

    fn naplps_blink(&mut self, entry: i32, to: i32, times: [i32; 3]) -> Result<Reply, Reply> {
        let index = self.naplps_defined(entry)?;
        let from = entry_at(index);
        let steady = to == NO_ENTRY;
        let to = if steady {
            0
        } else {
            u8::try_from(to).ok().filter(|to| i32::from(*to) < NAPLPS_COLOURS).ok_or_else(|| range("NAPLPS_COLOURS", "a blink's other entry is 0 to 15"))?
        };
        let mut tenths = [0_u8; 3];
        for (slot, value) in tenths.iter_mut().zip(times) {
            *slot = u8::try_from(value)
                .ok()
                .filter(|value| i32::from(*value) <= MAX_BLINK_TENTHS)
                .ok_or_else(|| range("MAX_BLINK_TENTHS", "a blink's on, off and delay are 0 to 63 tenths of a second"))?;
        }
        // The table as the receiver will hold it (section 5.3.2.7): a
        // blink replaces the one between the same two entries, and one
        // with an interval of nothing only ends it.
        let mut blinks = self.naplps.blinks.clone();
        let [on, off, _] = tenths;
        if steady {
            blinks.retain(|blink| blink.from != from);
        } else {
            blinks.retain(|blink| !(blink.from == from && blink.to == to));
            if on != 0 && off != 0 {
                if blinks.len() >= MAX_BLINKS {
                    return Err(range("MAX_BLINKS", "as many blink processes run as may"));
                }
                blinks.push(Blink { from, to, times: tenths });
            }
        }
        if !self.naplps.may_change(index) {
            return Err(range("MAX_COLOUR_SETS_PER_SECOND", "the entry's colour was changed too often in a second"));
        }
        let opening = if self.naplps.started { 0 } else { self.naplps_opening_bytes() };
        self.naplps_room(opening.saturating_add(BLINK_BYTES).saturating_add(size_of::<Blink>()))?;
        self.naplps.count_change(index);
        let mut painter = self.naplps.begin();
        let mut out = Vec::new();
        if steady {
            painter.steady(from, &mut out);
        } else {
            painter.blink(from, to, tenths, &mut out);
        }
        painter.end(&mut out);
        self.naplps.blinks = blinks;
        self.naplps.append(painter, &out);
        Ok(Reply::Undefined)
    }

    /// Moves the step's NAPLPS into its batch.
    pub(super) fn naplps_take(&mut self, batch: &mut Batch) {
        let title = self.context(ContextId::Page).map(|page| page.title.clone()).unwrap_or_default();
        self.naplps.take_batch(batch, &title);
    }

    /// `paint` and `repaint` (section 7.2). A painter program's bytes are
    /// held to the callback's and the second's allowances; a generation's
    /// opening, which the host owes, to the second's.
    fn naplps_paint(&mut self, repaint: bool, program: &[i32]) -> Result<Reply, Reply> {
        let tick = self.naplps.tick;
        let fresh = repaint || !self.naplps.started;
        if repaint {
            // Rate before work (section 10.1 rule 6): a generation is the
            // one expensive thing, and the count is never given back.
            if self.naplps.generation_tick.is_some_and(|last| tick < last.saturating_add(NAPLPS_REPAINT_TICKS)) {
                return Err(range("NAPLPS_REPAINT_TICKS", "a new generation is started at most once in 30 ticks"));
            }
            self.naplps.generation_tick = Some(tick);
        }
        let (callback, second) = self.naplps.rooms();
        let mut out = Vec::new();
        // A program that fails changes nothing: a new generation is
        // opened on a copy, and the painter is a copy.
        let mut next = fresh.then(|| self.naplps.clone());
        let mut painter = match next.as_mut() {
            Some(next) => next.opening(&mut out),
            None => self.naplps.painter.clone(),
        };
        let opening = out.len();
        if opening > second {
            return Err(range("NAPLPS_MAX_BYTES_PER_SECOND", "no more NAPLPS may be appended now"));
        }
        let own = second.saturating_sub(opening);
        let (room, bound) = if own < callback { (own, "NAPLPS_MAX_BYTES_PER_SECOND") } else { (callback, "NAPLPS_MAX_BYTES_PER_CALLBACK") };
        let outcome =
            painter.run(program, &self.naplps.language, &cast::Defined(&self.naplps.cast, ContextId::Page), &mut out, opening.saturating_add(room), bound)?;
        // What a repaint replaces is no longer part of the step's output.
        let replaced = if fresh { self.naplps.staged.len() } else { 0 };
        if self.batch_bytes().saturating_sub(replaced).saturating_add(out.len()) > self.batch_limit {
            return Err(range("MAX_BATCH_BYTES", "one step's output is full"));
        }
        if self.state_bytes().saturating_sub(replaced).saturating_add(out.len()) > self.state_limit {
            return Err(range("MAX_HOST_STATE_BYTES", "the host holds as much for this container as it may"));
        }
        if let Some(mut next) = next {
            next.staged.clear();
            next.started = true;
            next.new_generation = true;
            next.generation_tick = Some(tick);
            next.wait_tenths = 0;
            self.naplps = next;
        }
        self.naplps.wait_tenths = self.naplps.wait_tenths.saturating_add(outcome.wait_tenths);
        let (owed, own) = out.split_at_checked(opening).unwrap_or((&[], &out));
        self.naplps.stage_owed(owed);
        self.naplps.append(painter, own);
        Ok(Reply::Undefined)
    }

    /// `nt.naplps.measure` (section 7.3): the width and height, in dots,
    /// of `text` in a character field of `width` by `height`. The payload
    /// is the language tag, of `language_bytes` bytes, then the text. The
    /// renderer has no measuring call: text in the provided sets is one
    /// field a character, so the width is the cells the engine's encoder
    /// counts, times the field and its spacing.
    fn naplps_measure(&self, width: i32, height: i32, spacing: i32, language_bytes: i32, whole: &str) -> Result<Reply, Reply> {
        let width = limits_within(width, limits::CANVAS_WIDTH, "CANVAS_WIDTH", "a character field's width")?;
        let height = limits_within(height, limits::CANVAS_HEIGHT, "CANVAS_HEIGHT", "a character field's height")?;
        let quarters =
            usize::try_from(spacing).ok().and_then(|spacing| SPACING_QUARTERS.get(spacing)).ok_or_else(|| type_error("no such character spacing"))?;
        let split = usize::try_from(language_bytes).ok().filter(|length| *length <= MAX_LANGUAGE_BYTES);
        let split = split.ok_or_else(|| range("MAX_LANGUAGE_BYTES", "a language tag is too long"))?;
        let (Some(language), Some(text)) = (whole.get(..split), whole.get(split..)) else {
            return Err(type_error("the language's length is not a place between two characters of the payload"));
        };
        if text.chars().take(MAX_TEXT_CHARS.saturating_add(1)).count() > MAX_TEXT_CHARS {
            return Err(range("MAX_TEXT_CHARS", "the text is too long"));
        }
        let language = if language.is_empty() { self.naplps.language.as_str() } else { language };
        let cells = i64::from(writer::cells(&writer::runs(text, language)?));
        let wide = cells.saturating_mul(i64::from(width)).saturating_mul(*quarters).checked_div(QUARTERS).unwrap_or(0);
        Ok(Reply::Text(format!("{wide},{height}")))
    }
}

/// `value` when it is from 1 to `high`.
fn limits_within(value: i32, high: i32, bound: &str, what: &str) -> Result<i32, Reply> {
    if (1..=high).contains(&value) { Ok(value) } else { Err(Reply::Range(format!("{bound}: {what} is 1 to {high}, not {value}"))) }
}

/// The most bytes setting one colour appends: SO, SELECT COLOR and its
/// entry, SET COLOR and its six bytes, and the PDI's end.
const COLOUR_BYTES: usize = 11;
/// The most bytes one blink appends: SO, SELECT COLOR and its entry,
/// BLINK and its four operand bytes, and the PDI's end.
const BLINK_BYTES: usize = 9;

#[cfg(test)]
mod tests;
