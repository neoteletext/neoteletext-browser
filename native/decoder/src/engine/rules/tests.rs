//! Tests of the decoder as the host of the open page's rule table, second
//! form (neoteletext-common's `docs/page-rules.md`; REQ-6534, REQ-6535,
//! REQ-6615, DEC-6587, DEC-6599).

use neoteletext_audio::encode::{Command as SoundCommand, Sound as Writer, packets};
use neoteletext_core::consts::presentation_tick_due_us;
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_rules::command::{self, Command, Operand, TYPE_ANIMATE, TYPE_LOCAL_MIDI, TYPE_REPORT, TYPE_VARIABLE, VariableOperation};
use neoteletext_rules::table::{CLASS_NOTE, Framed, Key, KeyKind, KeyMatch, Node, Operator, Rule, SOUND_LOCAL, Table, Trigger, Value, default_send_all};
use protocol::audio::Audio;
use protocol::decoder::{FromDecoder, ToDecoder};
use protocol::rules::{
    INTEREST_FOCUS, INTEREST_KEYS, INTEREST_MOVES, INTEREST_POINTER_BUTTONS, INTEREST_TEXT, INTEREST_WHEEL, KeyAction, PageSub, Report, Reported, RuleInput,
    RulesOpen, WheelRect,
};

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
    let play =
        SoundCommand { channel: CHANNEL, sequence: 1, operation: 1, handle: 1, generation: 1, gain: 100, fade_ms: 5, flags: 0, duck: 0, page: Some(PAGE) };
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
    assert_eq!(told(&told_now), [RulesOpen { page: OPEN, generation: 1, interest: every, watch: vec![], wheel: vec![] }]);
    assert!(engine.rules_due_us().is_some(), "the evaluator wakes the decoder for its ticks");

    send_table(&mut engine, OPEN, 2, Some(&send_all(OPEN, true)));
    let told_now = told(&run(&mut engine, after(3)));
    let whole = WheelRect { x: 0, y: 0, width: 480, height: 500 };
    assert_eq!(
        told_now.last(),
        Some(&RulesOpen { page: OPEN, generation: 2, interest: every | INTEREST_WHEEL, watch: vec![], wheel: vec![whole] }),
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
    assert!(closed.contains(&FromDecoder::RulesOpen(RulesOpen { page: OPEN, generation: 0, interest: 0, watch: vec![], wheel: vec![] })));
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
    assert!(told(&paused).contains(&RulesOpen { page: OPEN, generation: 0, interest: 0, watch: vec![], wheel: vec![] }), "{paused:?}");
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
    assert!(told(&ended).contains(&RulesOpen { page: OPEN, generation: 0, interest: 0, watch: vec![], wheel: vec![] }));
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
    assert!(told(&dropped).contains(&RulesOpen { page: OPEN, generation: 0, interest: 0, watch: vec![], wheel: vec![] }), "{dropped:?}");
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
