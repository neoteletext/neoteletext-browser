//! The evaluator's host (page-rules.md RULE-58): the tables a channel sent,
//! the evaluator of the open page, its clock, and the engine's `World` for
//! it.
//!
//! Every number a table gives is untrusted when it is used (RULE-71): the
//! engine's decoder refuses a table whole or accepts it within every limit
//! (RULE-70), the evaluator works in fixed arrays, and an action that names
//! a sound or a target that is not there is a counted no-op here.

use std::collections::BTreeMap;

use neoteletext_audio::receiver::Receiver;
use neoteletext_audio::record::{Control, FLAG_LOOP, Operation as ControlOperation};
use neoteletext_rules::animate::Operation;
use neoteletext_rules::clock::Clock;
use neoteletext_rules::evaluator::{
    Bounds, Counters, Input, InputEvent, KeyAction, KeyIdentity, PointerAction, Report, Reported, SoundOperation, SoundState, TargetState,
};
use neoteletext_rules::limits::{MAX_POINTER_OBJECTS, MAX_RECTS, MAX_STATE_BYTES, TICKS_PER_SECOND};
use neoteletext_rules::state::PageState;
use neoteletext_rules::table::{Table, Target};
use neoteletext_rules::{Evaluator, World};
use protocol::audio::{Audio, WatchSlot};
use protocol::decoder::FromDecoder;
use protocol::rules::{QUERY_INTERVAL_TICKS, RuleCounters, RuleInput, RulesOpen, WheelRect};

use super::picture::Picture;
use crate::voice_sink::{Heard, Outbox};

/// The most pages a decoder keeps a table for: as many as the server
/// keeps for a connection (page-rules.md RULE-57, `MAX_KEPT_TABLES`). The
/// table that arrived longest ago is forgotten first; the open page's is
/// never forgotten.
pub const MAX_KEPT_TABLES: usize = 64;

/// The fault codes a decoder reports (RULE-48).
const FAULT_UNDECODABLE: u8 = 1;
const FAULT_TICK_OVERRUN: u8 = 2;
const FAULT_EVENTS_DROPPED: u8 = 4;
const FAULT_ACTIONS_SKIPPED: u8 = 5;
const FAULT_QUERY_DROPPED: u8 = 6;
const FAULT_ANIMATION_SLOWED: u8 = 8;
const _: () = assert!(FAULT_ANIMATION_SLOWED == protocol::rules::MAX_FAULT);
/// One more than the highest fault code: a place for each code's count.
const FAULT_SLOTS: usize = protocol::rules::MAX_FAULT as usize + 1;

/// What the channel last said of a page's table.
enum Said {
    /// The page has no table (RULE-41, the header's flag clear).
    NoTable,
    /// A table the engine's decoder accepted: the record's body.
    Table(Vec<u8>),
    /// A table that could not be decoded and was refused whole (RULE-70):
    /// the page has none.
    Refused,
}

struct Kept {
    generation: u16,
    said: Said,
    /// When it arrived, among every table's arrival.
    arrival: u64,
}

/// The evaluator running for the open page.
struct Open {
    page: u16,
    generation: u16,
    evaluator: Box<Evaluator>,
    /// Which ticks are due, counted from `start_us`.
    clock: Clock,
    /// The decoder's time at which page tick 0 is due: the first tick of
    /// the presentation clock after the page was opened (RULE-4).
    start_us: u64,
    /// What the trusted core was last told the table asks for.
    told: RulesOpen,
    /// The ReGIS drawing the trusted core last said the pointer is on
    /// (`DRAWING_HOVER`); 0 for none.
    hovered: u16,
    /// Where the pointer last was, what it held and the control under it.
    pointer: (u16, u16, u8, u16),
    /// The page tick at which a state query was last answered.
    answered: Option<u32>,
    /// The counts already reported as faults.
    reported: Counters,
    /// Each fault's count waiting to be reported, by code.
    faults: [u16; FAULT_SLOTS],
    /// The page tick each fault was last reported at.
    fault_told: [Option<u32>; FAULT_SLOTS],
    /// The page tick the counters were last told at.
    counters_told: Option<u32>,
}

/// The engine's pieces the evaluator's world acts on.
pub(crate) struct Parts<'a, const OBJECTS: usize, const RECORDS: usize> {
    /// The audio receiver, which carries out a rule's `play`, `stop` and
    /// `gain`.
    pub audio: &'a mut Receiver<'static, OBJECTS, RECORDS>,
    /// The voice operations waiting to be sent.
    pub sounds: &'a mut Vec<Audio>,
    /// What plays on each audio channel.
    pub heard: &'a mut Heard,
    /// The open page's picture.
    pub picture: &'a mut dyn Picture,
    /// The sub-code of the sub-page shown.
    pub sub: u16,
}

/// The tables of one channel and the evaluator of its open page.
#[derive(Default)]
pub(crate) struct Rules {
    tables: BTreeMap<u16, Kept>,
    arrivals: u64,
    /// The page that is the open page of the active tab (RULE-58).
    page: Option<u16>,
    open: Option<Open>,
    /// What was produced outside a tick, for the trusted core.
    out: Vec<FromDecoder>,
    /// Room to decode a table into: 76 KB, kept off the stack.
    scratch: Option<Box<Table>>,
    /// When a query for a page with no table was last answered.
    plain_answered_us: Option<u64>,
    /// The page and generation the evaluator was last settled for.
    settled: Option<(u16, u16)>,
}

/// The evaluator's world for one tick: the picture, the sounds, and where
/// its outputs go.
struct PageWorld<'a, 'b, const OBJECTS: usize, const RECORDS: usize> {
    parts: &'a mut Parts<'b, OBJECTS, RECORDS>,
    page: u16,
    generation: u16,
    hovered: u16,
    /// A rule's MIDI messages, which leave before everything else of the
    /// tick (RULE-64).
    urgent: &'a mut Vec<FromDecoder>,
    out: &'a mut Vec<FromDecoder>,
}

impl<const OBJECTS: usize, const RECORDS: usize> PageWorld<'_, '_, OBJECTS, RECORDS> {
    /// Carries out a rule's `play`, `stop` or `gain` as a CONTROL record
    /// is carried out (RULE-20), for this page.
    fn command(&mut self, channel: u8, operation: ControlOperation, sound: (u8, u16), gain: u8, fade_ms: u16, flags: u8) {
        let control = Control { channel, sequence: 0, operation, handle: sound.0, generation: sound.1, gain, fade_ms, flags, duck: 0, page: Some(self.page) };
        self.parts.audio.rule_command(&control, &mut Outbox::new(self.parts.sounds, self.parts.heard));
    }
}

impl<const OBJECTS: usize, const RECORDS: usize> World for PageWorld<'_, '_, OBJECTS, RECORDS> {
    fn picture(&self) -> (u16, u16) {
        self.parts.picture.size()
    }

    fn target(&self, target: Target) -> Option<TargetState> {
        self.parts.picture.target(target)
    }

    fn objects_at(&self, x: u16, y: u16, objects: &mut [u16; MAX_POINTER_OBJECTS]) -> usize {
        let mut count = self.parts.picture.objects_at(x, y, objects).min(MAX_POINTER_OBJECTS);
        // The drawing of a ReGIS picture the trusted core's pick record
        // names under the pointer (RULE-46: `DRAWING_HOVER`).
        let listed = objects.get(..count).is_some_and(|listed| listed.contains(&self.hovered));
        if self.hovered != 0
            && !listed
            && let Some(slot) = objects.get_mut(count)
        {
            *slot = self.hovered;
            count = count.saturating_add(1);
        }
        count
    }

    fn sound(&self, channel: u8) -> SoundState {
        self.parts.heard.state(channel)
    }

    fn synth(&self, channel: u8, part: u8, class: u8, index: u16) -> i32 {
        self.parts.heard.synth(channel, part, class, index)
    }

    fn sound_operation(&mut self, operation: SoundOperation) -> bool {
        match operation {
            // RULE-21: only to a local sound, never to one whose blocks
            // the server sends.
            SoundOperation::Midi { channel, status, data1, data2 } => {
                if !self.parts.heard.is_local(channel) {
                    return false;
                }
                self.parts.heard.midi(channel, status, data1, data2);
                let message = Audio::LocalMidi { channel, status, data1, data2 };
                // After the sound's own start, when that waits to be sent.
                if self.parts.sounds.is_empty() {
                    self.urgent.push(FromDecoder::Audio(message));
                } else {
                    self.parts.sounds.push(message);
                }
                true
            }
            SoundOperation::Play { channel, handle, generation, gain, fade_ms, looped } => {
                self.command(channel, ControlOperation::Play, (handle, generation), gain, fade_ms, if looped { FLAG_LOOP } else { 0 });
                let now = self.parts.heard.state(channel);
                now.playing && now.handle == handle
            }
            SoundOperation::Stop { channel, fade_ms } => {
                let played = self.parts.heard.state(channel).playing;
                self.command(channel, ControlOperation::Stop, (0, 0), 0, fade_ms, 0);
                played
            }
            SoundOperation::Gain { channel, gain, fade_ms } => {
                let played = self.parts.heard.state(channel).playing;
                self.command(channel, ControlOperation::Gain, (0, 0), gain, fade_ms, 0);
                played
            }
        }
    }

    fn animate(&mut self, operation: Operation, tick: u32) -> bool {
        self.parts.picture.animate(operation, tick)
    }

    fn report(&mut self, report: Report) {
        let what = match report.what {
            Reported::Fired => protocol::rules::Reported::Fired,
            Reported::Input { sequence } => protocol::rules::Reported::Input { sequence },
            Reported::Value { value, reading } => protocol::rules::Reported::Value { value: value.to_bytes(), reading },
        };
        self.out.push(FromDecoder::Report(protocol::rules::Report {
            page: self.page,
            generation: self.generation,
            rule: report.rule,
            tick: report.tick,
            target_kind: report.target.kind.code(),
            target_number: report.target.number,
            what,
        }));
    }

    fn advance_cycles(&mut self, tick: u32) {
        self.parts.picture.advance_cycles(tick);
    }

    fn page_state(&mut self, state: &PageState) {
        if let Some(answer) = page_state(self.page, state) {
            self.out.push(answer);
        }
    }
}

/// A state answer as the trusted core takes it (RULE-47).
fn page_state(page: u16, state: &PageState) -> Option<FromDecoder> {
    let mut body = [0_u8; MAX_STATE_BYTES];
    let length = neoteletext_rules::state::encode(state, &mut body).ok()?;
    Some(FromDecoder::PageState { page, identifier: state.identifier, body: body.get(..length)?.to_vec() })
}

/// The evaluator's input event for what the trusted core gave.
fn event(sequence: u32, input: RuleInput, open: &mut Open, picture: &dyn Picture) -> InputEvent {
    let input = match input {
        RuleInput::Key { action, named, code, base, mods } => {
            let action = match action {
                protocol::rules::KeyAction::Press => KeyAction::Press,
                protocol::rules::KeyAction::Repeat => KeyAction::Repeat,
                protocol::rules::KeyAction::Release => KeyAction::Release,
            };
            Input::Key { action, key: if named { KeyIdentity::Named(code) } else { KeyIdentity::Character(code) }, base, mods }
        }
        RuleInput::Text => Input::Text,
        RuleInput::Pointer { action, button, x, y, control } => {
            let action = match action {
                protocol::rules::PointerAction::Press => PointerAction::Press,
                protocol::rules::PointerAction::Release => PointerAction::Release,
                protocol::rules::PointerAction::Move => PointerAction::Move,
                protocol::rules::PointerAction::Gone => PointerAction::Gone,
            };
            // A hidden control takes no pointer event (ANIM-6).
            let control = picture.control_shown(control);
            if action == PointerAction::Gone {
                open.hovered = 0;
            }
            if action == PointerAction::Move {
                open.pointer = (x, y, button, control);
            } else {
                open.pointer = (x, y, open.pointer.2, control);
            }
            Input::Pointer { action, button, x, y, control }
        }
        RuleInput::Wheel { x, y, control } => Input::Wheel { x, y, control: picture.control_shown(control) },
        RuleInput::Focus => Input::Focus,
        RuleInput::Blur => Input::Blur,
        // The pointer came onto or went off a drawing where it is: a move
        // that changes what it is inside (RULE-46).
        RuleInput::Drawing { leave, drawing } => {
            if !leave {
                open.hovered = drawing;
            } else if open.hovered == drawing {
                open.hovered = 0;
            }
            let (x, y, button, control) = open.pointer;
            Input::Pointer { action: PointerAction::Move, button, x, y, control }
        }
    };
    InputEvent { sequence, input }
}

impl Open {
    /// What the table asks for now: its interest mask, its watch list and
    /// where an enabled `wheel` rule applies (RULE-52 item 1, RULE-33 item
    /// 1, RULE-51 item 4).
    fn asks(&self, picture: (u16, u16)) -> RulesOpen {
        let index = self.evaluator.index();
        let watch = index.watch.iter().map(|slot| WatchSlot { channel: slot.channel, kind: slot.kind, part: slot.part }).collect();
        let empty = Bounds { left: 0, top: 0, right: 0, bottom: 0 };
        let mut bounds = [empty; MAX_RECTS];
        let count = self.evaluator.wheel_bounds(&mut bounds);
        let wheel = bounds
            .iter()
            .take(count)
            .filter_map(|bounds| bounds.cut_to(picture.0, picture.1))
            .filter_map(|bounds| {
                let (x, y) = (u16::try_from(bounds.left).ok()?, u16::try_from(bounds.top).ok()?);
                let width = u16::try_from(bounds.right.saturating_sub(bounds.left)).ok()?;
                let height = u16::try_from(bounds.bottom.saturating_sub(bounds.top)).ok()?;
                Some(WheelRect { x, y, width, height })
            })
            .collect();
        RulesOpen { page: self.page, generation: self.generation, interest: index.interest, watch, wheel }
    }

    /// Counts a fault to report (RULE-48).
    fn fault(&mut self, code: u8, count: u32) {
        if let Some(slot) = self.faults.get_mut(usize::from(code)) {
            *slot = slot.saturating_add(u16::try_from(count).unwrap_or(u16::MAX));
        }
    }

    /// What the last tick counted that is a fault: an overrun, events
    /// dropped and actions skipped (RULE-48 codes 2, 4 and 5).
    fn count_faults(&mut self) {
        let now = *self.evaluator.counters();
        self.fault(FAULT_TICK_OVERRUN, now.overruns.saturating_sub(self.reported.overruns));
        self.fault(FAULT_EVENTS_DROPPED, now.events_dropped.saturating_sub(self.reported.events_dropped));
        self.fault(FAULT_ACTIONS_SKIPPED, now.skipped.saturating_sub(self.reported.skipped));
        self.reported = now;
    }

    /// The faults due to be told: at most one a second for each code
    /// (RULE-48).
    fn tell_faults(&mut self, out: &mut Vec<FromDecoder>) {
        let tick = self.evaluator.page_tick();
        for (code, (count, told)) in (0_u8..).zip(self.faults.iter_mut().zip(self.fault_told.iter_mut())) {
            let due = told.is_none_or(|told| tick.saturating_sub(told) >= TICKS_PER_SECOND);
            if *count != 0 && due {
                out.push(FromDecoder::RulesFault { page: self.page, generation: self.generation, fault: code, count: *count });
                *count = 0;
                *told = Some(tick);
            }
        }
    }

    /// The counts for the console, once a second (RULE-72).
    fn tell_counters(&mut self, out: &mut Vec<FromDecoder>) {
        let tick = self.evaluator.page_tick();
        if self.counters_told.is_some_and(|told| tick.saturating_sub(told) < TICKS_PER_SECOND) {
            return;
        }
        self.counters_told = Some(tick);
        let counters = self.evaluator.counters();
        out.push(FromDecoder::RuleCounters(RuleCounters {
            page: self.page,
            ticks: counters.ticks,
            fired: counters.fired,
            actions: counters.actions,
            skipped: counters.skipped,
            events_dropped: counters.events_dropped,
            sends: counters.sends,
            overruns: counters.overruns,
        }));
    }
}

impl Rules {
    /// The page whose table is evaluated, if one is.
    pub(crate) fn open_page(&self) -> Option<u16> {
        self.open.as_ref().map(|open| open.page)
    }

    /// The page that is the open page of the active tab, as last told.
    pub(crate) const fn page(&self) -> Option<u16> {
        self.page
    }

    /// A RULES record arrived (RULE-41): the page's table is replaced when
    /// its generation differs. A body the engine's decoder refuses leaves
    /// the page with no table, and is reported as fault 1 while that page
    /// is the open page (RULE-70).
    pub(crate) fn record(&mut self, page: u16, generation: u16, body: &[u8]) {
        if self.tables.get(&page).is_some_and(|kept| kept.generation == generation) {
            return;
        }
        let scratch = self.scratch.get_or_insert_with(|| Box::new(Table::new()));
        let said = match neoteletext_rules::decode(body, scratch) {
            Ok(header) if header.has_table => Said::Table(body.to_vec()),
            Ok(_) => Said::NoTable,
            Err(_) => Said::Refused,
        };
        self.arrivals = self.arrivals.saturating_add(1);
        self.tables.insert(page, Kept { generation, said, arrival: self.arrivals });
        // The table that arrived longest ago goes, never the open page's.
        while self.tables.len() > MAX_KEPT_TABLES {
            let oldest = self.tables.iter().filter(|(held, _)| Some(**held) != self.page).min_by_key(|(_, kept)| kept.arrival).map(|(held, _)| *held);
            match oldest {
                Some(oldest) => self.tables.remove(&oldest),
                None => break,
            };
        }
    }

    /// The page and generation an evaluator is wanted for: the open page
    /// of the active tab and what the channel last said of its table.
    fn wanted(&self) -> Option<(u16, u16)> {
        self.page.and_then(|page| self.tables.get(&page).map(|kept| (page, kept.generation)))
    }

    /// The decoder's time at which the open page's next tick is due, or 0,
    /// at once, while a page or a table that changed has not been settled
    /// or something waits to be told. `None` while no open page has a
    /// table: the decoder then keeps no wake for rules (RULE-58).
    pub(crate) fn due_us(&self) -> Option<u64> {
        if self.wanted() != self.settled || !self.out.is_empty() {
            return Some(0);
        }
        self.open.as_ref().map(|open| open.start_us.saturating_add(open.clock.next_due()))
    }

    /// An input event of the viewer, which waits for the next tick
    /// (RULE-5 step 2).
    pub(crate) fn input(&mut self, page: u16, sequence: u32, input: RuleInput, picture: &dyn Picture) {
        let Some(open) = self.open.as_mut().filter(|open| open.page == page) else { return };
        let event = event(sequence, input, open, picture);
        // A full queue drops the event, which the evaluator counts (RULE-5).
        let _ = open.evaluator.push(event);
    }

    /// The newest metered report (RULE-33 item 5): taken only at the
    /// length of the watch list told.
    pub(crate) fn metered(&mut self, page: u16, values: &[u8]) {
        if let Some(open) = self.open.as_mut().filter(|open| open.page == page && open.told.watch.len() == values.len()) {
            open.evaluator.set_metered(values);
        }
    }

    /// The animation of the open page's picture held `count` repaints
    /// back for its budget since it was last asked (ANIM-16): told as
    /// fault 8, the animation slowed, with the other faults, at most once
    /// a second (RULE-48).
    pub(crate) fn animation_slowed(&mut self, page: u16, count: u32) {
        if let Some(open) = self.open.as_mut().filter(|open| open.page == page && count != 0) {
            open.fault(FAULT_ANIMATION_SLOWED, count);
        }
    }

    /// A state query (RULE-47): answered once, at the end of the next
    /// tick, for the open page; at most one a second, the rest dropped and
    /// reported as fault 6. A query for a page that is not open is
    /// ignored.
    pub(crate) fn query(&mut self, page: u16, identifier: u32, now_us: u64) {
        if self.page != Some(page) {
            return;
        }
        if let Some(open) = self.open.as_mut().filter(|open| open.page == page) {
            let tick = open.evaluator.page_tick();
            if open.answered.is_some_and(|answered| tick.saturating_sub(answered) < QUERY_INTERVAL_TICKS) {
                open.fault(FAULT_QUERY_DROPPED, 1);
                return;
            }
            open.answered = Some(tick);
            open.evaluator.query(identifier);
            return;
        }
        // The page has no table: generation 0 and nothing else (RULE-47).
        let second_us = neoteletext_core::consts::presentation_tick_due_us(QUERY_INTERVAL_TICKS);
        if self.plain_answered_us.is_some_and(|answered| now_us.saturating_sub(answered) < second_us) {
            return;
        }
        self.plain_answered_us = Some(now_us);
        self.out.extend(page_state(page, &PageState { identifier, table: None }));
    }

    /// Stops the evaluator of the page that was open: the events still
    /// waiting are followed in one tick (RULE-8: the releases the trusted
    /// core gave for what was held), then the final tick runs its `close`
    /// rules (RULE-5), every local sound of the view ends (RULE-60), and
    /// the trusted core is told the page asks for nothing.
    fn close<const OBJECTS: usize, const RECORDS: usize>(&mut self, parts: &mut Parts<'_, OBJECTS, RECORDS>) {
        let Some(mut open) = self.open.take() else { return };
        let mut urgent = Vec::new();
        {
            let mut world = PageWorld { parts, page: open.page, generation: open.generation, hovered: open.hovered, urgent: &mut urgent, out: &mut self.out };
            open.evaluator.tick(&mut world);
            open.evaluator.close(&mut world);
        }
        self.out.splice(0..0, urgent);
        let locals: Vec<u8> = parts.heard.locals().collect();
        for channel in locals {
            let control = Control {
                channel,
                sequence: 0,
                operation: ControlOperation::Stop,
                handle: 0,
                generation: 0,
                gain: 0,
                fade_ms: neoteletext_audio::MIN_FADE_MS,
                flags: 0,
                duck: 0,
                page: Some(open.page),
            };
            parts.audio.rule_command(&control, &mut Outbox::new(parts.sounds, parts.heard));
        }
        self.out.push(FromDecoder::RulesOpen(RulesOpen { page: open.page, generation: 0, interest: 0, watch: Vec::new(), wheel: Vec::new() }));
    }

    /// Makes the evaluator of the open page of the active tab, when the
    /// channel sent that page a table (RULE-58), or stops the one running
    /// when the page or its table changed. Page tick 0 is due at the first
    /// tick of the presentation clock after now (RULE-4).
    pub(crate) fn settle<const OBJECTS: usize, const RECORDS: usize>(&mut self, now_us: u64, parts: &mut Parts<'_, OBJECTS, RECORDS>) {
        let wanted = self.wanted();
        if wanted == self.settled {
            return;
        }
        self.settled = wanted;
        self.close(parts);
        let Some((page, generation)) = wanted else { return };
        let Some(kept) = self.tables.get(&page) else { return };
        let body = match &kept.said {
            Said::Table(body) => body,
            Said::NoTable => return,
            Said::Refused => {
                self.out.push(FromDecoder::RulesFault { page, generation, fault: FAULT_UNDECODABLE, count: 1 });
                return;
            }
        };
        let scratch = self.scratch.get_or_insert_with(|| Box::new(Table::new()));
        if neoteletext_rules::decode(body, scratch).is_err() {
            return;
        }
        let Ok(evaluator) = Evaluator::new(scratch, generation, parts.sub) else { return };
        let tick = neoteletext_core::consts::presentation_tick_at_us(now_us).saturating_add(1);
        let mut open = Open {
            page,
            generation,
            evaluator: Box::new(evaluator),
            clock: Clock::new(),
            start_us: neoteletext_core::consts::presentation_tick_due_us(tick),
            told: RulesOpen { page, generation, interest: 0, watch: Vec::new(), wheel: Vec::new() },
            hovered: 0,
            pointer: (0, 0, 0, 0),
            answered: None,
            reported: Counters::default(),
            faults: [0; FAULT_SLOTS],
            fault_told: [None; FAULT_SLOTS],
            counters_told: None,
        };
        open.told = open.asks(parts.picture.size());
        self.out.push(FromDecoder::RulesOpen(open.told.clone()));
        self.open = Some(open);
    }

    /// The page that is the open page of the active tab changed.
    pub(crate) fn set_page(&mut self, page: Option<u16>) {
        self.page = page;
    }

    /// Evaluates each tick due at `now_us` (RULE-4), at most
    /// `MAX_CATCH_UP_TICKS` in one wake, and gives everything produced for
    /// the trusted core since the last call: a rule's MIDI messages first
    /// (RULE-64), then what was produced outside a tick, then the reports
    /// and the rest in the order the actions ran.
    pub(crate) fn run<const OBJECTS: usize, const RECORDS: usize>(&mut self, now_us: u64, parts: &mut Parts<'_, OBJECTS, RECORDS>) -> Vec<FromDecoder> {
        self.settle(now_us, parts);
        let mut urgent = Vec::new();
        let mut out = core::mem::take(&mut self.out);
        if let Some(open) = self.open.as_mut().filter(|open| now_us >= open.start_us) {
            let due = open.clock.wake(now_us.saturating_sub(open.start_us));
            open.evaluator.set_subpage(parts.sub);
            for _ in 0..due {
                let mut world =
                    PageWorld { parts: &mut *parts, page: open.page, generation: open.generation, hovered: open.hovered, urgent: &mut urgent, out: &mut out };
                open.evaluator.tick(&mut world);
            }
            if due != 0 {
                open.count_faults();
                open.tell_faults(&mut out);
                open.tell_counters(&mut out);
                // The watch list is the table's and never changes; the
                // wheel rectangles follow the rules enabled and the
                // targets' places (RULE-51 item 4).
                let asks = open.asks(parts.picture.size());
                if asks != open.told {
                    open.told = asks;
                    out.push(FromDecoder::RulesOpen(open.told.clone()));
                }
            }
        }
        urgent.append(&mut out);
        urgent
    }
}
