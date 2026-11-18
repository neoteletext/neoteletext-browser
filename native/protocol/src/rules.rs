//! A page's rule table between the processes (neoteletext-common's
//! `docs/page-rules.md`, RULE-52, RULE-58 and RULE-63): the decoder of a
//! page's channel decodes and evaluates the table; the trusted core is the
//! gate. These are the payloads they exchange: the viewer's input given to
//! the decoder with a sequence number, the metered values, what the
//! decoder tells of a table (its interest mask, watch list and wheel
//! rectangles), the reports it asks for, a state query and its answer, a
//! fault and the counters.
//!
//! The decoder never supplies an input event's bytes (RULE-52 item 3): a
//! report names a sequence number, and the trusted core builds the
//! `RULE_EVENT` from its own kept copy.

use neoteletext_rules::limits::{MAX_CONTROL, MAX_RECTS, MAX_RULES, MAX_STATE_BYTES};
use neoteletext_rules::table::VALUE_BYTES;

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
/// The most bytes of a state answer after its stamp (RULE-47): the rule
/// table's `MAX_STATE_BYTES`, which bounds the answer with its stamp.
pub const MAX_PAGE_STATE: usize = MAX_STATE_BYTES;
/// The bytes of a value a report carries (RULE-44).
pub const REPORT_VALUE_BYTES: usize = VALUE_BYTES;
/// The highest fault code (RULE-48).
pub const MAX_FAULT: u8 = 8;
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

/// What a decoder tells of the open page's table (RULE-52 item 1, RULE-33
/// item 1, RULE-51 item 4). It is told when a table is decoded for the
/// open page, again when the watch list or the wheel rectangles change,
/// and with `generation` 0 and nothing else when the page has no table or
/// stops being the open page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulesOpen {
    /// The page.
    pub page: u16,
    /// The table's generation, 1 to 65,535; 0 when the page has no table.
    pub generation: u16,
    /// The classes of input the table asks for: `INTEREST_` bits.
    pub interest: u8,
    /// The metered values the table reads, at most [`MAX_WATCH_SLOTS`].
    pub watch: Vec<WatchSlot>,
    /// Where an enabled `wheel` rule applies, at most [`MAX_WHEEL_RECTS`].
    pub wheel: Vec<WheelRect>,
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
    /// The page.
    pub page: u16,
    /// The table's generation, never 0.
    pub generation: u16,
    /// The rule, 0 to 63.
    pub rule: u8,
    /// The page tick at which it fired.
    pub tick: u32,
    /// The kind of the target the pointer was inside, 0 to 3.
    pub target_kind: u8,
    /// Its number.
    pub target_number: u16,
    /// What is reported.
    pub what: Reported,
}

/// The receiver's own counts for the open page (RULE-72), for the console.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuleCounters {
    /// The page.
    pub page: u16,
    /// Ticks evaluated.
    pub ticks: u32,
    /// Rules fired.
    pub fired: u32,
    /// Actions run.
    pub actions: u32,
    /// Actions skipped for a missing target or sound.
    pub skipped: u32,
    /// Input events dropped from a full queue.
    pub events_dropped: u32,
    /// Reports asked for.
    pub sends: u32,
    /// Rule firings dropped by the cap on a tick's actions.
    pub overruns: u32,
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
    crate::fields::check_page(open.page)?;
    payload.extend_from_slice(&open.page.to_be_bytes());
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
    // What would be refused on arrival is refused on sending.
    read_open(&mut Fields::new(payload)).map(|_| ())
}

pub(crate) fn read_open(fields: &mut Fields<'_>) -> Result<RulesOpen, ProtocolError> {
    let page = fields.channel_page()?;
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
    // A page with no table asks for nothing (RULE-50).
    if generation == 0 && (interest != 0 || !watch.is_empty() || !wheel.is_empty()) {
        return Err(ProtocolError::BadValue("rules of no table"));
    }
    Ok(RulesOpen { page, generation, interest, watch, wheel })
}

pub(crate) fn push_report(payload: &mut Vec<u8>, report: &Report) -> Result<(), ProtocolError> {
    crate::fields::check_page(report.page)?;
    payload.extend_from_slice(&report.page.to_be_bytes());
    payload.extend_from_slice(&report.generation.to_be_bytes());
    payload.push(report.rule);
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
    let page = fields.channel_page()?;
    let generation = fields.u16()?;
    let rule = fields.u8()?;
    let tick = fields.u32()?;
    let target_kind = fields.u8()?;
    let target_number = fields.u16()?;
    if generation == 0 || usize::from(rule) >= MAX_RULES || target_kind > MAX_TARGET_KIND {
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
    Ok(Report { page, generation, rule, tick, target_kind, target_number, what })
}

/// A state query's fields: its page and its identifier (RULE-47).
pub(crate) fn push_query(payload: &mut Vec<u8>, page: u16, identifier: u32) -> Result<(), ProtocolError> {
    crate::fields::check_page(page)?;
    payload.extend_from_slice(&page.to_be_bytes());
    payload.extend_from_slice(&identifier.to_be_bytes());
    Ok(())
}

pub(crate) fn push_counters(payload: &mut Vec<u8>, counters: &RuleCounters) -> Result<(), ProtocolError> {
    crate::fields::check_page(counters.page)?;
    payload.extend_from_slice(&counters.page.to_be_bytes());
    for figure in [counters.ticks, counters.fired, counters.actions, counters.skipped, counters.events_dropped, counters.sends, counters.overruns] {
        payload.extend_from_slice(&figure.to_be_bytes());
    }
    Ok(())
}

pub(crate) fn read_counters(fields: &mut Fields<'_>) -> Result<RuleCounters, ProtocolError> {
    Ok(RuleCounters {
        page: fields.channel_page()?,
        ticks: fields.u32()?,
        fired: fields.u32()?,
        actions: fields.u32()?,
        skipped: fields.u32()?,
        events_dropped: fields.u32()?,
        sends: fields.u32()?,
        overruns: fields.u32()?,
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
            let message = ToDecoder::RuleInput { page: 0x101, sequence: u32::MAX, input };
            assert_eq!(to_decoder(&message).unwrap(), Some(message));
        }
        for values in [vec![], vec![7], vec![255; MAX_WATCH_SLOTS]] {
            let message = ToDecoder::Metered { page: 0x899, values };
            assert_eq!(to_decoder(&message).unwrap(), Some(message));
        }
        let query = ToDecoder::RulesQuery { page: 0x100, identifier: 0x0102_0304 };
        assert_eq!(to_decoder(&query).unwrap(), Some(query));
        for page in [None, Some(0x101)] {
            assert_eq!(to_decoder(&ToDecoder::RulesPage(page)).unwrap(), Some(ToDecoder::RulesPage(page)));
        }
        assert!(to_decoder(&ToDecoder::RulesPage(Some(0x0FF))).is_err(), "no page of a channel");

        let full = RulesOpen {
            page: 0x101,
            generation: 65_535,
            interest: INTEREST_ALL,
            watch: (0..MAX_WATCH_SLOTS).map(|slot| WatchSlot { channel: 3, kind: 2, part: u8::try_from(slot % 16).unwrap() }).collect(),
            wheel: vec![WheelRect { x: 0, y: 0, width: 800, height: 480 }; MAX_WHEEL_RECTS],
        };
        let none = RulesOpen { page: 0x101, generation: 0, interest: 0, watch: vec![], wheel: vec![] };
        for open in [full, none] {
            assert_eq!(from_decoder(&FromDecoder::RulesOpen(open.clone())).unwrap(), Some(FromDecoder::RulesOpen(open)));
        }
        for what in [Reported::Fired, Reported::Input { sequence: 9 }, Reported::Value { value: [5, 0, 0, 1, 0, 0, 0, 0], reading: -3 }] {
            let target = if matches!(what, Reported::Input { .. }) { (2, 101) } else { (0, 0) };
            let report = Report { page: 0x101, generation: 1, rule: 63, tick: u32::MAX, target_kind: target.0, target_number: target.1, what };
            assert_eq!(from_decoder(&FromDecoder::Report(report)).unwrap(), Some(FromDecoder::Report(report)));
        }
        for body in [vec![0; 10], vec![1; MAX_PAGE_STATE]] {
            let state = FromDecoder::PageState { page: 0x101, identifier: 77, body };
            assert_eq!(from_decoder(&state).unwrap(), Some(state));
        }
        for fault in 1..=MAX_FAULT {
            let message = FromDecoder::RulesFault { page: 0x101, generation: 3, fault, count: 12 };
            assert_eq!(from_decoder(&message).unwrap(), Some(message));
        }
        let counters =
            FromDecoder::RuleCounters(RuleCounters { page: 0x101, ticks: 1, fired: 2, actions: 3, skipped: 4, events_dropped: 5, sends: 6, overruns: 7 });
        assert_eq!(from_decoder(&counters).unwrap(), Some(counters));
    }

    /// RULE-63, RULE-52 (REQ-6534): what is out of bounds is refused by
    /// the writer and by the reader.
    #[test]
    fn rule_messages_out_of_bounds_are_refused() {
        assert!(to_decoder(&ToDecoder::RuleInput { page: 0x101, sequence: 1, input: RuleInput::Drawing { leave: false, drawing: 0 } }).is_err());
        let control = RuleInput::Pointer { action: PointerAction::Move, button: 0, x: 0, y: 0, control: 33 };
        assert!(to_decoder(&ToDecoder::RuleInput { page: 0x101, sequence: 1, input: control }).is_err(), "a control past the engine's most");
        let surrogate = RuleInput::Key { action: KeyAction::Press, named: false, code: 0xD800, base: None, mods: 0 };
        assert!(to_decoder(&ToDecoder::RuleInput { page: 0x101, sequence: 1, input: surrogate }).is_err());
        assert!(to_decoder(&ToDecoder::Metered { page: 0x101, values: vec![0; MAX_WATCH_SLOTS + 1] }).is_err());

        let open = |change: &dyn Fn(&mut RulesOpen)| {
            let mut open = RulesOpen { page: 0x101, generation: 1, interest: 1, watch: vec![], wheel: vec![] };
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
            let mut report = Report { page: 0x101, generation: 1, rule: 0, tick: 0, target_kind: 0, target_number: 0, what: Reported::Fired };
            change(&mut report);
            from_decoder(&FromDecoder::Report(report)).is_err()
        };
        assert!(!report(&|_| {}));
        assert!(report(&|report| report.rule = 64));
        assert!(report(&|report| report.generation = 0));
        assert!(report(&|report| report.target_kind = 4));
        assert!(report(&|report| report.target_number = 1), "a firing with no event names no target");
        assert!(from_decoder(&FromDecoder::PageState { page: 0x101, identifier: 1, body: vec![0; MAX_PAGE_STATE + 1] }).is_err());
        assert!(from_decoder(&FromDecoder::RulesFault { page: 0x101, generation: 1, fault: 0, count: 1 }).is_err());
        assert!(from_decoder(&FromDecoder::RulesFault { page: 0x101, generation: 1, fault: 9, count: 1 }).is_err());
    }

    /// RULE-63 (REQ-6534): each side accepts only the kinds meant for it:
    /// a decoder cannot give itself input or a metered report, and the
    /// trusted core's reader of a decoder refuses them.
    #[test]
    fn a_decoder_cannot_send_what_only_the_core_sends() {
        let mut bytes = Vec::new();
        write_to_decoder(&mut bytes, &ToDecoder::RuleInput { page: 0x101, sequence: 1, input: RuleInput::Focus }).unwrap();
        assert!(read_from_decoder(&mut bytes.as_slice()).is_err());
        let mut bytes = Vec::new();
        write_to_decoder(&mut bytes, &ToDecoder::Metered { page: 0x101, values: vec![1] }).unwrap();
        assert!(read_from_decoder(&mut bytes.as_slice()).is_err());
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &FromDecoder::RulesFault { page: 0x101, generation: 1, fault: 1, count: 1 }).unwrap();
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
        write_from_sound(&mut bytes, &FromSound::LocalOverrun { channel: 9 }).unwrap();
        assert_eq!(read_from_sound(&mut bytes.as_slice()).unwrap(), Some(FromSound::LocalOverrun { channel: 9 }));
        let query = FromFetcher::RulesQuery { page: 0x101, identifier: u32::MAX };
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
}
