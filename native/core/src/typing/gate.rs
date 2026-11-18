//! The trusted core as the gate of a page's rule table (neoteletext-common's
//! `docs/page-rules.md` RULE-50 and RULE-52, REQ-6523): nothing a viewer
//! does leaves for a page's channel unless that page's table asks for it,
//! and what leaves is built here, from this core's own copy of the
//! viewer's event. No table is read here: the page's decoder says which
//! classes of input its table asks for and, for each `send`, which event
//! by the sequence number it was given under.
//!
//! - **In** (item 1): an input event goes to the open page's decoder only
//!   while that decoder has said the page has a table, and only of a class
//!   in its interest mask.
//! - **Kept** (item 2): the last [`INPUT_RING`] events given, as encoded.
//! - **Out** (item 3): a report names a rule, a tick and a sequence number,
//!   a value or nothing; the `RULE_EVENT` is built from the kept copy,
//!   only for an event given for that page within [`SEND_WINDOW_TICKS`]
//!   and not yet reported for that rule.
//! - **Where** (item 4): only on the session of that page's own channel,
//!   while that page is the open page of its view.
//! - **How much** (item 5): [`SEND_BUDGET`] reports a second, of which
//!   [`TIMED_SEND_BUDGET`] with no input event; the rest are dropped and
//!   reported as fault 3.
//! - **State** (item 6): a `PAGE_STATE` passes only in answer to a query
//!   the channel's fetcher received for that page, once.

use std::collections::VecDeque;

use neoteletext_core::consts::presentation_tick_due_us;
use neoteletext_input::event::{self, Action, Event, Item, KeyCode, MouseAction, Stamp};
use neoteletext_input::rules::{FaultCode, PageStateEvent, RuleBody, RuleEvent, RulesFault};
use neoteletext_rules::table::{Target, TargetKind, Value};
use protocol::rules::{
    INPUT_RING, INTEREST_MOVES, INTEREST_POINTER_BUTTONS, KeyAction, PageSub, PointerAction, Report, Reported, RuleInput, RulesOpen, SEND_BUDGET,
    SEND_WINDOW_TICKS, TIMED_SEND_BUDGET, WheelRect,
};

use super::{Channel, Send, clock};

/// The most state queries of a channel waiting for their answer.
const MAX_QUERIES: usize = 4;
/// One more than the highest fault code: a place for each code.
const FAULT_SLOTS: usize = protocol::rules::MAX_FAULT as usize + 1;
/// Fault 3: reports dropped by the send budget (RULE-48).
const FAULT_SENDS_DROPPED: u8 = 3;
/// The micro-units of one report in a budget: a budget of `n` a second
/// earns `n` of them a microsecond.
const REPORT_COST: u64 = 1_000_000;
/// Microseconds in the second a fault is reported at most once in.
const SECOND_US: u64 = 1_000_000;
/// A teletext cell's width and height in dots of the page image.
const CELL_WIDTH: u32 = interface_core::canvas::CELL_WIDTH as u32;
const CELL_HEIGHT: u32 = interface_core::canvas::CELL_HEIGHT as u32;

/// What the open page's table asks for, as its decoder said.
#[derive(Debug)]
pub(super) struct Asked {
    /// The page and sub-code (RULE-112).
    pub(super) page: PageSub,
    pub(super) generation: u16,
    /// The classes of input: `INTEREST_` bits.
    pub(super) interest: u8,
    /// Where an enabled `wheel` rule applies, in the picture's pixels.
    pub(super) wheel: Vec<WheelRect>,
    /// The page as a stamp: what a report with no event is stamped with.
    stamp: Stamp,
    /// The page stopped being the open page at this time: its decoder's
    /// final tick may still report, for [`SEND_WINDOW_TICKS`].
    closing_us: Option<u64>,
}

/// One input event given to the open page's decoder, kept as encoded.
#[derive(Debug)]
struct Kept {
    sequence: u32,
    /// The page and sub-code it was given for (RULE-112).
    stamp: Stamp,
    /// The event exactly as push protocol 8 encoded it, from its `0xFF`,
    /// or a run of raw text.
    bytes: Vec<u8>,
    /// Its PICK, after a pointer event on a page the ReGIS renderer draws.
    pick: Vec<u8>,
    /// A MOUSE, TOUCH or WHEEL: what a PICK may follow.
    pointed: bool,
    given_us: u64,
    /// The rules and rows it was already reported for (RULE-52 item 3),
    /// each the rule in the high byte and the row's place in the low. At
    /// most one a report taken from the budget, so at most
    /// [`SEND_BUDGET`] within the window.
    reported: Vec<u16>,
}

/// A budget of reports a second.
#[derive(Debug, Default)]
struct Budget {
    /// What is saved, in micro-units of a report.
    saved: u64,
    at_us: u64,
}

impl Budget {
    /// Takes one report from a budget of `rate` a second, which holds at
    /// most a second's worth; false when it has none.
    fn take(&mut self, rate: u32, now_us: u64) -> bool {
        let rate = u64::from(rate);
        let most = rate.saturating_mul(REPORT_COST);
        let earned = now_us.saturating_sub(self.at_us).saturating_mul(rate);
        // A budget not yet used starts full.
        self.saved = if self.at_us == 0 && self.saved == 0 { most } else { self.saved.saturating_add(earned).min(most) };
        self.at_us = now_us.max(1);
        match self.saved.checked_sub(REPORT_COST) {
            Some(left) => {
                self.saved = left;
                true
            }
            None => false,
        }
    }
}

/// The gate's state for one channel's open page.
#[derive(Debug, Default)]
pub(super) struct Gate {
    pub(super) asked: Option<Asked>,
    ring: VecDeque<Kept>,
    next_sequence: u32,
    reports: Budget,
    timed: Budget,
    /// Reports the budget dropped since fault 3 was last told.
    dropped: u16,
    /// Requests refused for naming an event that was not given for the
    /// page, was given too long ago or was already reported for the rule
    /// (RULE-52 item 3): counted, and sent nowhere.
    pub(super) refused: u32,
    /// When each fault was last passed to the channel, by code.
    fault_told_us: [Option<u64>; FAULT_SLOTS],
    /// The state queries the fetcher received, waiting for their answer.
    queries: VecDeque<(PageSub, u32)>,
    /// The pixel of the page's picture the pointer is on, as the trusted
    /// core derived it for the event being given.
    pub(super) pixel: Option<(u16, u16)>,
    /// The teletext control under the event being given, numbered from 1.
    pub(super) control: u16,
    /// The control the pointer was last said to be over (HOVER).
    hover: u16,
    /// Where the pointer last was, in the picture's pixels.
    at: (u16, u16),
    /// The page's decoder was given that the page holds the focus, and not
    /// since that it lost it.
    focused: bool,
}

impl Gate {
    /// Whether a table asks for input for the page and sub-code of
    /// `stamp` now (RULE-112: another sub-code is another page).
    fn asks(&self, stamp: Stamp) -> Option<&Asked> {
        self.asked.as_ref().filter(|asked| asked.stamp == stamp && asked.closing_us.is_none())
    }

    /// Whether the pointer, where it last was, is inside a rectangle an
    /// enabled `wheel` rule applies in (RULE-51 item 4).
    pub(super) fn wheel_here(&self) -> bool {
        self.asked.as_ref().is_some_and(|asked| asked.closing_us.is_none() && asked.wheel.iter().any(|rect| rect.contains(self.at.0, self.at.1)))
    }

    /// The pointer is at `at`, a point of a pointer intent.
    pub(super) fn pointer_at(&mut self, at: event::Point) {
        self.at = self.pixel_of(at);
    }

    /// The pixel of the page's picture of `at`: the trusted core's own
    /// for a picture the ReGIS or NAPLPS renderer draws, else the dot of
    /// the teletext page image, or its cell's middle.
    fn pixel_of(&self, at: event::Point) -> (u16, u16) {
        let narrow = |value: u32| u16::try_from(value).unwrap_or(u16::MAX);
        self.pixel.unwrap_or_else(|| match at.pixels {
            Some((x, y)) => (narrow(x), narrow(y)),
            None => cell_middle(at.column, at.row),
        })
    }

    /// The page stopped being the open page: nothing more is given, and
    /// its decoder's final tick may report for a little longer (RULE-5's
    /// `close` rules, RULE-52 item 4).
    pub(super) fn closing(&mut self, now_us: u64) {
        if let Some(asked) = self.asked.as_mut().filter(|asked| asked.closing_us.is_none()) {
            asked.closing_us = Some(now_us);
        }
        self.focused = false;
        self.hover = 0;
        self.queries.clear();
    }

    /// Forgets everything: the connection or the channel went.
    pub(super) fn forget(&mut self) {
        *self = Self::default();
    }

    /// A state query the fetcher received for `page` (RULE-52 item 6).
    pub(super) fn query(&mut self, page: PageSub, identifier: u32) {
        if self.queries.len() >= MAX_QUERIES {
            self.queries.pop_front();
        }
        self.queries.push_back((page, identifier));
    }
}

/// The page an input event is stamped with, when it carries a stamp.
const fn stamp_of(event: &Event<'_>) -> Option<Stamp> {
    match event {
        Event::Key(key) => Some(key.on),
        Event::Mouse(mouse) => Some(mouse.on),
        Event::Touch(touch) => Some(touch.on),
        Event::Wheel(wheel) => Some(wheel.on),
        Event::Paste { on, .. } | Event::Compose { on, .. } | Event::Hover { on, .. } | Event::DrawingHover { on, .. } => Some(*on),
        Event::Pick(pick) => Some(pick.on),
        _ => None,
    }
}

/// The middle of a teletext cell, in dots of the page image.
fn cell_middle(column: u32, row: u32) -> (u16, u16) {
    let middle = |cell: u32, size: u32| u16::try_from(cell.saturating_mul(size).saturating_add(size.checked_div(2).unwrap_or(0))).unwrap_or(u16::MAX);
    (middle(column, CELL_WIDTH), middle(row, CELL_HEIGHT))
}

/// Whether a table whose interest mask is `interest` is given `input`
/// (RULE-52 item 1). The pointer's gone goes to a table that asked for
/// its buttons too, so what it held is released (RULE-8).
fn wanted(interest: u8, input: &RuleInput) -> bool {
    let class = match input {
        RuleInput::Pointer { action: PointerAction::Gone, .. } => INTEREST_MOVES | INTEREST_POINTER_BUTTONS,
        other => other.interest(),
    };
    interest & class != 0
}

impl Channel {
    /// What the evaluator matches for `item`, an input event as it goes
    /// (RULE-52 item 1); `None` for a PICK, which follows its pointer
    /// event and is no event of its own.
    fn rule_input(&mut self, item: &Item<'_>) -> Option<RuleInput> {
        let event = match item {
            Item::Text(_) => return Some(RuleInput::Text),
            Item::Event(event) => event,
        };
        Some(match event {
            Event::Key(key) => {
                let action = match key.action {
                    Action::Press => KeyAction::Press,
                    Action::Repeat => KeyAction::Repeat,
                    Action::Release => KeyAction::Release,
                };
                let (named, code) = match key.key {
                    KeyCode::Character(character) => (false, u32::from(character)),
                    KeyCode::Functional(code) => (true, code),
                };
                RuleInput::Key { action, named, code, base: key.base.map(u32::from), mods: key.mods }
            }
            Event::Paste { .. } | Event::Compose { .. } => RuleInput::Text,
            Event::Mouse(mouse) => {
                let (x, y) = self.gate.pixel_of(mouse.at);
                self.pointer_input(mouse.action, mouse.button, x, y)
            }
            Event::Touch(touch) => {
                let at = event::Point { column: touch.column, row: touch.row, pixels: touch.pixels, mods: 0 };
                let (x, y) = self.gate.pixel_of(at);
                self.pointer_input(touch.action, 0, x, y)
            }
            Event::Wheel(wheel) => {
                let (x, y) = self.gate.pixel_of(wheel.at);
                RuleInput::Wheel { x, y, control: self.gate.control }
            }
            Event::FocusIn => {
                self.gate.focused = true;
                RuleInput::Focus
            }
            Event::FocusOut => {
                self.gate.focused = false;
                RuleInput::Blur
            }
            // The pointer came onto a control or went off it: a move that
            // changes the control it is over (RULE-46).
            Event::Hover { leave, .. } => {
                self.gate.hover = if *leave { 0 } else { self.gate.control };
                let (x, y) = self.gate.at;
                RuleInput::Pointer { action: PointerAction::Move, button: self.held, x, y, control: self.gate.hover }
            }
            Event::DrawingHover { leave, drawing, .. } if *drawing != 0 => RuleInput::Drawing { leave: *leave, drawing: *drawing },
            _ => return None,
        })
    }

    /// A pointer event for the evaluator. A press or a release names the
    /// control under its cell; a move the control the pointer was last
    /// said to be over, so that a control's `enter` and `leave` fire on
    /// its HOVER (RULE-46).
    fn pointer_input(&mut self, action: MouseAction, button: u8, x: u16, y: u16) -> RuleInput {
        self.gate.at = (x, y);
        let (action, control) = match action {
            MouseAction::Press => (PointerAction::Press, self.gate.control),
            MouseAction::Release => (PointerAction::Release, self.gate.control),
            MouseAction::Move => (PointerAction::Move, self.gate.hover),
            MouseAction::Gone => {
                self.gate.hover = 0;
                (PointerAction::Gone, 0)
            }
        };
        RuleInput::Pointer { action, button, x, y, control }
    }

    /// Gives `item`, an input event of the viewer, to the open page's
    /// decoder, if its table asks for that class (RULE-52 items 1 and 2):
    /// the event is encoded as it would have gone, kept, and the decoder
    /// is given what the evaluator matches under a sequence number. With
    /// no table nothing is given and nothing is kept (RULE-50). Whether it
    /// was given.
    pub(super) fn give(&mut self, channel: u32, item: &Item<'_>, now_us: u64, sends: &mut Vec<Send>) -> bool {
        let stamp = match item {
            Item::Event(event) => stamp_of(event),
            Item::Text(_) => None,
        }
        .or(self.on_screen);
        let Some(stamp) = stamp else { return false };
        let Some(interest) = self.gate.asks(stamp).map(|asked| asked.interest) else { return false };
        // The capabilities agreed still decide what goes (DOC-1922).
        let Some(carried) = self.session.carried(item) else { return false };
        let Some(input) = self.rule_input(&carried) else { return false };
        if !wanted(interest, &input) {
            return false;
        }
        let mut out = [0_u8; neoteletext_input::MAX_MESSAGE];
        let Ok(length) = event::write(&carried, &mut out) else { return false };
        let bytes = out.get(..length).unwrap_or(&[]).to_vec();
        let sequence = self.gate.next_sequence;
        self.gate.next_sequence = sequence.wrapping_add(1);
        let pointed = matches!(carried, Item::Event(Event::Mouse(_) | Event::Touch(_) | Event::Wheel(_)));
        if self.gate.ring.len() >= INPUT_RING {
            self.gate.ring.pop_front();
        }
        #[cfg(test)]
        let kept = bytes.clone();
        self.gate.ring.push_back(Kept { sequence, stamp, bytes, pick: Vec::new(), pointed, given_us: now_us, reported: Vec::new() });
        sends.push(Send::Decoder {
            channel,
            page: PageSub::new(stamp.page, stamp.sub),
            sequence,
            input,
            #[cfg(test)]
            kept,
        });
        true
    }

    /// Gives the open page's decoder that the page holds the focus, when it
    /// does and the decoder has not been given so: as its table is taken,
    /// and as the page is first on screen after it (RULE-8).
    pub(super) fn give_focus_owed(&mut self, channel: u32, now_us: u64, sends: &mut Vec<Send>) {
        if self.focus_told == Some(true) && !self.gate.focused {
            self.send(channel, &Item::Event(Event::FocusIn), now_us, sends);
        }
    }

    /// The PICK of the pointer event just given (the engine's REQ-6337):
    /// kept with it, to follow it inside its `RULE_EVENT` (RULE-46).
    pub(super) fn keep_pick(&mut self, pick: event::Pick) -> bool {
        let Some(carried) = self.session.carried(&Item::Event(Event::Pick(pick))) else { return false };
        let mut out = [0_u8; neoteletext_input::MAX_MESSAGE];
        let Ok(length) = event::write(&carried, &mut out) else { return false };
        match self.gate.ring.back_mut().filter(|kept| kept.pointed && kept.stamp == pick.on && kept.reported.is_empty()) {
            Some(kept) => {
                kept.pick = out.get(..length).unwrap_or(&[]).to_vec();
                true
            }
            None => false,
        }
    }

    /// The page's decoder said what its table asks for (RULE-52 item 1).
    /// `open` says whether that page and sub-code are the open page of the
    /// view now. A table for the open page is taken; the end of a table
    /// ends what was asked for that page.
    pub(super) fn rules_told(&mut self, told: &RulesOpen, open: bool) {
        if told.generation == 0 {
            if self.gate.asked.as_ref().is_some_and(|asked| asked.page == told.page) {
                self.gate.asked = None;
                self.gate.focused = false;
            }
            return;
        }
        let Some(stamp) = Stamp::new(told.page.page, told.page.sub) else { return };
        if !open {
            return;
        }
        let same = self.gate.asked.as_ref().is_some_and(|asked| asked.page == told.page && asked.generation == told.generation && asked.closing_us.is_none());
        self.gate.asked =
            Some(Asked { page: told.page, generation: told.generation, interest: told.interest, wheel: told.wheel.clone(), stamp, closing_us: None });
        // A new table's evaluator starts afresh, and has heard nothing.
        if !same {
            self.gate.focused = false;
        }
    }

    /// Passes fault 3 for the reports the budget dropped, at most once a
    /// second (RULE-48, RULE-52 item 5).
    fn tell_dropped(&mut self, channel: u32, now_us: u64, sends: &mut Vec<Send>) {
        if self.gate.dropped == 0 {
            return;
        }
        let Some((page, generation)) = self.gate.asked.as_ref().map(|asked| (asked.page, asked.generation)) else { return };
        let count = self.gate.dropped;
        if self.fault(channel, page, generation, FAULT_SENDS_DROPPED, count, now_us, sends) {
            self.gate.dropped = 0;
        }
    }

    /// A report a decoder asks for (RULE-52 items 3 to 5): the
    /// `RULE_EVENT` is built here and given to the channel's fetcher, or
    /// the request is dropped and counted.
    pub(super) fn report(&mut self, channel: u32, report: &Report, now_us: u64, sends: &mut Vec<Send>) {
        let window_us = presentation_tick_due_us(SEND_WINDOW_TICKS);
        let Some(asked) = self.gate.asked.as_ref().filter(|asked| asked.page == report.page && asked.generation == report.generation) else {
            self.gate.refused = self.gate.refused.saturating_add(1);
            return;
        };
        // Where: only while the page is the open page, or just after, for
        // its final tick.
        if asked.closing_us.is_some_and(|closing| now_us.saturating_sub(closing) > window_us) {
            return;
        }
        let Some(kind) = TargetKind::from_code(report.target_kind) else { return };
        let target = Target { kind, number: report.target_number };
        let rule_row = u16::from_be_bytes([report.rule, report.row]);
        let mut body = Vec::new();
        let (on, what) = match report.what {
            Reported::Input { sequence } => {
                let Some(kept) = self.gate.ring.iter_mut().find(|kept| kept.sequence == sequence) else {
                    self.gate.refused = self.gate.refused.saturating_add(1);
                    return;
                };
                // Given for that page and sub-code, within the window, and
                // not yet reported for that rule and row.
                let given_for = PageSub::new(kept.stamp.page, kept.stamp.sub);
                if given_for != report.page
                    || now_us.saturating_sub(kept.given_us) > window_us
                    || kept.reported.contains(&rule_row)
                    || kept.reported.len() >= SEND_BUDGET as usize
                {
                    self.gate.refused = self.gate.refused.saturating_add(1);
                    return;
                }
                kept.reported.push(rule_row);
                body.extend_from_slice(&kept.bytes);
                body.extend_from_slice(&kept.pick);
                (kept.stamp, None)
            }
            Reported::Fired => (asked.stamp, Some(RuleBody::Fired)),
            Reported::Value { value, reading } => {
                let Ok(value) = Value::from_bytes(&value) else { return };
                (asked.stamp, Some(RuleBody::Value { value, reading }))
            }
        };
        // How much: every report from the one budget, and those with no
        // input event from the smaller one too.
        let within = (what.is_none() || self.gate.timed.take(TIMED_SEND_BUDGET, now_us)) && self.gate.reports.take(SEND_BUDGET, now_us);
        if !within {
            self.dropped();
            self.tell_dropped(channel, now_us, sends);
            return;
        }
        let event = RuleEvent {
            on,
            generation: report.generation,
            rule: report.rule,
            row: report.row,
            tick: report.tick,
            target,
            body: what.unwrap_or(RuleBody::Input(&body)),
        };
        self.write(channel, &Item::Event(Event::RuleEvent(event)), now_us, sends);
        self.tell_dropped(channel, now_us, sends);
    }

    /// Counts a report dropped.
    fn dropped(&mut self) {
        self.gate.dropped = self.gate.dropped.saturating_add(1);
    }

    /// A fault of the page's table, the decoder's or this gate's: passed
    /// to the channel at most once a second for each code (RULE-48).
    /// Whether it went.
    #[allow(clippy::too_many_arguments, reason = "a fault is its page, generation, code and count, and goes like every send")]
    pub(super) fn fault(&mut self, channel: u32, page: PageSub, generation: u16, code: u8, count: u16, now_us: u64, sends: &mut Vec<Send>) -> bool {
        let Some(fault) = FaultCode::from_code(code) else { return false };
        let Some(on) = Stamp::new(page.page, page.sub) else { return false };
        let Some(told) = self.gate.fault_told_us.get_mut(usize::from(code)) else { return false };
        if told.is_some_and(|told| now_us.saturating_sub(told) < SECOND_US) {
            return false;
        }
        *told = Some(now_us);
        let event = Event::RulesFault(RulesFault { on, generation, fault, count });
        self.write(channel, &Item::Event(event), now_us, sends);
        true
    }

    /// The answer to a state query (RULE-52 item 6): passed only for a
    /// query the fetcher received for that page, once, and only at a
    /// length the protocol allows, beginning with that query's identifier.
    pub(super) fn page_state(&mut self, channel: u32, page: PageSub, identifier: u32, body: &[u8], now_us: u64, sends: &mut Vec<Send>) {
        let Some(place) = self.gate.queries.iter().position(|query| *query == (page, identifier)) else { return };
        self.gate.queries.remove(place);
        let Some(on) = Stamp::new(page.page, page.sub) else { return };
        if body.len() > protocol::rules::MAX_PAGE_STATE || !body.starts_with(&identifier.to_be_bytes()) {
            return;
        }
        self.write(channel, &Item::Event(Event::PageState(PageStateEvent { on, body })), now_us, sends);
    }

    /// Writes one of the rule table's messages on the channel's session.
    fn write(&mut self, channel: u32, item: &Item<'_>, now_us: u64, sends: &mut Vec<Send>) {
        let mut out = [0_u8; neoteletext_input::MAX_MESSAGE];
        if let Ok(length @ 1..) = self.session.send(item, clock(now_us), &mut out) {
            sends.push(Send::Fetcher { channel, bytes: out.get(..length).unwrap_or(&[]).to_vec(), move_of: None, more: false });
        }
    }
}
