//! Tests of the decoder as the host of the open page's rule table
//! (neoteletext-common's `docs/page-rules.md`; REQ-6534, REQ-6535).

use neoteletext_audio::encode::{Command, Sound as Writer, packets};
use neoteletext_core::consts::presentation_tick_due_us;
use neoteletext_rules::table::{
    Action, CLASS_NOTE, Key, KeyKind, Node, Operator, Rule, SOUND_LEVEL, SOUND_LOCAL, Table, Trigger, Value, any_key, default_send_all,
};
use protocol::audio::{Audio, WatchSlot};
use protocol::decoder::FromDecoder;
use protocol::rules::{
    INTEREST_FOCUS, INTEREST_KEYS, INTEREST_MOVES, INTEREST_POINTER_BUTTONS, INTEREST_TEXT, INTEREST_WHEEL, KeyAction, Report, Reported, RuleInput, RulesOpen,
    WheelRect,
};

use crate::Engine;

/// The page the tests open.
const PAGE: u16 = 0x101;
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

/// A table of `rules`, on the heap: a table is 76 KB.
fn table(rules: &[Rule]) -> Box<Table> {
    let mut table = Box::new(Table::new());
    for rule in rules {
        table.rules.push(*rule).unwrap();
    }
    table
}

fn rule(trigger: Trigger, actions: &[Action]) -> Rule {
    let mut rule = Rule::new(trigger);
    for action in actions {
        rule.actions.push(*action).unwrap();
    }
    rule
}

fn becomes(value: Value, operator: Operator, constant: i32, actions: &[Action]) -> Rule {
    let mut rule = rule(Trigger::Becomes, actions);
    rule.condition.push(Node::Compare { value, operator, constant }).unwrap();
    rule
}

/// A rule on the character key `character`, going down or coming up.
fn on_key(character: char, up: bool, actions: &[Action]) -> Rule {
    let mut keys = any_key();
    keys.push(Key { kind: KeyKind::Character, code: u32::from(character) }).unwrap();
    let trigger = if up { Trigger::KeyUp { keys, mods_mask: 0, mods_value: 0 } } else { Trigger::Key { keys, repeat: false, mods_mask: 0, mods_value: 0 } };
    rule(trigger, actions)
}

/// The pushed packets of a RULES record whose body is `body`.
fn rules_record(body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    neoteletext_web::records::rules::encode(body, 0, 1, &mut |packet| bytes.extend_from_slice(&packet.bytes)).unwrap();
    bytes
}

/// Feeds the engine the RULES record of `table` for [`PAGE`].
fn send_table(engine: &mut Engine, generation: u16, table: Option<&Table>) {
    let mut body = vec![0_u8; neoteletext_rules::limits::MAX_TABLE_BYTES];
    let length = neoteletext_rules::encode(PAGE, generation, table, &mut body).unwrap();
    engine.feed_whole(true, true, &rules_record(&body[..length]));
}

/// An engine whose open page is [`PAGE`], with `table` sent for it and
/// its evaluator made: what the trusted core was told, and the engine.
fn opened(table: &Table) -> (Engine, Vec<FromDecoder>) {
    let mut engine = Engine::new();
    send_table(&mut engine, 1, Some(table));
    engine.rule_page(Some(PAGE));
    let told = engine.run_rules(T0);
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
    let play = Command { channel: CHANNEL, sequence: 1, operation: 1, handle: 1, generation: 1, gain: 100, fade_ms: 5, flags: 0, duck: 0, page: Some(PAGE) };
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

/// RULE-58, RULE-52 item 1, RULE-51 item 4 (REQ-6534): when a table is
/// decoded for the open page the trusted core is told what it asks for:
/// the default table asks for every class but the wheel, and with its
/// `wheel` rule the wheel over the whole picture.
#[test]
fn a_table_for_the_open_page_tells_the_trusted_core_what_it_asks_for() {
    let mut plain = Box::new(Table::new());
    default_send_all(false, &mut plain);
    let (mut engine, told) = opened(&plain);
    let every = INTEREST_KEYS | INTEREST_TEXT | INTEREST_POINTER_BUTTONS | INTEREST_MOVES | INTEREST_FOCUS;
    assert_eq!(told, [FromDecoder::RulesOpen(RulesOpen { page: PAGE, generation: 1, interest: every, watch: vec![], wheel: vec![] })]);
    assert!(engine.rules_due_us().is_some(), "the evaluator wakes the decoder for its ticks");

    let mut wheel = Box::new(Table::new());
    default_send_all(true, &mut wheel);
    send_table(&mut engine, 2, Some(&wheel));
    assert_eq!(engine.rules_due_us(), Some(0), "a new table is settled at once");
    let told = engine.run_rules(after(3));
    assert_eq!(
        told.first(),
        Some(&FromDecoder::RulesOpen(RulesOpen { page: PAGE, generation: 0, interest: 0, watch: vec![], wheel: vec![] })),
        "the old table's end"
    );
    assert!(told.contains(&FromDecoder::RulesOpen(RulesOpen { page: PAGE, generation: 2, interest: every | INTEREST_WHEEL, watch: vec![], wheel: vec![] })));
    // The first tick's snapshot knows the picture: the wheel applies over
    // the whole teletext page.
    let told = engine.run_rules(after(5));
    let whole = WheelRect { x: 0, y: 0, width: 480, height: 500 };
    assert!(
        told.contains(&FromDecoder::RulesOpen(RulesOpen { page: PAGE, generation: 2, interest: every | INTEREST_WHEEL, watch: vec![], wheel: vec![whole] })),
        "{told:?}"
    );
}

/// RULE-50, RULE-58 (REQ-6534): a page with no table evaluates nothing
/// and wakes the decoder for nothing, whether the channel sent no RULES
/// record or one that says the page has none; and a page that is not the
/// open page of the active tab has no evaluator, table or not.
#[test]
fn no_table_no_evaluator_and_no_wake() {
    let mut engine = Engine::new();
    engine.rule_page(Some(PAGE));
    assert_eq!(engine.run_rules(T0), []);
    assert_eq!(engine.rules_due_us(), None, "no sixty-a-second wake for rules");
    engine.rule_input(PAGE, 1, key(KeyAction::Press, 'a'));
    assert_eq!(engine.run_rules(after(2)), []);
    send_table(&mut engine, 1, None);
    assert_eq!(engine.run_rules(after(3)), [], "a record that says the page has no table");
    assert_eq!(engine.rules_due_us(), None);

    let mut all = Box::new(Table::new());
    default_send_all(false, &mut all);
    let mut hidden = Engine::new();
    send_table(&mut hidden, 1, Some(&all));
    assert_eq!(hidden.run_rules(T0), [], "no page of the channel is the open page");
    assert_eq!(hidden.rules_due_us(), None);
}

/// RULE-70, RULE-48 (REQ-6534): a table the engine's decoder refuses is
/// refused whole: the page has no table, nothing of it is evaluated, and
/// fault 1 is reported.
#[test]
fn a_table_that_cannot_be_decoded_is_refused_whole_and_reported() {
    let mut all = Box::new(Table::new());
    default_send_all(false, &mut all);
    let mut body = vec![0_u8; neoteletext_rules::limits::MAX_TABLE_BYTES];
    let length = neoteletext_rules::encode(PAGE, 7, Some(&all), &mut body).unwrap();
    // The record's own reader checks only the length, the page and the
    // generation; a reserved byte that is not 0 is the table decoder's to
    // refuse.
    body[11] = 1;
    let mut bytes = Vec::new();
    neoteletext_web::records::write_packets(neoteletext_web::records::TYPE_RULES, &body[..length], 0, 1, &mut |packet| bytes.extend_from_slice(&packet.bytes))
        .unwrap();
    let mut engine = Engine::new();
    engine.feed_whole(true, true, &bytes);
    engine.rule_page(Some(PAGE));
    assert_eq!(engine.run_rules(T0), [FromDecoder::RulesFault { page: PAGE, generation: 7, fault: 1, count: 1 }]);
    assert_eq!(engine.rules_due_us(), None, "no evaluator runs");
    engine.rule_input(PAGE, 1, key(KeyAction::Press, 'a'));
    assert_eq!(engine.run_rules(after(2)), []);
}

/// RULE-22, RULE-52 item 3 (REQ-6534): a rule's `send` asks the trusted
/// core for a report that names the input event by the sequence number it
/// was given under, in the tick after the event arrived, and never
/// carries the event.
#[test]
fn a_send_names_the_input_by_its_sequence_number() {
    let mut all = Box::new(Table::new());
    default_send_all(false, &mut all);
    let (mut engine, _) = opened(&all);
    assert_eq!(reports(&engine.run_rules(after(1))), [], "page tick 0: nothing happened");
    engine.rule_input(PAGE, 41, key(KeyAction::Press, 'a'));
    engine.rule_input(PAGE, 42, RuleInput::Focus);
    assert_eq!(engine.run_rules(after(1)), [], "an event waits for the next tick");
    let sent = reports(&engine.run_rules(after(2)));
    let report = |rule, sequence| Report { page: PAGE, generation: 1, rule, tick: 1, target_kind: 0, target_number: 0, what: Reported::Input { sequence } };
    // The default table's rule 0 is `keys` and its rule 10 `focus`.
    assert_eq!(sent, [report(0, 41), report(10, 42)]);
    engine.rule_input(0x102, 43, key(KeyAction::Press, 'b'));
    assert_eq!(reports(&engine.run_rules(after(3))), [], "another page's input is not this table's");
}

/// RULE-21, RULE-60, RULE-64 (REQ-6535): a key press whose rule holds a
/// `midi` action becomes a `LocalMidi` for the local sound on that audio
/// channel in the first tick after the key, and it is the first thing the
/// decoder writes for that tick; with no local sound playing the action
/// does nothing and is counted as skipped (fault 5).
#[test]
fn a_key_plays_the_local_sound_in_the_next_tick_before_anything_else() {
    let note_on = Action::Midi { channel: CHANNEL, status: 0x90, data1: 60, data2: 100 };
    let note_off = Action::Midi { channel: CHANNEL, status: 0x80, data1: 60, data2: 0 };
    let rules = table(&[on_key('a', false, &[Action::Send(None), note_on]), on_key('a', true, &[note_off])]);
    let (mut engine, told) = opened(&rules);
    assert!(matches!(told.as_slice(), [FromDecoder::RulesOpen(open)] if open.interest == INTEREST_KEYS), "{told:?}");
    engine.run_rules(after(1));

    // No local sound yet: the action is a counted no-op (RULE-20).
    engine.rule_input(PAGE, 1, key(KeyAction::Press, 'a'));
    let produced = engine.run_rules(after(2));
    assert!(!produced.iter().any(|message| matches!(message, FromDecoder::Audio(Audio::LocalMidi { .. }))), "{produced:?}");
    assert!(produced.contains(&FromDecoder::RulesFault { page: PAGE, generation: 1, fault: 5, count: 1 }), "{produced:?}");
    engine.rule_input(PAGE, 2, key(KeyAction::Release, 'a'));
    engine.run_rules(after(3));

    play_local(&mut engine);
    let sounds = engine.take_sounds();
    assert!(
        matches!(sounds.as_slice(), [Audio::LocalBank { channel: CHANNEL, index: 0, .. }, Audio::Local { channel: CHANNEL, start }]
            if start.family == SYNTH106 && start.setup == [0, 0xC0, 0]),
        "{sounds:?}"
    );

    engine.rule_input(PAGE, 3, key(KeyAction::Press, 'a'));
    let produced = engine.run_rules(after(4));
    assert_eq!(produced.first(), Some(&FromDecoder::Audio(Audio::LocalMidi { channel: CHANNEL, status: 0x90, data1: 60, data2: 100 })), "{produced:?}");
    assert_eq!(reports(&produced).len(), 1, "and its send, after the sound");
    engine.rule_input(PAGE, 4, key(KeyAction::Release, 'a'));
    let produced = engine.run_rules(after(5));
    assert_eq!(produced.first(), Some(&FromDecoder::Audio(Audio::LocalMidi { channel: CHANNEL, status: 0x80, data1: 60, data2: 0 })));
}

/// RULE-30, RULE-27 (REQ-6535): a local sound's tracked values are read
/// in the decoder, from the family's tracker fed with every `midi` action:
/// `note[60]` reads 1 in the tick after the rule's note on, with nothing
/// asked of the sound helper; and `local` reads 1 while the sound plays.
#[test]
fn a_local_sounds_notes_are_read_from_its_tracker() {
    let note_on = Action::Midi { channel: CHANNEL, status: 0x90, data1: 60, data2: 100 };
    let note = Value::synth(CHANNEL, SYNTH106, 0, 0, CLASS_NOTE, 60);
    let rules = table(&[
        on_key('a', false, &[note_on]),
        becomes(note, Operator::Equal, 1, &[Action::Send(Some(note))]),
        becomes(Value::sound(CHANNEL, SOUND_LOCAL), Operator::Equal, 1, &[Action::Send(None)]),
    ]);
    let (mut engine, _) = opened(&rules);
    engine.run_rules(after(1));
    play_local(&mut engine);
    engine.take_sounds();
    let fired = reports(&engine.run_rules(after(2)));
    assert!(matches!(fired.as_slice(), [Report { rule: 2, what: Reported::Fired, .. }]), "the local sound is seen: {fired:?}");
    engine.rule_input(PAGE, 1, key(KeyAction::Press, 'a'));
    let played = engine.run_rules(after(3));
    assert!(matches!(played.first(), Some(FromDecoder::Audio(Audio::LocalMidi { .. }))));
    assert_eq!(reports(&played), [], "nothing an action does is seen in its own tick (RULE-6)");
    let seen = reports(&engine.run_rules(after(4)));
    assert_eq!(
        seen,
        [Report {
            page: PAGE,
            generation: 1,
            rule: 1,
            tick: 3,
            target_kind: 0,
            target_number: 0,
            what: Reported::Value { value: note.to_bytes(), reading: 1 }
        }]
    );
}

/// RULE-33 items 1 and 5, RULE-65 (REQ-6535): the watch list told is the
/// table's metered values, a metered report of its length is taken at the
/// next tick, and a `becomes` rule on a level fires in that tick; a
/// report of another length is not taken.
#[test]
fn a_metered_level_fires_a_becomes_rule_within_a_tick() {
    let level = Value::sound(CHANNEL, SOUND_LEVEL);
    let rules = table(&[becomes(level, Operator::GreaterOrEqual, 64, &[Action::Send(Some(level))])]);
    let (mut engine, told) = opened(&rules);
    let watch = vec![WatchSlot { channel: CHANNEL, kind: 1, part: 0 }];
    assert_eq!(told, [FromDecoder::RulesOpen(RulesOpen { page: PAGE, generation: 1, interest: 0, watch, wheel: vec![] })]);
    assert_eq!(reports(&engine.run_rules(after(1))), []);
    engine.rule_metered(PAGE, &[200, 1]);
    assert_eq!(reports(&engine.run_rules(after(2))), [], "a report that is not the watch list's length");
    engine.rule_metered(PAGE, &[200]);
    let fired = reports(&engine.run_rules(after(3)));
    assert_eq!(
        fired,
        [Report {
            page: PAGE,
            generation: 1,
            rule: 0,
            tick: 2,
            target_kind: 0,
            target_number: 0,
            what: Reported::Value { value: level.to_bytes(), reading: 200 }
        }]
    );
    engine.rule_metered(PAGE, &[201]);
    assert_eq!(reports(&engine.run_rules(after(4))), [], "it fires when it becomes true, not while it stays");
}

/// RULE-58, RULE-60, RULE-5 (REQ-6534, REQ-6535): when the page stops
/// being the open page of the active tab its final tick runs its `close`
/// rules, its local sound ends, the trusted core is told it asks for
/// nothing, and the decoder wakes for rules no more.
#[test]
fn a_page_that_is_no_longer_open_closes_its_rules_and_its_local_sound() {
    let rules = table(&[rule(Trigger::Close, &[Action::Send(None)]), on_key('a', true, &[Action::Send(None)])]);
    let (mut engine, _) = opened(&rules);
    engine.run_rules(after(1));
    play_local(&mut engine);
    engine.take_sounds();
    // The release the trusted core gives for a key still held (RULE-8).
    engine.rule_input(PAGE, 9, key(KeyAction::Release, 'a'));
    engine.rule_page(None);
    assert_eq!(engine.rules_due_us(), Some(0));
    let closing = engine.run_rules(after(2));
    let kinds: Vec<Reported> = reports(&closing).iter().map(|report| report.what).collect();
    assert_eq!(kinds, [Reported::Input { sequence: 9 }, Reported::Fired], "the release, then the close rule");
    assert_eq!(closing.last(), Some(&FromDecoder::Audio(Audio::Stop { channel: CHANNEL, fade_ms: 5 })), "{closing:?}");
    assert!(closing.contains(&FromDecoder::RulesOpen(RulesOpen { page: PAGE, generation: 0, interest: 0, watch: vec![], wheel: vec![] })));
    assert_eq!(engine.rules_due_us(), None);
    assert_eq!(engine.run_rules(after(30)), []);
}

/// RULE-47 (REQ-6534): a state query for the open page is answered once,
/// at the end of the next tick; a second within a second is dropped and
/// reported as fault 6; a query for a page that is not open is ignored;
/// and a page with no table answers generation 0.
#[test]
fn a_state_query_is_answered_once_a_second_for_the_open_page_only() {
    let rules = table(&[rule(Trigger::Open, &[Action::Send(None)])]);
    let (mut engine, _) = opened(&rules);
    engine.run_rules(after(1));
    engine.rule_query(0x102, 5);
    engine.rule_query(PAGE, 6);
    let answered = engine.run_rules(after(2));
    let states: Vec<&FromDecoder> = answered.iter().filter(|message| matches!(message, FromDecoder::PageState { .. })).collect();
    let [FromDecoder::PageState { page: PAGE, identifier: 6, body }] = states.as_slice() else { panic!("{answered:?}") };
    let state = neoteletext_rules::state::decode(body).unwrap();
    assert_eq!((state.identifier, state.table.map(|table| table.generation)), (6, Some(1)));
    engine.rule_query(PAGE, 7);
    let dropped = engine.run_rules(after(3));
    assert!(!dropped.iter().any(|message| matches!(message, FromDecoder::PageState { .. })));
    assert!(dropped.contains(&FromDecoder::RulesFault { page: PAGE, generation: 1, fault: 6, count: 1 }), "{dropped:?}");
    // A second of ticks, each in its turn.
    for tick in 4..70 {
        engine.run_rules(after(tick));
    }
    engine.rule_query(PAGE, 8);
    let later = engine.run_rules(after(70));
    assert!(later.iter().any(|message| matches!(message, FromDecoder::PageState { identifier: 8, .. })), "a second later: {later:?}");

    let mut plain = Engine::new();
    plain.rule_page(Some(PAGE));
    plain.rule_query(PAGE, 9);
    let answered = plain.run_rules(T0);
    let [FromDecoder::PageState { page: PAGE, identifier: 9, body }] = answered.as_slice() else { panic!("{answered:?}") };
    assert_eq!(neoteletext_rules::state::decode(body).unwrap().table, None, "no table: generation 0");
}

/// RULE-4, RULE-72 (REQ-6534): every due tick is evaluated in order, at
/// most six in one wake, page time slowing for the rest; and the counts
/// are told for the console once a second.
#[test]
fn a_late_wake_evaluates_at_most_six_ticks() {
    let every = rule(Trigger::Every { timer: neoteletext_rules::limits::PAGE_TIMER, ticks: 1 }, &[Action::Send(None)]);
    let (mut engine, _) = opened(&table(&[every]));
    let counted = |messages: &[FromDecoder]| {
        messages.iter().find_map(|message| if let FromDecoder::RuleCounters(counters) = message { Some(counters.ticks) } else { None })
    };
    let first = engine.run_rules(after(1));
    assert_eq!(counted(&first), Some(1), "told as the page opens");
    assert_eq!(reports(&first), [], "the page timer's first tick is not yet over");
    // 200 ticks later, 199 are overdue: six run, each in order.
    let late: Vec<u32> = reports(&engine.run_rules(after(200))).iter().map(|report| report.tick).collect();
    assert_eq!(late, [1, 2, 3, 4, 5, 6]);
    assert_eq!(reports(&engine.run_rules(after(201))).len(), 1, "then one tick at a time again");
}

/// RULE-64, RULE-65, RULE-26, RULE-33, ANIM-6 (REQ-6534, REQ-6535,
/// REQ-6536; RULE-93's table in small): on a ReGIS page with a local
/// sound, a key going down writes the note to the sound helper first and
/// shows the key's pressed drawing in the tick that ran the rule, and the
/// key coming up hides it; a metered level that passes a threshold shows
/// a meter drawing in the tick the report is taken, and hides it when the
/// level falls. Nothing is sent to the channel: no rule has a `send`.
#[test]
fn a_key_shows_its_drawing_and_plays_and_a_level_lights_the_meter_in_the_same_tick() {
    use crate::engine::animation_tests::{Receiver, regis, square};
    use neoteletext_rules::animate::{Operation, OperationKind};
    use neoteletext_rules::table::Target;
    /// The pressed look of a key, and a segment of the meter (RULE-92).
    const PRESSED: u16 = 201;
    const SEGMENT: u16 = 301;
    let animate = |number: u16, kind: OperationKind| Action::Animate(Operation { target: Target::object(number), kind });
    let note_on = Action::Midi { channel: CHANNEL, status: 0x90, data1: 60, data2: 100 };
    let note_off = Action::Midi { channel: CHANNEL, status: 0x80, data1: 60, data2: 0 };
    let level = Value::sound(CHANNEL, SOUND_LEVEL);
    let rules = table(&[
        on_key('a', false, &[note_on, animate(PRESSED, OperationKind::Show)]),
        on_key('a', true, &[note_off, animate(PRESSED, OperationKind::Hide)]),
        becomes(level, Operator::GreaterOrEqual, 32, &[animate(SEGMENT, OperationKind::Show)]),
        becomes(level, Operator::Less, 32, &[animate(SEGMENT, OperationKind::Hide)]),
    ]);

    let mut engine = Engine::new();
    let scene = format!("{}{}D(S{PRESSED}(X100,Y50,V0),S{SEGMENT}(X300,Y50,V0),C)", square(PRESSED, 5), square(SEGMENT, 6));
    engine.feed_whole(true, true, &regis(PAGE, 1, 0, scene.as_bytes()));
    send_table(&mut engine, 1, Some(&rules));
    engine.rule_page(Some(PAGE));
    let told = engine.run_rules(T0);
    assert!(matches!(told.as_slice(), [FromDecoder::RulesOpen(open)] if open.interest == INTEREST_KEYS && open.watch.len() == 1), "{told:?}");
    play_local(&mut engine);
    engine.take_sounds();
    let mut receiver = Receiver::default();
    assert!(receiver.regis_of(&mut engine, PAGE, 0) && receiver.take_sent().1, "the first picture is whole");
    assert_eq!((receiver.at(105, 55), receiver.at(305, 55)), (0, 0), "both drawings begin hidden");
    engine.run_rules(after(1));

    // The key goes down: the note leaves first, and the picture taken
    // after the same tick holds the pressed key, as its rectangle alone.
    engine.rule_input(PAGE, 1, key(KeyAction::Press, 'a'));
    let produced = engine.run_rules(after(2));
    assert_eq!(produced.first(), Some(&FromDecoder::Audio(Audio::LocalMidi { channel: CHANNEL, status: 0x90, data1: 60, data2: 100 })), "{produced:?}");
    assert_eq!(reports(&produced), [], "nothing is sent to the channel");
    assert!(receiver.regis_of(&mut engine, PAGE, 0), "the tick that ran the rule has a picture to send");
    assert_eq!(receiver.at(105, 55), 5, "the pressed key is shown");
    let (sent, whole) = receiver.take_sent();
    assert!(!whole && sent <= 40 * 40, "only the key's rectangle is sent: {sent} pixels");

    // The level of the buffer the note began in arrives: the segment
    // lights in the tick that takes it.
    engine.rule_metered(PAGE, &[90]);
    engine.run_rules(after(3));
    assert!(receiver.regis_of(&mut engine, PAGE, 0));
    assert_eq!((receiver.at(105, 55), receiver.at(305, 55)), (5, 6), "the key stays pressed and the segment is shown");

    // The key comes up, and the level falls.
    engine.rule_input(PAGE, 2, key(KeyAction::Release, 'a'));
    let produced = engine.run_rules(after(4));
    assert_eq!(produced.first(), Some(&FromDecoder::Audio(Audio::LocalMidi { channel: CHANNEL, status: 0x80, data1: 60, data2: 0 })));
    assert!(receiver.regis_of(&mut engine, PAGE, 0));
    assert_eq!((receiver.at(105, 55), receiver.at(305, 55)), (0, 6), "the key is hidden again; the note still sounds");
    engine.rule_metered(PAGE, &[3]);
    engine.run_rules(after(5));
    assert!(receiver.regis_of(&mut engine, PAGE, 0));
    assert_eq!(receiver.at(305, 55), 0, "the segment falls with the level");
}

/// TODO-6576 item 5, RULE-13 (REQ-6534): a NAPLPS picture is the middle
/// 640 pixels of the 800 the trusted core gives the pointer in, so the
/// evaluator is given the pointer 80 pixels to the left, and a pointer in
/// either margin is on nothing.
#[test]
fn a_naplps_pages_pointer_is_given_in_the_pictures_own_pixels() {
    use super::{NAPLPS_WIDTH, naplps_input};
    use protocol::rules::PointerAction;
    let at = |x: u16| match naplps_input(RuleInput::Pointer { action: PointerAction::Move, button: 0, x, y: 7, control: 0 }) {
        RuleInput::Pointer { x, y: 7, .. } => x,
        other => panic!("{other:?}"),
    };
    assert_eq!((at(80), at(81), at(719)), (0, 1, 639), "the picture's first and last columns");
    assert_eq!((at(0), at(79), at(720), at(799)), (NAPLPS_WIDTH, NAPLPS_WIDTH, NAPLPS_WIDTH, NAPLPS_WIDTH), "the margins are outside it");
    assert_eq!(naplps_input(RuleInput::Wheel { x: 100, y: 3, control: 0 }), RuleInput::Wheel { x: 20, y: 3, control: 0 });
    assert_eq!(naplps_input(RuleInput::Focus), RuleInput::Focus);
}

/// RULE-48 fault 8, ANIM-16 (REQ-6534, REQ-6536): the repaints the
/// animation held back for its budget are told to the trusted core as
/// fault 8 with the table's generation, at most once a second, the count
/// carried to the next telling; for a page that is not the open one
/// nothing is told.
#[test]
fn held_back_repaints_are_told_as_the_animation_slowed() {
    let rules = table(&[on_key('a', false, &[Action::Send(None)])]);
    let (mut engine, _) = opened(&rules);
    engine.rules.animation_slowed(PAGE, 3);
    engine.rules.animation_slowed(PAGE + 1, 9);
    let produced = engine.run_rules(after(1));
    assert!(produced.contains(&FromDecoder::RulesFault { page: PAGE, generation: 1, fault: 8, count: 3 }), "{produced:?}");
    engine.rules.animation_slowed(PAGE, 2);
    let soon = engine.run_rules(after(2));
    assert!(!soon.iter().any(|message| matches!(message, FromDecoder::RulesFault { fault: 8, .. })), "within the second: {soon:?}");
    let later: Vec<FromDecoder> = (3..=2 + neoteletext_rules::limits::TICKS_PER_SECOND).flat_map(|tick| engine.run_rules(after(tick))).collect();
    assert!(later.contains(&FromDecoder::RulesFault { page: PAGE, generation: 1, fault: 8, count: 2 }), "{later:?}");
}

/// RULE-27, RULE-28, RULE-30 (REQ-6534, TODO-6576 item 2): of a
/// synthesizer sound played from the server's blocks a rule reads the
/// family, and the tracked values from the receiver that renders it, at
/// the position rendered: a controller the sound's first block sets reads
/// its value, and `playing` is false again once the stream has ended.
#[test]
fn a_streamed_sounds_values_are_read_from_the_receiver_and_it_stops_playing_at_its_end() {
    use neoteletext_audio_instruments::synth106::{Model, Patch, interface};
    use neoteletext_rules::table::{CLASS_CONTROLLER, SOUND_PLAYING};
    /// The controller the sound's first block sets: the modulation wheel.
    const CONTROLLER: u16 = 1;
    const HANDLE: u8 = 9;
    const BANK: u8 = 2;
    let controller = Value::synth(CHANNEL, SYNTH106, 0, 0, CLASS_CONTROLLER, CONTROLLER);
    let playing = Value::sound(CHANNEL, SOUND_PLAYING);
    let rules = table(&[
        becomes(controller, Operator::Equal, 99, &[Action::Send(Some(controller))]),
        becomes(playing, Operator::Equal, 1, &[Action::Send(Some(playing))]),
        becomes(playing, Operator::Equal, 0, &[Action::Send(Some(playing))]),
    ]);
    let (mut engine, _) = opened(&rules);
    engine.run_rules(after(1));

    // Two seconds of 20 ms blocks, more than the receiver renders ahead:
    // the first sets the modulation wheel, controller 1, to 99 at time 0.
    let mut blocks = vec![Vec::new(); 100];
    blocks[0] = vec![0, 0xB0, 1, 99];
    let mut patch = [0; interface::MOST_PATCH_BYTES];
    let length = interface::pack_patch(Model::Synth106, &Patch::DEFAULT, 0, 0, &mut patch);
    let bank = Writer::bank(BANK, 1, neoteletext_audio::Codec::Synth106, &patch[..length]).unwrap();
    let mut stored = Writer::events(HANDLE, 1, neoteletext_audio::Codec::Synth106, &blocks).unwrap();
    stored.banks = vec![BANK];
    let play =
        Command { channel: CHANNEL, sequence: 1, operation: 1, handle: HANDLE, generation: 1, gain: 100, fade_ms: 0, flags: 0, duck: 0, page: Some(PAGE) };
    let mut records = vec![bank.object_record().unwrap()];
    records.extend(bank.data_records());
    records.push(stored.object_record().unwrap());
    records.extend(stored.data_records());
    records.push(play.record());
    engine.open_page(Some(PAGE));
    engine.tick_audio(1_000);
    engine.feed(&packets(0, 0, &records).0.iter().flat_map(|packet| packet.bytes).collect::<Vec<u8>>());
    engine.finish();
    let sounds = engine.take_sounds();
    assert!(!sounds.iter().any(|audio| matches!(audio, Audio::EndStream { .. })), "it is still being rendered: {:?}", sounds.last());
    let mut seen = reports(&engine.run_rules(after(2)));
    seen.extend(reports(&engine.run_rules(after(3))));
    let read: Vec<(u8, Reported)> = seen.iter().map(|report| (report.rule, report.what)).collect();
    assert_eq!(
        read,
        [(0, Reported::Value { value: controller.to_bytes(), reading: 99 }), (1, Reported::Value { value: playing.to_bytes(), reading: 1 })],
        "the controller is read from the receiver's tracker, and the sound plays"
    );

    // The receiver renders the rest: the stream ends, and so does `playing`.
    for second in 2..6 {
        engine.tick_audio(second * 1_000);
    }
    assert!(engine.take_sounds().iter().any(|audio| matches!(audio, Audio::EndStream { .. })));
    let seen = reports(&engine.run_rules(after(4)));
    assert!(matches!(seen.as_slice(), [Report { rule: 2, what: Reported::Value { reading: 0, .. }, .. }]), "{seen:?}");
}
