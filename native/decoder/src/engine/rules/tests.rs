//! Tests of the decoder as the host of the open page's rule table, second
//! form (neoteletext-common's `docs/page-rules.md`; REQ-6534, REQ-6535,
//! REQ-6615, DEC-6587, DEC-6599).

use neoteletext_audio::encode::{Command as SoundCommand, Sound as Writer, packets};
use neoteletext_audio::record::{FLAG_AFTER_CURRENT, FLAG_LOOP};
use neoteletext_core::consts::presentation_tick_due_us;
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_rules::command::{self, Command, Operand, TYPE_ANIMATE, TYPE_CONTROL, TYPE_LOCAL_MIDI, TYPE_REPORT, TYPE_VARIABLE, VariableOperation};
use neoteletext_rules::table::{
    CLASS_NOTE, Framed, Key, KeyKind, KeyMatch, Node, Operator, Rule, SOUND_HANDLE, SOUND_LOCAL, SOUND_PLAYING, Table, Trigger, Value, default_send_all,
};
use protocol::audio::Audio;
use protocol::decoder::{FromDecoder, ToDecoder};
use protocol::rules::{
    INTEREST_FOCUS, INTEREST_KEYS, INTEREST_MOVES, INTEREST_POINTER_BUTTONS, INTEREST_TEXT, INTEREST_WHEEL, KeyAction, PageSub, Report, Reported, RuleInput,
    RulesOpen, WheelRect,
};

use super::super::tests::{OPUS_FRAME_SAMPLES, OPUS_PACKET, opus_frames};
use crate::Engine;

/// The page number the tests open.
const PAGE: u16 = 0x101;
/// Its first sub-page, the one the tests' tables are for.
const SUB: u16 = 1;
/// The page and sub-code the tests open.
const OPEN: PageSub = PageSub::new(PAGE, SUB);
/// Another sub-code of the same page number: another page (RULE-112).
const OTHER: PageSub = PageSub::new(PAGE, 2);
/// The audio channel the local sound plays on.
const CHANNEL: u8 = 0;
/// The synth106 family's codec code.
const SYNTH106: u8 = 14;
/// A time well after the decoder's start, on a tick.
const T0: u64 = 1_000_000;

/// The time `ticks` ticks after [`T0`].
fn after(ticks: u32) -> u64 {
    T0 + presentation_tick_due_us(ticks)
}

/// Builds a table for `page`: each rule is a trigger, an optional
/// condition and the commands of its group (RULE-3, RULE-118).
struct Builder {
    table: Box<Table>,
}

impl Builder {
    fn new(page: PageSub) -> Self {
        Self { table: Box::new(Table::new(page.page, page.sub)) }
    }

    /// The body of `made` for this table's page.
    fn body(&self, made: &Command) -> Vec<u8> {
        let mut body = [0_u8; neoteletext_rules::limits::MAX_COMMAND_BYTES];
        let length = command::encode(made, self.table.page, self.table.sub, &mut body).unwrap();
        body[..length].to_vec()
    }

    /// A rule on `trigger`, with `condition`, running a group of
    /// `commands`, each its record type and body.
    fn rule(mut self, trigger: Trigger, condition: &[Node], commands: &[(u8, Vec<u8>)]) -> Self {
        let framed: Vec<Framed<'_>> = commands.iter().map(|(kind, body)| Framed { kind: *kind, body, bindings: &[] }).collect();
        let group = self.table.push_group(&framed).unwrap();
        let mut rule = Rule::new(trigger, group);
        rule.condition = neoteletext_rules::table::condition(condition);
        self.table.rules.push(rule).unwrap();
        self
    }

    fn report(&self, value: Option<Value>) -> (u8, Vec<u8>) {
        (TYPE_REPORT, self.body(&Command::Report { value }))
    }

    fn midi(&self, status: u8, data1: u8, data2: u8) -> (u8, Vec<u8>) {
        (TYPE_LOCAL_MIDI, self.body(&Command::LocalMidi { channel: CHANNEL, status, data1, data2 }))
    }

    fn inc(&self, variable: u8) -> (u8, Vec<u8>) {
        (TYPE_VARIABLE, self.body(&Command::Variable { operation: VariableOperation::Inc, variable, operand: Operand::None }))
    }
}

/// A key trigger on the character `character`, going down or coming up.
fn on_key(character: char, up: bool) -> Trigger {
    let keys = KeyMatch::of(&[Key::of(KeyKind::Character, u32::from(character))]);
    if up { Trigger::KeyUp { keys } } else { Trigger::Key { keys, repeat: false } }
}

fn address(page: PageSub) -> (PageAddress, SubCode) {
    (PageAddress::from_mpp(page.page).unwrap(), SubCode::new(page.sub))
}

/// The pushed packets of the RULES records of `table`'s image for its
/// page, or of the record that says `page` has none.
fn rules_records(page: PageSub, generation: u16, table: Option<&Table>) -> Vec<u8> {
    let mut bytes = Vec::new();
    let (address, subcode) = address(page);
    let mut id = 0_u8;
    let mut next = |_| {
        id = id.wrapping_add(1);
        id
    };
    if let Some(table) = table {
        let mut image = vec![0_u8; neoteletext_rules::limits::MAX_TABLE_BYTES];
        let length = neoteletext_rules::encode(table, neoteletext_web::records::body_check, &mut image).unwrap();
        neoteletext_web::records::rules::encode_image(address, subcode, generation, &image[..length], 0, &mut next, &mut |packet| {
            bytes.extend_from_slice(&packet.bytes);
        })
        .unwrap();
    } else {
        let record = neoteletext_web::records::rules::RulesRecord { page: address, subcode, generation, fragment: None };
        neoteletext_web::records::rules::encode(&record, 0, 1, &mut |packet| bytes.extend_from_slice(&packet.bytes)).unwrap();
    }
    bytes
}

/// Feeds the engine the RULES record of `table` for `page`.
fn send_table(engine: &mut Engine, page: PageSub, generation: u16, table: Option<&Table>) {
    engine.feed_whole(true, true, &rules_records(page, generation, table));
}

/// Tells the engine which page is open, at `now`, and what it produced.
fn set_open(engine: &mut Engine, page: Option<PageSub>, now: u64) -> Vec<FromDecoder> {
    engine.set_now_us(now);
    engine.rule_message(ToDecoder::RulesPage(page)).unwrap()
}

/// What the engine produced for an input at `now`.
fn input(engine: &mut Engine, page: PageSub, sequence: u32, given: RuleInput, now: u64) -> Vec<FromDecoder> {
    engine.set_now_us(now);
    engine.rule_message(ToDecoder::RuleInput { page, sequence, input: given }).unwrap()
}

/// The ticks due at `now`.
fn run(engine: &mut Engine, now: u64) -> Vec<FromDecoder> {
    engine.set_now_us(now);
    engine.run_rules(now)
}

/// An engine whose open page is [`OPEN`], with `table` sent for it: what
/// the trusted core was told, and the engine. Page tick 0 runs at
/// `after(1)`.
fn opened(table: &Table) -> (Engine, Vec<FromDecoder>) {
    let mut engine = Engine::new();
    let mut told = set_open(&mut engine, Some(OPEN), T0);
    send_table(&mut engine, OPEN, 1, Some(table));
    told.extend(run(&mut engine, T0));
    (engine, told)
}

/// Starts a local synth106 sound on [`CHANNEL`] for [`PAGE`] (RULE-60),
/// with its bank of one patch, as page-rules.md RULE-61's example does.
fn play_local(engine: &mut Engine) {
    use neoteletext_audio_instruments::synth106::{Model, Patch, interface};
    /// The bank's handle.
    const BANK: u8 = 2;
    let mut patch = [0; interface::MOST_PATCH_BYTES];
    let length = interface::pack_patch(Model::Synth106, &Patch::DEFAULT, 0, 0, &mut patch);
    let bank = Writer::bank(BANK, 1, neoteletext_audio::Codec::Synth106, &patch[..length]).unwrap();
    // The setup: program change to program 0 at tick 0 (MIDI-23).
    let mut local = Writer::local(1, 1, neoteletext_audio::Codec::Synth106, &[0, 0xC0, 0]).unwrap();
    local.banks = vec![BANK];
    let play = SoundCommand {
        channel: CHANNEL,
        sequence: 1,
        operation: 1,
        handle: 1,
        generation: 1,
        gain: 100,
        fade_ms: 5,
        flags: 0,
        duck: 0,
        page: Some(PAGE),
        music: None,
    };
    let mut records = vec![bank.object_record().unwrap()];
    records.extend(bank.data_records());
    records.push(local.object_record().unwrap());
    records.extend(local.data_records());
    records.push(play.record());
    engine.open_page(Some(PAGE));
    engine.feed(&packets(0, 0, &records).0.iter().flat_map(|packet| packet.bytes).collect::<Vec<u8>>());
    engine.finish();
}

fn key(action: KeyAction, character: char) -> RuleInput {
    RuleInput::Key { action, named: false, code: u32::from(character), base: None, mods: 0 }
}

fn reports(messages: &[FromDecoder]) -> Vec<Report> {
    messages.iter().filter_map(|message| if let FromDecoder::Report(report) = message { Some(*report) } else { None }).collect()
}

fn told(messages: &[FromDecoder]) -> Vec<RulesOpen> {
    messages.iter().filter_map(|message| if let FromDecoder::RulesOpen(open) = message { Some(open.clone()) } else { None }).collect()
}

fn midi(messages: &[FromDecoder]) -> Vec<(u8, u8)> {
    messages
        .iter()
        .filter_map(|message| if let FromDecoder::Audio(Audio::LocalMidi { status, data1, .. }) = message { Some((*status, *data1)) } else { None })
        .collect()
}

/// The table that sends every kind of input (RULE-53) for `page`.
fn send_all(page: PageSub, wheel: bool) -> Box<Table> {
    let mut table = Box::new(Table::new(page.page, page.sub));
    default_send_all(wheel, page.page, page.sub, &mut table);
    table
}

/// RULE-58, RULE-52 item 1, RULE-51 item 4, RULE-112 (REQ-6534): when a
/// table is decoded for the open page and sub-code the trusted core is
/// told what it asks for, under that page and sub-code: the default table
/// asks for every class but the wheel, and with its `wheel` rule the wheel
/// over the whole picture. A replacing table is taken whole.
#[test]
fn a_table_for_the_open_page_tells_the_trusted_core_what_it_asks_for() {
    let (mut engine, told_now) = opened(&send_all(OPEN, false));
    let every = INTEREST_KEYS | INTEREST_TEXT | INTEREST_POINTER_BUTTONS | INTEREST_MOVES | INTEREST_FOCUS;
    // REQ-7037: its `keys` rule is on any key, the whole keyboard.
    let every_key = protocol::rules::KeyAsk::EVERY_KEY;
    assert_eq!(told(&told_now), [RulesOpen { page: OPEN, generation: 1, interest: every, watch: vec![], wheel: vec![], keys: vec![every_key] }]);
    assert!(engine.rules_due_us().is_some(), "the evaluator wakes the decoder for its ticks");

    send_table(&mut engine, OPEN, 2, Some(&send_all(OPEN, true)));
    let told_now = told(&run(&mut engine, after(3)));
    let whole = WheelRect { x: 0, y: 0, width: 480, height: 500 };
    assert_eq!(
        told_now.last(),
        Some(&RulesOpen { page: OPEN, generation: 2, interest: every | INTEREST_WHEEL, watch: vec![], wheel: vec![whole], keys: vec![every_key] }),
        "{told_now:?}"
    );
}

/// RULE-50, RULE-58 (REQ-6534): a page with no table evaluates nothing
/// and wakes the decoder for nothing, whether the channel sent no RULES
/// record or one that says the page has none; and a table for a page that
/// is not the open page while one is open is not taken.
#[test]
fn no_table_no_evaluator_and_no_wake() {
    let mut engine = Engine::new();
    set_open(&mut engine, Some(OPEN), T0);
    assert_eq!(run(&mut engine, T0), []);
    assert_eq!(engine.rules_due_us(), None, "no sixty-a-second wake for rules");
    assert_eq!(input(&mut engine, OPEN, 1, key(KeyAction::Press, 'a'), after(1)), []);
    send_table(&mut engine, OPEN, 1, None);
    assert_eq!(run(&mut engine, after(3)), [], "a record that says the page has no table");
    assert_eq!(engine.rules_due_us(), None);
    send_table(&mut engine, OTHER, 1, Some(&send_all(OTHER, false)));
    assert_eq!(run(&mut engine, after(4)), [], "a table for another sub-code of the page is another page's");
    assert_eq!(engine.rules_due_us(), None);
}

/// RULE-70, RULE-48 (REQ-6534): a table the engine's decoder refuses is
/// refused whole: the page has no table, nothing of it is evaluated, and
/// fault 1 is reported.
#[test]
fn a_table_that_cannot_be_decoded_is_refused_whole_and_reported() {
    let table = send_all(OPEN, false);
    let mut image = vec![0_u8; neoteletext_rules::limits::MAX_TABLE_BYTES];
    let length = neoteletext_rules::encode(&table, neoteletext_web::records::body_check, &mut image).unwrap();
    // The image's format byte, which only the table's decoder reads.
    image[0] ^= 0xFF;
    // Written as one fragment by the record's own writer, which leaves the
    // image to the table's decoder: `encode_image` checks it first.
    let mut bytes = Vec::new();
    let (address, subcode) = address(OPEN);
    let total = u16::try_from(length).unwrap();
    let fragment = neoteletext_web::records::rules::Fragment { total, offset: 0, data: &image[..length] };
    let record = neoteletext_web::records::rules::RulesRecord { page: address, subcode, generation: 7, fragment: Some(fragment) };
    neoteletext_web::records::rules::encode(&record, 0, 1, &mut |packet| bytes.extend_from_slice(&packet.bytes)).unwrap();
    let mut engine = Engine::new();
    set_open(&mut engine, Some(OPEN), T0);
    engine.feed_whole(true, true, &bytes);
    assert_eq!(run(&mut engine, T0), [FromDecoder::RulesFault { page: OPEN, generation: 7, fault: 1, count: 1 }]);
    assert_eq!(engine.rules_due_us(), None, "no evaluator runs");
    assert_eq!(input(&mut engine, OPEN, 1, key(KeyAction::Press, 'a'), after(2)), []);
}

/// RULE-22, RULE-52 item 3, RULE-46 (REQ-6534): a group's `REPORT` asks
/// the trusted core for a report that names the input event by its
/// sequence number, with the rule, its row (none) and the page and
/// sub-code, and never carries the event.
#[test]
fn a_report_names_the_input_by_its_sequence_number() {
    let (mut engine, _) = opened(&send_all(OPEN, false));
    assert_eq!(reports(&run(&mut engine, after(1))), [], "page tick 0: nothing happened");
    let mut sent = input(&mut engine, OPEN, 41, key(KeyAction::Press, 'a'), after(1) + 10);
    sent.extend(input(&mut engine, OPEN, 42, RuleInput::Focus, after(1) + 20));
    sent.extend(run(&mut engine, after(2)));
    let sent = reports(&sent);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(sent.iter().all(|report| report.page == OPEN && report.generation == 1 && report.row == neoteletext_rules::limits::NO_RULE));
    let sequences: Vec<Reported> = sent.iter().map(|report| report.what).collect();
    assert_eq!(sequences, [Reported::Input { sequence: 41 }, Reported::Input { sequence: 42 }]);
    let other = input(&mut engine, OTHER, 43, key(KeyAction::Press, 'b'), after(2) + 10);
    assert_eq!(reports(&[other, run(&mut engine, after(3))].concat()), [], "another sub-code's input is not this table's");
}

/// RULE-96, RULE-21, RULE-60, RULE-64, DEC-6580 item 1 (REQ-6535): a key
/// whose rule's group holds a `LOCAL_MIDI` gives the sound helper the
/// note the moment the key is taken, before any tick; with no local sound
/// the command does nothing and is counted as skipped (fault 5).
#[test]
fn a_key_plays_the_local_sound_the_moment_it_arrives() {
    let builder = Builder::new(OPEN);
    let (on, off) = (builder.midi(0x90, 60, 100), builder.midi(0x80, 60, 0));
    let builder = builder.rule(on_key('a', false), &[], &[on]).rule(on_key('a', true), &[], &[off]);
    let (mut engine, told_now) = opened(&builder.table);
    assert!(matches!(told(&told_now).as_slice(), [open] if open.interest == INTEREST_KEYS), "{told_now:?}");
    run(&mut engine, after(1));

    // No local sound yet: the command is a counted no-op (RULE-118).
    let produced = input(&mut engine, OPEN, 1, key(KeyAction::Press, 'a'), after(1) + 100);
    assert_eq!(midi(&produced), [], "{produced:?}");
    input(&mut engine, OPEN, 2, key(KeyAction::Release, 'a'), after(1) + 200);
    let produced: Vec<FromDecoder> = (2..=2 + neoteletext_rules::limits::TICKS_PER_SECOND).flat_map(|tick| run(&mut engine, after(tick))).collect();
    assert!(produced.iter().any(|message| matches!(message, FromDecoder::RulesFault { page: OPEN, fault: 5, .. })), "{produced:?}");

    play_local(&mut engine);
    let sounds = engine.take_sounds();
    assert!(
        matches!(sounds.as_slice(), [Audio::LocalBank { channel: CHANNEL, index: 0, .. }, Audio::Local { channel: CHANNEL, start }]
            if start.family == SYNTH106 && start.setup == [0, 0xC0, 0]),
        "{sounds:?}"
    );
    let tick = 3 + neoteletext_rules::limits::TICKS_PER_SECOND;
    run(&mut engine, after(tick));
    // Between two ticks: the note leaves at once, with no tick run.
    let produced = input(&mut engine, OPEN, 3, key(KeyAction::Press, 'a'), after(tick) + 1_000);
    assert_eq!(produced.first(), Some(&FromDecoder::Audio(Audio::LocalMidi { channel: CHANNEL, status: 0x90, data1: 60, data2: 100 })), "{produced:?}");
    let produced = input(&mut engine, OPEN, 4, key(KeyAction::Release, 'a'), after(tick) + 2_000);
    assert_eq!(midi(&produced), [(0x80, 60)]);
}

/// RULE-30, RULE-27 (REQ-6535): a local sound's tracked values are read
/// in the decoder, from the family's tracker fed with every `LOCAL_MIDI`:
/// `note[60]` reads 1 after the group's note on, with nothing asked of the
/// sound helper; and `local` reads 1 while the sound plays.
#[test]
fn a_local_sounds_notes_are_read_from_its_tracker() {
    let note = Value::synth(CHANNEL, SYNTH106, 0, 0, CLASS_NOTE, 60);
    let local = Value::sound(CHANNEL, SOUND_LOCAL);
    let builder = Builder::new(OPEN);
    let (on, sent_note, sent_local) = (builder.midi(0x90, 60, 100), builder.report(Some(note)), builder.report(None));
    let builder = builder.rule(on_key('a', false), &[], &[on]).rule(Trigger::Becomes, &[Node::compare(note, Operator::Equal, 1)], &[sent_note]).rule(
        Trigger::Becomes,
        &[Node::compare(local, Operator::Equal, 1)],
        &[sent_local],
    );
    let (mut engine, _) = opened(&builder.table);
    run(&mut engine, after(1));
    play_local(&mut engine);
    engine.take_sounds();
    let fired = reports(&run(&mut engine, after(2)));
    assert!(matches!(fired.as_slice(), [Report { rule: 2, what: Reported::Fired, .. }]), "the local sound is seen: {fired:?}");
    let played = input(&mut engine, OPEN, 1, key(KeyAction::Press, 'a'), after(2) + 100);
    assert_eq!(midi(&played), [(0x90, 60)]);
    let mut seen = reports(&run(&mut engine, after(3)));
    seen.extend(reports(&run(&mut engine, after(4))));
    assert!(matches!(seen.as_slice(), [Report { rule: 1, what: Reported::Value { reading: 1, .. }, .. }]), "the note is read from the tracker: {seen:?}");
}

/// RULE-2, RULE-112, RULE-111 case 1 (REQ-6615): two sub-codes of one page
/// number hold separate tables: one for 101/0001 is not evaluated while
/// 101/0002 is open; going from one to the other closes the first, whose
/// `close` rule reports, and opens the second at its page tick 0 with
/// every variable zero; a local sound of the page closed ends.
#[test]
fn a_sub_page_is_a_page_of_its_own() {
    let builder = Builder::new(OPEN);
    let (count, said) = (builder.inc(0), builder.report(Some(Value::variable(0))));
    let close = builder.report(None);
    let first = builder.rule(on_key('a', false), &[], &[count, said]).rule(Trigger::Close, &[], &[close]);
    let (mut engine, _) = opened(&first.table);
    run(&mut engine, after(1));
    play_local(&mut engine);
    engine.take_sounds();
    // DEC-6635: the key's group runs as it is handled, its report with it.
    let counted = reports(&input(&mut engine, OPEN, 1, key(KeyAction::Press, 'a'), after(1) + 10));
    assert!(matches!(counted.as_slice(), [Report { what: Reported::Value { reading: 1, .. }, .. }]), "{counted:?}");

    // The other sub-code is opened: the first page closes.
    let closed = set_open(&mut engine, Some(OTHER), after(3));
    let closing = reports(&closed);
    assert!(matches!(closing.as_slice(), [Report { page: OPEN, rule: 1, what: Reported::Fired, .. }]), "the close rule's report: {closed:?}");
    assert!(closed.contains(&FromDecoder::RulesOpen(RulesOpen { page: OPEN, generation: 0, interest: 0, watch: vec![], wheel: vec![], keys: vec![] })));
    assert!(closed.iter().any(|message| matches!(message, FromDecoder::Audio(Audio::Stop { channel: CHANNEL, .. }))), "its local sound ends: {closed:?}");
    assert_eq!(engine.rules_due_us(), None, "nothing of the first page is evaluated");
    assert_eq!(input(&mut engine, OPEN, 2, key(KeyAction::Press, 'a'), after(3) + 10), [], "nor given input");

    // The second page's own table, which counts from zero.
    let builder = Builder::new(OTHER);
    let (count, said) = (builder.inc(0), builder.report(Some(Value::variable(0))));
    let second = builder.rule(on_key('a', false), &[], &[count, said]);
    send_table(&mut engine, OTHER, 1, Some(&second.table));
    run(&mut engine, after(4));
    run(&mut engine, after(5));
    let counted = reports(&input(&mut engine, OTHER, 3, key(KeyAction::Press, 'a'), after(5) + 10));
    assert!(matches!(counted.as_slice(), [Report { page: OTHER, what: Reported::Value { reading: 1, .. }, .. }]), "{counted:?}");
}

/// RULE-111 case 4, RULE-4 (REQ-6615): a page whose tab is not the active
/// one is paused: it asks the trusted core for nothing, evaluates no tick,
/// takes no input and its page tick does not advance; when it is the open
/// page again it goes on with its variables as they were, and `open` does
/// not fire again.
#[test]
fn a_paused_page_keeps_its_state_and_does_nothing() {
    let builder = Builder::new(OPEN);
    let (count, said, opening) = (builder.inc(0), builder.report(Some(Value::variable(0))), builder.report(None));
    let builder = builder.rule(on_key('a', false), &[], &[count, said]).rule(Trigger::Open, &[], &[opening]);
    let (mut engine, _) = opened(&builder.table);
    let opened_now = reports(&run(&mut engine, after(1)));
    assert!(matches!(opened_now.as_slice(), [Report { rule: 1, tick: 0, .. }]), "{opened_now:?}");
    assert_eq!(reports(&input(&mut engine, OPEN, 1, key(KeyAction::Press, 'a'), after(1) + 10)).len(), 1);
    run(&mut engine, after(2));
    let ticks = engine.rules.counters().unwrap().ticks;

    let paused = set_open(&mut engine, None, after(2) + 10);
    assert!(told(&paused).contains(&RulesOpen { page: OPEN, generation: 0, interest: 0, watch: vec![], wheel: vec![], keys: vec![] }), "{paused:?}");
    assert_eq!(engine.rules_due_us(), None, "no wake while paused");
    assert_eq!(input(&mut engine, OPEN, 2, key(KeyAction::Press, 'a'), after(3)), []);
    assert_eq!(run(&mut engine, after(100)), []);
    assert_eq!(engine.rules.counters().unwrap().ticks, ticks, "no tick ran");

    let resumed = set_open(&mut engine, Some(OPEN), after(200));
    assert!(matches!(told(&resumed).as_slice(), [open] if open.generation == 1 && open.interest == INTEREST_KEYS), "{resumed:?}");
    let mut seen = reports(&input(&mut engine, OPEN, 3, key(KeyAction::Press, 'a'), after(200) + 10));
    seen.extend(reports(&run(&mut engine, after(201))));
    seen.extend(reports(&run(&mut engine, after(202))));
    assert!(
        matches!(seen.as_slice(), [Report { rule: 0, what: Reported::Value { reading: 2, .. }, .. }]),
        "v0 went on from 1, and open did not fire: {seen:?}"
    );
}

/// RULE-58, RULE-111 (REQ-6534), TODO-7214: the table of the page a tab
/// goes to is taken only once the page left is closed or paused. Sent
/// while the page left is still the open page it is another page's and is
/// dropped, and the page then opens with no table; so the trusted core
/// says the page left is no longer open before its channel's answer to a
/// selection or a redirect can arrive. Told so first, the table is held
/// and starts when its page is the open page, with its `open` rule.
#[test]
fn the_table_of_the_page_gone_to_is_taken_once_the_page_left_is_not_open() {
    let next = Builder::new(OTHER);
    let opening = next.report(None);
    let next = next.rule(Trigger::Open, &[], &[opening]);

    // The page left still the open page: the table sent is dropped.
    let (mut engine, _) = opened(&send_all(OPEN, false));
    run(&mut engine, after(1));
    send_table(&mut engine, OTHER, 1, Some(&next.table));
    let mut said = set_open(&mut engine, None, after(2));
    said.extend(set_open(&mut engine, Some(OTHER), after(3)));
    said.extend(run(&mut engine, after(5)));
    assert!(told(&said).iter().all(|open| open.generation == 0), "no table for the page gone to: {said:?}");
    assert_eq!(reports(&said), [], "{said:?}");

    // The page left paused first: the table is held, and opens.
    let (mut engine, _) = opened(&send_all(OPEN, false));
    run(&mut engine, after(1));
    set_open(&mut engine, None, after(2));
    send_table(&mut engine, OTHER, 1, Some(&next.table));
    let mut said = set_open(&mut engine, Some(OTHER), after(3));
    said.extend(run(&mut engine, after(5)));
    assert!(told(&said).iter().any(|open| open.page == OTHER && open.generation == 1), "{said:?}");
    assert!(matches!(reports(&said).as_slice(), [Report { page: OTHER, rule: 0, tick: 0, .. }]), "its open rule fires: {said:?}");
}

/// RULE-111 case 2 (REQ-6615): when the view's connection ends everything
/// of the page is dropped at once: no `close` tick runs and nothing is
/// reported, its local sound stops, and nothing more is evaluated until a
/// table comes again.
#[test]
fn a_connection_that_ends_drops_everything_with_no_close() {
    let builder = Builder::new(OPEN);
    let close = builder.report(None);
    let builder = builder.rule(Trigger::Close, &[], &[close]);
    let (mut engine, _) = opened(&builder.table);
    run(&mut engine, after(1));
    play_local(&mut engine);
    engine.take_sounds();
    engine.set_now_us(after(2));
    let ended = engine.rule_message(ToDecoder::ConnectionEnded).unwrap();
    assert_eq!(reports(&ended), [], "no close tick: {ended:?}");
    assert!(ended.iter().any(|message| matches!(message, FromDecoder::Audio(Audio::Stop { channel: CHANNEL, .. }))), "the local sound stops: {ended:?}");
    assert!(told(&ended).contains(&RulesOpen { page: OPEN, generation: 0, interest: 0, watch: vec![], wheel: vec![], keys: vec![] }));
    assert_eq!(engine.rules_due_us(), None);
}

/// RULE-111, RULE-41 (REQ-6615): a RULES record that says the page has no
/// table drops the table and its rule state; the page then asks for
/// nothing.
#[test]
fn an_empty_rules_record_drops_the_table() {
    let (mut engine, _) = opened(&send_all(OPEN, false));
    run(&mut engine, after(1));
    send_table(&mut engine, OPEN, 2, None);
    let dropped = run(&mut engine, after(2));
    assert!(told(&dropped).contains(&RulesOpen { page: OPEN, generation: 0, interest: 0, watch: vec![], wheel: vec![], keys: vec![] }), "{dropped:?}");
    assert_eq!(engine.rules_due_us(), None);
}

/// RULE-114, RULE-50 (REQ-6534): a `VARIABLE` record the server sends the
/// open page is given to its evaluator; one for another sub-code, or for a
/// page with no table, is dropped and counted.
#[test]
fn a_command_the_server_sends_goes_to_the_open_pages_evaluator() {
    let builder = Builder::new(OPEN);
    let said = builder.report(Some(Value::variable(3)));
    let builder = builder.rule(Trigger::Becomes, &[Node::compare(Value::variable(3), Operator::Equal, 1)], &[said]);
    let (mut engine, _) = opened(&builder.table);
    run(&mut engine, after(1));
    let record = |page: PageSub| {
        let made = Command::Variable { operation: VariableOperation::Inc, variable: 3, operand: Operand::None };
        let (address, subcode) = address(page);
        let mut bytes = Vec::new();
        neoteletext_web::records::commands::encode(&made, address, subcode, 0, 9, &mut |packet| bytes.extend_from_slice(&packet.bytes)).unwrap();
        bytes
    };
    engine.feed_whole(true, true, &record(OTHER));
    assert_eq!(engine.rules.commands_dropped(), 1, "another sub-code's");
    engine.feed_whole(true, true, &record(OPEN));
    let mut seen = reports(&run(&mut engine, after(2)));
    seen.extend(reports(&run(&mut engine, after(3))));
    assert!(matches!(seen.as_slice(), [Report { what: Reported::Value { reading: 1, .. }, .. }]), "{seen:?}");
}

/// RULE-109 (REQ-6534): a cue the sound helper heard, given to the decoder,
/// fires the open page's `cue` rule.
#[test]
fn a_heard_cue_fires_the_cue_rule() {
    let builder = Builder::new(OPEN);
    let said = builder.report(None);
    let builder = builder.rule(Trigger::Cue { channel: CHANNEL, number: 7 }, &[], &[said]);
    let (mut engine, _) = opened(&builder.table);
    run(&mut engine, after(1));
    engine.set_now_us(after(1) + 10);
    assert_eq!(engine.rule_message(ToDecoder::CueHeard { audio_channel: CHANNEL, number: 6 }).unwrap(), []);
    let mut seen = reports(&engine.rule_message(ToDecoder::CueHeard { audio_channel: CHANNEL, number: 7 }).unwrap());
    seen.extend(reports(&run(&mut engine, after(2))));
    seen.extend(reports(&run(&mut engine, after(3))));
    assert_eq!(seen.len(), 1, "{seen:?}");
}

/// RULE-47 (REQ-6534): a state query for the open page is answered once,
/// as it is handled, at most once a second, the rest dropped
/// and reported as fault 6; a query for another sub-code, or while the
/// page is paused, gets no answer.
#[test]
fn a_state_query_is_answered_once_a_second_for_the_open_page_only() {
    let (mut engine, _) = opened(&send_all(OPEN, false));
    run(&mut engine, after(1));
    let answers = |messages: &[FromDecoder]| messages.iter().filter(|message| matches!(message, FromDecoder::PageState { .. })).count();
    // DEC-6635: a query is an event, answered as it is handled.
    let mut asked = |page, identifier| engine.rule_message(ToDecoder::RulesQuery { page, identifier }).unwrap();
    assert_eq!(answers(&asked(OTHER, 1)), 0, "another sub-code");
    assert_eq!(answers(&asked(OPEN, 2)), 1);
    assert_eq!(answers(&asked(OPEN, 3)), 0, "a second within the second");
    set_open(&mut engine, None, after(5));
    assert_eq!(answers(&engine.rule_message(ToDecoder::RulesQuery { page: OPEN, identifier: 4 }).unwrap()), 0, "paused");
    assert_eq!(answers(&run(&mut engine, after(200))), 0);
}

/// RULE-4 (REQ-6534): a wake two hundred ticks late evaluates at most six,
/// each in order, the clock moved on for the rest, and the late
/// evaluation is counted.
#[test]
fn a_late_wake_evaluates_at_most_six_ticks() {
    let builder = Builder::new(OPEN);
    let said = builder.report(None);
    let builder = builder.rule(Trigger::Every { timer: neoteletext_rules::limits::PAGE_TIMER, ticks: 1 }, &[], &[said]);
    let (mut engine, _) = opened(&builder.table);
    run(&mut engine, after(1));
    let late = reports(&run(&mut engine, after(200)));
    assert_eq!(late.len(), usize::try_from(neoteletext_rules::limits::MAX_CATCH_UP_TICKS).unwrap(), "{late:?}");
    let ticks: Vec<u32> = late.iter().map(|report| report.tick).collect();
    assert!(ticks.windows(2).all(|pair| pair[1] == pair[0] + 1), "each in order: {ticks:?}");
    assert_eq!(reports(&run(&mut engine, after(201))).len(), 1, "then one tick at a time again");
}

/// RULE-118, RULE-65 (REQ-6534, REQ-6536): a group's `ANIMATE` is applied
/// by the code that applies the record from the server: a key's group
/// shows a ReGIS picture's hidden drawing.
#[test]
fn a_groups_animate_is_applied_as_the_record_is() {
    use crate::engine::animation_tests::{Receiver, regis, square};
    use neoteletext_rules::animate::{Operation, OperationKind};
    use neoteletext_rules::table::Target;
    /// The drawing shown.
    const DRAWING: u16 = 201;
    let builder = Builder::new(PageSub::new(PAGE, 0));
    let mut body = [0_u8; neoteletext_rules::limits::MAX_COMMAND_BYTES];
    let length =
        neoteletext_rules::animate::write_record(PAGE, 0, &[Operation { target: Target::object(DRAWING), kind: OperationKind::Show }], &mut body).unwrap();
    let builder = builder.rule(on_key('a', false), &[], &[(TYPE_ANIMATE, body[..length].to_vec())]);
    let page = PageSub::new(PAGE, 0);
    let mut engine = Engine::new();
    let scene = format!("{}D(S{DRAWING}(X100,Y50,V0),C)", square(DRAWING, 5));
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, scene.as_bytes()));
    set_open(&mut engine, Some(page), T0);
    send_table(&mut engine, page, 1, Some(&builder.table));
    run(&mut engine, T0);
    let mut receiver = Receiver::default();
    assert!(receiver.regis_of(&mut engine, PAGE, 0));
    assert_eq!(receiver.at(105, 55), 0, "the drawing begins hidden");
    run(&mut engine, after(1));
    input(&mut engine, page, 1, key(KeyAction::Press, 'a'), after(1) + 10);
    run(&mut engine, after(2));
    assert!(receiver.regis_of(&mut engine, PAGE, 0));
    assert_eq!(receiver.at(105, 55), 5, "the group's ANIMATE showed it");
}

/// Renderer parity PAR-20, PAR-44: the shared wire vectors' table whose rule
/// `open` runs a group of a NAPLPS picture's links and cursor, sent as its
/// RULES record row for the open NAPLPS sub-page 101/0001, makes object
/// 301 the cursor and a link to 302 when the page opens, as the
/// `drawing_links` and `regis_cursor` rows sent directly do.
#[test]
fn the_shared_picture_links_group_applies_its_links_and_cursor() {
    use crate::engine::wire_vectors_tests::{picture_stream, records_row};
    /// The object the vectors' group names.
    const OBJECT: u16 = 301;
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &picture_stream(PAGE, SUB, true, b"A"));
    engine.feed_whole(true, true, &records_row("rules_picture_links_group"));
    assert_eq!(engine.naplps().cursor(PAGE, SUB), 0, "no cursor before the page opens");
    set_open(&mut engine, Some(OPEN), T0);
    run(&mut engine, T0);
    run(&mut engine, after(1));
    assert_eq!(engine.naplps().cursor(PAGE, SUB), OBJECT, "the group's cursor");
    #[cfg(feature = "links")]
    assert_eq!(engine.regis().take_links(), [(PAGE, SUB, vec![(OBJECT, "302".to_owned())])], "the group's link, for the trusted core");
}

/// Renderer parity PAR-20, RULE-119 (TODO-6794 step 3): a group's
/// `DRAWING_LINKS`, one whole sending, and its `REGIS_CURSOR` are applied as
/// the records the server sends directly are: a key's group makes a ReGIS
/// picture's drawing a link, told to the trusted core, and its cursor.
#[cfg(feature = "links")]
#[test]
fn a_groups_picture_links_and_cursor_are_applied_as_the_records_are() {
    use crate::engine::animation_tests::{regis, square};
    use neoteletext_rules::command::{TYPE_DRAWING_LINKS, TYPE_REGIS_CURSOR};
    /// The drawing that becomes a link and the cursor.
    const DRAWING: u16 = 7;
    let page = PageSub::new(PAGE, 0);
    let target = b"200";
    let mut links = PAGE.to_be_bytes().to_vec();
    links.extend_from_slice(&0_u16.to_be_bytes());
    // Part 0 of 1, one link.
    links.extend_from_slice(&[0, 1, 1]);
    links.extend_from_slice(&DRAWING.to_be_bytes());
    links.extend_from_slice(&u16::try_from(target.len()).unwrap().to_be_bytes());
    links.extend_from_slice(target);
    let mut cursor = PAGE.to_be_bytes().to_vec();
    cursor.extend_from_slice(&0_u16.to_be_bytes());
    cursor.extend_from_slice(&DRAWING.to_be_bytes());
    let builder = Builder::new(page).rule(on_key('a', false), &[], &[(TYPE_DRAWING_LINKS, links), (TYPE_REGIS_CURSOR, cursor)]);
    let mut engine = Engine::new();
    let scene = format!("{}D(S{DRAWING}(X100,Y50),C)", square(DRAWING, 5));
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, scene.as_bytes()));
    set_open(&mut engine, Some(page), T0);
    send_table(&mut engine, page, 1, Some(&builder.table));
    run(&mut engine, T0);
    let _ = engine.regis().take_links();
    assert_eq!(engine.regis().cursor(PAGE, 0), 0, "no cursor before the key");
    run(&mut engine, after(1));
    input(&mut engine, page, 1, key(KeyAction::Press, 'a'), after(1) + 10);
    run(&mut engine, after(2));
    assert_eq!(engine.regis().take_links(), [(PAGE, 0, vec![(DRAWING, "200".to_owned())])], "the group's link, for the trusted core");
    assert_eq!(engine.regis().cursor(PAGE, 0), DRAWING, "the group's cursor");
}

/// RAS-5 (DEC-6853, DEC-6856 item 3): a NAPLPS page's pointer is a dot of
/// the teletext raster, passed to the evaluator unchanged, as its objects'
/// bounds are: a press at the dot (130, 110) on an object of (120, 100,
/// 24, 20) fires that object's rule, and at (100, 10), a dot of row 0, it
/// fires the object whose bounds reach line 10 and no other; a press on
/// neither fires nothing; and the evaluator's world is 480 by 500.
#[test]
fn a_naplps_pages_pointer_is_a_raster_dot_the_rules_take_unchanged() {
    use crate::engine::wire_vectors_tests::picture_stream;
    use crate::rules::Picture as _;
    use neoteletext_rules::table::{Target, TriggerTarget};
    use neoteletext_web::records::naplps_objects::{NaplpsObject, ObjectKind, encode};
    use protocol::rules::PointerAction;
    let region = |number, x, y| NaplpsObject {
        number,
        kind: ObjectKind::Region,
        shown: true,
        detectable: true,
        layer: 1,
        order: 0,
        x,
        y,
        width: 24,
        height: 20,
        origin: (0, 0),
        cover: 0,
        looks: [0; 3],
        frames: &[],
    };
    let press = |number| Trigger::Press { target: TriggerTarget::of(Target::object(number)), button: 0 };
    let builder = Builder::new(OPEN);
    let report = builder.report(None);
    let builder = builder.rule(press(1), &[], core::slice::from_ref(&report)).rule(press(2), &[], &[report]);
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &picture_stream(PAGE, SUB, true, b"A"));
    let mut packets = Vec::new();
    encode((address(OPEN).0, address(OPEN).1, 1), &[region(1, 120, 100), region(2, 90, 0)], 0, 0, &mut |packet| packets.extend_from_slice(&packet.bytes))
        .unwrap();
    engine.feed_whole(true, true, &packets);
    set_open(&mut engine, Some(OPEN), T0);
    send_table(&mut engine, OPEN, 1, Some(&builder.table));
    run(&mut engine, T0);
    run(&mut engine, after(1));
    let mut sequence = 0;
    let mut fired = |engine: &mut Engine, x, y, tick| {
        sequence += 1;
        let at = after(tick) + 10;
        let mut sent = input(engine, OPEN, sequence, RuleInput::Pointer { action: PointerAction::Press, button: 0, x, y, control: 0 }, at);
        sent.extend(input(engine, OPEN, sequence + 100, RuleInput::Pointer { action: PointerAction::Release, button: 0, x, y, control: 0 }, at + 1));
        sent.extend(run(engine, after(tick + 1)));
        reports(&sent).iter().map(|report| report.rule).collect::<Vec<_>>()
    };
    assert_eq!(fired(&mut engine, 130, 110, 1), [0], "object 1's rule, at its own dot");
    assert_eq!(fired(&mut engine, 100, 10, 3), [1], "object 2's rule, at a dot of row 0 its bounds reach");
    assert_eq!(fired(&mut engine, 100, 30, 5), [] as [u8; 0], "neither object holds the dot (100, 30)");
    assert_eq!(engine.animation_picture(PAGE).size(), (480, 500), "the evaluator's world: the teletext raster");
}

/// The audio channel the stored sounds of the tests of what a page hears
/// play on.
const HEARD_CHANNEL: u8 = 1;
/// The handle of the first stored sound those tests play.
const FIRST_SOUND: u8 = 5;
/// The handle of the second, played after the first.
const SECOND_SOUND: u8 = 6;
/// Milliseconds of one Opus frame: [`OPUS_FRAME_SAMPLES`] at 48 kHz.
const OPUS_FRAME_MS: u64 = 20;
/// The 48 kHz rate of the Opus frames, in samples a millisecond.
const OPUS_SAMPLES_PER_MS: u64 = 48;
const _: () = assert!(OPUS_FRAME_SAMPLES as u64 == OPUS_SAMPLES_PER_MS * OPUS_FRAME_MS);
/// Microseconds in a millisecond: the decoder's wake gives the sounds'
/// clock in milliseconds (`process.rs`, `tick`).
const MICROSECONDS_PER_MILLISECOND: u64 = 1_000;

/// What the decoder does at each wake, as `process.rs`'s `tick` does it:
/// the sounds' clock first, then each tick of the open page's rules due.
fn wake(engine: &mut Engine, now: u64) -> Vec<FromDecoder> {
    engine.tick_audio(now / MICROSECONDS_PER_MILLISECOND);
    run(engine, now)
}

/// The records of a stored Opus sound of `frames` 20 ms frames on
/// [`HEARD_CHANNEL`], whose samples are exactly those frames, so that it
/// is `frames × 20` ms long, and of its play for [`PAGE`] with `flags`.
fn stored_sound(handle: u8, frames: usize, sequence: u16, flags: u8) -> Vec<Vec<u8>> {
    let samples = u32::try_from(frames * OPUS_FRAME_SAMPLES).unwrap();
    let stored = Writer::opus(handle, 1, OPUS_PACKET, &opus_frames(frames), samples).unwrap();
    let mut records = vec![stored.object_record().unwrap()];
    records.extend(stored.data_records());
    let play = SoundCommand {
        channel: HEARD_CHANNEL,
        sequence,
        operation: 1,
        handle,
        generation: 1,
        gain: 80,
        fade_ms: 0,
        flags,
        duck: 0,
        page: Some(PAGE),
        music: None,
    };
    records.push(play.record());
    records
}

/// Gives the engine `records` from the server, with [`PAGE`]'s sounds
/// open, at the sounds' clock `now`.
fn play_records(engine: &mut Engine, records: &[Vec<u8>], now: u64) {
    engine.tick_audio(now / MICROSECONDS_PER_MILLISECOND);
    engine.open_page(Some(PAGE));
    engine.feed(&packets(0, 0, records).0.iter().flat_map(|packet| packet.bytes).collect::<Vec<u8>>());
    engine.finish();
}

/// The body of a group's `CONTROL` that stops [`HEARD_CHANNEL`], as the
/// audio profile frames its body: no sequence and the table's page
/// (RULE-119).
fn stop_body() -> Vec<u8> {
    let stop = SoundCommand {
        channel: HEARD_CHANNEL,
        sequence: 0,
        operation: 2,
        handle: 0,
        generation: 0,
        gain: 0,
        fade_ms: 0,
        flags: 0,
        duck: 0,
        page: Some(PAGE),
        music: None,
    };
    let record = stop.record();
    record[neoteletext_audio::RECORD_HEADER..record.len() - neoteletext_audio::RECORD_CRC].to_vec()
}

/// The readings rule `rule` reported in `messages`, in order.
fn readings(messages: &[FromDecoder], rule: u8) -> Vec<i32> {
    reports(messages)
        .iter()
        .filter(|report| report.rule == rule)
        .filter_map(|report| if let Reported::Value { reading, .. } = report.what { Some(reading) } else { None })
        .collect()
}

/// Which rules fired with no value in `messages`.
fn fired(messages: &[FromDecoder]) -> Vec<u8> {
    reports(messages).iter().filter(|report| report.what == Reported::Fired).map(|report| report.rule).collect()
}

/// A table on [`OPEN`] whose rules 0 to 2 report [`HEARD_CHANNEL`]'s
/// `playing`, `handle` and `local` at every tick step; rule 3 fires when
/// `playing` becomes 0; rule 4 reports `playing` when the key `k` is
/// pressed; and rule 5 stops the channel by a group's `CONTROL` when the
/// key `s` is.
fn hearing_table() -> Box<Table> {
    let every = || Trigger::Every { timer: neoteletext_rules::limits::PAGE_TIMER, ticks: 1 };
    let playing = Value::sound(HEARD_CHANNEL, SOUND_PLAYING);
    let builder = Builder::new(OPEN);
    let (said_playing, said_handle, said_local) = (
        builder.report(Some(playing)),
        builder.report(Some(Value::sound(HEARD_CHANNEL, SOUND_HANDLE))),
        builder.report(Some(Value::sound(HEARD_CHANNEL, SOUND_LOCAL))),
    );
    let (ended, keyed) = (builder.report(None), builder.report(Some(playing)));
    let stop = (TYPE_CONTROL, stop_body());
    builder
        .rule(every(), &[], &[said_playing])
        .rule(every(), &[], &[said_handle])
        .rule(every(), &[], &[said_local])
        .rule(Trigger::Becomes, &[Node::compare(playing, Operator::Equal, 0)], &[ended])
        .rule(on_key('k', false), &[], &[keyed])
        .rule(on_key('s', false), &[], &[stop])
        .table
}

/// RULE-27, RULE-7 "The world" (DEC-7143 items 1, 3 and 4; TODO-7141 item
/// 3): a stored sound played on an audio channel reads `playing` 1 and its
/// handle; once the trusted core says the view's sound is not heard
/// (`ToDecoder::SoundHeard(false)`, which `process.rs` gives to
/// `Engine::set_sound_heard`) it reads 0, with `handle` -1 and `local` 0;
/// the change is seen by the next handling, a key's, and a `becomes` rule
/// on `playing` being 0 fires at the next tick step, not in the key's
/// handling; heard again, it reads 1 with no new start of the sound.
#[test]
fn a_sound_reads_as_playing_only_while_the_view_hears_it() {
    let (mut engine, _) = opened(&hearing_table());
    play_records(&mut engine, &stored_sound(FIRST_SOUND, 10, 1, FLAG_LOOP), T0);
    let sounds = engine.take_sounds();
    assert!(sounds.iter().any(|sound| matches!(sound, Audio::Start { channel: HEARD_CHANNEL, .. })), "{sounds:?}");
    wake(&mut engine, after(1));
    let heard = wake(&mut engine, after(2));
    assert_eq!(readings(&heard, 0), [1], "playing while heard: {heard:?}");
    assert_eq!(readings(&heard, 1), [i32::from(FIRST_SOUND)], "its handle");
    assert_eq!(readings(&heard, 2), [0], "not a local sound");
    assert_eq!(fired(&heard), [] as [u8; 0]);

    engine.set_sound_heard(false);
    let keyed = input(&mut engine, OPEN, 1, key(KeyAction::Press, 'k'), after(2) + 10);
    assert_eq!(readings(&keyed, 4), [0], "the next handling reads the change: {keyed:?}");
    assert_eq!(fired(&keyed), [] as [u8; 0], "no `becomes` within the key's handling");
    let unheard = wake(&mut engine, after(3));
    assert_eq!(fired(&unheard), [3], "`becomes` fires at the next tick step: {unheard:?}");
    assert_eq!(readings(&unheard, 0), [0], "not playing while unheard");
    assert_eq!(readings(&unheard, 1), [-1], "no handle while not playing");
    assert_eq!(readings(&unheard, 2), [0]);
    let later = readings(&wake(&mut engine, after(30)), 0);
    assert!(!later.is_empty() && later.iter().all(|reading| *reading == 0), "still unheard later: {later:?}");

    engine.set_sound_heard(true);
    let again = wake(&mut engine, after(31));
    assert_eq!(readings(&again, 0), [1], "heard again: {again:?}");
    assert_eq!(readings(&again, 1), [i32::from(FIRST_SOUND)]);
    let sounds = engine.take_sounds();
    assert!(!sounds.iter().any(|sound| matches!(sound, Audio::Start { .. } | Audio::Define { .. })), "the page did not start it again: {sounds:?}");
}

/// RULE-27, RULE-118, RULE-21 (DEC-7143 item 4): while the view is not
/// heard, a group's `CONTROL` stop still stops what the page started, so
/// that it is not heard again when sound comes back; and a key's
/// `LOCAL_MIDI` still leaves for a local sound, which reads `local` 0
/// while unheard and 1 when heard again.
#[test]
fn what_a_page_started_is_still_stopped_and_played_while_unheard() {
    let (mut engine, _) = opened(&hearing_table());
    play_records(&mut engine, &stored_sound(FIRST_SOUND, 10, 1, FLAG_LOOP), T0);
    engine.take_sounds();
    wake(&mut engine, after(1));
    engine.set_sound_heard(false);
    assert_eq!(readings(&wake(&mut engine, after(2)), 0), [0]);
    let stopped = input(&mut engine, OPEN, 1, key(KeyAction::Press, 's'), after(2) + 10);
    assert!(stopped.iter().any(|message| matches!(message, FromDecoder::Audio(Audio::Stop { channel: HEARD_CHANNEL, .. }))), "{stopped:?}");
    engine.set_sound_heard(true);
    let after_stop = wake(&mut engine, after(3));
    assert_eq!(readings(&after_stop, 0), [0], "stopped while unheard, so not playing when heard: {after_stop:?}");
    assert_eq!(readings(&after_stop, 1), [-1]);

    // A local sound on audio channel CHANNEL, played by a key's LOCAL_MIDI.
    let builder = Builder::new(OPEN);
    let local = Value::sound(CHANNEL, SOUND_LOCAL);
    let (on, said) = (builder.midi(0x90, 60, 100), builder.report(Some(local)));
    let every = Trigger::Every { timer: neoteletext_rules::limits::PAGE_TIMER, ticks: 1 };
    let builder = builder.rule(on_key('a', false), &[], &[on]).rule(every, &[], &[said]);
    let (mut engine, _) = opened(&builder.table);
    play_local(&mut engine);
    engine.take_sounds();
    wake(&mut engine, after(1));
    assert_eq!(readings(&wake(&mut engine, after(2)), 1), [1], "local while heard");
    engine.set_sound_heard(false);
    assert_eq!(readings(&wake(&mut engine, after(3)), 1), [0], "local reads 0 while unheard");
    let played = input(&mut engine, OPEN, 1, key(KeyAction::Press, 'a'), after(3) + 10);
    assert_eq!(midi(&played), [(0x90, 60)], "the note still leaves: {played:?}");
    engine.set_sound_heard(true);
    assert_eq!(readings(&wake(&mut engine, after(4)), 1), [1], "local again when heard");
}

/// The readings of rule 0 of [`hearing_table`], `playing`, at the wakes of
/// ticks `ticks`, one each: from the wake at `after(2)`, the first whose
/// tick step is past page tick 0, when `every 1` first fires (RULE-15).
fn playing_at(engine: &mut Engine, ticks: core::ops::RangeInclusive<u32>) -> Vec<(u32, i32)> {
    ticks
        .map(|tick| {
            let read = readings(&wake(engine, after(tick)), 0);
            assert_eq!(read.len(), 1, "one reading at tick {tick}: {read:?}");
            (tick, read[0])
        })
        .collect()
}

/// The wakes of `ticks` that read `playing` 1, and those that read 0.
fn split(read: &[(u32, i32)]) -> (Vec<u32>, Vec<u32>) {
    (
        read.iter().filter(|(_, reading)| *reading == 1).map(|(tick, _)| *tick).collect(),
        read.iter().filter(|(_, reading)| *reading == 0).map(|(tick, _)| *tick).collect(),
    )
}

/// RULE-27, TODO-6576 item 2 (DEC-7143 item 5): a stored sound of samples
/// that does not repeat, 200 ms long and started at [`T0`], reads
/// `playing` 1 at every wake before T0 + 200 ms (the last at tick 11,
/// T0 + 183 ms) and 0 from the wake at T0 + 200 ms (tick 12) on, by the
/// sounds' clock the wake moves first; a sound that repeats reads 1 all
/// the while.
#[test]
fn a_stored_sound_that_does_not_repeat_stops_playing_at_its_length() {
    /// Ten 20 ms frames: 200 ms, ending exactly at tick 12's time.
    const FRAMES: usize = 10;
    let length_ms = u64::try_from(FRAMES).unwrap() * OPUS_FRAME_MS;
    assert_eq!(presentation_tick_due_us(12), length_ms * MICROSECONDS_PER_MILLISECOND, "tick 12 is due as the sound ends");
    let (mut engine, _) = opened(&hearing_table());
    play_records(&mut engine, &stored_sound(FIRST_SOUND, FRAMES, 1, 0), T0);
    wake(&mut engine, after(1));
    let read = playing_at(&mut engine, 2..=30);
    assert_eq!(split(&read), ((2..=11).collect(), (12..=30).collect()), "{read:?}");

    let (mut engine, _) = opened(&hearing_table());
    play_records(&mut engine, &stored_sound(FIRST_SOUND, FRAMES, 1, FLAG_LOOP), T0);
    wake(&mut engine, after(1));
    let read = playing_at(&mut engine, 2..=60);
    assert_eq!(split(&read), ((2..=60).collect(), Vec::new()), "a repeating sound has no end: {read:?}");
}

/// RULE-27 (DEC-7143 item 5): a sound started "after the current one"
/// behind a stored sound of 200 ms is timed from that one's end: a 100 ms
/// sound reads `playing` 1 until T0 + 300 ms (the last wake at tick 17,
/// T0 + 283 ms) and 0 from tick 18, T0 + 300 ms, on.
#[test]
fn a_sound_after_the_current_one_ends_at_the_sum_of_their_lengths() {
    assert_eq!(presentation_tick_due_us(18), (10 + 5) * OPUS_FRAME_MS * MICROSECONDS_PER_MILLISECOND, "tick 18 is due as the second ends");
    let (mut engine, _) = opened(&hearing_table());
    let mut records = stored_sound(FIRST_SOUND, 10, 1, 0);
    records.extend(stored_sound(SECOND_SOUND, 5, 2, FLAG_AFTER_CURRENT));
    play_records(&mut engine, &records, T0);
    let starts: Vec<(u8, bool)> = engine
        .take_sounds()
        .iter()
        .filter_map(|sound| if let Audio::Start { sound, after_current, .. } = sound { Some((sound.handle, *after_current)) } else { None })
        .collect();
    assert_eq!(starts, [(FIRST_SOUND, false), (SECOND_SOUND, true)], "both given to the sound helper, the second to wait");
    wake(&mut engine, after(1));
    let read = playing_at(&mut engine, 2..=30);
    assert_eq!(split(&read), ((2..=17).collect(), (18..=30).collect()), "{read:?}");
}

/// RULE-27 (DEC-7143 item 5): a stream has no length the decoder times;
/// it reads `playing` 1 until the receiver ends it, here a live sound
/// whose blocks stopped, which the receiver fades two seconds after its
/// last block (the engine's REQ-AUD-014), and 0 from then on.
#[test]
fn a_streamed_sound_ends_only_when_the_receiver_ends_it() {
    let live = Writer::live(FIRST_SOUND, 1, neoteletext_audio::Codec::Opus, 48_000, OPUS_PACKET).unwrap();
    let play = SoundCommand {
        channel: HEARD_CHANNEL,
        sequence: 1,
        operation: 1,
        handle: FIRST_SOUND,
        generation: 1,
        gain: 50,
        fade_ms: 0,
        flags: 0,
        duck: 0,
        page: Some(PAGE),
        music: None,
    };
    let mut records = vec![live.object_record().unwrap(), play.record()];
    for (index, frame) in (0_u16..).zip(opus_frames(4).chunks(usize::from(OPUS_PACKET))) {
        records.extend(live.packet_records(index, frame).unwrap());
    }
    let (mut engine, _) = opened(&hearing_table());
    play_records(&mut engine, &records, T0);
    let sounds = engine.take_sounds();
    assert!(sounds.iter().any(|sound| matches!(sound, Audio::Stream { channel: HEARD_CHANNEL, .. })), "a stream: {sounds:?}");
    // The stall, LIVE_STALL_MS after the last block at T0: the first tick
    // due then.
    let stall_tick = neoteletext_core::consts::presentation_tick_at_us(neoteletext_audio::LIVE_STALL_MS * MICROSECONDS_PER_MILLISECOND);
    assert_eq!(presentation_tick_due_us(stall_tick), neoteletext_audio::LIVE_STALL_MS * MICROSECONDS_PER_MILLISECOND, "a tick is due as it stalls");
    wake(&mut engine, after(1));
    let read = playing_at(&mut engine, 2..=stall_tick + 10);
    assert_eq!(split(&read), ((2..stall_tick).collect(), (stall_tick..=stall_tick + 10).collect()), "{read:?}");
}

/// REQ-7037 (TODO-7158): the keys a table names, as the decoder tells the
/// trusted core with what the table asks for; the interface gives the page
/// these keys and keeps every other key for the viewer.
mod key_asks {
    use super::*;
    use neoteletext_rules::table::{INPUT_CHARACTER, INPUT_NAMED, INPUT_PHYSICAL, RowRun};
    use protocol::rules::{KeyAsk, MAX_KEY_ASKS};

    /// A named key's code. The decoder passes a named key's code on
    /// unread (only the input protocol's table knows them, RULE-10), so any
    /// code serves, and the input crate is not built without typing.
    fn f2() -> u32 {
        57_365
    }

    fn ask(kind: KeyKind, code: u32, mods_mask: u8, mods_value: u8) -> KeyAsk {
        KeyAsk { key: Some(Key::of(kind, code)), mods_mask, mods_value }
    }

    /// A key match on `keys` under the modifier mask and value.
    fn keys_under(keys: &[Key], mods_mask: u8, mods_value: u8) -> KeyMatch {
        KeyMatch { mods_mask, mods_value, ..KeyMatch::of(keys) }
    }

    /// The keys told for `table`, opened as the open page.
    fn told_keys(table: &Table) -> Vec<KeyAsk> {
        let (_, told_now) = opened(table);
        let told = told(&told_now);
        assert_eq!(told.len(), 1, "{told_now:?}");
        told[0].keys.clone()
    }

    // REQ-7037 item 2: each key of a `key` or `key_up` trigger, of each
    // kind, with that trigger's modifier mask and value, in the table's
    // order.
    #[test]
    fn named_keys_of_each_kind_are_told_with_their_modifiers() {
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let shift = 1;
        let table = builder
            .rule(
                Trigger::Key { keys: keys_under(&[Key::of(KeyKind::Character, u32::from('a'))], shift, shift), repeat: true },
                &[],
                std::slice::from_ref(&said),
            )
            .rule(Trigger::KeyUp { keys: KeyMatch::of(&[Key::of(KeyKind::Named, f2())]) }, &[], std::slice::from_ref(&said))
            .rule(Trigger::Key { keys: keys_under(&[Key::of(KeyKind::Physical, u32::from('q'))], shift, 0), repeat: false }, &[], &[said])
            .table;
        assert_eq!(
            told_keys(&table),
            [ask(KeyKind::Character, u32::from('a'), shift, shift), ask(KeyKind::Named, f2(), 0, 0), ask(KeyKind::Physical, u32::from('q'), shift, 0)]
        );
    }

    // REQ-7037 item 2: a trigger whose key set is empty is any key; under
    // no modifier comparison it is the whole keyboard, told as that one
    // entry whatever else the table names.
    #[test]
    fn an_any_key_trigger_is_the_whole_keyboard() {
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let table =
            builder.rule(on_key('a', false), &[], std::slice::from_ref(&said)).rule(Trigger::Key { keys: KeyMatch::any(), repeat: false }, &[], &[said]).table;
        assert_eq!(told_keys(&table), [KeyAsk::EVERY_KEY]);
    }

    // REQ-7037 item 2: any key under a modifier comparison is an entry of
    // its own, and the table's other keys are told beside it.
    #[test]
    fn any_key_under_a_modifier_is_told_with_its_modifiers() {
        let control = 2;
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let table = builder
            .rule(on_key('a', false), &[], std::slice::from_ref(&said))
            .rule(Trigger::Key { keys: KeyMatch { mods_mask: control, mods_value: control, ..KeyMatch::any() }, repeat: false }, &[], &[said])
            .table;
        assert_eq!(told_keys(&table), [ask(KeyKind::Character, u32::from('a'), 0, 0), KeyAsk { key: None, mods_mask: control, mods_value: control }]);
    }

    // REQ-7037 item 2: a `key_up` trigger names its keys as a `key`
    // trigger does.
    #[test]
    fn a_key_up_trigger_names_its_keys() {
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let table = builder.rule(on_key('z', true), &[], &[said]).table;
        assert_eq!(told_keys(&table), [ask(KeyKind::Character, u32::from('z'), 0, 0)]);
    }

    // REQ-7037 item 2 with RULE-105: a rule with one key a row names every
    // key of its set.
    #[test]
    fn a_by_row_key_set_names_each_of_its_keys() {
        let mut builder = Builder::new(OPEN);
        let said = builder.report(None);
        let keys = [Key::of(KeyKind::Character, u32::from('1')), Key::of(KeyKind::Character, u32::from('2')), Key::of(KeyKind::Character, u32::from('3'))];
        let group = {
            let body = said.1.clone();
            builder.table.push_group(&[Framed { kind: said.0, body: &body, bindings: &[] }]).unwrap()
        };
        for row in 1..=3 {
            builder.table.rows.push(row).unwrap();
        }
        let mut rule = Rule::new(Trigger::Key { keys: KeyMatch::by_row(&keys), repeat: false }, group);
        rule.rows = Some(RowRun { first: 0, count: 3 });
        builder.table.rules.push(rule).unwrap();
        assert_eq!(told_keys(&builder.table), keys.map(|key| KeyAsk { key: Some(key), ..KeyAsk::EVERY_KEY }));
    }

    // REQ-7037 item 2, RULE-24: a key whose state a value reads is asked
    // for under no modifier, of each kind.
    #[test]
    fn a_key_state_value_of_each_kind_asks_for_its_key() {
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let down = |kind, code| Node::compare(Value::key(kind, code), Operator::Equal, 1);
        let table = builder
            .rule(Trigger::Becomes, &[down(INPUT_CHARACTER, u32::from('x'))], std::slice::from_ref(&said))
            .rule(Trigger::Becomes, &[down(INPUT_NAMED, f2())], std::slice::from_ref(&said))
            .rule(Trigger::Becomes, &[down(INPUT_PHYSICAL, u32::from('w'))], &[said])
            .table;
        let mut told = told_keys(&table);
        told.sort_by_key(|ask| ask.key.map(|key| (key.kind.code(), key.code)));
        assert_eq!(told, [ask(KeyKind::Character, u32::from('x'), 0, 0), ask(KeyKind::Named, f2(), 0, 0), ask(KeyKind::Physical, u32::from('w'), 0, 0)]);
    }

    // REQ-7037 item 2, RULE-24: the count of keys down changes with every
    // key, so a table that reads it asks for the whole keyboard.
    #[test]
    fn the_count_of_keys_down_asks_for_every_key() {
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let table = builder
            .rule(on_key('a', false), &[], std::slice::from_ref(&said))
            .rule(Trigger::Becomes, &[Node::compare(Value::keys_down(), Operator::Equal, 2)], &[said])
            .table;
        assert_eq!(told_keys(&table), [KeyAsk::EVERY_KEY]);
    }

    // REQ-7037 item 2: a key named twice, by two triggers or by a trigger
    // and a value, is told once.
    #[test]
    fn a_key_named_twice_is_told_once() {
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let table = builder
            .rule(on_key('a', false), &[], std::slice::from_ref(&said))
            .rule(on_key('a', true), &[Node::compare(Value::key(INPUT_CHARACTER, u32::from('a')), Operator::Equal, 1)], &[said])
            .table;
        assert_eq!(told_keys(&table), [ask(KeyKind::Character, u32::from('a'), 0, 0)]);
    }

    /// A table naming `count` distinct characters, sixteen to a trigger.
    fn naming(count: u32) -> Box<Table> {
        let mut builder = Builder::new(OPEN);
        let said = builder.report(None);
        let all: Vec<Key> = (0..count).map(|offset| Key::of(KeyKind::Character, u32::from('A') + offset)).collect();
        for chunk in all.chunks(neoteletext_rules::limits::MAX_KEYS_PER_TRIGGER) {
            builder = builder.rule(Trigger::Key { keys: KeyMatch::of(chunk), repeat: false }, &[], std::slice::from_ref(&said));
        }
        builder.table
    }

    // REQ-7037 item 2: exactly `MAX_KEY_ASKS` distinct keys are told as
    // they are; one more is told as any key, the whole keyboard.
    #[test]
    fn the_bound_of_key_asks_is_exact() {
        let bound = u32::try_from(MAX_KEY_ASKS).unwrap();
        let told = told_keys(&naming(bound));
        assert_eq!(told.len(), MAX_KEY_ASKS);
        assert_eq!(told, (0..bound).map(|offset| ask(KeyKind::Character, u32::from('A') + offset, 0, 0)).collect::<Vec<_>>());
        assert_eq!(told_keys(&naming(bound + 1)), [KeyAsk::EVERY_KEY]);
    }

    // REQ-7037 item 2: a table with no key trigger and no key value names
    // no key.
    #[test]
    fn a_table_with_no_key_trigger_names_none() {
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let table = builder.rule(Trigger::Becomes, &[Node::compare(Value::variable(1), Operator::Equal, 1)], &[said]).table;
        let (_, told_now) = opened(&table);
        assert!(told(&told_now).iter().all(|open| open.keys.is_empty()), "{told_now:?}");
    }

    // REQ-7037 item 2: the list is the table's as written: a rule's
    // condition does not remove its keys.
    #[test]
    fn a_rules_condition_does_not_change_its_keys() {
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let never = Node::compare(Value::variable(1), Operator::Equal, 99);
        let table = builder.rule(on_key('k', false), &[never], &[said]).table;
        assert_eq!(told_keys(&table), [ask(KeyKind::Character, u32::from('k'), 0, 0)]);
    }

    // REQ-7037 item 2 with the protocol's reader: a trigger whose modifier
    // value has a bit its mask does not compare matches no key (RULE-9),
    // and the table's rules accept it; the decoder must still tell what
    // the table asks for, not fail its message. Recorded as TODO-7207.
    #[test]
    #[ignore = "fault: a key trigger whose modifier value has a bit outside its mask, which the table's checks accept, makes the decoder's RulesOpen message unsendable (the protocol's reader refuses it), reported for a Fable fix"]
    fn a_modifier_value_outside_its_mask_is_still_told() {
        let builder = Builder::new(OPEN);
        let said = builder.report(None);
        let table = builder.rule(Trigger::Key { keys: keys_under(&[Key::of(KeyKind::Character, u32::from('a'))], 0, 1), repeat: false }, &[], &[said]).table;
        let (_, told_now) = opened(&table);
        let told = told(&told_now);
        assert_eq!(told.len(), 1, "the trusted core is told of the table: {told_now:?}");
        let mut bytes = Vec::new();
        let sent = protocol::decoder::write_to_core(&mut bytes, &FromDecoder::RulesOpen(told[0].clone()));
        assert!(sent.is_ok(), "the decoder's message is sent: {sent:?}");
    }
}
