//! What the host keeps for one container, and the shared core's
//! operations (script-api.md sections 3.3 to 3.8 and 10.1 to 10.6).
//!
//! Every host call arrives as a context, an operation number, whole
//! numbers and at most one payload ([`Host::call`]). The seven rules of
//! section 10.1 hold for each: lengths are bounded before anything is
//! copied; the work is constant or linear in a bounded length; an
//! operation validates everything and only then changes anything, so one
//! that fails changes nothing; handles and timer and scope numbers are
//! checked indices of the calling context's own tables; rates are counted
//! before the work and never given back; and every byte held here is
//! counted against [`MAX_HOST_STATE_BYTES`] and one step's output against
//! [`MAX_BATCH_BYTES`].

use std::collections::VecDeque;
use std::fmt::Write as _;

use crate::limits::{
    AUDIO_CHANNELS, CONTEXTS, MAX_BATCH_BYTES, MAX_CONSECUTIVE_FAILURES, MAX_GAIN, MAX_HOST_STATE_BYTES, MAX_KEY_NAME_BYTES, MAX_KEYS_DOWN, MAX_LISTEN_KINDS,
    MAX_LISTENS, MAX_LOG_CHARS, MAX_LOG_LEVEL, MAX_LOG_LINES, MAX_MIDI_DATA, MAX_NAVIGATIONS_PER_SECOND, MAX_NOTE_TICKS, MAX_RANDOM_BYTES, MAX_SCOPE_DEPTH,
    MAX_SCOPES, MAX_SCRIPT_BYTES, MAX_SHARED_BYTES, MAX_SHARED_KEY_BYTES, MAX_SHARED_KEYS, MAX_SHARED_VALUE_BYTES, MAX_SOUND_NUMBER,
    MAX_SOUND_OPERATIONS_PER_CALLBACK, MAX_SOUND_OPERATIONS_PER_SECOND, MAX_SOUND_OPERATIONS_PER_STEP, MAX_SYNTH_VARIANT, MAX_TIMER_TICKS, MAX_TIMERS,
    MAX_TITLE_BYTES, MAX_URL_BYTES, QUICKJS_HEAP_BYTES, TICKS_PER_SECOND,
};
use protocol::script::batch::{ContextState, LogLine, MAX_SOUND_OPERATIONS, SoundOperation, Verdict, Watch};
use protocol::script::{Batch, ContextId, ContextOpen, FailReason, MAX_WATCHED_PAINTINGS, Renderer};

use crate::link::{Context as _, context_numbered};
use crate::ops::{DispatchBit, EventKind, FAMILIES, Handle, Info, KeyBit, Op, PointerAction};
use crate::random::Generator;

pub mod naplps;

/// MIDI's channel messages a script may send (MIDI 1.0 detailed
/// specification, Table 1): note off, note on, control change, program
/// change, pitch bend change.
const MIDI_NOTE_OFF: u8 = 0x80;
const MIDI_NOTE_ON: u8 = 0x90;
const MIDI_CONTROL: u8 = 0xB0;
const MIDI_PROGRAM: u8 = 0xC0;
const MIDI_BEND: u8 = 0xE0;
/// The most notes waiting for their release.
const MAX_PENDING_NOTES: usize = 256;
/// The most shared-store changes waiting to be told to the other context:
/// one for each key, each way.
const MAX_SHARED_NOTICES: usize = MAX_SHARED_KEYS * CONTEXTS;
/// What every string held here is counted as costing beside its bytes.
const STRING_OVERHEAD: usize = size_of::<String>();
/// A whole percentage's ceiling.
const PERCENT: u64 = 100;
/// The bits of a tick count operation `NOW` gives at a time, and their mask.
const TICK_LOW_BITS: u32 = 31;
const TICK_LOW_MASK: u64 = 0x7FFF_FFFF;
/// The bits of a pointer event's first whole number that hold the action
/// (see [`PointerAction`]).
const POINTER_ACTION_MASK: i32 = 0x0F;

/// The log level the host's own notices are written at: a warning.
const LEVEL_WARN: u8 = 3;
/// The sub-code of a page that has no sub-pages (EN 300 706 clause
/// A.1: sub-code 0000), for which `nt.page.subpage` is null.
const NO_SUBCODE: u16 = 0;

/// The name the API gives a renderer.
const fn renderer_name(renderer: Renderer) -> &'static str {
    match renderer {
        Renderer::Teletext => "teletext",
        Renderer::Regis => "regis",
        Renderer::Naplps => "naplps",
    }
}

/// The payload of an event as the host gives it to a context: what the
/// step's bytes are for the event's kind.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InputPayload {
    /// None.
    #[default]
    None,
    /// A string.
    Text(String),
    /// Bytes.
    Bytes(Vec<u8>),
    /// Whole numbers.
    Ints(Vec<i32>),
}

/// One event for a context: a kind of section 10.3 with its four whole
/// numbers and its payload, read from a step's event or made by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// Its kind.
    pub kind: EventKind,
    /// Its four whole numbers.
    pub ints: [i32; 4],
    /// Its payload.
    pub payload: InputPayload,
}

/// A host call's payload, as the shim found it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload<'a> {
    /// None.
    None,
    /// A string's bytes, which QuickJS wrote as UTF-8.
    Text(&'a [u8]),
    /// A `Uint8Array`'s bytes.
    Bytes(&'a [u8]),
    /// An `Int32Array`'s bytes, four to a number, little-endian.
    Ints(&'a [u8]),
}

/// A host call's reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// `undefined`.
    Undefined,
    /// A whole number.
    Int(i32),
    /// A string.
    Text(String),
    /// A `Uint8Array`.
    Bytes(Vec<u8>),
    /// A `RangeError`; the message begins with the bound's name.
    Range(String),
    /// A `TypeError`.
    Type(String),
    /// Any other error.
    Error(String),
}

/// A `RangeError` naming `bound`.
fn range(bound: &str, detail: &str) -> Reply {
    Reply::Range(format!("{bound}: {detail}"))
}

/// A `TypeError`.
fn type_error(detail: &str) -> Reply {
    Reply::Type(detail.to_owned())
}

/// One timer of a context.
#[derive(Debug, Clone, Copy)]
struct Timer {
    id: i32,
    /// The ticks between its firings.
    every: u64,
    /// The context tick it is next due at.
    due: u64,
    repeat: bool,
    /// The scope it belongs to, or 0.
    scope: i32,
    /// The order it was made in, which orders timers due at one tick.
    made: u64,
}

/// A note waiting for its release.
#[derive(Debug, Clone, Copy)]
struct PendingNote {
    /// The presentation tick it is released at.
    due: u64,
    part: u8,
    note: u8,
    context: ContextId,
}

/// Counts over the last second of presentation ticks.
#[derive(Debug, Default)]
struct RateWindow {
    counts: VecDeque<(u64, u32)>,
}

impl RateWindow {
    /// The count in the second ending at `tick`.
    fn count(&mut self, tick: u64) -> u32 {
        while self.counts.front().is_some_and(|(at, _)| at.saturating_add(TICKS_PER_SECOND) <= tick) {
            self.counts.pop_front();
        }
        self.counts.iter().fold(0_u32, |sum, (_, count)| sum.saturating_add(*count))
    }

    /// Counts one more at `tick`.
    fn add(&mut self, tick: u64) {
        match self.counts.back_mut() {
            Some((at, count)) if *at == tick => *count = count.saturating_add(1),
            _ => self.counts.push_back((tick, 1)),
        }
    }
}

/// The pointer as of this step, for a context that claimed it.
#[derive(Debug, Clone, Copy, Default)]
struct PointerState {
    x: i32,
    y: i32,
    buttons: i32,
    inside: bool,
    handle: i32,
}

/// The page a channel context was last told is shown.
#[derive(Debug, Clone)]
struct PageShown {
    page: u16,
    subcode: Option<u16>,
    renderer: &'static str,
    url: String,
}

/// What the host keeps for one context.
struct ContextHost {
    open: ContextOpen,
    /// The presentation tick of its first step: its tick 0.
    origin: Option<u64>,
    timers: Vec<Timer>,
    next_timer: i32,
    timers_made: u64,
    /// The scopes not yet ended, and those among them still open.
    scopes: Vec<i32>,
    scope_stack: Vec<i32>,
    next_scope: i32,
    keys_down: Vec<String>,
    keys_pressed: Vec<String>,
    pointer: PointerState,
    /// The things and kinds of event it has listeners for.
    listens: Vec<(i32, EventKind)>,
    generator: Generator,
    failures: u32,
    stopped: Option<FailReason>,
    navigations: RateWindow,
    /// A channel context's page shown; none before the first `pageshow`.
    shown: Option<PageShown>,
    title: String,
    script_bytes: usize,
    /// The voices it has a sound playing on and the parts it started a
    /// synthesizer on, a bit each.
    voices: u8,
    parts: u8,
}

/// The callback running now.
#[derive(Debug, Clone, Copy)]
struct Callback {
    /// Whether it handles a key going down, a pointer release or a
    /// component's activation: the web's user activation (HTML section
    /// 6.4), which a navigation to another channel needs.
    activation: bool,
    sound_operations: u32,
}

/// The time figures of the budget report, kept by the container.
#[derive(Debug, Clone, Copy, Default)]
pub struct TimeFigures {
    /// The share of the callback limit the last callback used, a whole percentage.
    pub callback_percent: u32,
    /// The most any callback used.
    pub callback_percent_max: u32,
    /// The most of the step budget any step used.
    pub step_percent_max: u32,
    /// The share of the second's budget used in the last second.
    pub second_percent: u32,
}

/// A whole percentage of `limit` that `used` is, from 0 to 100 and no
/// finer, so that the budget report is no fine clock (section 3.7).
#[must_use]
pub fn percent(used: core::time::Duration, limit: core::time::Duration) -> u32 {
    let limit = u64::try_from(limit.as_micros()).unwrap_or(u64::MAX).max(1);
    let used = u64::try_from(used.as_micros()).unwrap_or(u64::MAX);
    u32::try_from(used.saturating_mul(PERCENT).checked_div(limit).unwrap_or(PERCENT)).unwrap_or(u32::MAX)
}

/// The host's state for one container.
#[expect(clippy::struct_excessive_bools, reason = "four facts about the tab and the step, each read where it is named")]
pub struct Host {
    contexts: [Option<ContextHost>; CONTEXTS],
    /// The shared store: keys and their values as JSON text (section 3.6).
    shared: Vec<(String, String)>,
    /// Changes of the shared store not yet told: the context to tell, and the key.
    shared_notices: VecDeque<(ContextId, String)>,
    log: Vec<LogLine>,
    log_dropped: u32,
    sound: Vec<SoundOperation>,
    sound_step: u32,
    sound_second: RateWindow,
    pending_notes: Vec<PendingNote>,
    title: Option<String>,
    navigate: Option<String>,
    watch_changed: bool,
    /// The paintings the decoder was last asked to watch.
    painted_told: Vec<i32>,
    /// The presentation tick of the step running, or of the last one.
    tick: u64,
    /// The clock a script is given: milliseconds since 1970 at `clock_tick`.
    clock_ms: u64,
    clock_tick: u64,
    zone_minutes: i16,
    sound_allowed: bool,
    sound_refusal_logged: bool,
    hidden: bool,
    callback: Option<(ContextId, Callback)>,
    /// QuickJS's heap in use, read from the module at each host call.
    pub(crate) heap_bytes: usize,
    /// The time figures of the budget report.
    pub(crate) figures: TimeFigures,
    /// The bounds on the bytes held and on one step's output: the named
    /// constants, lowered only by this module's own tests.
    state_limit: usize,
    batch_limit: usize,
    /// The open page's NAPLPS (work package N1).
    naplps: naplps::Naplps,
}

/// The whole numbers of a host call, read with their count checked.
struct Ints<'a> {
    op: Op,
    ints: &'a [i32],
}

impl Ints<'_> {
    /// Exactly `count` whole numbers, or a `TypeError` saying so.
    fn exactly<const N: usize>(&self) -> Result<[i32; N], Reply> {
        <[i32; N]>::try_from(self.ints).map_err(|_| Reply::Type(format!("operation {:?} takes {N} whole numbers, not {}", self.op, self.ints.len())))
    }
}

/// The payload as a string, or a `TypeError`.
fn text_of(op: Op, payload: Payload<'_>) -> Result<&str, Reply> {
    match payload {
        Payload::Text(bytes) => core::str::from_utf8(bytes).map_err(|_| Reply::Type(format!("operation {op:?}'s string is not well-formed Unicode"))),
        _ => Err(Reply::Type(format!("operation {op:?} takes a string"))),
    }
}

/// No payload, or a `TypeError`.
fn no_payload(op: Op, payload: Payload<'_>) -> Result<(), Reply> {
    if payload == Payload::None { Ok(()) } else { Err(Reply::Type(format!("operation {op:?} takes no payload"))) }
}

/// `value` as a number from `low` to `high`, or a `RangeError` naming `bound`.
fn within(value: i32, low: i32, high: i32, bound: &str, what: &str) -> Result<u8, Reply> {
    if value < low || value > high {
        return Err(Reply::Range(format!("{bound}: {what} is {low} to {high}, not {value}")));
    }
    u8::try_from(value).map_err(|_| Reply::Range(format!("{bound}: {what} is {low} to {high}, not {value}")))
}

/// Writes `text` as a JSON string.
fn json_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            control if u32::from(control) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

/// Whether `text` holds a control character (Unicode's category Cc).
fn has_control(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// What a string held here is counted as costing.
const fn cost(text: &str) -> usize {
    text.len().saturating_add(STRING_OVERHEAD)
}

impl ContextHost {
    fn new(open: ContextOpen, script_bytes: usize) -> Self {
        let generator = Generator::new(&open.seed);
        Self {
            open,
            origin: None,
            timers: Vec::new(),
            next_timer: 1,
            timers_made: 0,
            scopes: Vec::new(),
            scope_stack: Vec::new(),
            next_scope: 1,
            keys_down: Vec::new(),
            keys_pressed: Vec::new(),
            pointer: PointerState::default(),
            listens: Vec::new(),
            generator,
            failures: 0,
            stopped: None,
            navigations: RateWindow::default(),
            shown: None,
            title: String::new(),
            script_bytes,
            voices: 0,
            parts: 0,
        }
    }

    /// Its tick count at the presentation tick `tick`.
    fn now(&self, tick: u64) -> u64 {
        self.origin.map_or(0, |origin| tick.saturating_sub(origin))
    }

    /// The bytes held for it.
    fn state_bytes(&self) -> usize {
        let strings = [&self.open.language, &self.open.channel_url, &self.open.page_url, &self.open.script_url, &self.open.channel_title, &self.title];
        let mut bytes = strings.iter().fold(size_of::<Self>(), |sum, text| sum.saturating_add(cost(text)));
        bytes = self.open.keys.iter().chain(&self.keys_down).chain(&self.keys_pressed).fold(bytes, |sum, key| sum.saturating_add(cost(key)));
        bytes = bytes.saturating_add(self.open.sounds.len());
        bytes = bytes.saturating_add(self.timers.len().saturating_mul(size_of::<Timer>()));
        bytes = bytes.saturating_add(self.scopes.len().saturating_add(self.scope_stack.len()).saturating_mul(size_of::<i32>()));
        bytes = bytes.saturating_add(self.listens.len().saturating_mul(size_of::<(i32, EventKind)>()));
        bytes.saturating_add(self.shown.as_ref().map_or(0, |shown| cost(&shown.url)))
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    /// A container's host, with no context.
    #[must_use]
    pub fn new() -> Self {
        Self {
            contexts: [None, None],
            shared: Vec::new(),
            shared_notices: VecDeque::new(),
            log: Vec::new(),
            log_dropped: 0,
            sound: Vec::new(),
            sound_step: 0,
            sound_second: RateWindow::default(),
            pending_notes: Vec::new(),
            title: None,
            navigate: None,
            watch_changed: false,
            painted_told: Vec::new(),
            tick: 0,
            clock_ms: 0,
            clock_tick: 0,
            zone_minutes: 0,
            sound_allowed: false,
            sound_refusal_logged: false,
            hidden: false,
            callback: None,
            heap_bytes: 0,
            figures: TimeFigures::default(),
            state_limit: MAX_HOST_STATE_BYTES,
            batch_limit: MAX_BATCH_BYTES,
            naplps: naplps::Naplps::default(),
        }
    }

    fn context(&self, context: ContextId) -> Option<&ContextHost> {
        self.contexts.get(context.index()).and_then(Option::as_ref)
    }

    fn context_mut(&mut self, context: ContextId) -> Option<&mut ContextHost> {
        self.contexts.get_mut(context.index()).and_then(Option::as_mut)
    }

    /// Whether `context` is open.
    #[must_use]
    pub fn is_open(&self, context: ContextId) -> bool {
        self.context(context).is_some()
    }

    /// Opens a context: its tables are empty and its clock starts.
    pub fn open(&mut self, open: ContextOpen, script_bytes: usize) {
        // The clock never goes back: a context opened later moves it on to
        // its own start if that is later than the clock reads.
        let start = open.epoch_seconds.saturating_mul(1000);
        if start > self.clock_now_ms() {
            self.clock_ms = start;
            self.clock_tick = self.tick;
        }
        self.zone_minutes = open.zone_minutes;
        self.sound_allowed |= open.flags.sound_allowed;
        let context = open.context;
        self.naplps.open(&open);
        if let Some(slot) = self.contexts.get_mut(context.index()) {
            *slot = Some(ContextHost::new(open, script_bytes));
        }
    }

    /// Ends a context: every timer, scope, listener and key of it is
    /// dropped, and its sound released (section 3.5). The shared store
    /// goes with the channel context (section 3.6).
    pub fn close(&mut self, context: ContextId) {
        self.release_sound(context);
        self.naplps.close(context);
        if let Some(slot) = self.contexts.get_mut(context.index()) {
            if slot.as_ref().is_some_and(|held| !held.listens.is_empty()) {
                self.watch_changed = true;
            }
            *slot = None;
        }
        self.shared_notices.retain(|(to, _)| *to != context);
        if context == ContextId::Channel {
            self.shared.clear();
            self.shared_notices.clear();
        }
    }

    /// Stops what `context` plays: a stop for each voice it has a sound on
    /// and for each part it started, and its waiting notes forgotten.
    /// These are the host's own operations, not the script's, and are not
    /// counted against its rates.
    pub fn release_sound(&mut self, context: ContextId) {
        let Some(held) = self.context_mut(context) else { return };
        let (voices, parts) = (held.voices, held.parts);
        held.voices = 0;
        held.parts = 0;
        self.pending_notes.retain(|note| note.context != context);
        for channel in 0..u8::try_from(AUDIO_CHANNELS).unwrap_or(0) {
            let bit = 1_u8.checked_shl(u32::from(channel)).unwrap_or(0);
            if voices & bit != 0 {
                self.sound.push(SoundOperation::Stop { voice: channel });
            }
            if parts & bit != 0 {
                self.sound.push(SoundOperation::SynthStop { part: channel });
            }
        }
    }

    /// The clock a script is given, in milliseconds since 1970: it moves
    /// only with the presentation tick (departures D13 and D14).
    #[must_use]
    pub fn clock_now_ms(&self) -> u64 {
        let ticks = self.tick.saturating_sub(self.clock_tick);
        self.clock_ms.saturating_add(ticks.saturating_mul(1000).checked_div(TICKS_PER_SECOND).unwrap_or(0))
    }

    /// The presentation ticks since the container's clock started, as
    /// nanoseconds: the module's monotonic clock.
    #[must_use]
    pub fn monotonic_ns(&self) -> u64 {
        self.tick.saturating_mul(1_000_000_000).checked_div(TICKS_PER_SECOND).unwrap_or(0)
    }

    /// Begins a step at the presentation tick `tick`: the per-step counts
    /// start again, and notes due are released.
    pub fn begin_step(&mut self, tick: u64, sound_allowed: bool) {
        self.tick = self.tick.max(tick);
        self.sound_step = 0;
        self.sound_allowed |= sound_allowed;
        self.naplps.begin_step(self.tick);
        for held in self.contexts.iter_mut().flatten() {
            held.keys_pressed.clear();
            if held.origin.is_none() {
                held.origin = Some(self.tick);
            }
        }
        // Releases come first and count against the step's sound
        // operations, so that a batch never holds more than the bound.
        let now = self.tick;
        let mut index = 0;
        while let Some(note) = self.pending_notes.get(index).copied() {
            if note.due <= now && self.sound_step < MAX_SOUND_OPERATIONS_PER_STEP {
                self.sound.push(SoundOperation::Midi { part: note.part, status: MIDI_NOTE_OFF, data1: note.note, data2: 0 });
                self.sound_step = self.sound_step.saturating_add(1);
                self.pending_notes.remove(index);
            } else {
                index = index.saturating_add(1);
            }
        }
    }

    /// Whether sound was just allowed by this step, to tell the contexts.
    #[must_use]
    pub const fn sound_allowed(&self) -> bool {
        self.sound_allowed
    }

    /// Sets whether the tab is hidden.
    pub const fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    /// Begins a callback of `context` for an event of `kind` (none for a
    /// top level) whose first whole number is `first`.
    pub fn begin_callback(&mut self, context: ContextId, kind: Option<EventKind>, first: i32) {
        let activation = match kind {
            Some(EventKind::KEY_DOWN | EventKind::UI) => true,
            Some(EventKind::POINTER) => first & POINTER_ACTION_MASK == PointerAction::UP.number(),
            _ => false,
        };
        self.callback = Some((context, Callback { activation, sound_operations: 0 }));
        self.naplps.begin_callback();
    }

    /// Ends the callback: scopes it left open are closed to new things.
    pub fn end_callback(&mut self) {
        if let Some((context, _)) = self.callback.take()
            && let Some(held) = self.context_mut(context)
        {
            held.scope_stack.clear();
        }
    }

    /// Takes in an input event before it is given to `context`: the keys
    /// held and the pointer's place follow it whether or not a listener
    /// hears it. False when the context's manifest did not claim it, so
    /// that it is never given (section 4.3).
    pub fn take_in(&mut self, context: ContextId, event: &Input) -> bool {
        let Some(held) = self.context_mut(context) else { return false };
        let [first, second, third, fourth] = event.ints;
        match event.kind {
            EventKind::KEY_DOWN | EventKind::KEY_UP => {
                let InputPayload::Text(name) = &event.payload else { return false };
                if !held.open.keys.iter().any(|claimed| claimed == name) {
                    return false;
                }
                held.keys_down.retain(|down| down != name);
                if event.kind == EventKind::KEY_DOWN {
                    if held.keys_down.len() < MAX_KEYS_DOWN {
                        held.keys_down.push(name.clone());
                    }
                    if first & KeyBit::REPEAT_OR_LOST.number() == 0 && !held.keys_pressed.contains(name) && held.keys_pressed.len() < MAX_KEYS_DOWN {
                        held.keys_pressed.push(name.clone());
                    }
                }
                true
            }
            EventKind::TEXT => held.open.flags.text,
            EventKind::POINTER => {
                let action = first & POINTER_ACTION_MASK;
                // The manifest's `pointer`: "move" claims everything, "press"
                // everything but moves.
                let flags = held.open.flags;
                let claimed = flags.pointer_move || (flags.pointer_press && action != PointerAction::MOVE.number());
                if claimed {
                    let buttons = match &event.payload {
                        InputPayload::Ints(ints) => ints.get(1).copied().unwrap_or(0),
                        _ => 0,
                    };
                    let gone = action == PointerAction::CANCEL.number() || (action == PointerAction::LEAVE.number() && fourth == Handle::NT.number());
                    held.pointer = PointerState { x: second, y: third, buttons, inside: !gone, handle: if gone { 0 } else { fourth } };
                }
                claimed
            }
            EventKind::WHEEL => held.open.flags.wheel,
            EventKind::PAGE_SHOW => {
                if context != ContextId::Channel {
                    return false;
                }
                let url = match &event.payload {
                    InputPayload::Text(url) => url.clone(),
                    _ => String::new(),
                };
                // The renderer by the code the messages give it; a page with
                // no sub-page has the sub-code 0, as a context's opening has.
                let renderer = renderer_name(u8::try_from(third).ok().and_then(|code| Renderer::of(code).ok()).unwrap_or(Renderer::Teletext));
                let subcode = u16::try_from(second).ok().filter(|subcode| *subcode != NO_SUBCODE);
                held.shown = Some(PageShown { page: u16::try_from(first).unwrap_or(0), subcode, renderer, url });
                true
            }
            EventKind::PAGE_HIDE => context == ContextId::Channel,
            EventKind::FOCUS
            | EventKind::BLUR
            | EventKind::VISIBILITY
            | EventKind::NAVIGATE
            | EventKind::PAINTED
            | EventKind::CYCLE_END
            | EventKind::SOUND_CUE
            | EventKind::SOUND_ENDED
            | EventKind::FAULT
            | EventKind::UI
            | EventKind::SOUND_ALLOWED
            | EventKind::CLOSE
            | EventKind::SHARED
            | EventKind::TIMER
            | EventKind::TICK => true,
        }
    }

    /// The names of the keys any context holds as down.
    #[must_use]
    pub fn keys_held(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for name in self.contexts.iter().flatten().flat_map(|held| &held.keys_down) {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        names
    }

    /// Whether any context sees a pointer button as down.
    #[must_use]
    pub fn pointer_pressed(&self) -> bool {
        self.contexts.iter().flatten().any(|held| held.pointer.buttons != 0)
    }

    /// Whether `context` has a listener for events of `kind` on any thing.
    #[must_use]
    pub fn listens(&self, context: ContextId, kind: EventKind) -> bool {
        self.context(context).is_some_and(|held| held.listens.iter().any(|(_, listened)| *listened == kind))
    }

    /// Whether `context` has a listener for events of `kind` on `handle`.
    #[must_use]
    pub fn listens_to(&self, context: ContextId, handle: i32, kind: EventKind) -> bool {
        self.context(context).is_some_and(|held| held.listens.contains(&(handle, kind)))
    }

    /// The context's tick count.
    #[must_use]
    pub fn now(&self, context: ContextId) -> u64 {
        self.context(context).map_or(0, |held| held.now(self.tick))
    }

    /// The manifest's tick divisor of `context`.
    #[must_use]
    pub fn tick_divisor(&self, context: ContextId) -> u8 {
        self.context(context).map_or(0, |held| held.open.tick_divisor)
    }

    /// The timers of `context` due now, in order of due tick and then of
    /// making. Each one-shot timer is forgotten; each repeating timer is
    /// set again from now, so that one due several times fires once.
    pub fn due_timers(&mut self, context: ContextId) -> Vec<i32> {
        let tick = self.tick;
        let Some(held) = self.context_mut(context) else { return Vec::new() };
        let now = held.now(tick);
        let mut due: Vec<Timer> = held.timers.iter().filter(|timer| timer.due <= now).copied().collect();
        due.sort_by_key(|timer| (timer.due, timer.made));
        due.iter().map(|timer| timer.id).collect()
    }

    /// Marks the timer `id` of `context` as fired: forgotten if it does
    /// not repeat, otherwise due again a whole period from now. False when
    /// there is no such timer (a callback cancelled it meanwhile).
    pub fn fire_timer(&mut self, context: ContextId, id: i32) -> bool {
        let tick = self.tick;
        let Some(held) = self.context_mut(context) else { return false };
        let now = held.now(tick);
        let Some(index) = held.timers.iter().position(|timer| timer.id == id) else { return false };
        let Some(timer) = held.timers.get_mut(index) else { return false };
        if timer.repeat {
            timer.due = now.saturating_add(timer.every);
        } else {
            held.timers.remove(index);
        }
        true
    }

    /// The presentation tick `context` next needs a step at with no input:
    /// its earliest timer or held note.
    #[must_use]
    pub fn next_due(&self, context: ContextId) -> Option<u64> {
        let held = self.context(context)?;
        let origin = held.origin.unwrap_or(self.tick);
        let timer = held.timers.iter().map(|timer| origin.saturating_add(timer.due)).min();
        let note = self.pending_notes.iter().filter(|note| note.context == context).map(|note| note.due).min();
        match (timer, note) {
            (Some(timer), Some(note)) => Some(timer.min(note)),
            (one, other) => one.or(other),
        }
    }

    /// The next change of the shared store to tell, with the context to tell.
    pub fn take_shared_notice(&mut self) -> Option<(ContextId, String)> {
        self.shared_notices.pop_front()
    }

    /// Counts a failure of `context`, or a success; true when the failures
    /// in a row have reached [`MAX_CONSECUTIVE_FAILURES`] and the context
    /// is stopped.
    pub fn count_failure(&mut self, context: ContextId, failed: bool) -> bool {
        let Some(held) = self.context_mut(context) else { return false };
        if !failed {
            held.failures = 0;
            return false;
        }
        held.failures = held.failures.saturating_add(1);
        if held.failures >= MAX_CONSECUTIVE_FAILURES {
            held.stopped = Some(FailReason::Failures);
            return true;
        }
        false
    }

    /// Whether `context` is open and not stopped.
    #[must_use]
    pub fn runs(&self, context: ContextId) -> bool {
        self.context(context).is_some_and(|held| held.stopped.is_none())
    }

    /// Marks every context stopped for `reason`.
    pub fn stop_all(&mut self, reason: FailReason) {
        for held in self.contexts.iter_mut().flatten() {
            held.stopped.get_or_insert(reason);
        }
    }

    /// Adds a line to the log, cut to [`MAX_LOG_CHARS`] and with its
    /// control characters made spaces. A step's log keeps one line fewer
    /// than [`MAX_LOG_LINES`]; more are counted and dropped, and the last
    /// line of the batch says how many. A line of no context (the
    /// engine's own, or the container's) is given to the context whose
    /// callback is running, or else to the first that is open.
    pub fn log_line(&mut self, level: u8, context: Option<ContextId>, text: &str) {
        if self.log.len() >= MAX_LOG_LINES.saturating_sub(1) {
            self.log_dropped = self.log_dropped.saturating_add(1);
            return;
        }
        let context = self.log_context(context);
        let text: String = text.chars().take(MAX_LOG_CHARS).map(|character| if character.is_control() { ' ' } else { character }).collect();
        self.log.push(LogLine { level, context, text });
    }

    /// The context a log line is written under.
    fn log_context(&self, context: Option<ContextId>) -> ContextId {
        context
            .or(self.callback.map(|(running, _)| running))
            .or_else(|| [ContextId::Channel, ContextId::Page].into_iter().find(|open| self.is_open(*open)))
            .unwrap_or(ContextId::Page)
    }

    /// The bytes held for the container outside the WebAssembly memory:
    /// every table, the staged output, the log and the shared store.
    #[must_use]
    pub fn state_bytes(&self) -> usize {
        let mut bytes = self.contexts.iter().flatten().fold(size_of::<Self>(), |sum, held| sum.saturating_add(held.state_bytes()));
        bytes = self.shared.iter().fold(bytes, |sum, (key, value)| sum.saturating_add(cost(key)).saturating_add(cost(value)));
        bytes = self.shared_notices.iter().fold(bytes, |sum, (_, key)| sum.saturating_add(cost(key)).saturating_add(size_of::<ContextId>()));
        bytes = bytes.saturating_add(self.pending_notes.len().saturating_mul(size_of::<PendingNote>()));
        bytes = bytes.saturating_add(self.naplps.state_bytes());
        bytes.saturating_add(self.batch_bytes())
    }

    /// The bytes of the step's output so far, as the batch will hold them.
    #[must_use]
    pub fn batch_bytes(&self) -> usize {
        let mut bytes = self.log.iter().fold(0_usize, |sum, line| sum.saturating_add(cost(&line.text)).saturating_add(size_of::<LogLine>()));
        bytes = bytes.saturating_add(self.sound.len().saturating_mul(size_of::<SoundOperation>()));
        bytes = bytes.saturating_add(self.title.as_deref().map_or(0, cost));
        bytes = bytes.saturating_add(self.naplps.staged_bytes());
        bytes.saturating_add(self.navigate.as_deref().map_or(0, cost))
    }

    /// Room for `more` bytes of state, `output` of them in the step's
    /// output, or the `RangeError` naming the bound that has none.
    fn room(&self, more: usize, output: usize) -> Result<(), Reply> {
        if self.batch_bytes().saturating_add(output) > self.batch_limit {
            return Err(range("MAX_BATCH_BYTES", "one step's output is full"));
        }
        if self.state_bytes().saturating_add(more) > self.state_limit {
            return Err(range("MAX_HOST_STATE_BYTES", "the host holds as much for this container as it may"));
        }
        Ok(())
    }

    /// Everything the step changed, as its batch; the staged output is
    /// then empty. `verdicts` holds the dispatch bits of each event of
    /// the step. A batch holds at most [`MAX_SOUND_OPERATIONS`] sound
    /// operations: the host's own stops for a context that ended, if they
    /// would pass that, leave in the next batch.
    pub fn take_batch(&mut self, sequence: u32, verdicts: &[u8]) -> Batch {
        let mut batch = Batch::new(sequence);
        if !verdicts.is_empty() {
            let cancelled = u8::try_from(DispatchBit::CANCELLED.number()).unwrap_or(0);
            let handled = u8::try_from(DispatchBit::HANDLED.number()).unwrap_or(0);
            batch.verdicts = Some(verdicts.iter().map(|bits| Verdict { cancelled: bits & cancelled != 0, handled: bits & handled != 0 }).collect());
        }
        if !self.sound.is_empty() {
            let later = self.sound.split_off(self.sound.len().min(MAX_SOUND_OPERATIONS));
            batch.sound = Some(core::mem::replace(&mut self.sound, later));
        }
        if core::mem::take(&mut self.watch_changed) {
            let painted = self.watched_paintings();
            if painted != self.painted_told {
                self.painted_told.clone_from(&painted);
                batch.watch = Some(Watch { painted: Some(painted), tracked: None, wheel: None });
            }
        }
        self.naplps.take_batch(&mut batch);
        batch.title = self.title.take();
        batch.navigate = self.navigate.take();
        let dropped = core::mem::take(&mut self.log_dropped);
        if dropped > 0 {
            let context = self.log_context(None);
            self.log.push(LogLine { level: LEVEL_WARN, context, text: format!("MAX_LOG_LINES: {dropped} more log lines were dropped") });
        }
        if !self.log.is_empty() {
            batch.log = Some(core::mem::take(&mut self.log));
        }
        let states: Vec<ContextState> = [ContextId::Channel, ContextId::Page]
            .into_iter()
            .filter_map(|context| {
                let held = self.context(context)?;
                Some(ContextState {
                    context,
                    stopped: held.stopped,
                    failures: u16::try_from(held.failures).unwrap_or(u16::MAX),
                    next_due: self.next_due(context),
                })
            })
            .collect();
        if !states.is_empty() {
            batch.state = Some(states);
        }
        batch
    }

    /// The things whose painting some context listens for: what the
    /// decoder is asked to watch. The shared core has no thing that is
    /// painted, so this is empty until a renderer adds its handles.
    fn watched_paintings(&self) -> Vec<i32> {
        let mut handles: Vec<i32> = Vec::new();
        for (handle, kind) in self.contexts.iter().flatten().flat_map(|held| &held.listens) {
            if *kind == EventKind::PAINTED && *handle > Handle::NT.number() && !handles.contains(handle) && handles.len() < MAX_WATCHED_PAINTINGS {
                handles.push(*handle);
            }
        }
        handles
    }

    /// One host call of `context`: operation `op` with its whole numbers
    /// and payload.
    pub fn call(&mut self, context: i32, op: i32, ints: &[i32], payload: Payload<'_>) -> Reply {
        let Some(context) = context_numbered(context) else { return type_error("no such context") };
        if !self.is_open(context) {
            return type_error("the context is not open");
        }
        let Some(op) = Op::from_number(op) else { return Reply::Type(format!("operation {op} is not one this host has")) };
        let ints = Ints { op, ints };
        match self.operate(context, op, &ints, payload) {
            Ok(reply) | Err(reply) => reply,
        }
    }

    #[expect(clippy::too_many_lines, reason = "one arm for each operation of section 10.2's table, in the table's order")]
    fn operate(&mut self, context: ContextId, op: Op, ints: &Ints<'_>, payload: Payload<'_>) -> Result<Reply, Reply> {
        match op {
            Op::LOG => {
                let [level] = ints.exactly()?;
                let text = text_of(op, payload)?;
                let level = within(level, 0, MAX_LOG_LEVEL, "MAX_LOG_LEVEL", "a log level")?;
                self.log_line(level, Some(context), text);
                Ok(Reply::Undefined)
            }
            Op::NOW => {
                no_payload(op, payload)?;
                // The low 31 bits; asked with 1, the bits above them
                // (section 10.2: "low 31 bits; high by 2,1").
                let high = match ints.ints {
                    [] | [0] => false,
                    [1] => true,
                    _ => return Err(type_error("the tick count is asked for whole (no number or 0) or for its high part (1)")),
                };
                let now = self.now(context);
                let part = if high { now.checked_shr(TICK_LOW_BITS).unwrap_or(0) } else { now };
                Ok(Reply::Int(i32::try_from(part & TICK_LOW_MASK).unwrap_or(0)))
            }
            Op::INFO => {
                let [field] = ints.exactly()?;
                no_payload(op, payload)?;
                self.info(context, field)
            }
            Op::TIMER => {
                let [ticks, repeat] = ints.exactly()?;
                no_payload(op, payload)?;
                self.timer(context, ticks, repeat)
            }
            Op::CANCEL => {
                let [id] = ints.exactly()?;
                no_payload(op, payload)?;
                if let Some(held) = self.context_mut(context) {
                    held.timers.retain(|timer| timer.id != id);
                }
                Ok(Reply::Undefined)
            }
            Op::BUDGET => {
                let [] = ints.exactly()?;
                no_payload(op, payload)?;
                Ok(Reply::Text(self.budget(context)))
            }
            Op::KEY_STATE => {
                let [which] = ints.exactly()?;
                let name = text_of(op, payload)?;
                if name.len() > MAX_KEY_NAME_BYTES {
                    return Err(range("MAX_KEY_NAME_BYTES", "a key's name is too long"));
                }
                let held = self.context(context).ok_or_else(|| type_error("the context is not open"))?;
                let keys = match which {
                    0 => &held.keys_down,
                    1 => &held.keys_pressed,
                    _ => return Err(type_error("a key's state is asked as 0 (down) or 1 (pressed)")),
                };
                Ok(Reply::Int(i32::from(keys.iter().any(|key| key == name))))
            }
            Op::POINTER_STATE => {
                let [] = ints.exactly()?;
                no_payload(op, payload)?;
                let pointer = self.context(context).map(|held| held.pointer).unwrap_or_default();
                Ok(Reply::Text(format!("{},{},{},{},{}", pointer.x, pointer.y, pointer.buttons, i32::from(pointer.inside), pointer.handle)))
            }
            Op::TITLE => {
                let [] = ints.exactly()?;
                let text = text_of(op, payload)?;
                if context != ContextId::Page {
                    return Err(type_error("only the page context sets the page's title"));
                }
                if text.len() > MAX_TITLE_BYTES {
                    return Err(range("MAX_TITLE_BYTES", "the title is too long"));
                }
                if has_control(text) {
                    return Err(type_error("a title holds no control character"));
                }
                // The new title replaces one staged earlier in the step.
                let staged = self.title.as_deref().map_or(0, cost);
                self.room(cost(text).saturating_mul(2).saturating_sub(staged), cost(text).saturating_sub(staged))?;
                if let Some(page) = self.context_mut(context) {
                    text.clone_into(&mut page.title);
                }
                self.title = Some(text.to_owned());
                Ok(Reply::Undefined)
            }
            Op::NAVIGATE => {
                let [] = ints.exactly()?;
                let target = text_of(op, payload)?;
                self.navigate(context, target)
            }
            Op::SCOPE_BEGIN | Op::SCOPE_LEAVE | Op::SCOPE_END => self.scope(context, op, ints, payload),
            Op::SHARED_GET | Op::SHARED_SET | Op::SHARED_DELETE | Op::SHARED_KEYS => self.shared(context, op, ints, payload),
            Op::RANDOM => {
                let [count] = ints.exactly()?;
                no_payload(op, payload)?;
                let count = usize::try_from(count)
                    .ok()
                    .filter(|count| *count <= MAX_RANDOM_BYTES)
                    .ok_or_else(|| range("MAX_RANDOM_BYTES", "random bytes are asked for 0 to 65,536 at a time"))?;
                let mut bytes = vec![0_u8; count];
                let filled = self.context_mut(context).is_some_and(|held| held.generator.fill(&mut bytes));
                if filled { Ok(Reply::Bytes(bytes)) } else { Err(Reply::Error("the random source is not available".to_owned())) }
            }
            Op::CLOCK => {
                let [] = ints.exactly()?;
                no_payload(op, payload)?;
                Ok(Reply::Text(self.clock_now_ms().to_string()))
            }
            Op::ZONE => {
                let [] = ints.exactly()?;
                no_payload(op, payload)?;
                Ok(Reply::Int(i32::from(self.zone_minutes)))
            }
            Op::LISTEN => {
                let [handle, kind, on] = ints.exactly()?;
                no_payload(op, payload)?;
                self.listen(context, handle, kind, on)
            }
            Op::SOUND_PLAY | Op::SOUND_STOP | Op::SYNTH_START | Op::SYNTH_MIDI | Op::SYNTH_NOTE | Op::SYNTH_STOP => {
                no_payload(op, payload)?;
                self.sound_operation(context, op, ints)
            }
            Op::NP_COLOUR_DEFINE | Op::NP_COLOUR_SET | Op::NP_BLINK | Op::NP_PAINT | Op::NP_REPAINT | Op::NP_MEASURE => {
                self.naplps_operation(context, op, ints, payload)
            }
        }
    }

    fn info(&self, context: ContextId, field: i32) -> Result<Reply, Reply> {
        let field = Info::from_number(field).ok_or_else(|| type_error("no such field"))?;
        let held = self.context(context).ok_or_else(|| type_error("the context is not open"))?;
        // A channel context is told of the page shown by `pageshow`; until
        // the first one it has no page (section 3.3).
        let channel = context == ContextId::Channel;
        let text = |text: &str| Reply::Text(text.to_owned());
        Ok(match field {
            Info::VERSION => Reply::Int(i32::from(crate::API_VERSION)),
            Info::LEVEL => Reply::Int(context.number()),
            Info::CHANNEL_URL => text(&held.open.channel_url),
            Info::CHANNEL_TITLE => text(&held.open.channel_title),
            Info::PAGE_NUMBER => match (channel, &held.shown) {
                (true, None) => Reply::Undefined,
                (true, Some(shown)) => Reply::Text(format!("{:03X}", shown.page)),
                (false, _) => Reply::Text(format!("{:03X}", held.open.page)),
            },
            Info::PAGE_SUBPAGE => {
                let subcode = if channel {
                    held.shown.as_ref().and_then(|shown| shown.subcode)
                } else {
                    Some(held.open.subcode).filter(|subcode| *subcode != NO_SUBCODE)
                };
                subcode.map_or(Reply::Undefined, |subcode| Reply::Text(format!("{subcode:04X}")))
            }
            Info::PAGE_URL => match (channel, &held.shown) {
                (true, None) => Reply::Undefined,
                (true, Some(shown)) => text(&shown.url),
                (false, _) => text(&held.open.page_url),
            },
            Info::PAGE_RENDERER => match (channel, &held.shown) {
                (true, None) => Reply::Undefined,
                (true, Some(shown)) => text(shown.renderer),
                (false, _) => text(renderer_name(held.open.renderer)),
            },
            Info::PAGE_TITLE => {
                // The page's title is the page context's to set; the
                // channel context reads what it last set.
                let page = self.context(ContextId::Page).map_or("", |page| page.title.as_str());
                text(page)
            }
            Info::HIDDEN => Reply::Int(i32::from(self.hidden)),
            Info::SOUND_ALLOWED => Reply::Int(i32::from(self.sound_allowed)),
            Info::API => Reply::Int(i32::from(held.open.api)),
            Info::SCRIPT_URL => text(&held.open.script_url),
            Info::LANGUAGE => text(&held.open.language),
            Info::ORIGIN => {
                let since = held.now(self.tick).saturating_mul(1000).checked_div(TICKS_PER_SECOND).unwrap_or(0);
                Reply::Text(self.clock_now_ms().saturating_sub(since).to_string())
            }
        })
    }

    fn timer(&mut self, context: ContextId, ticks: i32, repeat: i32) -> Result<Reply, Reply> {
        if !(1..=MAX_TIMER_TICKS).contains(&ticks) {
            return Err(range("MAX_TIMER_TICKS", "a timer's delay is 1 to 216,000 ticks"));
        }
        if !(0..=1).contains(&repeat) {
            return Err(type_error("a timer repeats (1) or does not (0)"));
        }
        self.room(size_of::<Timer>(), 0)?;
        let tick = self.tick;
        let held = self.context_mut(context).ok_or_else(|| type_error("the context is not open"))?;
        if held.timers.len() >= MAX_TIMERS {
            return Err(range("MAX_TIMERS", "the context has as many timers as it may"));
        }
        // A number no live timer has; numbers go round after two thousand
        // million timers.
        let mut id = held.next_timer;
        while held.timers.iter().any(|timer| timer.id == id) {
            id = id.checked_add(1).unwrap_or(1);
        }
        held.next_timer = id.checked_add(1).unwrap_or(1);
        let every = u64::try_from(ticks).unwrap_or(1);
        let made = held.timers_made;
        held.timers_made = made.saturating_add(1);
        let scope = held.scope_stack.last().copied().unwrap_or(0);
        held.timers.push(Timer { id, every, due: held.now(tick).saturating_add(every), repeat: repeat == 1, scope, made });
        Ok(Reply::Int(id))
    }

    fn navigate(&mut self, context: ContextId, target: &str) -> Result<Reply, Reply> {
        if target.len() > MAX_URL_BYTES {
            return Err(range("MAX_URL_BYTES", "the address is too long"));
        }
        // Counted before anything else, and never given back (rule 6).
        let tick = self.tick;
        let activation = self.callback.is_some_and(|(calling, callback)| calling == context && callback.activation);
        let held = self.context_mut(context).ok_or_else(|| type_error("the context is not open"))?;
        if held.navigations.count(tick) >= MAX_NAVIGATIONS_PER_SECOND {
            return Err(range("MAX_NAVIGATIONS_PER_SECOND", "the context asked to navigate too often"));
        }
        held.navigations.add(tick);
        let inside = if is_page_target(target) {
            true
        } else {
            let lower = target.get(..target.len().min(8)).unwrap_or("").to_ascii_lowercase();
            if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                return Err(type_error("a navigation's target is a page number, a page and sub-page, or an http: or https: address"));
            }
            if has_control(target) || target.contains(' ') {
                return Err(type_error("an address holds no space or control character"));
            }
            // Whether an address is inside the channel is the trusted
            // core's to decide (`ChannelPolicy`, section 2). Here the
            // channel's own address as a prefix is taken as inside, and
            // anything else needs the viewer's activation.
            !held.open.channel_url.is_empty() && target.starts_with(held.open.channel_url.as_str())
        };
        if !inside && !activation {
            return Err(type_error("leaving the channel is allowed only while a key press, a pointer release or a component's activation is handled"));
        }
        let staged = self.navigate.as_deref().map_or(0, cost);
        self.room(cost(target).saturating_sub(staged), cost(target).saturating_sub(staged))?;
        self.navigate = Some(target.to_owned());
        Ok(Reply::Undefined)
    }

    fn scope(&mut self, context: ContextId, op: Op, ints: &Ints<'_>, payload: Payload<'_>) -> Result<Reply, Reply> {
        no_payload(op, payload)?;
        if op == Op::SCOPE_BEGIN {
            let [] = ints.exactly()?;
            self.room(size_of::<i32>().saturating_mul(2), 0)?;
            let held = self.context_mut(context).ok_or_else(|| type_error("the context is not open"))?;
            if held.scope_stack.len() >= MAX_SCOPE_DEPTH {
                return Err(range("MAX_SCOPE_DEPTH", "scopes nest no deeper"));
            }
            if held.scopes.len() >= MAX_SCOPES {
                return Err(range("MAX_SCOPES", "the context has as many scopes as it may"));
            }
            let mut id = held.next_scope.max(1);
            while held.scopes.contains(&id) {
                id = id.checked_add(1).unwrap_or(1);
            }
            held.next_scope = id.checked_add(1).unwrap_or(1);
            held.scopes.push(id);
            held.scope_stack.push(id);
            return Ok(Reply::Int(id));
        }
        let [id] = ints.exactly()?;
        let held = self.context_mut(context).ok_or_else(|| type_error("the context is not open"))?;
        if !held.scopes.contains(&id) {
            return Err(type_error("no such scope"));
        }
        if op == Op::SCOPE_LEAVE {
            if held.scope_stack.last() != Some(&id) {
                return Err(type_error("the scope left is not the innermost one open"));
            }
            held.scope_stack.pop();
        } else {
            // Everything in it goes: here, its timers.
            held.scopes.retain(|scope| *scope != id);
            held.scope_stack.retain(|scope| *scope != id);
            held.timers.retain(|timer| timer.scope != id);
        }
        Ok(Reply::Undefined)
    }

    fn shared(&mut self, context: ContextId, op: Op, ints: &Ints<'_>, payload: Payload<'_>) -> Result<Reply, Reply> {
        match op {
            Op::SHARED_KEYS => {
                let [] = ints.exactly()?;
                no_payload(op, payload)?;
                let mut out = String::from("[");
                for (index, (key, _)) in self.shared.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    json_string(&mut out, key);
                }
                out.push(']');
                Ok(Reply::Text(out))
            }
            Op::SHARED_GET | Op::SHARED_DELETE => {
                let [] = ints.exactly()?;
                let key = text_of(op, payload)?;
                if key.len() > MAX_SHARED_KEY_BYTES {
                    return Err(range("MAX_SHARED_KEY_BYTES", "the key is too long"));
                }
                let found = self.shared.iter().position(|(held, _)| held == key);
                if op == Op::SHARED_GET {
                    return Ok(found.and_then(|index| self.shared.get(index)).map_or(Reply::Undefined, |(_, value)| Reply::Text(value.clone())));
                }
                if let Some(index) = found {
                    self.shared.remove(index);
                    self.notice(context, key);
                }
                Ok(Reply::Undefined)
            }
            _ => {
                let [key_length] = ints.exactly()?;
                let whole = text_of(op, payload)?;
                let key_length = usize::try_from(key_length).map_err(|_| type_error("the key's length is not negative"))?;
                if key_length > MAX_SHARED_KEY_BYTES {
                    return Err(range("MAX_SHARED_KEY_BYTES", "the key is too long"));
                }
                let (Some(key), Some(value)) = (whole.get(..key_length), whole.get(key_length..)) else {
                    return Err(type_error("the key's length is not a place between two characters of the payload"));
                };
                if value.len() > MAX_SHARED_VALUE_BYTES {
                    return Err(range("MAX_SHARED_VALUE_BYTES", "the value is too long as JSON"));
                }
                let found = self.shared.iter().position(|(held, _)| held == key);
                if found.is_none() && self.shared.len() >= MAX_SHARED_KEYS {
                    return Err(range("MAX_SHARED_KEYS", "the shared store has as many keys as it may"));
                }
                let other: usize = self
                    .shared
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| Some(*index) != found)
                    .fold(0, |sum, (_, (key, value))| sum.saturating_add(key.len()).saturating_add(value.len()));
                if other.saturating_add(key.len()).saturating_add(value.len()) > MAX_SHARED_BYTES {
                    return Err(range("MAX_SHARED_BYTES", "the shared store is full"));
                }
                self.room(cost(key).saturating_mul(2).saturating_add(cost(value)), 0)?;
                match found.and_then(|index| self.shared.get_mut(index)) {
                    Some((_, held)) => value.clone_into(held),
                    None => self.shared.push((key.to_owned(), value.to_owned())),
                }
                self.notice(context, key);
                Ok(Reply::Undefined)
            }
        }
    }

    /// Queues the telling of the other context that `key` changed, once.
    fn notice(&mut self, from: ContextId, key: &str) {
        let to = from.other();
        if !self.is_open(to) || self.shared_notices.iter().any(|(waiting, held)| *waiting == to && held == key) {
            return;
        }
        if self.shared_notices.len() < MAX_SHARED_NOTICES {
            self.shared_notices.push_back((to, key.to_owned()));
        }
    }

    fn listen(&mut self, context: ContextId, handle: i32, kind: i32, on: i32) -> Result<Reply, Reply> {
        if !(0..MAX_LISTEN_KINDS).contains(&kind) {
            return Err(type_error("no such kind of event"));
        }
        let kind = EventKind::from_number(kind).ok_or_else(|| type_error("no such kind of event"))?;
        if !(0..=1).contains(&on) {
            return Err(type_error("a listener is there (1) or is not (0)"));
        }
        // The shared core has two things, `nt` and `nt.sound`; a renderer's
        // things are added to this check with the renderer.
        if Handle::from_number(handle).is_none() && !self.naplps.is_thing(context, handle) {
            return Err(type_error("no such thing"));
        }
        self.room(size_of::<(i32, EventKind)>(), 0)?;
        let held = self.context_mut(context).ok_or_else(|| type_error("the context is not open"))?;
        let found = held.listens.iter().position(|entry| *entry == (handle, kind));
        match (on == 1, found) {
            (true, None) => {
                if held.listens.len() >= MAX_LISTENS {
                    return Err(range("MAX_LISTENS", "the context listens to as many things as it may"));
                }
                held.listens.push((handle, kind));
                self.watch_changed = true;
            }
            (false, Some(index)) => {
                held.listens.remove(index);
                self.watch_changed = true;
            }
            _ => {}
        }
        Ok(Reply::Undefined)
    }

    fn sound_operation(&mut self, context: ContextId, op: Op, ints: &Ints<'_>) -> Result<Reply, Reply> {
        let channels = AUDIO_CHANNELS.saturating_sub(1);
        // Validate first: an operation that fails changes nothing, and is
        // not counted.
        let (operation, note) = match op {
            Op::SOUND_PLAY => {
                let [sound, voice, gain] = ints.exactly()?;
                let sound = within(sound, 1, MAX_SOUND_NUMBER, "MAX_SOUND_NUMBER", "a sound's handle")?;
                let voice = within(voice, 0, channels, "AUDIO_CHANNELS", "a voice")?;
                let gain = within(gain, 0, MAX_GAIN, "MAX_GAIN", "a gain")?;
                if !self.context(context).is_some_and(|held| held.open.sounds.contains(&sound)) {
                    return Err(type_error("the channel announced no such sound"));
                }
                (SoundOperation::Play { voice, sound, gain }, None)
            }
            Op::SOUND_STOP => {
                let [voice] = ints.exactly()?;
                (SoundOperation::Stop { voice: within(voice, 0, channels, "AUDIO_CHANNELS", "a voice")? }, None)
            }
            Op::SYNTH_START => {
                let [part, family, variant, gain] = ints.exactly()?;
                let part = within(part, 0, channels, "AUDIO_CHANNELS", "a part")?;
                let family = u8::try_from(family)
                    .ok()
                    .filter(|family| FAMILIES.iter().any(|(_, codec)| codec.code() == *family))
                    .ok_or_else(|| type_error("no such synthesizer family"))?;
                let variant = within(variant, 0, MAX_SYNTH_VARIANT, "MAX_SYNTH_VARIANT", "a variant")?;
                let gain = within(gain, 0, MAX_GAIN, "MAX_GAIN", "a gain")?;
                (SoundOperation::SynthStart { part, family, variant, gain }, None)
            }
            Op::SYNTH_MIDI => {
                let [part, status, data1, data2] = ints.exactly()?;
                let part = within(part, 0, channels, "AUDIO_CHANNELS", "a part")?;
                let status = u8::try_from(status).ok().filter(|status| [MIDI_NOTE_OFF, MIDI_NOTE_ON, MIDI_CONTROL, MIDI_PROGRAM, MIDI_BEND].contains(status));
                let status = status.ok_or_else(|| type_error("a MIDI message is note off, note on, control change, program change or pitch bend"))?;
                let data1 = within(data1, 0, MAX_MIDI_DATA, "MAX_MIDI_DATA", "a MIDI data byte")?;
                let data2 = within(data2, 0, MAX_MIDI_DATA, "MAX_MIDI_DATA", "a MIDI data byte")?;
                (SoundOperation::Midi { part, status, data1, data2 }, None)
            }
            Op::SYNTH_NOTE => {
                let [part, note, velocity, ticks] = ints.exactly()?;
                let part = within(part, 0, channels, "AUDIO_CHANNELS", "a part")?;
                let note = within(note, 0, MAX_MIDI_DATA, "MAX_MIDI_DATA", "a note")?;
                let velocity = within(velocity, 0, MAX_MIDI_DATA, "MAX_MIDI_DATA", "a velocity")?;
                if !(1..=MAX_NOTE_TICKS).contains(&ticks) {
                    return Err(range("MAX_NOTE_TICKS", "a note is held for 1 to 600 ticks"));
                }
                if self.pending_notes.len() >= MAX_PENDING_NOTES {
                    return Err(range("MAX_PENDING_NOTES", "as many notes wait for their release as may"));
                }
                let due = self.tick.saturating_add(u64::try_from(ticks).unwrap_or(1));
                (SoundOperation::Midi { part, status: MIDI_NOTE_ON, data1: note, data2: velocity }, Some(PendingNote { due, part, note, context }))
            }
            _ => {
                let [part] = ints.exactly()?;
                (SoundOperation::SynthStop { part: within(part, 0, channels, "AUDIO_CHANNELS", "a part")? }, None)
            }
        };
        // Then count, before any work, and never give the count back
        // (rule 6): stops count as operations.
        let tick = self.tick;
        let in_callback = self.callback.map_or(0, |(_, callback)| callback.sound_operations);
        if in_callback >= MAX_SOUND_OPERATIONS_PER_CALLBACK {
            return Err(range("MAX_SOUND_OPERATIONS_PER_CALLBACK", "the callback asked for too many sound operations"));
        }
        if self.sound_step >= MAX_SOUND_OPERATIONS_PER_STEP {
            return Err(range("MAX_SOUND_OPERATIONS_PER_STEP", "the step asked for too many sound operations"));
        }
        if self.sound_second.count(tick) >= MAX_SOUND_OPERATIONS_PER_SECOND {
            return Err(range("MAX_SOUND_OPERATIONS_PER_SECOND", "too many sound operations in a second"));
        }
        self.room(size_of::<SoundOperation>().saturating_add(size_of::<PendingNote>()), size_of::<SoundOperation>())?;
        if let Some((_, callback)) = self.callback.as_mut() {
            callback.sound_operations = callback.sound_operations.saturating_add(1);
        }
        self.sound_step = self.sound_step.saturating_add(1);
        self.sound_second.add(tick);
        // Until the viewer has pressed a key or the pointer in this tab,
        // sound does nothing and says so once (section 3.5).
        if !self.sound_allowed {
            if !self.sound_refusal_logged {
                self.sound_refusal_logged = true;
                self.log_line(3, Some(context), "sound does nothing until the viewer has pressed a key or the pointer in this tab");
            }
            return Ok(Reply::Undefined);
        }
        if let Some(held) = self.context_mut(context) {
            match operation {
                SoundOperation::Play { voice, .. } => held.voices |= 1_u8.checked_shl(u32::from(voice)).unwrap_or(0),
                SoundOperation::Stop { voice } => held.voices &= !1_u8.checked_shl(u32::from(voice)).unwrap_or(0),
                SoundOperation::SynthStart { part, .. } => held.parts |= 1_u8.checked_shl(u32::from(part)).unwrap_or(0),
                SoundOperation::SynthStop { part } => held.parts &= !1_u8.checked_shl(u32::from(part)).unwrap_or(0),
                SoundOperation::Midi { .. } => {}
            }
        }
        if let SoundOperation::SynthStop { part } = operation {
            self.pending_notes.retain(|waiting| waiting.part != part);
        }
        self.sound.push(operation);
        if let Some(note) = note {
            self.pending_notes.push(note);
        }
        Ok(Reply::Undefined)
    }

    /// The budget report (section 3.7), as JSON text.
    fn budget(&self, context: ContextId) -> String {
        let held = self.context(context);
        let count = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        let mut second = RateWindow { counts: self.sound_second.counts.clone() };
        let entries: [(&str, u64, u64); 14] = [
            ("scriptBytes", held.map_or(0, |held| count(held.script_bytes)), count(MAX_SCRIPT_BYTES)),
            ("heapBytes", count(self.heap_bytes), u64::from(QUICKJS_HEAP_BYTES.unsigned_abs())),
            ("timers", held.map_or(0, |held| count(held.timers.len())), count(MAX_TIMERS)),
            ("listeners", held.map_or(0, |held| count(held.listens.len())), count(MAX_LISTENS)),
            // The network is work package W2's: the core has no request and no socket.
            ("requests", 0, 0),
            ("sockets", 0, 0),
            ("soundOperations", u64::from(second.count(self.tick)), u64::from(MAX_SOUND_OPERATIONS_PER_SECOND)),
            ("callbackPercent", u64::from(self.figures.callback_percent), PERCENT),
            ("callbackPercentMax", u64::from(self.figures.callback_percent_max), PERCENT),
            ("stepPercentMax", u64::from(self.figures.step_percent_max), PERCENT),
            ("secondPercent", u64::from(self.figures.second_percent), PERCENT),
            ("batchBytes", count(self.batch_bytes()), count(self.batch_limit)),
            ("hostBytes", count(self.state_bytes()), count(self.state_limit)),
            ("failures", held.map_or(0, |held| u64::from(held.failures)), u64::from(MAX_CONSECUTIVE_FAILURES)),
        ];
        let mut out = String::from("{");
        for (index, (name, used, limit)) in entries.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{name}\":{{\"used\":{used},\"limit\":{limit}}}");
        }
        self.naplps.budget(&mut out);
        out.push('}');
        out
    }

    /// Lowers the bounds on held bytes and on a step's output, so that
    /// tests can reach them with the shared core's small tables.
    #[cfg(test)]
    pub(crate) const fn set_limits_for_test(&mut self, state: usize, batch: usize) {
        self.state_limit = state;
        self.batch_limit = batch;
    }
}

/// Whether `target` is a page of this channel by number: three
/// hexadecimal digits from 100 to 8FF, or those, a slash and four
/// hexadecimal digits of sub-page.
fn is_page_target(target: &str) -> bool {
    let (page, subpage) = target.split_once('/').map_or((target, None), |(page, subpage)| (page, Some(subpage)));
    let hexadecimal = |text: &str, length: usize| text.len() == length && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    hexadecimal(page, 3) && page.bytes().next().is_some_and(|first| (b'1'..=b'8').contains(&first)) && subpage.is_none_or(|subpage| hexadecimal(subpage, 4))
}

#[cfg(test)]
mod tests;
