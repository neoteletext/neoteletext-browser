//! A page's rule table between the processes (neoteletext-common's
//! `docs/page-rules.md`, RULE-52, RULE-58 and RULE-63): the decoder of a
//! page's channel decodes and evaluates the table; the trusted core is the
//! gate. These are the payloads they exchange: the viewer's input given to
//! the decoder with a sequence number, the metered values, what the
//! decoder tells of a table (its interest mask, the keys it names, its
//! watch list and wheel rectangles), the reports it asks for, a state query and its answer, a
//! fault and the counters.
//!
//! The decoder never supplies an input event's bytes (RULE-52 item 3): a
//! report names a sequence number, and the trusted core builds the
//! `RULE_EVENT` from its own kept copy.

/// The greatest cue number (RULE-109), which a cue and a cue heard carry.
pub use neoteletext_rules::limits::MAX_CUE;
/// The rule and row of a run no rule made (RULE-46), as a report carries them.
pub use neoteletext_rules::limits::NO_RULE;
use neoteletext_rules::limits::VARIABLES;
use neoteletext_rules::limits::{LAST_SUBCODE, MAX_CONTROL, MAX_RECTS, MAX_ROWS, MAX_RULES, MAX_STATE_BYTES};
use neoteletext_rules::table::VALUE_BYTES;
pub use neoteletext_rules::table::{Key, KeyKind};

use crate::audio::{MAX_WATCH_SLOTS, WatchSlot};
use crate::{Fields, ProtocolError};

/// The input events the trusted core keeps for each open page, so that a
/// report can only name one it gave (RULE-52 item 2, RULE-57).
pub const INPUT_RING: usize = 64;
/// The ticks after an event was given within which a report may name it
/// (RULE-52 item 3).
pub const SEND_WINDOW_TICKS: u32 = 4;
/// The most reports a second the trusted core sends for a page, and the
/// most it holds (RULE-52 item 5).
pub const SEND_BUDGET: u32 = 120;
/// The most reports a second with no input event: a bare firing or a
/// value (RULE-52 item 5).
pub const TIMED_SEND_BUDGET: u32 = 20;
/// The least ticks between two state queries answered for a page: one a
/// second (RULE-47).
pub const QUERY_INTERVAL_TICKS: u32 = 60;

/// The most wheel rectangles a decoder tells (RULE-51 item 4): the rule
/// table's `MAX_RECTS`.
pub const MAX_WHEEL_RECTS: usize = MAX_RECTS;
/// The most keys a decoder tells of a table (REQ-7037). The keys the
/// component library asks every page for to select pages by are 28: ten
/// of the digit row, ten of the keypad, F1 to F4 and the four coloured
/// keys (RULE-54 item 5); this leaves a page 100 of its own, a musical
/// keyboard on every letter row among them. A table that names more is
/// told as asking for any key, which is the whole keyboard, so the bound
/// costs such a page the viewer's keys and nothing of its own. A count
/// fits the message's one byte.
pub const MAX_KEY_ASKS: usize = 128;
const _: () = assert!(MAX_KEY_ASKS <= u8::MAX as usize);
/// The most bytes of a state answer after its stamp (RULE-47): the rule
/// table's `MAX_STATE_BYTES`, which bounds the answer with its stamp.
pub const MAX_PAGE_STATE: usize = MAX_STATE_BYTES;
/// The bytes of a value a report carries (RULE-44).
pub const REPORT_VALUE_BYTES: usize = VALUE_BYTES;
/// The highest fault code (RULE-48): 10, `queue_full`.
pub const MAX_FAULT: u8 = 10;
/// The withdrawn fault code 2, `tick_overrun`, which is never sent and
/// never reused (RULE-48).
pub const WITHDRAWN_FAULT: u8 = 2;
/// The most sound commands a tick the trusted core passes from one view's
/// decoder to the sound helper (RULE-52 item 5), however they arose; the
/// rest are dropped and told as fault 10.
pub const SOUND_COMMAND_BUDGET: u32 = 512;
/// The bits of an interest mask (RULE-52 item 1): keys, text, pointer
/// buttons, pointer moves, the wheel and focus changes.
pub const INTEREST_ALL: u8 = 0b11_1111;
/// The interest mask's bits, as the rule table's index gives them.
pub use neoteletext_rules::table::{INTEREST_FOCUS, INTEREST_KEYS, INTEREST_MOVES, INTEREST_POINTER_BUTTONS, INTEREST_TEXT, INTEREST_WHEEL};

/// The highest target kind a report names (RULE-12): 0 the page, 1 a
/// control, 2 an object, 3 a rectangle.
const MAX_TARGET_KIND: u8 = 3;

const INPUT_KEY: u8 = 1;
const INPUT_TEXT: u8 = 2;
const INPUT_POINTER: u8 = 3;
const INPUT_WHEEL: u8 = 4;
const INPUT_FOCUS: u8 = 5;
const INPUT_BLUR: u8 = 6;
const INPUT_DRAWING: u8 = 7;

const REPORT_FIRED: u8 = 0;
const REPORT_INPUT: u8 = 1;
const REPORT_VALUE: u8 = 2;

/// No base character: a key whose `base` is absent.
const NO_BASE: u32 = u32::MAX;
/// The highest key or pointer action.
const MAX_ACTION: u8 = 2;
/// The highest pointer action: gone.
const MAX_POINTER_ACTION: u8 = 3;

/// What a key did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Went down.
    Press,
    /// Repeated while held.
    Repeat,
    /// Came up.
    Release,
}

/// What the pointer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerAction {
    /// A button went down.
    Press,
    /// A button came up.
    Release,
    /// It moved.
    Move,
    /// It left the page, or the page stopped taking input.
    Gone,
}

/// One input event of the viewer as the trusted core gives it to the open
/// page's decoder (RULE-52 item 1): what the evaluator matches triggers
/// against, never the event's bytes, which the trusted core keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleInput {
    /// A key.
    Key {
        /// What it did.
        action: KeyAction,
        /// A functional code that names a key, not a character.
        named: bool,
        /// The character's scalar value, or the functional code.
        code: u32,
        /// The key's character without modifiers, when it differs.
        base: Option<u32>,
        /// The modifier bits held.
        mods: u8,
    },
    /// Text that came from no key, a paste or a composition.
    Text,
    /// The pointer, or a contact of a touch screen.
    Pointer {
        /// What it did.
        action: PointerAction,
        /// On a press or release the button; on a move the buttons held.
        button: u8,
        /// The pixel of the page's picture.
        x: u16,
        /// The pixel of the page's picture.
        y: u16,
        /// The teletext control under the pointer, numbered from 1 in the
        /// order the page declares them; 0 for none.
        control: u16,
    },
    /// The wheel.
    Wheel {
        /// The pixel of the page's picture.
        x: u16,
        /// The pixel of the page's picture.
        y: u16,
        /// The teletext control under the pointer; 0 for none.
        control: u16,
    },
    /// The page gained the keyboard.
    Focus,
    /// The page lost the keyboard.
    Blur,
    /// The pointer came onto a numbered drawing of the page's ReGIS
    /// picture that its pick record names, or went off it (the engine's
    /// `DRAWING_HOVER`).
    Drawing {
        /// It went off the drawing.
        leave: bool,
        /// The drawing's number, never 0.
        drawing: u16,
    },
}

impl RuleInput {
    /// The interest class this input belongs to (RULE-52 item 1): one of
    /// the `INTEREST_` bits.
    #[must_use]
    pub const fn interest(&self) -> u8 {
        match self {
            Self::Key { .. } => INTEREST_KEYS,
            Self::Text => INTEREST_TEXT,
            Self::Pointer { action: PointerAction::Press | PointerAction::Release, .. } => INTEREST_POINTER_BUTTONS,
            Self::Pointer { .. } | Self::Drawing { .. } => INTEREST_MOVES,
            Self::Wheel { .. } => INTEREST_WHEEL,
            Self::Focus | Self::Blur => INTEREST_FOCUS,
        }
    }
}

/// A rectangle of a page's picture, in its pixels, inside which an enabled
/// `wheel` rule applies (RULE-51 item 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WheelRect {
    /// The left edge.
    pub x: u16,
    /// The top edge.
    pub y: u16,
    /// At least 1.
    pub width: u16,
    /// At least 1.
    pub height: u16,
}

impl WheelRect {
    /// Whether the pixel is inside.
    #[must_use]
    pub const fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && y >= self.y && x.saturating_sub(self.x) < self.width && y.saturating_sub(self.y) < self.height
    }
}

/// Whether `fault` is a fault code a receiver sends (RULE-48): 1 to
/// [`MAX_FAULT`], but never the withdrawn 2.
#[must_use]
pub const fn is_fault(fault: u8) -> bool {
    fault >= 1 && fault <= MAX_FAULT && fault != WITHDRAWN_FAULT
}

/// Checks a heard cue's audio channel and number (RULE-109).
pub(crate) fn check_cue(audio_channel: u8, number: u8) -> Result<(), ProtocolError> {
    if usize::from(audio_channel) >= neoteletext_rules::limits::AUDIO_CHANNELS || number > MAX_CUE {
        return Err(ProtocolError::BadValue("cue"));
    }
    Ok(())
}

/// The least refresh period a `FramePhase` may give, in microseconds
/// (RULE-98 item 1): a display of 500 hertz.
pub const LEAST_REFRESH_MICROSECONDS: u32 = 2_000;
/// The greatest refresh period a `FramePhase` may give, in microseconds
/// (RULE-98 item 1): a display of 10 hertz.
pub const MOST_REFRESH_MICROSECONDS: u32 = 100_000;
/// How far the interface's last `FramePhase` may be from the times it
/// predicts before it sends another (RULE-98 item 1).
pub const PHASE_REPORT_MICROSECONDS: u64 = 500;
/// How near a whole multiple of 60 hertz a display's rate must be for the
/// decoder to lock its ticks to it, in parts a thousand (RULE-98 item 3).
pub const LOCK_TOLERANCE_PARTS_PER_THOUSAND: u64 = 5;
/// How long before a composition a locked tick is due (RULE-98 items 3
/// and 5): a millisecond each for the evaluation and repaint, the frame's
/// way through the trusted core and its way to the interface, and one to
/// spare.
pub const TICK_LEAD_MICROSECONDS: u64 = 4_000;
/// The most a locked clock's start moves in one tick (RULE-98 item 3).
pub const PHASE_SLEW_MICROSECONDS: u64 = 100;
/// The distance from its target under which a clock is locked and not
/// moved (RULE-98 item 3).
pub const PHASE_LOCKED_MICROSECONDS: u64 = 200;
/// How long a decoder keeps a `FramePhase` before its clock runs free
/// (RULE-98 item 4).
pub const PHASE_STALE_MILLISECONDS: u64 = 1_000;

/// The interface's frames (RULE-98 item 1): its display's refresh period
/// and how long it is until its next composition.
///
/// RULE-98 gives the composition as a time on "the machine's monotonic
/// clock, which every process of the browser reads the same"; the standard
/// library reads that clock only as an opaque `Instant`, and no system
/// call library is taken for it (DEC-6659 item 12). So the phase travels
/// relative: each sender measures, on its own monotonic clock at the
/// moment it writes the message, the time until the next composition, and
/// each receiver adds it to its own clock's reading at the moment it reads
/// the message. The time the message spends in the pipe is the error,
/// measured by the timed test (RULE-66).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramePhase {
    /// The refresh period, [`LEAST_REFRESH_MICROSECONDS`] to
    /// [`MOST_REFRESH_MICROSECONDS`].
    pub period_us: u32,
    /// Microseconds from the message's writing to the next composition,
    /// less than `period_us`.
    pub next_in_us: u32,
}

impl FramePhase {
    /// The phase of compositions every `period_us` microseconds, one of
    /// them at `composition_us`, as told at `now_us` (both on the sender's
    /// clock): the time to the first composition not before `now_us`.
    /// `None` for a period out of bounds.
    #[must_use]
    pub fn at(period_us: u32, composition_us: u64, now_us: u64) -> Option<Self> {
        if !(LEAST_REFRESH_MICROSECONDS..=MOST_REFRESH_MICROSECONDS).contains(&period_us) {
            return None;
        }
        let period = u64::from(period_us);
        let next_in = match composition_us.checked_sub(now_us) {
            Some(ahead) => ahead.checked_rem(period)?,
            // A composition in the past: the next one after now.
            None => period.checked_sub(now_us.checked_sub(composition_us)?.checked_rem(period)?)?.checked_rem(period)?,
        };
        Some(Self { period_us, next_in_us: u32::try_from(next_in).ok()? })
    }

    /// The time of the next composition on the receiver's clock, which
    /// read `received_us` as it read the message.
    #[must_use]
    pub const fn composition_after(self, received_us: u64) -> u64 {
        received_us.saturating_add(self.next_in_us as u64)
    }
}

fn check_frame_phase(phase: FramePhase) -> Result<(), ProtocolError> {
    if !(LEAST_REFRESH_MICROSECONDS..=MOST_REFRESH_MICROSECONDS).contains(&phase.period_us) {
        return Err(ProtocolError::BadValue("refresh period"));
    }
    if phase.next_in_us >= phase.period_us {
        return Err(ProtocolError::BadValue("time to the next composition"));
    }
    Ok(())
}

pub(crate) fn push_frame_phase(payload: &mut Vec<u8>, phase: FramePhase) -> Result<(), ProtocolError> {
    check_frame_phase(phase)?;
    payload.extend_from_slice(&phase.period_us.to_be_bytes());
    payload.extend_from_slice(&phase.next_in_us.to_be_bytes());
    Ok(())
}

pub(crate) fn read_frame_phase(fields: &mut Fields<'_>) -> Result<FramePhase, ProtocolError> {
    let phase = FramePhase { period_us: fields.u32()?, next_in_us: fields.u32()? };
    check_frame_phase(phase)?;
    Ok(phase)
}

/// A page as the rule table knows it: a page number and a sub-code
/// together (page-rules.md RULE-112, DEC-6599). Nothing of the rule table
/// is addressed by a page number alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PageSub {
    /// The page number, of a channel's pages.
    pub page: u16,
    /// The sub-code, 0 to `LAST_SUBCODE`: never 3F7F, which names no
    /// sub-page (RULE-112).
    pub sub: u16,
}

impl PageSub {
    /// The page `page` with sub-code `sub`.
    #[must_use]
    pub const fn new(page: u16, sub: u16) -> Self {
        Self { page, sub }
    }
}

pub(crate) fn push_page_sub(payload: &mut Vec<u8>, key: PageSub) -> Result<(), ProtocolError> {
    crate::fields::check_page(key.page)?;
    if key.sub > LAST_SUBCODE {
        return Err(ProtocolError::BadValue("sub-code"));
    }
    payload.extend_from_slice(&key.page.to_be_bytes());
    payload.extend_from_slice(&key.sub.to_be_bytes());
    Ok(())
}

pub(crate) fn read_page_sub(fields: &mut Fields<'_>) -> Result<PageSub, ProtocolError> {
    let page = fields.channel_page()?;
    let sub = fields.u16()?;
    if sub > LAST_SUBCODE {
        return Err(ProtocolError::BadValue("sub-code"));
    }
    Ok(PageSub { page, sub })
}

/// A key the open page's table asks for, with the modifiers it compares
/// (RULE-9, RULE-10): what a `key` or `key_up` trigger names, or a key
/// whose state a value of the table reads (RULE-24). The interface gives
/// a key to the page only when one of these names it, and keeps every
/// other key for the browser's own viewer (REQ-7037).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyAsk {
    /// The key, named as the table names it; `None` for any key.
    pub key: Option<Key>,
    /// The modifier bits compared.
    pub mods_mask: u8,
    /// What they must be: a key is asked for when
    /// `(mods AND mask) = value`.
    pub mods_value: u8,
}

impl KeyAsk {
    /// Any key, whatever the modifiers: the whole keyboard.
    pub const EVERY_KEY: Self = Self { key: None, mods_mask: 0, mods_value: 0 };
}

/// What of the keyboard the open page's table asks for (REQ-7037).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyAsks {
    /// It asks for text that is no key: a paste, a composition and its
    /// committed text (the interest mask's text class, RULE-52 item 1).
    pub text: bool,
    /// The keys it names, at most [`MAX_KEY_ASKS`].
    pub keys: Vec<KeyAsk>,
}

/// What a decoder tells of the open page's table (RULE-52 item 1, RULE-33
/// item 1, RULE-51 item 4, REQ-7037). It is told when a table is decoded for the
/// open page, again when the watch list or the wheel rectangles change,
/// and with `generation` 0 and nothing else when the page has no table or
/// stops being the open page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulesOpen {
    /// The page and sub-code.
    pub page: PageSub,
    /// The table's generation, 1 to 65,535; 0 when the page has no table.
    pub generation: u16,
    /// The classes of input the table asks for: `INTEREST_` bits.
    pub interest: u8,
    /// The metered values the table reads, at most [`MAX_WATCH_SLOTS`].
    pub watch: Vec<WatchSlot>,
    /// Where an enabled `wheel` rule applies, at most [`MAX_WHEEL_RECTS`].
    pub wheel: Vec<WheelRect>,
    /// The keys the table names, at most [`MAX_KEY_ASKS`]; none unless
    /// `interest` has the keys' class (REQ-7037).
    pub keys: Vec<KeyAsk>,
}

/// What a report is of (RULE-22, RULE-52 item 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reported {
    /// The rule fired, with no event.
    Fired,
    /// The input event the trusted core gave under this sequence number.
    Input {
        /// The sequence number.
        sequence: u32,
    },
    /// One value and its reading.
    Value {
        /// The value, as the rule table names it (RULE-44).
        value: [u8; REPORT_VALUE_BYTES],
        /// Its reading.
        reading: i32,
    },
}

/// A report a decoder asks the trusted core to send for a rule's `send` or
/// `echo` (RULE-52 item 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// The page and sub-code.
    pub page: PageSub,
    /// The table's generation, never 0.
    pub generation: u16,
    /// The rule, 0 to 63; `NO_RULE` (255) for a run no rule made (RULE-46).
    pub rule: u8,
    /// The place of the firing row within the rule's rows, 0 to 63;
    /// `NO_RULE` (255) for none (RULE-46).
    pub row: u8,
    /// The page tick at which it fired.
    pub tick: u32,
    /// The kind of the target the pointer was inside, 0 to 3.
    pub target_kind: u8,
    /// Its number.
    pub target_number: u16,
    /// What is reported.
    pub what: Reported,
}

/// The receiver's own counts for the open page (RULE-72), for the console:
/// totals since the page opened, the evaluator's and the host's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleCounters {
    /// The page and sub-code.
    pub page: PageSub,
    /// Ticks evaluated.
    pub ticks: u32,
    /// Rules fired.
    pub fired: u32,
    /// Groups run.
    pub groups: u32,
    /// Commands applied.
    pub commands: u32,
    /// Commands that did nothing for a missing target or sound.
    pub skipped: u32,
    /// Input events dropped from a full queue.
    pub events_dropped: u32,
    /// Events handled (DEC-6635: inputs, the server's records, cues and
    /// state queries).
    pub events_handled: u32,
    /// Events that waited for a later tick's budget.
    pub events_waited: u32,
    /// Sequence starts that came late.
    pub starts_late: u32,
    /// Sequence starts skipped as too far ahead.
    pub starts_skipped: u32,
    /// Cues heard.
    pub cues: u32,
    /// Reports asked for.
    pub reports: u32,
    /// Ticks whose evaluation ran late (RULE-4, fault 9).
    pub late_ticks: u32,
    /// The longest tick's evaluation, in microseconds.
    pub longest_tick_us: u32,
    /// Ticks and paint points whose frame was finished after the
    /// composition it was meant for (RULE-98 item 5).
    pub late_frames: u32,
    /// The sixteen variables, `v0` first.
    pub variables: [i32; VARIABLES],
}

pub(crate) fn push_input(payload: &mut Vec<u8>, input: &RuleInput) {
    match *input {
        RuleInput::Key { action, named, code, base, mods } => {
            let action = match action {
                KeyAction::Press => 0,
                KeyAction::Repeat => 1,
                KeyAction::Release => 2,
            };
            payload.extend_from_slice(&[INPUT_KEY, action, u8::from(named)]);
            payload.extend_from_slice(&code.to_be_bytes());
            payload.extend_from_slice(&base.unwrap_or(NO_BASE).to_be_bytes());
            payload.push(mods);
        }
        RuleInput::Text => payload.push(INPUT_TEXT),
        RuleInput::Pointer { action, button, x, y, control } => {
            let action = match action {
                PointerAction::Press => 0,
                PointerAction::Release => 1,
                PointerAction::Move => 2,
                PointerAction::Gone => 3,
            };
            payload.extend_from_slice(&[INPUT_POINTER, action, button]);
            for field in [x, y, control] {
                payload.extend_from_slice(&field.to_be_bytes());
            }
        }
        RuleInput::Wheel { x, y, control } => {
            payload.push(INPUT_WHEEL);
            for field in [x, y, control] {
                payload.extend_from_slice(&field.to_be_bytes());
            }
        }
        RuleInput::Focus => payload.push(INPUT_FOCUS),
        RuleInput::Blur => payload.push(INPUT_BLUR),
        RuleInput::Drawing { leave, drawing } => {
            payload.extend_from_slice(&[INPUT_DRAWING, u8::from(leave)]);
            payload.extend_from_slice(&drawing.to_be_bytes());
        }
    }
}

fn control(fields: &mut Fields<'_>) -> Result<u16, ProtocolError> {
    let control = fields.u16()?;
    if control > MAX_CONTROL { Err(ProtocolError::BadValue("control")) } else { Ok(control) }
}

pub(crate) fn read_input(fields: &mut Fields<'_>) -> Result<RuleInput, ProtocolError> {
    Ok(match fields.u8()? {
        INPUT_KEY => {
            let action = match fields.u8()? {
                0 => KeyAction::Press,
                1 => KeyAction::Repeat,
                MAX_ACTION => KeyAction::Release,
                _ => return Err(ProtocolError::BadValue("key action")),
            };
            let named = fields.flag("named key")?;
            let code = fields.u32()?;
            let base = Some(fields.u32()?).filter(|base| *base != NO_BASE);
            // A character and a base are Unicode scalar values.
            if (!named && char::from_u32(code).is_none()) || base.is_some_and(|base| char::from_u32(base).is_none()) {
                return Err(ProtocolError::BadValue("key"));
            }
            RuleInput::Key { action, named, code, base, mods: fields.u8()? }
        }
        INPUT_TEXT => RuleInput::Text,
        INPUT_POINTER => {
            let action = match fields.u8()? {
                0 => PointerAction::Press,
                1 => PointerAction::Release,
                2 => PointerAction::Move,
                MAX_POINTER_ACTION => PointerAction::Gone,
                _ => return Err(ProtocolError::BadValue("pointer action")),
            };
            RuleInput::Pointer { action, button: fields.u8()?, x: fields.u16()?, y: fields.u16()?, control: control(fields)? }
        }
        INPUT_WHEEL => RuleInput::Wheel { x: fields.u16()?, y: fields.u16()?, control: control(fields)? },
        INPUT_FOCUS => RuleInput::Focus,
        INPUT_BLUR => RuleInput::Blur,
        INPUT_DRAWING => {
            let leave = fields.flag("drawing leave")?;
            let drawing = fields.u16()?;
            if drawing == 0 {
                return Err(ProtocolError::BadValue("drawing"));
            }
            RuleInput::Drawing { leave, drawing }
        }
        _ => return Err(ProtocolError::BadValue("rule input")),
    })
}

pub(crate) fn push_open(payload: &mut Vec<u8>, open: &RulesOpen) -> Result<(), ProtocolError> {
    push_page_sub(payload, open.page)?;
    payload.extend_from_slice(&open.generation.to_be_bytes());
    payload.push(open.interest);
    crate::push_count_u8(payload, open.watch.len(), MAX_WATCH_SLOTS, "watch list")?;
    for slot in &open.watch {
        payload.extend_from_slice(&[slot.channel, slot.kind, slot.part]);
    }
    crate::push_count_u8(payload, open.wheel.len(), MAX_WHEEL_RECTS, "wheel rectangles")?;
    for rect in &open.wheel {
        for field in [rect.x, rect.y, rect.width, rect.height] {
            payload.extend_from_slice(&field.to_be_bytes());
        }
    }
    push_key_asks(payload, &open.keys)?;
    // What would be refused on arrival is refused on sending.
    read_open(&mut Fields::new(payload)).map(|_| ())
}

/// The byte of a key ask that names no key: any key. A key's kind is 1
/// to 3 (RULE-10).
const ANY_KEY: u8 = 0;

pub(crate) fn push_key_asks(payload: &mut Vec<u8>, asks: &[KeyAsk]) -> Result<(), ProtocolError> {
    crate::push_count_u8(payload, asks.len(), MAX_KEY_ASKS, "key asks")?;
    for ask in asks {
        payload.push(ask.key.map_or(ANY_KEY, |key| key.kind.code()));
        payload.extend_from_slice(&ask.key.map_or(0, |key| key.code).to_be_bytes());
        payload.extend_from_slice(&[ask.mods_mask, ask.mods_value]);
    }
    Ok(())
}

pub(crate) fn read_key_asks(fields: &mut Fields<'_>) -> Result<Vec<KeyAsk>, ProtocolError> {
    let count = fields.count_u8(MAX_KEY_ASKS, "key asks")?;
    let mut asks = Vec::with_capacity(count);
    for _ in 0..count {
        let (kind, code) = (fields.u8()?, fields.u32()?);
        let key = match (kind, KeyKind::from_code(kind)) {
            (ANY_KEY, _) if code == 0 => None,
            // A character or a physical key is named by a Unicode scalar
            // value; a named key by a functional code, which only the
            // input protocol's table knows (RULE-10).
            (_, Some(kind)) if kind == KeyKind::Named || char::from_u32(code).is_some() => Some(Key::of(kind, code)),
            _ => return Err(ProtocolError::BadValue("key ask")),
        };
        let (mods_mask, mods_value) = (fields.u8()?, fields.u8()?);
        // A value with a bit the mask does not compare matches no key.
        if mods_value & !mods_mask != 0 {
            return Err(ProtocolError::BadValue("key ask modifiers"));
        }
        asks.push(KeyAsk { key, mods_mask, mods_value });
    }
    Ok(asks)
}

pub(crate) fn read_open(fields: &mut Fields<'_>) -> Result<RulesOpen, ProtocolError> {
    let page = read_page_sub(fields)?;
    let generation = fields.u16()?;
    let interest = fields.u8()?;
    if interest & !INTEREST_ALL != 0 {
        return Err(ProtocolError::BadValue("interest mask"));
    }
    let count = fields.count_u8(MAX_WATCH_SLOTS, "watch list")?;
    let mut watch = Vec::with_capacity(count);
    for _ in 0..count {
        watch.push(crate::audio::watch_slot(fields)?);
    }
    let count = fields.count_u8(MAX_WHEEL_RECTS, "wheel rectangles")?;
    let mut wheel = Vec::with_capacity(count);
    for _ in 0..count {
        let rect = WheelRect { x: fields.u16()?, y: fields.u16()?, width: fields.u16()?, height: fields.u16()? };
        if rect.width == 0 || rect.height == 0 {
            return Err(ProtocolError::BadValue("wheel rectangle"));
        }
        wheel.push(rect);
    }
    let keys = read_key_asks(fields)?;
    // A page with no table asks for nothing (RULE-50).
    if generation == 0 && (interest != 0 || !watch.is_empty() || !wheel.is_empty() || !keys.is_empty()) {
        return Err(ProtocolError::BadValue("rules of no table"));
    }
    // Keys are named only by a table that asks for the keys' class.
    if interest & INTEREST_KEYS == 0 && !keys.is_empty() {
        return Err(ProtocolError::BadValue("key asks without the keys' class"));
    }
    Ok(RulesOpen { page, generation, interest, watch, wheel, keys })
}

pub(crate) fn push_report(payload: &mut Vec<u8>, report: &Report) -> Result<(), ProtocolError> {
    push_page_sub(payload, report.page)?;
    payload.extend_from_slice(&report.generation.to_be_bytes());
    payload.push(report.rule);
    payload.push(report.row);
    payload.extend_from_slice(&report.tick.to_be_bytes());
    payload.push(report.target_kind);
    payload.extend_from_slice(&report.target_number.to_be_bytes());
    match report.what {
        Reported::Fired => payload.push(REPORT_FIRED),
        Reported::Input { sequence } => {
            payload.push(REPORT_INPUT);
            payload.extend_from_slice(&sequence.to_be_bytes());
        }
        Reported::Value { value, reading } => {
            payload.push(REPORT_VALUE);
            payload.extend_from_slice(&value);
            payload.extend_from_slice(&reading.to_be_bytes());
        }
    }
    read_report(&mut Fields::new(payload)).map(|_| ())
}

pub(crate) fn read_report(fields: &mut Fields<'_>) -> Result<Report, ProtocolError> {
    let page = read_page_sub(fields)?;
    let generation = fields.u16()?;
    let rule = fields.u8()?;
    let row = fields.u8()?;
    let tick = fields.u32()?;
    let target_kind = fields.u8()?;
    let target_number = fields.u16()?;
    // RULE-46: 255 for a run no rule made and for no row; a row only of a
    // rule.
    let ruled = usize::from(rule) < MAX_RULES || rule == NO_RULE;
    let rowed = row == NO_RULE || (usize::from(row) < MAX_ROWS && rule != NO_RULE);
    if generation == 0 || !ruled || !rowed || target_kind > MAX_TARGET_KIND {
        return Err(ProtocolError::BadValue("report"));
    }
    let what = match fields.u8()? {
        REPORT_FIRED => Reported::Fired,
        REPORT_INPUT => Reported::Input { sequence: fields.u32()? },
        REPORT_VALUE => Reported::Value { value: fields.array()?, reading: fields.u32()?.cast_signed() },
        _ => return Err(ProtocolError::BadValue("report kind")),
    };
    // Only an input event has a pointer that was inside something
    // (RULE-46).
    if !matches!(what, Reported::Input { .. }) && (target_kind != 0 || target_number != 0) {
        return Err(ProtocolError::BadValue("report target"));
    }
    Ok(Report { page, generation, rule, row, tick, target_kind, target_number, what })
}

/// A state query's fields: its page and sub-code and its identifier
/// (RULE-47).
pub(crate) fn push_query(payload: &mut Vec<u8>, page: PageSub, identifier: u32) -> Result<(), ProtocolError> {
    push_page_sub(payload, page)?;
    payload.extend_from_slice(&identifier.to_be_bytes());
    Ok(())
}

/// The unsigned figures of [`RuleCounters`] on the pipe, in the order
/// [`push_counters`] writes them, before the variables.
const COUNTER_FIGURES: usize = 15;

pub(crate) fn push_counters(payload: &mut Vec<u8>, counters: &RuleCounters) -> Result<(), ProtocolError> {
    push_page_sub(payload, counters.page)?;
    for figure in [
        counters.ticks,
        counters.fired,
        counters.groups,
        counters.commands,
        counters.skipped,
        counters.events_dropped,
        counters.events_handled,
        counters.events_waited,
        counters.starts_late,
        counters.starts_skipped,
        counters.cues,
        counters.reports,
        counters.late_ticks,
        counters.longest_tick_us,
        counters.late_frames,
    ] {
        payload.extend_from_slice(&figure.to_be_bytes());
    }
    for variable in counters.variables {
        payload.extend_from_slice(&variable.to_be_bytes());
    }
    Ok(())
}

pub(crate) fn read_counters(fields: &mut Fields<'_>) -> Result<RuleCounters, ProtocolError> {
    let page = read_page_sub(fields)?;
    let mut figures = [0_u32; COUNTER_FIGURES];
    for figure in &mut figures {
        *figure = fields.u32()?;
    }
    let [
        ticks,
        fired,
        groups,
        commands,
        skipped,
        events_dropped,
        events_handled,
        events_waited,
        starts_late,
        starts_skipped,
        cues,
        reports,
        late_ticks,
        longest_tick_us,
        late_frames,
    ] = figures;
    let mut variables = [0_i32; VARIABLES];
    for variable in &mut variables {
        *variable = fields.u32()?.cast_signed();
    }
    Ok(RuleCounters {
        page,
        ticks,
        fired,
        groups,
        commands,
        skipped,
        events_dropped,
        events_handled,
        events_waited,
        starts_late,
        starts_skipped,
        cues,
        reports,
        late_ticks,
        longest_tick_us,
        late_frames,
        variables,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]
mod tests {
    use super::*;
    use crate::decoder::{FromDecoder, ToDecoder, read_from_core, read_from_decoder, write_to_core, write_to_decoder};
    use crate::fetcher::{FromFetcher, read_from_fetcher, write_to_core as fetcher_writes};
    use crate::sound::{FromSound, read_from_sound, write_from_sound};

    fn to_decoder(message: &ToDecoder) -> Result<Option<ToDecoder>, ProtocolError> {
        let mut bytes = Vec::new();
        write_to_decoder(&mut bytes, message)?;
        read_from_core(&mut bytes.as_slice())
    }

    fn from_decoder(message: &FromDecoder) -> Result<Option<FromDecoder>, ProtocolError> {
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, message)?;
        read_from_decoder(&mut bytes.as_slice())
    }

    fn every_input() -> Vec<RuleInput> {
        vec![
            RuleInput::Key { action: KeyAction::Press, named: false, code: u32::from('a'), base: None, mods: 1 },
            RuleInput::Key { action: KeyAction::Repeat, named: true, code: 57_350, base: Some(u32::from('q')), mods: 0 },
            RuleInput::Key { action: KeyAction::Release, named: false, code: 0x1F600, base: None, mods: 255 },
            RuleInput::Text,
            RuleInput::Pointer { action: PointerAction::Press, button: 2, x: 799, y: 479, control: 32 },
            RuleInput::Pointer { action: PointerAction::Release, button: 0, x: 0, y: 0, control: 0 },
            RuleInput::Pointer { action: PointerAction::Move, button: 5, x: 1, y: 2, control: 3 },
            RuleInput::Pointer { action: PointerAction::Gone, button: 0, x: 9, y: 9, control: 0 },
            RuleInput::Wheel { x: 10, y: 20, control: 1 },
            RuleInput::Focus,
            RuleInput::Blur,
            RuleInput::Drawing { leave: false, drawing: 101 },
            RuleInput::Drawing { leave: true, drawing: 65_535 },
        ]
    }

    /// RULE-63 (REQ-6534): each message between the trusted core and a
    /// decoder round trips at its bounds.
    #[test]
    fn the_rule_messages_round_trip() {
        for input in every_input() {
            let message = ToDecoder::RuleInput { page: PageSub::new(0x101, 1), sequence: u32::MAX, input };
            assert_eq!(to_decoder(&message).unwrap(), Some(message));
        }
        for values in [vec![], vec![7], vec![255; MAX_WATCH_SLOTS]] {
            let message = ToDecoder::Metered { page: PageSub::new(0x899, 1), values };
            assert_eq!(to_decoder(&message).unwrap(), Some(message));
        }
        let query = ToDecoder::RulesQuery { page: PageSub::new(0x100, 1), identifier: 0x0102_0304 };
        assert_eq!(to_decoder(&query).unwrap(), Some(query));
        for page in [None, Some(PageSub::new(0x101, 0)), Some(PageSub::new(0x101, LAST_SUBCODE))] {
            assert_eq!(to_decoder(&ToDecoder::RulesPage(page)).unwrap(), Some(ToDecoder::RulesPage(page)));
        }
        assert!(to_decoder(&ToDecoder::RulesPage(Some(PageSub::new(0x0FF, 0)))).is_err(), "no page of a channel");
        assert!(to_decoder(&ToDecoder::RulesPage(Some(PageSub::new(0x101, 0x3F7F)))).is_err(), "3F7F names no sub-page (RULE-112)");
        for message in [
            ToDecoder::ConnectionEnded,
            ToDecoder::CueHeard { audio_channel: 3, number: MAX_CUE },
            ToDecoder::FramePhase(FramePhase { period_us: LEAST_REFRESH_MICROSECONDS, next_in_us: LEAST_REFRESH_MICROSECONDS - 1 }),
            ToDecoder::FramePhase(FramePhase { period_us: MOST_REFRESH_MICROSECONDS, next_in_us: 0 }),
        ] {
            assert_eq!(to_decoder(&message).unwrap(), Some(message));
        }
        assert!(to_decoder(&ToDecoder::CueHeard { audio_channel: 4, number: 0 }).is_err());
        assert!(to_decoder(&ToDecoder::CueHeard { audio_channel: 0, number: 128 }).is_err());
        assert!(to_decoder(&ToDecoder::FramePhase(FramePhase { period_us: LEAST_REFRESH_MICROSECONDS - 1, next_in_us: 0 })).is_err());
        assert!(to_decoder(&ToDecoder::FramePhase(FramePhase { period_us: MOST_REFRESH_MICROSECONDS + 1, next_in_us: 0 })).is_err());
        assert!(to_decoder(&ToDecoder::FramePhase(FramePhase { period_us: 16_667, next_in_us: 16_667 })).is_err(), "the next composition is within one period");
        // RULE-98: a phase told relative to the sender's clock.
        assert_eq!(FramePhase::at(16_667, 1_000_000, 990_000), Some(FramePhase { period_us: 16_667, next_in_us: 10_000 }));
        assert_eq!(FramePhase::at(16_667, 1_000_000, 1_020_000), Some(FramePhase { period_us: 16_667, next_in_us: 13_334 }));
        assert_eq!(FramePhase::at(16_667, 1_000_000, 1_000_000 + 2 * 16_667), Some(FramePhase { period_us: 16_667, next_in_us: 0 }));
        assert_eq!(FramePhase::at(1_000, 0, 0), None);
        assert_eq!(FramePhase { period_us: 16_667, next_in_us: 500 }.composition_after(2_000), 2_500);

        let full = RulesOpen {
            page: PageSub::new(0x101, 1),
            generation: 65_535,
            interest: INTEREST_ALL,
            watch: (0..MAX_WATCH_SLOTS).map(|slot| WatchSlot { channel: 3, kind: 2, part: u8::try_from(slot % 16).unwrap() }).collect(),
            wheel: vec![WheelRect { x: 0, y: 0, width: 800, height: 480 }; MAX_WHEEL_RECTS],
            keys: vec![KeyAsk { key: Some(Key::of(KeyKind::Physical, 'q' as u32)), mods_mask: 1, mods_value: 1 }; MAX_KEY_ASKS],
        };
        let none = RulesOpen { page: PageSub::new(0x101, 1), generation: 0, interest: 0, watch: vec![], wheel: vec![], keys: vec![] };
        for open in [full, none] {
            assert_eq!(from_decoder(&FromDecoder::RulesOpen(open.clone())).unwrap(), Some(FromDecoder::RulesOpen(open)));
        }
        for what in [Reported::Fired, Reported::Input { sequence: 9 }, Reported::Value { value: [5, 0, 0, 1, 0, 0, 0, 0], reading: -3 }] {
            let target = if matches!(what, Reported::Input { .. }) { (2, 101) } else { (0, 0) };
            let report =
                Report { page: PageSub::new(0x101, 1), generation: 1, rule: 63, row: 63, tick: u32::MAX, target_kind: target.0, target_number: target.1, what };
            assert_eq!(from_decoder(&FromDecoder::Report(report)).unwrap(), Some(FromDecoder::Report(report)));
        }
        for body in [vec![0; 10], vec![1; MAX_PAGE_STATE]] {
            let state = FromDecoder::PageState { page: PageSub::new(0x101, 1), identifier: 77, body };
            assert_eq!(from_decoder(&state).unwrap(), Some(state));
        }
        for fault in (1..=MAX_FAULT).filter(|fault| *fault != WITHDRAWN_FAULT) {
            let message = FromDecoder::RulesFault { page: PageSub::new(0x101, 1), generation: 3, fault, count: 12 };
            assert_eq!(from_decoder(&message).unwrap(), Some(message));
        }
        let counters = FromDecoder::RuleCounters(RuleCounters {
            page: PageSub::new(0x101, 1),
            ticks: 1,
            fired: 2,
            groups: 3,
            commands: 4,
            skipped: 5,
            events_dropped: 6,
            events_handled: 7,
            events_waited: 8,
            starts_late: 9,
            starts_skipped: 10,
            cues: 11,
            reports: 12,
            late_ticks: 13,
            longest_tick_us: u32::MAX,
            late_frames: 7,
            variables: [i32::MIN; VARIABLES],
        });
        assert_eq!(from_decoder(&counters).unwrap(), Some(counters));
    }

    /// RULE-63, RULE-52 (REQ-6534): what is out of bounds is refused by
    /// the writer and by the reader.
    #[test]
    fn rule_messages_out_of_bounds_are_refused() {
        assert!(
            to_decoder(&ToDecoder::RuleInput { page: PageSub::new(0x101, 1), sequence: 1, input: RuleInput::Drawing { leave: false, drawing: 0 } }).is_err()
        );
        let control = RuleInput::Pointer { action: PointerAction::Move, button: 0, x: 0, y: 0, control: 33 };
        assert!(to_decoder(&ToDecoder::RuleInput { page: PageSub::new(0x101, 1), sequence: 1, input: control }).is_err(), "a control past the engine's most");
        let surrogate = RuleInput::Key { action: KeyAction::Press, named: false, code: 0xD800, base: None, mods: 0 };
        assert!(to_decoder(&ToDecoder::RuleInput { page: PageSub::new(0x101, 1), sequence: 1, input: surrogate }).is_err());
        assert!(to_decoder(&ToDecoder::Metered { page: PageSub::new(0x101, 1), values: vec![0; MAX_WATCH_SLOTS + 1] }).is_err());

        let open = |change: &dyn Fn(&mut RulesOpen)| {
            let mut open = RulesOpen { page: PageSub::new(0x101, 1), generation: 1, interest: 1, watch: vec![], wheel: vec![], keys: vec![] };
            change(&mut open);
            from_decoder(&FromDecoder::RulesOpen(open)).is_err()
        };
        assert!(!open(&|_| {}));
        assert!(open(&|open| open.interest = 0x40), "a bit no class has");
        assert!(open(&|open| open.watch = vec![WatchSlot { channel: 0, kind: 1, part: 0 }; MAX_WATCH_SLOTS + 1]));
        assert!(open(&|open| open.watch = vec![WatchSlot { channel: 4, kind: 1, part: 0 }]), "an audio channel past the last");
        assert!(open(&|open| open.watch = vec![WatchSlot { channel: 0, kind: 5, part: 0 }]), "a kind no slot has");
        assert!(open(&|open| open.watch = vec![WatchSlot { channel: 0, kind: 2, part: 16 }]), "a part past the last");
        assert!(open(&|open| open.wheel = vec![WheelRect { x: 0, y: 0, width: 1, height: 1 }; MAX_WHEEL_RECTS + 1]));
        assert!(open(&|open| open.wheel = vec![WheelRect { x: 0, y: 0, width: 0, height: 1 }]), "an empty rectangle");
        assert!(open(&|open| open.generation = 0), "no table asks for nothing");

        let report = |change: &dyn Fn(&mut Report)| {
            let mut report =
                Report { page: PageSub::new(0x101, 1), generation: 1, rule: 0, row: NO_RULE, tick: 0, target_kind: 0, target_number: 0, what: Reported::Fired };
            change(&mut report);
            from_decoder(&FromDecoder::Report(report)).is_err()
        };
        assert!(!report(&|_| {}));
        assert!(report(&|report| report.rule = 64));
        assert!(!report(&|report| report.rule = NO_RULE), "a run no rule made");
        assert!(report(&|report| report.row = 64));
        assert!(report(&|report| (report.rule, report.row) = (NO_RULE, 0)), "a row only of a rule");
        assert!(report(&|report| report.generation = 0));
        assert!(report(&|report| report.target_kind = 4));
        assert!(report(&|report| report.target_number = 1), "a firing with no event names no target");
        assert!(from_decoder(&FromDecoder::PageState { page: PageSub::new(0x101, 1), identifier: 1, body: vec![0; MAX_PAGE_STATE + 1] }).is_err());
        assert!(from_decoder(&FromDecoder::RulesFault { page: PageSub::new(0x101, 1), generation: 1, fault: 0, count: 1 }).is_err());
        assert!(from_decoder(&FromDecoder::RulesFault { page: PageSub::new(0x101, 1), generation: 1, fault: MAX_FAULT + 1, count: 1 }).is_err());
        assert!(
            from_decoder(&FromDecoder::RulesFault { page: PageSub::new(0x101, 1), generation: 1, fault: WITHDRAWN_FAULT, count: 1 }).is_err(),
            "code 2 is withdrawn"
        );
    }

    /// RULE-63 (REQ-6534): each side accepts only the kinds meant for it:
    /// a decoder cannot give itself input or a metered report, and the
    /// trusted core's reader of a decoder refuses them.
    #[test]
    fn a_decoder_cannot_send_what_only_the_core_sends() {
        let mut bytes = Vec::new();
        write_to_decoder(&mut bytes, &ToDecoder::RuleInput { page: PageSub::new(0x101, 1), sequence: 1, input: RuleInput::Focus }).unwrap();
        assert!(read_from_decoder(&mut bytes.as_slice()).is_err());
        let mut bytes = Vec::new();
        write_to_decoder(&mut bytes, &ToDecoder::Metered { page: PageSub::new(0x101, 1), values: vec![1] }).unwrap();
        assert!(read_from_decoder(&mut bytes.as_slice()).is_err());
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &FromDecoder::RulesFault { page: PageSub::new(0x101, 1), generation: 1, fault: 1, count: 1 }).unwrap();
        assert!(read_from_core(&mut bytes.as_slice()).is_err());
    }

    /// RULE-33 item 3, RULE-47 (REQ-6534, REQ-6535): the sound helper's
    /// metered report and the fetcher's query round trip, and a report
    /// over the watch list's bound is refused.
    #[test]
    fn metered_and_query_round_trip() {
        for values in [vec![], vec![0, 128, 255], vec![9; MAX_WATCH_SLOTS]] {
            let message = FromSound::Metered { channel: 7, values };
            let mut bytes = Vec::new();
            write_from_sound(&mut bytes, &message).unwrap();
            assert_eq!(read_from_sound(&mut bytes.as_slice()).unwrap(), Some(message));
        }
        assert!(write_from_sound(&mut Vec::new(), &FromSound::Metered { channel: 7, values: vec![0; MAX_WATCH_SLOTS + 1] }).is_err());
        let mut bytes = Vec::new();
        write_from_sound(&mut bytes, &FromSound::LocalOverrun { channel: 9, count: 0 }).unwrap();
        assert_eq!(read_from_sound(&mut bytes.as_slice()).unwrap(), Some(FromSound::LocalOverrun { channel: 9, count: 0 }));
        let heard = FromSound::CueHeard { channel: 9, audio_channel: 2, number: 127 };
        let mut bytes = Vec::new();
        write_from_sound(&mut bytes, &heard).unwrap();
        assert_eq!(read_from_sound(&mut bytes.as_slice()).unwrap(), Some(heard));
        assert!(write_from_sound(&mut Vec::new(), &FromSound::CueHeard { channel: 9, audio_channel: 4, number: 0 }).is_err());
        let spent = FromSound::Spent { channel: 9, microseconds: u32::MAX };
        let mut bytes = Vec::new();
        write_from_sound(&mut bytes, &spent).unwrap();
        assert_eq!(read_from_sound(&mut bytes.as_slice()).unwrap(), Some(spent));
        let query = FromFetcher::RulesQuery { page: PageSub::new(0x101, 1), identifier: u32::MAX };
        let mut bytes = Vec::new();
        fetcher_writes(&mut bytes, &query).unwrap();
        assert_eq!(read_from_fetcher(&mut bytes.as_slice()).unwrap(), Some(query));
    }

    /// RULE-52 item 1: each input belongs to one interest class.
    #[test]
    fn each_input_has_its_interest_class() {
        let classes: Vec<u8> = every_input().iter().map(RuleInput::interest).collect();
        assert_eq!(
            classes,
            [
                INTEREST_KEYS,
                INTEREST_KEYS,
                INTEREST_KEYS,
                INTEREST_TEXT,
                INTEREST_POINTER_BUTTONS,
                INTEREST_POINTER_BUTTONS,
                INTEREST_MOVES,
                INTEREST_MOVES,
                INTEREST_WHEEL,
                INTEREST_FOCUS,
                INTEREST_FOCUS,
                INTEREST_MOVES,
                INTEREST_MOVES
            ]
        );
        assert_eq!(INTEREST_KEYS | INTEREST_TEXT | INTEREST_POINTER_BUTTONS | INTEREST_MOVES | INTEREST_WHEEL | INTEREST_FOCUS, INTEREST_ALL);
    }

    /// REQ-7037 (TODO-7158): the keys a table names, at their bound and of
    /// every form, cross from the decoder whole.
    #[allow(clippy::arithmetic_side_effects, reason = "a test's offsets into a payload")]
    fn every_form_of_key_ask() -> Vec<KeyAsk> {
        let mut asks = vec![
            KeyAsk::EVERY_KEY,
            KeyAsk { key: None, mods_mask: 3, mods_value: 2 },
            KeyAsk { key: Some(Key::of(KeyKind::Named, u32::MAX)), mods_mask: 0, mods_value: 0 },
            KeyAsk { key: Some(Key::of(KeyKind::Physical, u32::from('q'))), mods_mask: 0xFF, mods_value: 0x81 },
        ];
        let characters = (u32::from('A')..).take(MAX_KEY_ASKS - asks.len());
        asks.extend(characters.map(|code| KeyAsk { key: Some(Key::of(KeyKind::Character, code)), mods_mask: 0, mods_value: 0 }));
        asks
    }

    /// A table asking for the keys' class and naming `keys`.
    fn naming(keys: Vec<KeyAsk>) -> RulesOpen {
        RulesOpen { page: PageSub::new(0x101, 1), generation: 1, interest: INTEREST_KEYS, watch: vec![], wheel: vec![], keys }
    }

    /// The payload of `open`, which its writer has checked.
    fn open_payload(open: &RulesOpen) -> Vec<u8> {
        let mut payload = Vec::new();
        push_open(&mut payload, open).unwrap();
        payload
    }

    /// The bytes of one key ask on the wire: its kind, its 32-bit code,
    /// and the modifier mask and value.
    const ASK_BYTES: usize = 7;

    // REQ-7037 item 1 (c): exactly `MAX_KEY_ASKS` distinct keys of every
    // form round-trip in `RulesOpen`.
    #[test]
    fn key_asks_round_trip_in_rules_open_at_their_bound() {
        let open = naming(every_form_of_key_ask());
        assert_eq!(open.keys.len(), MAX_KEY_ASKS);
        assert_eq!(read_open(&mut Fields::new(&open_payload(&open))).unwrap(), open);
        assert_eq!(from_decoder(&FromDecoder::RulesOpen(open.clone())).unwrap(), Some(FromDecoder::RulesOpen(open)));
    }

    // REQ-7037 item 1: each malformed form of the list is refused, on
    // sending where the writer can see it and on arrival in any case.
    #[test]
    #[allow(clippy::arithmetic_side_effects, reason = "a test's offsets into a payload")]
    fn malformed_key_asks_are_refused() {
        let one = |ask: KeyAsk| naming(vec![ask]);
        let character = |code: u32| KeyAsk { key: Some(Key::of(KeyKind::Character, code)), mods_mask: 0, mods_value: 0 };
        // Refused by the writer, which reads what it wrote.
        assert!(push_open(&mut Vec::new(), &naming(vec![character(u32::from('a')); MAX_KEY_ASKS + 1])).is_err(), "one more than the bound");
        assert!(push_open(&mut Vec::new(), &one(character(0xD800))).is_err(), "a character that is no Unicode scalar value");
        assert!(
            push_open(&mut Vec::new(), &one(KeyAsk { key: Some(Key::of(KeyKind::Physical, 0x11_0000)), ..character(0) })).is_err(),
            "a physical key past Unicode"
        );
        assert!(
            push_open(&mut Vec::new(), &one(KeyAsk { mods_mask: 1, mods_value: 2, ..character(u32::from('a')) })).is_err(),
            "a modifier value bit outside its mask"
        );
        assert!(push_open(&mut Vec::new(), &RulesOpen { interest: INTEREST_TEXT, ..one(character(u32::from('a'))) }).is_err(), "keys without the keys' class");
        assert!(push_open(&mut Vec::new(), &RulesOpen { generation: 0, interest: 0, ..one(character(u32::from('a'))) }).is_err(), "keys of no table");

        // Refused on arrival: the bytes of a good list, changed.
        let good = open_payload(&one(character(u32::from('a'))));
        let ask_at = good.len() - ASK_BYTES;
        let changed = |at: usize, byte: u8| {
            let mut bytes = good.clone();
            bytes[at] = byte;
            read_open(&mut Fields::new(&bytes))
        };
        assert!(changed(ask_at, 4).is_err(), "a kind above 3");
        assert!(changed(ask_at, 0xFF).is_err(), "a kind of 255");
        // Kind 0, any key, with the code of `a` still in place.
        assert!(changed(ask_at, 0).is_err(), "any key with a code");
        assert!(changed(ask_at + 1, 0xFF).is_err(), "a character code past Unicode");
        assert!(changed(ask_at + 6, 1).is_err(), "a modifier value bit outside its mask of 0");
        assert!(changed(ask_at - 1, 2).is_err(), "a count past the asks given");
        // Kind 0 with code 0 is any key, and reads.
        let mut any = good.clone();
        any[ask_at..ask_at + 5].copy_from_slice(&[0; 5]);
        assert_eq!(read_open(&mut Fields::new(&any)).unwrap().keys, [KeyAsk::EVERY_KEY]);

        // 129 entries: the bound's list with one more ask and its count raised.
        let full = open_payload(&naming(every_form_of_key_ask()));
        let count_at = full.len() - MAX_KEY_ASKS * ASK_BYTES - 1;
        assert_eq!(usize::from(full[count_at]), MAX_KEY_ASKS);
        let mut over = full.clone();
        over[count_at] = u8::try_from(MAX_KEY_ASKS + 1).unwrap();
        over.extend_from_slice(&full[full.len() - ASK_BYTES..]);
        assert!(read_open(&mut Fields::new(&over)).is_err(), "129 entries");
    }
}
