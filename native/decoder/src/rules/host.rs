//! The evaluator's host (page-rules.md RULE-58): the one table of the
//! view's selected page and sub-code, its evaluator and clock, and the
//! engine's `World` for it.
//!
//! A page is a page number and a sub-code together (RULE-112, DEC-6599):
//! the host holds at most one table, that of one page and sub-code, and
//! nothing of it is shared with another sub-code of the same number. A page
//! ends or rests in exactly the four ways of RULE-111: closed while the
//! connection stands (another page or sub-code is opened: the one `close`
//! tick, then everything dropped), the connection ended (everything dropped
//! at once, no `close` tick), an empty RULES record (the table and its rule
//! state dropped), and paused (the tab is not the active one: the state is
//! kept, no tick runs, no input is taken, and it goes on at the next page
//! tick without `open`).
//!
//! Every number a table gives is untrusted when it is used (RULE-71): the
//! engine's decoder refuses a table whole or accepts it within every limit
//! (RULE-70), the evaluator works in fixed arrays, and a command that names
//! a sound or a target that is not there is a counted no-op here.

use neoteletext_audio::receiver::Receiver;
use neoteletext_audio::record::{Control, Operation as ControlOperation, Record};
use neoteletext_rules::clock::Clock;
use neoteletext_rules::command::{self, Command, TYPE_ANIMATE, TYPE_CONTROL, TYPE_LOCAL_MIDI, TYPE_NAPLPS_OBJECTS, TYPE_SPRITES};
use neoteletext_rules::evaluator::{Bounds, Counters, Input, InputEvent, KeyAction, KeyIdentity, PointerAction, Report, Reported, SoundState, TargetState};
use neoteletext_rules::limits::{MAX_POINTER_OBJECTS, MAX_RECTS, MAX_STATE_BYTES, TICKS_PER_SECOND};
use neoteletext_rules::state::PageState;
use neoteletext_rules::table::{SPACE_SYNTH, Table, Target};
use neoteletext_rules::{Evaluator, World};
use neoteletext_web::records::rules::{Assembled, Assembler, RulesRecord};
use protocol::audio::{Audio, WatchSlot};
use protocol::decoder::FromDecoder;
use protocol::rules::{PageSub, QUERY_INTERVAL_TICKS, RuleCounters, RuleInput, RulesOpen, WheelRect};

use super::picture::Picture;
use crate::voice_sink::{Heard, Outbox};

/// The fault codes a decoder reports (RULE-48).
const FAULT_UNDECODABLE: u8 = 1;
const FAULT_EVENTS_DROPPED: u8 = 4;
const FAULT_COMMANDS_SKIPPED: u8 = 5;
const FAULT_QUERY_DROPPED: u8 = 6;
const FAULT_ANIMATION_SLOWED: u8 = 8;
const FAULT_EVALUATION_LATE: u8 = 9;
const FAULT_QUEUE_FULL: u8 = 10;
const _: () = assert!(FAULT_QUEUE_FULL == protocol::rules::MAX_FAULT);
/// One more than the highest fault code: a place for each code's count.
const FAULT_SLOTS: usize = protocol::rules::MAX_FAULT as usize + 1;
/// The bytes a CONTROL record's frame adds to its body: what the audio
/// profile's reader takes (the engine's REQ-AUD-003).
const CONTROL_FRAME_BYTES: usize = neoteletext_audio::RECORD_HEADER + neoteletext_audio::RECORD_CRC;
/// Microseconds in a second.
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;

/// A picture command a group gave the world (RULE-118: `ANIMATE`,
/// `SPRITES`, `NAPLPS_OBJECTS`), with its amounts resolved: applied by the
/// engine with the code that applies the same record from the server.
pub(crate) type PictureCommand = (u8, Vec<u8>);

/// The one table the host holds and the evaluator running it.
struct Held {
    page: PageSub,
    generation: u16,
    evaluator: Box<Evaluator>,
    /// Which ticks are due, counted from `start_us`.
    clock: Clock,
    /// The decoder's time at which page tick 0 is due: the first tick of
    /// the presentation clock after the page was opened (RULE-4); `None`
    /// until the page is first the open page.
    start_us: Option<u64>,
    /// The page is selected but not the open page (RULE-111 case 4).
    paused: bool,
    /// When the pause began: the clock's start moves on by the time
    /// paused, so the page tick does not advance meanwhile (RULE-4).
    paused_at_us: Option<u64>,
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
    /// Ticks whose evaluation took longer than their own time (RULE-4,
    /// fault 9), and the longest one tick took, in microseconds.
    late_ticks: u32,
    longest_tick_us: u32,
}

/// The engine's pieces the evaluator's world acts on.
pub(crate) struct Parts<'a, const OBJECTS: usize, const RECORDS: usize> {
    /// The audio receiver, which carries out a group's `CONTROL`.
    pub audio: &'a mut Receiver<'static, OBJECTS, RECORDS>,
    /// The voice operations waiting to be sent.
    pub sounds: &'a mut Vec<Audio>,
    /// What plays on each audio channel.
    pub heard: &'a mut Heard,
    /// The open page's picture.
    pub picture: &'a mut dyn Picture,
}

/// The view's rule table and its evaluator.
#[derive(Default)]
pub(crate) struct Rules {
    /// The page and sub-code that is the open page of the active tab
    /// (RULE-58), or none: then the held page, if any, is paused.
    open: Option<PageSub>,
    held: Option<Held>,
    /// The fragments of a RULES record being assembled (RULE-41).
    assembler: Option<Box<Assembler>>,
    /// What was produced outside a tick, for the trusted core.
    out: Vec<FromDecoder>,
    /// The picture commands of the last evaluation, for the engine.
    picture: Vec<PictureCommand>,
    /// The pages closed or dropped since the engine last asked: their
    /// local state is the engine's to drop too (RULE-111).
    dropped: Vec<PageSub>,
    /// Room to decode a table into: 110 KB, kept off the stack.
    scratch: Option<Box<Table>>,
    /// When a query for a page with no table was last answered.
    plain_answered_us: Option<u64>,
    /// Records for a page with no table, or not the open page, that need
    /// its evaluator, dropped and counted (RULE-50).
    commands_dropped: u32,
}

/// The evaluator's world: the picture, the sounds, and where its outputs go.
struct PageWorld<'a, 'b, const OBJECTS: usize, const RECORDS: usize> {
    parts: &'a mut Parts<'b, OBJECTS, RECORDS>,
    page: PageSub,
    generation: u16,
    hovered: u16,
    /// A group's sound messages, which leave before everything else
    /// (RULE-64).
    urgent: &'a mut Vec<FromDecoder>,
    out: &'a mut Vec<FromDecoder>,
    picture: &'a mut Vec<PictureCommand>,
}

/// A `CONTROL` body read as the audio profile reads the record (the
/// engine's REQ-AUD-005): framed as the server frames it and parsed.
fn control_of(body: &[u8]) -> Option<Control> {
    let mut record = [0_u8; neoteletext_audio::record::CONTROL_BYTES + CONTROL_FRAME_BYTES];
    let length = neoteletext_audio::record::write(TYPE_CONTROL, body, &mut record)?;
    match neoteletext_audio::record::parse(record.get(..length)?) {
        Ok(Record::Control(control)) => Some(control),
        _ => None,
    }
}

impl<const OBJECTS: usize, const RECORDS: usize> PageWorld<'_, '_, OBJECTS, RECORDS> {
    /// What the audio receiver sent of a group's command leaves at once,
    /// with the group's other sound messages (RULE-96).
    fn sounds_now(&mut self) {
        self.urgent.extend(self.parts.sounds.drain(..).map(FromDecoder::Audio));
    }

    /// A `CONTROL` of a group (RULE-118): carried out as the record from
    /// the server is, for this page's number (RULE-112: the record names
    /// a page number only). Whether it did anything.
    fn control(&mut self, body: &[u8]) -> bool {
        let Some(mut control) = control_of(body) else { return false };
        control.page = Some(self.page.page);
        let before = self.parts.heard.state(control.channel);
        self.parts.audio.rule_command(&control, &mut Outbox::new(self.parts.sounds, self.parts.heard));
        self.sounds_now();
        let after = self.parts.heard.state(control.channel);
        match control.operation {
            ControlOperation::Play => after.playing && after.handle == control.handle,
            _ => before.playing,
        }
    }

    /// A `LOCAL_MIDI` (RULE-21): only to a local sound, never to one whose
    /// blocks the server sends.
    fn local_midi(&mut self, body: &[u8]) -> bool {
        let Ok((_, _, Command::LocalMidi { channel, status, data1, data2 })) = command::decode(TYPE_LOCAL_MIDI, body) else { return false };
        if !self.parts.heard.is_local(channel) {
            return false;
        }
        self.parts.heard.midi(channel, status, data1, data2);
        // After the sound's own start, when that waits to be sent.
        self.sounds_now();
        self.urgent.push(FromDecoder::Audio(Audio::LocalMidi { channel, status, data1, data2 }));
        true
    }

    /// A picture command (RULE-118): kept for the engine, which applies it
    /// as the same record from the server. Whether the target of an
    /// `ANIMATE`, or of one of its operations, is in the picture.
    fn picture_command(&mut self, kind: u8, body: &[u8]) -> bool {
        let applies = kind != TYPE_ANIMATE
            || neoteletext_rules::animate::read_record(body).is_ok_and(|(_, operations)| {
                neoteletext_rules::animate::operations(operations).any(|operation| self.parts.picture.target(operation.target).is_some())
            });
        if applies {
            self.picture.push((kind, body.to_vec()));
        }
        applies
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
        let mut state = self.parts.heard.state(channel);
        // A synthesizer sound played from the server's blocks is rendered
        // by the receiver, which holds its family and variant (RULE-28).
        if state.playing
            && !state.local
            && let Some(entry) = self.parts.audio.store().entry(state.handle)
        {
            (state.family, state.variant) = family_of(entry);
        }
        state
    }

    fn synth(&self, channel: u8, part: u8, class: u8, index: u16) -> i32 {
        if self.parts.heard.is_local(channel) {
            return self.parts.heard.synth(channel, part, class, index);
        }
        // RULE-30: of a sound whose blocks the server sends, the tracker
        // of the instrument the receiver renders it with, at the position
        // rendered; 0 where no such sound plays there.
        self.parts.audio.tracked(channel, class, part, index).unwrap_or(0)
    }

    /// Each command of a run the evaluator does not carry out itself
    /// (RULE-56, RULE-118), applied by the code that applies the same
    /// record from the server.
    fn command(&mut self, kind: u8, body: &[u8], _tick: u32) -> bool {
        match kind {
            TYPE_CONTROL => self.control(body),
            TYPE_LOCAL_MIDI => self.local_midi(body),
            TYPE_ANIMATE | TYPE_SPRITES | TYPE_NAPLPS_OBJECTS => self.picture_command(kind, body),
            _ => false,
        }
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
            row: report.row,
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

/// The family (its codec code) and variant of a stored sound as a rule
/// names them (RULE-28): the chip is synth6581's variant, 0 or 1, and the
/// OBJECT record's variant that of a family with several.
fn family_of(entry: &neoteletext_audio::store::Entry) -> (u8, u8) {
    let variant = match entry.codec {
        neoteletext_audio::Codec::Synth6581 => u8::from(entry.setting & neoteletext_audio::record::SYNTH6581_CHIP_8580 != 0),
        codec if codec.variants() > 1 => entry.setting,
        _ => 0,
    };
    (entry.codec.code(), variant)
}

/// The audio channels whose synthesizer values the open page's table
/// reads (RULE-28), which the receiver is asked to watch (RULE-30).
fn synth_channels(evaluator: &Evaluator) -> impl Iterator<Item = u8> + '_ {
    evaluator.index().values.iter().filter(|value| value.space == SPACE_SYNTH).map(|value| value.a)
}

/// A state answer as the trusted core takes it (RULE-47).
fn page_state(page: PageSub, state: &PageState) -> Option<FromDecoder> {
    let mut body = [0_u8; MAX_STATE_BYTES];
    let length = neoteletext_rules::state::encode(state, &mut body).ok()?;
    Some(FromDecoder::PageState { page, identifier: state.identifier, body: body.get(..length)?.to_vec() })
}

/// What the trusted core is told when a page has no table, or is not the
/// open page any more.
const fn nothing_asked(page: PageSub) -> RulesOpen {
    RulesOpen { page, generation: 0, interest: 0, watch: Vec::new(), wheel: Vec::new() }
}

/// The evaluator's input event for what the trusted core gave.
fn event(sequence: u32, input: RuleInput, held: &mut Held, picture: &dyn Picture) -> InputEvent {
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
                held.hovered = 0;
            }
            if action == PointerAction::Move {
                held.pointer = (x, y, button, control);
            } else {
                held.pointer = (x, y, held.pointer.2, control);
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
                held.hovered = drawing;
            } else if held.hovered == drawing {
                held.hovered = 0;
            }
            let (x, y, button, control) = held.pointer;
            Input::Pointer { action: PointerAction::Move, button, x, y, control }
        }
    };
    InputEvent { sequence, input }
}

impl Held {
    /// What the table asks for now: its interest mask, its watch list and
    /// where a `wheel` rule applies (RULE-52 item 1, RULE-33 item 1,
    /// RULE-51 item 4). A paused page asks for nothing.
    fn asks<const OBJECTS: usize, const RECORDS: usize>(&self, parts: &mut Parts<'_, OBJECTS, RECORDS>) -> RulesOpen {
        if self.paused {
            return nothing_asked(self.page);
        }
        let index = self.evaluator.index();
        let watch = index.watch.iter().map(|slot| WatchSlot { channel: slot.channel, kind: slot.kind, part: slot.part }).collect();
        let empty = Bounds { left: 0, top: 0, right: 0, bottom: 0 };
        let mut bounds = [empty; MAX_RECTS];
        let picture = parts.picture.size();
        let (mut urgent, mut out, mut commands) = (Vec::new(), Vec::new(), Vec::new());
        let world = PageWorld {
            parts,
            page: self.page,
            generation: self.generation,
            hovered: self.hovered,
            urgent: &mut urgent,
            out: &mut out,
            picture: &mut commands,
        };
        let count = self.evaluator.wheel_bounds(&world, &mut bounds);
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

    /// What the evaluator counted since it was last asked that is a fault:
    /// events dropped (4), commands skipped (5), and what a full queue or
    /// list dropped (10).
    fn count_faults(&mut self) {
        let now = *self.evaluator.counters();
        let before = self.reported;
        self.fault(FAULT_EVENTS_DROPPED, now.events_dropped.saturating_sub(before.events_dropped));
        self.fault(FAULT_COMMANDS_SKIPPED, now.skipped.saturating_sub(before.skipped));
        let full = now
            .not_armed
            .saturating_sub(before.not_armed)
            .saturating_add(now.commands_dropped.saturating_sub(before.commands_dropped))
            .saturating_add(now.cues_dropped.saturating_sub(before.cues_dropped));
        self.fault(FAULT_QUEUE_FULL, full);
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

    /// The counts for the console, once a second (RULE-72, REQ-6626).
    fn tell_counters(&mut self, out: &mut Vec<FromDecoder>) {
        let tick = self.evaluator.page_tick();
        if self.counters_told.is_some_and(|told| tick.saturating_sub(told) < TICKS_PER_SECOND) {
            return;
        }
        self.counters_told = Some(tick);
        out.push(FromDecoder::RuleCounters(self.counters()));
    }

    /// The evaluator's counts and the host's own (RULE-72).
    fn counters(&self) -> RuleCounters {
        let counters = self.evaluator.counters();
        RuleCounters {
            page: self.page,
            ticks: counters.ticks,
            fired: counters.fired,
            groups: counters.groups,
            commands: counters.commands,
            skipped: counters.skipped,
            events_dropped: counters.events_dropped,
            armed: counters.armed,
            armed_late: counters.armed_late,
            not_armed: counters.not_armed,
            cues: counters.cues,
            cues_dropped: counters.cues_dropped,
            reports: counters.reports,
            late_ticks: self.late_ticks,
            longest_tick_us: self.longest_tick_us,
            variables: *self.evaluator.variables(),
        }
    }
}

impl Rules {
    /// The page and sub-code whose table is held, and whether it is the
    /// open page now, not paused.
    pub(crate) fn held_page(&self) -> Option<(PageSub, bool)> {
        self.held.as_ref().map(|held| (held.page, !held.paused && self.open == Some(held.page)))
    }

    /// The held page's counts.
    #[cfg(test)]
    pub(crate) fn counters(&self) -> Option<RuleCounters> {
        self.held.as_ref().map(Held::counters)
    }

    /// The records dropped for a page with no table or not open (RULE-50).
    #[cfg(test)]
    pub(crate) const fn commands_dropped(&self) -> u32 {
        self.commands_dropped
    }

    /// The page and sub-code that is the open page of the active tab, as
    /// last told.
    pub(crate) const fn open(&self) -> Option<PageSub> {
        self.open
    }

    /// The picture commands of the last evaluation, taken.
    pub(crate) fn take_picture(&mut self) -> Vec<PictureCommand> {
        core::mem::take(&mut self.picture)
    }

    /// The pages closed or dropped since the last call, whose local state
    /// the engine drops too (RULE-111).
    pub(crate) fn take_dropped(&mut self) -> Vec<PageSub> {
        core::mem::take(&mut self.dropped)
    }

    /// The held page's evaluator while it is live: the open page, not
    /// paused, and opened.
    fn live(&mut self) -> Option<&mut Held> {
        let open = self.open;
        self.held.as_mut().filter(|held| !held.paused && open == Some(held.page) && held.start_us.is_some())
    }

    /// A RULES record arrived (RULE-41): its fragments are assembled, and
    /// a whole image replaces the page's table when its generation
    /// differs. An image that cannot be assembled or decoded leaves the
    /// page with no table, and is reported as fault 1 (RULE-70). A record
    /// for a page that is not the open page, while one is open, is
    /// dropped: the host holds one table, the selected page's (RULE-58).
    pub(crate) fn record<const OBJECTS: usize, const RECORDS: usize>(
        &mut self,
        record: &RulesRecord<'_>,
        now_us: u64,
        parts: &mut Parts<'_, OBJECTS, RECORDS>,
    ) {
        let page = PageSub::new(record.page.mpp(), record.subcode.value());
        if self.open.is_some_and(|open| open != page) {
            self.commands_dropped = self.commands_dropped.saturating_add(1);
            return;
        }
        let assembler = self.assembler.get_or_insert_with(|| Box::new(Assembler::new()));
        match assembler.push(record) {
            Assembled::Incomplete => {}
            Assembled::Removed { .. } => {
                // RULE-111: the table and its rule state go; the page's
                // sounds and picture stay, for it is still the page shown.
                if self.held.as_ref().is_some_and(|held| held.page == page) {
                    self.held = None;
                    forget_watches(parts);
                    self.out.push(FromDecoder::RulesOpen(nothing_asked(page)));
                }
            }
            Assembled::OutOfPlace { generation, .. } => self.refused(page, generation, parts),
            Assembled::Whole { generation, image, .. } => {
                if self.held.as_ref().is_some_and(|held| held.page == page && held.generation == generation) {
                    return;
                }
                let scratch = self.scratch.get_or_insert_with(|| Box::new(Table::new(page.page, page.sub)));
                let decoded = neoteletext_rules::decode(image, page.page, page.sub, neoteletext_web::records::body_check, scratch);
                let evaluator = decoded.and_then(|()| Evaluator::new(scratch, generation, neoteletext_web::records::body_check));
                match evaluator {
                    Ok(evaluator) => self.take_table(page, generation, Box::new(evaluator), now_us, parts),
                    Err(_) => self.refused(page, generation, parts),
                }
            }
        }
    }

    /// A table refused whole (RULE-70): the page has no table, and fault 1
    /// is reported.
    fn refused<const OBJECTS: usize, const RECORDS: usize>(&mut self, page: PageSub, generation: u16, parts: &mut Parts<'_, OBJECTS, RECORDS>) {
        if self.held.as_ref().is_some_and(|held| held.page == page) {
            self.held = None;
            forget_watches(parts);
            self.out.push(FromDecoder::RulesOpen(nothing_asked(page)));
        }
        let generation = generation.max(neoteletext_rules::limits::FIRST_GENERATION);
        self.out.push(FromDecoder::RulesFault { page, generation, fault: FAULT_UNDECODABLE, count: 1 });
    }

    /// A decoded table becomes the page's (RULE-2): what the page held
    /// before is dropped as opening the page drops it, and the new table's
    /// `open` rules fire at its page tick 0. A table for another page than
    /// the one held closes that one first.
    fn take_table<const OBJECTS: usize, const RECORDS: usize>(
        &mut self,
        page: PageSub,
        generation: u16,
        evaluator: Box<Evaluator>,
        now_us: u64,
        parts: &mut Parts<'_, OBJECTS, RECORDS>,
    ) {
        if self.held.as_ref().is_some_and(|held| held.page != page) {
            self.close(parts);
        }
        self.held = Some(Held {
            page,
            generation,
            evaluator,
            clock: Clock::new(),
            start_us: None,
            paused: self.open != Some(page),
            paused_at_us: None,
            told: nothing_asked(page),
            hovered: 0,
            pointer: (0, 0, 0, 0),
            answered: None,
            reported: Counters::default(),
            faults: [0; FAULT_SLOTS],
            fault_told: [None; FAULT_SLOTS],
            counters_told: None,
            late_ticks: 0,
            longest_tick_us: 0,
        });
        self.settle(now_us, parts);
    }

    /// Starts the held page's clock when it is the open page and has not
    /// started, and tells the trusted core what it asks for when that
    /// changed. Page tick 0 is due at the first tick of the presentation
    /// clock after now (RULE-4).
    fn settle<const OBJECTS: usize, const RECORDS: usize>(&mut self, now_us: u64, parts: &mut Parts<'_, OBJECTS, RECORDS>) {
        let open = self.open;
        let Some(held) = self.held.as_mut() else { return };
        held.paused = open != Some(held.page);
        if !held.paused && held.start_us.is_none() {
            let tick = neoteletext_core::consts::presentation_tick_at_us(now_us).saturating_add(1);
            held.start_us = Some(neoteletext_core::consts::presentation_tick_due_us(tick));
            // The receiver keeps what the table reads of a sound played
            // from blocks only for the channels it reads (RULE-30).
            for channel in synth_channels(&held.evaluator) {
                parts.audio.watch(channel, true);
            }
        }
        let asks = held.asks(parts);
        if asks != held.told {
            held.told = asks;
            self.out.push(FromDecoder::RulesOpen(held.told.clone()));
        }
    }

    /// RULE-111 case 1: the page is closed while its connection stands. Its
    /// final tick runs its `close` rules (RULE-5), whose reports leave
    /// (RULE-52 item 4); then every local sound of the page ends and
    /// everything local is dropped.
    fn close<const OBJECTS: usize, const RECORDS: usize>(&mut self, parts: &mut Parts<'_, OBJECTS, RECORDS>) {
        let Some(mut held) = self.held.take() else { return };
        if held.start_us.is_some() {
            let mut urgent = Vec::new();
            {
                let mut world = PageWorld {
                    parts,
                    page: held.page,
                    generation: held.generation,
                    hovered: held.hovered,
                    urgent: &mut urgent,
                    out: &mut self.out,
                    picture: &mut self.picture,
                };
                held.evaluator.close(&mut world);
            }
            self.out.splice(0..0, urgent);
        }
        self.drop_page(held.page, parts);
    }

    /// Everything local of `page` goes (RULE-111): its local sounds, with
    /// every note released, its watches, and, by the engine, its picture's
    /// animation state.
    fn drop_page<const OBJECTS: usize, const RECORDS: usize>(&mut self, page: PageSub, parts: &mut Parts<'_, OBJECTS, RECORDS>) {
        forget_watches(parts);
        parts.audio.end_local_sounds(&mut Outbox::new(parts.sounds, parts.heard));
        self.out.extend(parts.sounds.drain(..).map(FromDecoder::Audio));
        self.out.push(FromDecoder::RulesOpen(nothing_asked(page)));
        self.dropped.push(page);
    }

    /// The page and sub-code that is the open page of the active tab
    /// changed (RULE-58), or the tab stopped being the active one (none,
    /// the pause of RULE-111 case 4). Another page than the one held closes
    /// it (case 1); the same page goes on from where it was paused, with no
    /// `open`.
    pub(crate) fn set_open<const OBJECTS: usize, const RECORDS: usize>(
        &mut self,
        open: Option<PageSub>,
        now_us: u64,
        parts: &mut Parts<'_, OBJECTS, RECORDS>,
    ) -> Vec<FromDecoder> {
        let mut produced = Vec::new();
        let before = core::mem::replace(&mut self.open, open);
        if before == open {
            return produced;
        }
        let held = self.held.as_ref().map(|held| held.page);
        match (open, held) {
            // Another page is opened in the view: the one held is closed,
            // and so is a page whose table never came.
            (Some(open), _) if held.is_some_and(|held| held != open) || before.is_some_and(|before| before != open) => {
                if held.is_some_and(|held| held != open) {
                    self.close(parts);
                } else if let Some(before) = before.filter(|before| held != Some(*before)) {
                    self.drop_page(before, parts);
                }
                if let Some(assembler) = self.assembler.as_mut() {
                    assembler.clear();
                }
            }
            // The same page is the open page again: it goes on from the
            // next page tick, its clock moved on by the time paused.
            (Some(_), _) => {
                if let Some(held) = self.held.as_mut()
                    && let (Some(start), Some(paused)) = (held.start_us, held.paused_at_us.take())
                {
                    held.start_us = Some(start.saturating_add(now_us.saturating_sub(paused)));
                }
            }
            // Paused: what the lost releases the trusted core gave did is
            // followed in one last tick, and then nothing more is run.
            (None, Some(_)) => {
                produced = self.run(now_us, parts);
                if let Some(held) = self.held.as_mut() {
                    held.paused_at_us = Some(now_us);
                }
            }
            (None, None) => {}
        }
        self.settle(now_us, parts);
        produced.append(&mut self.out);
        produced
    }

    /// RULE-111 case 2: the view's connection ended. Everything local of
    /// the page is dropped at once, with every local sound stopped, and no
    /// `close` tick is run.
    pub(crate) fn connection_ended<const OBJECTS: usize, const RECORDS: usize>(&mut self, parts: &mut Parts<'_, OBJECTS, RECORDS>) {
        if let Some(assembler) = self.assembler.as_mut() {
            assembler.clear();
        }
        let page = self.held.take().map(|held| held.page).or(self.open);
        if let Some(page) = page {
            self.drop_page(page, parts);
        }
    }

    /// When the open page's next tick is due, on the engine's clock, or 0,
    /// at once, while something waits to be told. `None` while no open
    /// page has a table: the decoder then keeps no sixty-a-second wake for
    /// rules (RULE-58).
    pub(crate) fn due_us(&self) -> Option<u64> {
        if !self.out.is_empty() || !self.picture.is_empty() {
            return Some(0);
        }
        let held = self.held.as_ref().filter(|held| !held.paused)?;
        held.start_us.map(|start| start.saturating_add(held.clock.next_due()))
    }

    /// An input event of the viewer for the open page (RULE-52 item 1),
    /// decided on arrival (RULE-96): its sound commands are returned at
    /// once, for the caller to write before any other work.
    pub(crate) fn input<const OBJECTS: usize, const RECORDS: usize>(
        &mut self,
        page: PageSub,
        sequence: u32,
        input: RuleInput,
        parts: &mut Parts<'_, OBJECTS, RECORDS>,
    ) -> Vec<FromDecoder> {
        let mut urgent = Vec::new();
        let open = self.open;
        let Self { held, out, picture, .. } = self;
        let Some(held) = held.as_mut().filter(|held| held.page == page && !held.paused && open == Some(page) && held.start_us.is_some()) else {
            return urgent;
        };
        let event = event(sequence, input, held, &*parts.picture);
        let mut world = PageWorld { parts, page, generation: held.generation, hovered: held.hovered, urgent: &mut urgent, out, picture };
        // A full queue drops the event, which the evaluator counts.
        let _ = held.evaluator.push_now(event, &mut world);
        urgent
    }

    /// The newest metered report (RULE-33 item 5): taken only at the
    /// length of the watch list told.
    pub(crate) fn metered(&mut self, page: PageSub, values: &[u8]) {
        if let Some(held) = self.live().filter(|held| held.page == page && held.told.watch.len() == values.len()) {
            held.evaluator.set_metered(values);
        }
    }

    /// A cue the sound helper heard (RULE-109): for the open page only.
    pub(crate) fn cue(&mut self, audio_channel: u8, number: u8) {
        if let Some(held) = self.live() {
            let _ = held.evaluator.push_cue(audio_channel, number);
        }
    }

    /// A command record the server sent directly (RULE-114: `VARIABLE`,
    /// `TIMER`, `REPORT`, `RUN`, `SEQUENCE`): given to the open page's
    /// evaluator, or dropped and counted for a page with no table or not
    /// open (RULE-50).
    pub(crate) fn direct(&mut self, page: PageSub, kind: u8, body: &[u8]) {
        match self.live().filter(|held| held.page == page) {
            Some(held) => {
                let _ = held.evaluator.push_command(kind, body);
            }
            None => self.commands_dropped = self.commands_dropped.saturating_add(1),
        }
    }

    /// A `LOCAL_MIDI` record the server sent directly (RULE-21, RULE-114):
    /// to the open page's local sound at once, or dropped and counted for
    /// a page with no table or not open (RULE-50).
    pub(crate) fn direct_midi<const OBJECTS: usize, const RECORDS: usize>(
        &mut self,
        page: PageSub,
        body: &[u8],
        parts: &mut Parts<'_, OBJECTS, RECORDS>,
    ) -> Vec<FromDecoder> {
        let mut urgent = Vec::new();
        let open = self.open;
        let Self { held, out, picture, commands_dropped, .. } = self;
        match held.as_mut().filter(|held| held.page == page && !held.paused && open == Some(page)) {
            Some(held) => {
                let tick = held.evaluator.page_tick();
                let mut world = PageWorld { parts, page, generation: held.generation, hovered: held.hovered, urgent: &mut urgent, out, picture };
                let _ = world.command(TYPE_LOCAL_MIDI, body, tick);
            }
            None => *commands_dropped = commands_dropped.saturating_add(1),
        }
        urgent
    }

    /// The animation of the open page's picture held `count` repaints
    /// back for its budget since it was last asked (ANIM-16): told as
    /// fault 8 with the other faults, at most once a second (RULE-48).
    pub(crate) fn animation_slowed(&mut self, count: u32) {
        if let Some(held) = self.live().filter(|_| count != 0) {
            held.fault(FAULT_ANIMATION_SLOWED, count);
        }
    }

    /// A state query (RULE-47): answered once, at the end of the next
    /// tick, for the open page; at most one a second, the rest dropped and
    /// reported as fault 6. A query for a page that is not open, or is
    /// paused, is ignored.
    pub(crate) fn query(&mut self, page: PageSub, identifier: u32, now_us: u64) {
        if self.open != Some(page) {
            return;
        }
        if let Some(held) = self.held.as_mut().filter(|held| held.page == page) {
            if held.paused || held.start_us.is_none() {
                return;
            }
            let tick = held.evaluator.page_tick();
            if held.answered.is_some_and(|answered| tick.saturating_sub(answered) < QUERY_INTERVAL_TICKS) {
                held.fault(FAULT_QUERY_DROPPED, 1);
                return;
            }
            held.answered = Some(tick);
            held.evaluator.query(identifier);
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

    /// Evaluates each tick due at `now_us` (RULE-4), at most
    /// `MAX_CATCH_UP_TICKS` in one wake, and gives everything produced for
    /// the trusted core since the last call: a group's sound messages
    /// first (RULE-64), then what was produced outside a tick, then the
    /// reports and the rest in the order the commands ran. Ticks whose
    /// evaluation took longer than their own time are counted and told as
    /// fault 9 (RULE-4, page-rules.md section 12.4).
    pub(crate) fn run<const OBJECTS: usize, const RECORDS: usize>(&mut self, now_us: u64, parts: &mut Parts<'_, OBJECTS, RECORDS>) -> Vec<FromDecoder> {
        let mut urgent = Vec::new();
        let mut out = core::mem::take(&mut self.out);
        let open = self.open;
        let Self { held, picture, .. } = self;
        if let Some(held) = held.as_mut().filter(|held| !held.paused && open == Some(held.page))
            && let Some(start) = held.start_us.filter(|start| now_us >= *start)
        {
            let due = held.clock.wake(now_us.saturating_sub(start));
            let began = std::time::Instant::now();
            for _ in 0..due {
                let mut world = PageWorld {
                    parts: &mut *parts,
                    page: held.page,
                    generation: held.generation,
                    hovered: held.hovered,
                    urgent: &mut urgent,
                    out: &mut out,
                    picture,
                };
                held.evaluator.tick(&mut world);
            }
            if due != 0 {
                let took_us = u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX);
                let own_us = u64::from(due).saturating_mul(MICROSECONDS_PER_SECOND).checked_div(u64::from(TICKS_PER_SECOND)).unwrap_or(0);
                let per_tick = took_us.checked_div(u64::from(due)).unwrap_or(0);
                held.longest_tick_us = held.longest_tick_us.max(u32::try_from(per_tick).unwrap_or(u32::MAX));
                if took_us > own_us {
                    held.late_ticks = held.late_ticks.saturating_add(due);
                    held.fault(FAULT_EVALUATION_LATE, due);
                }
                held.count_faults();
                held.tell_faults(&mut out);
                held.tell_counters(&mut out);
                // The watch list is the table's and never changes; the
                // wheel rectangles follow the targets' places (RULE-51
                // item 4).
                let asks = held.asks(parts);
                if asks != held.told {
                    held.told = asks;
                    out.push(FromDecoder::RulesOpen(held.told.clone()));
                }
            }
        }
        urgent.append(&mut out);
        urgent
    }
}

/// No table reads the channel's sounds any more (RULE-30).
fn forget_watches<const OBJECTS: usize, const RECORDS: usize>(parts: &mut Parts<'_, OBJECTS, RECORDS>) {
    for channel in 0..neoteletext_audio::CHANNELS {
        parts.audio.watch(channel, false);
    }
}
