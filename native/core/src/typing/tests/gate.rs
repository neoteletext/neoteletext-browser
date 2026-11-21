//! Tests of the trusted core as the gate of a page's rule table
//! (neoteletext-common's `docs/page-rules.md` RULE-50 to RULE-52; REQ-6523,
//! REQ-6534): nothing leaves for a page unless its table asks, and a
//! decoder, however hostile, can cause only the viewer's own input on its
//! own page to reach that page's own channel, within the budgets.

use super::*;
use neoteletext_input::caps::TOUCH_CAP;
use neoteletext_input::rules::RuleBody;
use protocol::interaction::TouchPhase;
use protocol::rules::{
    INTEREST_KEYS, INTEREST_MOVES, INTEREST_POINTER_BUTTONS, INTEREST_TEXT, INTEREST_WHEEL, KeyAction, PointerAction, Report, Reported, RuleInput, RulesOpen,
    SEND_BUDGET, SEND_WINDOW_TICKS, TIMED_SEND_BUDGET, WheelRect,
};

/// A session welcomed, its page open and on screen, and no table told.
fn untold() -> Typing {
    let mut typing = Typing::default();
    typing.input(PAGE.channel, true, false, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    typing.control(PAGE.channel, &welcome(SELECT_CAP | TOUCH_CAP), 0);
    typing.drawn(PAGE, 0);
    typing
}

fn asks(interest: u8) -> RulesOpen {
    RulesOpen { page: OPEN, generation: 1, interest, watch: Vec::new(), wheel: Vec::new() }
}

fn intent(intent: Intent) -> InputIntent {
    InputIntent { key: PAGE, intent }
}

fn point(column: u32, row: u32) -> Point {
    let (x, y) = (column.saturating_mul(12).saturating_add(3), row.saturating_mul(20).saturating_add(4));
    Point { column, row, pixels: Some((x, y)), mods: 0 }
}

/// One intent of every kind of input a viewer can make on a page.
fn every_kind() -> Vec<InputIntent> {
    vec![
        key('a'),
        intent(Intent::Key { action: Action::Release, key: KeyCode::Character('a'), mods: 0, text: None, base: None }),
        intent(Intent::Text("typed".into())),
        intent(Intent::Paste("pasted".into())),
        intent(Intent::Compose { cursor: 1, text: "ab".into() }),
        intent(Intent::Mouse { action: MouseAction::Press, button: 0, at: point(3, 4) }),
        intent(Intent::Mouse { action: MouseAction::Move, button: 0, at: point(4, 4) }),
        intent(Intent::Mouse { action: MouseAction::Release, button: 0, at: point(4, 4) }),
        intent(Intent::Wheel { dx: 0, dy: 120, at: point(4, 4) }),
        intent(Intent::Hover { leave: false, column: 4, row: 4 }),
        intent(Intent::Hover { leave: true, column: 4, row: 4 }),
        intent(Intent::Touch { phase: TouchPhase::Began, contact: 1, at: point(5, 5) }),
        intent(Intent::Touch { phase: TouchPhase::Ended, contact: 1, at: point(5, 5) }),
        intent(Intent::Gone),
    ]
}

/// What went to the fetcher and to the decoder.
fn went(sends: &[Send]) -> (usize, usize) {
    let fetcher = sends.iter().filter(|send| matches!(send, Send::Fetcher { .. })).count();
    let decoder = sends.iter().filter(|send| matches!(send, Send::Decoder { .. })).count();
    (fetcher, decoder)
}

/// The sequence numbers given in `sends`.
fn sequences(sends: &[Send]) -> Vec<u32> {
    sends.iter().filter_map(|send| if let Send::Decoder { sequence, .. } = send { Some(*sequence) } else { None }).collect()
}

fn report(what: Reported) -> Report {
    Report { page: OPEN, generation: 1, rule: 0, row: neoteletext_rules::limits::NO_RULE, tick: 5, target_kind: 0, target_number: 0, what }
}

/// The `RULE_EVENT`s among `sends`, each as its rule, its target's number
/// and what its body carries.
fn rule_events(sends: &[Send]) -> Vec<(u8, u16, String)> {
    let mut out = Vec::new();
    for send in sends {
        let Send::Fetcher { bytes, .. } = send else { continue };
        read_message(bytes, &mut |item| {
            if let Item::Event(Event::RuleEvent(rule)) = item {
                let body = match rule.body {
                    RuleBody::Fired => String::from("fired"),
                    RuleBody::Value { reading, .. } => format!("value {reading}"),
                    RuleBody::Input(_) => {
                        let (carried, pick) = rule.input().unwrap();
                        let kind = format!("{carried:?}");
                        let kind = kind.split(['(', ' ']).nth(1).unwrap_or("Text").to_owned();
                        if pick.is_some() { format!("{kind}+Pick") } else { kind }
                    }
                };
                out.push((rule.rule, rule.target.number, body));
            }
        })
        .unwrap();
    }
    out
}

/// The time `ticks` ticks on.
fn ticks(ticks: u32) -> u64 {
    neoteletext_core::consts::presentation_tick_due_us(ticks)
}

/// RULE-50 (REQ-6523): for a page whose view holds no table, nothing the
/// viewer does is given to its decoder or sent to its channel: no key,
/// text, paste, composition, press, release, move, wheel, hover, touch or
/// focus change. SELECT, the session's own, still goes.
#[test]
fn a_page_with_no_table_is_sent_no_input_of_any_kind() {
    let mut typing = untold();
    for (index, input) in every_kind().iter().enumerate() {
        let sends = typing.input_intent(input, u64::try_from(index).unwrap() * 20 * MS);
        assert_eq!(went(&sends), (0, 0), "{:?}", input.intent);
    }
    assert_eq!(went(&typing.take_due(SECOND)), (0, 0), "no move waited");
    assert_eq!(went(&typing.focused(false, SECOND)), (0, 0), "no FOCUS_OUT");
    assert_eq!(went(&typing.focused(true, SECOND)), (0, 0), "no FOCUS_IN");
    let hover = typing.drawing_hover(PAGE, Stamp { page: PAGE.page, sub: 0 }, false, 4, 1, SECOND);
    assert_eq!(went(&hover), (0, 0), "no DRAWING_HOVER");
    // A decoder that reports all the same is given nothing to report.
    for sequence in 0..16 {
        assert_eq!(went(&typing.report(PAGE.channel, &report(Reported::Input { sequence }), SECOND)), (0, 0));
    }
    assert_eq!(went(&typing.report(PAGE.channel, &report(Reported::Fired), SECOND)), (0, 0), "nor a firing: no table was told");
    let next = PageKey { page: 0x201, ..PAGE };
    assert_eq!(selects(&typing.opened(Some(PAGE.channel), Some(next), SECOND)).len(), 1, "SELECT still tells the channel the page");
}

/// A second, in microseconds.
const SECOND: u64 = 1_000_000;

/// RULE-52 item 1 (REQ-6534): only the classes of input in the table's
/// interest mask are given, and each with a sequence number of its own.
#[test]
fn only_the_classes_a_table_asks_for_are_given() {
    let mut typing = untold();
    typing.rules_told(PAGE.channel, &asks(INTEREST_KEYS), 0);
    let mut given = Vec::new();
    for (index, input) in every_kind().iter().enumerate() {
        given.extend(typing.input_intent(input, u64::try_from(index).unwrap() * 20 * MS));
    }
    assert_eq!(items(&given).len(), 2, "the key's press and its release, and nothing of the pointer, text or wheel: {given:?}");
    assert_eq!(sequences(&given), [0, 1]);
    assert_eq!(went(&typing.focused(false, SECOND)), (0, 0), "the focus was not asked for");

    let mut typing = untold();
    typing.rules_told(PAGE.channel, &asks(INTEREST_POINTER_BUTTONS), 0);
    let mut given = Vec::new();
    for (index, input) in every_kind().iter().enumerate() {
        given.extend(typing.input_intent(input, u64::try_from(index).unwrap() * 20 * MS));
    }
    let inputs: Vec<RuleInput> = given.iter().filter_map(|send| if let Send::Decoder { input, .. } = send { Some(*input) } else { None }).collect();
    assert!(
        matches!(
            inputs.as_slice(),
            [
                RuleInput::Pointer { action: PointerAction::Press, x: 39, y: 84, .. },
                RuleInput::Pointer { action: PointerAction::Release, .. },
                RuleInput::Pointer { action: PointerAction::Press, .. },
                RuleInput::Pointer { action: PointerAction::Release, .. }
            ]
        ),
        "the mouse's press and release and the finger's, at the picture's pixel, and no move: {inputs:?}"
    );
}

/// RULE-52 item 3, RULE-46 (REQ-6534): a report that names an input by
/// its sequence number is answered with a `RULE_EVENT` this core builds
/// from its own kept copy of the event, naming the rule and its target.
#[test]
fn a_report_is_built_from_the_cores_own_copy_of_the_event() {
    let mut typing = ready(SELECT_CAP);
    let given = typing.input_intent(&key('a'), MS);
    assert!(
        matches!(given.as_slice(), [Send::Decoder { page: OPEN, sequence, input: RuleInput::Key { action: KeyAction::Press, named: false, code: 97, .. }, .. }] if *sequence == 1),
        "{given:?}"
    );
    let asked = Report { rule: 7, target_kind: 1, target_number: 3, ..report(Reported::Input { sequence: 1 }) };
    let sent = typing.report(PAGE.channel, &asked, 2 * MS);
    assert_eq!(rule_events(&sent), [(7, 3, "Key".to_owned())]);
    assert_eq!(items(&sent), [KEY_A], "the key exactly as it was encoded when the viewer pressed it");
    let fired = typing.report(PAGE.channel, &report(Reported::Fired), 3 * MS);
    assert_eq!(rule_events(&fired), [(0, 0, "fired".to_owned())]);
    let value = [1, 0, 0, 0, 0, 0, 0, 0];
    let valued = typing.report(PAGE.channel, &report(Reported::Value { value, reading: -4 }), 4 * MS);
    assert_eq!(rule_events(&valued), [(0, 0, "value -4".to_owned())]);
}

/// RULE-52 item 3 (REQ-6534), the hostile-decoder harness: a decoder that
/// asks to send a sequence number it was never given, one of another
/// page, an old one, or the same one twice for one rule, gets none of
/// them sent; nor one under a generation that is not the table's.
#[test]
fn a_hostile_decoder_cannot_send_what_it_was_not_given() {
    let mut typing = ready(SELECT_CAP);
    let given = typing.input_intent(&key('a'), MS);
    let sequence = sequences(&given)[0];
    let input = |sequence| report(Reported::Input { sequence });
    assert_eq!(went(&typing.report(PAGE.channel, &input(sequence + 1), 2 * MS)), (0, 0), "never given");
    assert_eq!(went(&typing.report(PAGE.channel, &input(u32::MAX), 2 * MS)), (0, 0), "never given");
    let other_page = Report { page: protocol::rules::PageSub::new(0x201, 0), ..input(sequence) };
    assert_eq!(went(&typing.report(PAGE.channel, &other_page, 2 * MS)), (0, 0), "another page");
    let other_sub = Report { page: protocol::rules::PageSub::new(PAGE.page, 1), ..input(sequence) };
    assert_eq!(went(&typing.report(PAGE.channel, &other_sub, 2 * MS)), (0, 0), "another sub-code of the page, which is another page (RULE-112)");
    assert_eq!(went(&typing.report(8, &input(sequence), 2 * MS)), (0, 0), "another channel's decoder");
    let other_table = Report { generation: 2, ..input(sequence) };
    assert_eq!(went(&typing.report(PAGE.channel, &other_table, 2 * MS)), (0, 0), "a generation that is not the table's");
    assert_eq!(went(&typing.report(PAGE.channel, &input(sequence), 2 * MS)), (1, 0), "the one it was given");
    assert_eq!(went(&typing.report(PAGE.channel, &input(sequence), 2 * MS)), (0, 0), "the same one twice for one rule and row");
    let other_rule = Report { rule: 1, ..input(sequence) };
    assert_eq!(went(&typing.report(PAGE.channel, &other_rule, 2 * MS)), (1, 0), "once for each rule that sends it");
    let first_row = Report { rule: 1, row: 0, ..input(sequence) };
    assert_eq!(went(&typing.report(PAGE.channel, &first_row, 2 * MS)), (1, 0), "once for each row of a rule (RULE-52 item 3)");
    assert_eq!(went(&typing.report(PAGE.channel, &first_row, 2 * MS)), (0, 0), "and only once for that row");
    // An old one: given more than SEND_WINDOW_TICKS ago.
    let given = typing.input_intent(&key('b'), 100 * MS);
    let old = sequences(&given)[0];
    let late = 100 * MS + ticks(SEND_WINDOW_TICKS) + 1;
    assert_eq!(went(&typing.report(PAGE.channel, &input(old), late)), (0, 0), "past the window");
    let given = typing.input_intent(&key('c'), 300 * MS);
    assert_eq!(went(&typing.report(PAGE.channel, &input(sequences(&given)[0]), 300 * MS + ticks(SEND_WINDOW_TICKS))), (1, 0), "within it");
    assert_eq!(
        typing.rules_refused(PAGE.channel),
        8,
        "each refusal counted: never given twice, another page, another sub-code, another generation, twice for a rule, twice for a row, too old"
    );
}

/// RULE-52 item 5, RULE-48 (REQ-6534): a decoder that asks for 1,000
/// reports in a second gets at most `SEND_BUDGET` sent, of which at most
/// `TIMED_SEND_BUDGET` with no input event, and what was dropped is told
/// the channel as fault 3, once in the second.
#[test]
fn reports_are_held_to_the_budgets_and_the_rest_reported_as_dropped() {
    let mut typing = ready(SELECT_CAP);
    let mut sent = Vec::new();
    for n in 0..1_000_u64 {
        sent.extend(typing.report(PAGE.channel, &report(Reported::Fired), n * MS / 2));
    }
    let fired = rule_events(&sent).len();
    assert!(fired <= usize::try_from(TIMED_SEND_BUDGET).unwrap() + 10, "{fired} bare firings in half a second");
    assert!(fired >= usize::try_from(TIMED_SEND_BUDGET).unwrap());
    let faults: Vec<String> = items(&sent).into_iter().filter(|item| item.contains("RulesFault")).collect();
    assert_eq!(faults.len(), 1, "fault 3, once in the second: {faults:?}");
    assert!(faults[0].contains("ReportsDropped"), "{faults:?}");

    let mut typing = ready(SELECT_CAP);
    let mut sent = 0;
    for n in 0..1_000_u32 {
        let given = typing.input_intent(&key(char::from(b'a' + u8::try_from(n % 26).unwrap())), u64::from(n) * 100);
        sent += rule_events(&reported(&mut typing, &given, u64::from(n) * 100)).len();
    }
    // The session's own rate of 120 units a second (REQ-INT-109) holds
    // them too, with the RESIZE its start sent.
    let budget = usize::try_from(SEND_BUDGET).unwrap();
    assert!((budget - 1..=budget + 12).contains(&sent), "{sent} input reports in a tenth of a second");
}

/// RULE-52 item 4, RULE-5 (REQ-6534): reports go only while the page is
/// the open page of its view; its decoder's final tick may still report
/// its `close` rule just after, and nothing later.
#[test]
fn reports_go_only_for_the_open_page() {
    let mut typing = ready(SELECT_CAP);
    let given = typing.input_intent(&key('a'), MS);
    let left = typing.opened(Some(PAGE.channel), None, 2 * MS);
    assert!(left.contains(&Send::RulesPage { channel: PAGE.channel, page: None }), "the decoder is told no page is the open page: {left:?}");
    assert_eq!(went(&typing.input_intent(&key('b'), 3 * MS)), (0, 0), "nothing more is given");
    let closing = typing.report(PAGE.channel, &report(Reported::Fired), 3 * MS);
    assert_eq!(rule_events(&closing), [(0, 0, "fired".to_owned())], "the close rule's report, in the final tick");
    let _ = given;
    let late = typing.report(PAGE.channel, &report(Reported::Fired), 2 * MS + ticks(SEND_WINDOW_TICKS) + 1);
    assert_eq!(went(&late), (0, 0), "past the window nothing goes for a page that is not open");
    // Once its decoder says the page asks for nothing, nothing goes at all.
    let mut typing = ready(SELECT_CAP);
    typing.opened(Some(PAGE.channel), None, 2 * MS);
    typing.rules_told(PAGE.channel, &RulesOpen { generation: 0, interest: 0, wheel: Vec::new(), ..send_all() }, 2 * MS);
    assert_eq!(went(&typing.report(PAGE.channel, &report(Reported::Fired), 2 * MS)), (0, 0));
    // A decoder cannot make its page the open page by saying so.
    let other = RulesOpen { page: protocol::rules::PageSub::new(0x300, 0), ..send_all() };
    assert!(!typing.rules_told(PAGE.channel, &other, 3 * MS).1);
    let from_other = Report { page: protocol::rules::PageSub::new(0x300, 0), ..report(Reported::Fired) };
    assert_eq!(went(&typing.report(PAGE.channel, &from_other, 3 * MS)), (0, 0));
}

/// RULE-52 item 6, RULE-47 (REQ-6534): a `PAGE_STATE` is passed only in
/// answer to a query the channel's fetcher received for that page, once,
/// and only when it begins with that query's identifier.
#[test]
fn a_page_state_passes_only_in_answer_to_a_query_once() {
    let mut typing = ready(SELECT_CAP);
    let mut body = [0_u8; 64];
    let state = neoteletext_rules::state::PageState { identifier: 77, table: None };
    let length = neoteletext_rules::state::encode(&state, &mut body).unwrap();
    let body = &body[..length];
    let states = |sends: &[Send]| items(sends).into_iter().filter(|item| item.contains("PageState")).count();
    assert_eq!(states(&typing.page_state(PAGE.channel, OPEN, 77, body, MS)), 0, "no query was received");
    assert!(!typing.rules_query(PAGE.channel, protocol::rules::PageSub::new(0x201, 0), 77), "a query for a page that is not open is not passed on");
    assert!(!typing.rules_query(PAGE.channel, protocol::rules::PageSub::new(PAGE.page, 1), 77), "nor for another sub-code of it (RULE-112)");
    assert!(typing.rules_query(PAGE.channel, OPEN, 77));
    assert_eq!(states(&typing.page_state(PAGE.channel, OPEN, 78, body, MS)), 0, "another identifier");
    assert_eq!(states(&typing.page_state(PAGE.channel, OPEN, 77, body, MS)), 1);
    assert_eq!(states(&typing.page_state(PAGE.channel, OPEN, 77, body, MS)), 0, "once");
    assert!(typing.rules_query(PAGE.channel, OPEN, 79));
    assert_eq!(states(&typing.page_state(PAGE.channel, OPEN, 79, body, MS)), 0, "an answer that does not begin with its query's identifier");
}

/// RULE-51 item 4 (REQ-6534): the wheel is the page's only while the
/// pointer is inside a rectangle its decoder said an enabled `wheel` rule
/// applies in; the interface is told as the pointer crosses it, and the
/// trusted core gives the wheel nowhere else.
#[test]
fn the_wheel_is_the_pages_only_inside_a_wheel_rule() {
    let mut typing = untold();
    let rect = WheelRect { x: 120, y: 100, width: 60, height: 40 };
    typing.rules_told(PAGE.channel, &RulesOpen { wheel: vec![rect], ..asks(INTEREST_WHEEL | INTEREST_MOVES) }, 0);
    let wheel = |column, row| intent(Intent::Wheel { dx: 0, dy: 120, at: point(column, row) });
    let wheels = |sends: &[Send]| sends.iter().filter(|send| matches!(send, Send::Decoder { input: RuleInput::Wheel { .. }, .. })).count();
    let told = |sends: &[Send]| sends.iter().find_map(|send| if let Send::Interface(state) = send { Some(state.wheel) } else { None });
    // Cell (2, 2) is at pixel (27, 44): outside.
    assert_eq!(wheels(&typing.input_intent(&wheel(2, 2), MS)), 0);
    // Cell (10, 5) is at pixel (123, 104): inside.
    let inside = typing.input_intent(&intent(Intent::Mouse { action: MouseAction::Move, button: 0, at: point(10, 5) }), 40 * MS);
    assert_eq!(told(&inside), Some(true), "the interface is told the wheel is the page's here: {inside:?}");
    assert_eq!(wheels(&typing.input_intent(&wheel(10, 5), 60 * MS)), 1);
    let outside = typing.input_intent(&intent(Intent::Mouse { action: MouseAction::Move, button: 0, at: point(2, 2) }), 100 * MS);
    assert_eq!(told(&outside), Some(false));
    assert_eq!(wheels(&typing.input_intent(&wheel(2, 2), 120 * MS)), 0, "the browser's own again");
    // With no wheel rule, nowhere.
    let mut typing = untold();
    typing.rules_told(PAGE.channel, &asks(INTEREST_WHEEL), 0);
    assert_eq!(wheels(&typing.input_intent(&wheel(10, 5), MS)), 0);
}

/// The `asks` and `keys` the interface was last told among `sends`.
fn told_input(sends: &[Send]) -> Option<(bool, bool)> {
    sends.iter().rev().find_map(|send| if let Send::Interface(state) = send { Some((state.asks, state.keys)) } else { None })
}

/// REQ-6632 item 2 (DEC-6633 item 2): the interface is told whether the
/// open page's table asks for input, as its table is taken and as it ends,
/// so that over the picture of such a page the wheel is the page's and the
/// pointer the ordinary arrow; a table that asks for nothing, or no table,
/// leaves the wheel the browser's to step. On the code before, the
/// interface was told nothing of it.
///
/// REQ-7037: it is told too whether the table asks for the keyboard,
/// `keys`: true exactly while its interest holds the keys' class or the
/// text's (RULE-52 item 1), from the same mask as `asks`; told again when a
/// new table of the page asks otherwise, though `asks` is unchanged; false
/// once the table ends, while a page left closes it, for a table of another
/// sub-code, and when the connection goes. Before REQ-7037 the interface
/// had no such word and gave every key to the page.
#[test]
fn the_interface_hears_whether_the_open_pages_table_asks_for_input() {
    let told = |sends: &[Send]| sends.iter().find_map(|send| if let Send::Interface(state) = send { Some(state.asks) } else { None });
    let mut typing = untold();
    let (taken, open) = typing.rules_told(PAGE.channel, &asks(INTEREST_KEYS | INTEREST_POINTER_BUTTONS), 0);
    assert!(open);
    assert_eq!(told(&taken), Some(true), "a table asking for keys and presses: {taken:?}");
    let (ended, _) = typing.rules_told(PAGE.channel, &RulesOpen { generation: 0, ..asks(0) }, MS);
    assert_eq!(told(&ended), Some(false), "the table ended: {ended:?}");
    let mut typing = untold();
    let (nothing, _) = typing.rules_told(PAGE.channel, &RulesOpen { watch: Vec::new(), ..asks(0) }, 0);
    assert_ne!(told(&nothing), Some(true), "a table asking for no input: {nothing:?}");
    assert_eq!(typing.state().map(|state| state.keys), Some(false), "nor for the keyboard");
    // REQ-7037: `keys`, from the same mask, says whether the table asks
    // for the keyboard (see the doc comment above).
    let generation = |generation: u16, interest: u8| RulesOpen { generation, ..asks(interest) };
    for (interest, expected) in [
        (INTEREST_KEYS, (true, true)),
        (INTEREST_TEXT, (true, true)),
        (INTEREST_KEYS | INTEREST_POINTER_BUTTONS, (true, true)),
        (INTEREST_POINTER_BUTTONS, (true, false)),
        (INTEREST_MOVES, (true, false)),
        (INTEREST_WHEEL | INTEREST_MOVES, (true, false)),
        (neoteletext_rules::table::INTEREST_FOCUS, (true, false)),
    ] {
        let mut typing = untold();
        let (taken, open) = typing.rules_told(PAGE.channel, &asks(interest), 0);
        assert!(open);
        assert_eq!(told_input(&taken), Some(expected), "interest {interest:#08b}: {taken:?}");
    }
    // A new table of the same page: re-sent when only `keys` changes.
    let mut typing = untold();
    typing.rules_told(PAGE.channel, &asks(INTEREST_POINTER_BUTTONS), 0);
    let (texts, _) = typing.rules_told(PAGE.channel, &generation(2, INTEREST_POINTER_BUTTONS | INTEREST_TEXT), MS);
    assert_eq!(told_input(&texts), Some((true, true)), "a table that now asks for text: {texts:?}");
    let (pointer, _) = typing.rules_told(PAGE.channel, &generation(3, INTEREST_POINTER_BUTTONS), 2 * MS);
    assert_eq!(told_input(&pointer), Some((true, false)), "and back to the pointer alone: {pointer:?}");
    let (ended, _) = typing.rules_told(PAGE.channel, &generation(0, 0), 3 * MS);
    assert_eq!(told_input(&ended), Some((false, false)), "the table ended: {ended:?}");
    // Another sub-code on screen closes the table of the one left
    // (RULE-112): while it is closing it asks for nothing.
    let mut typing = untold();
    typing.rules_told(PAGE.channel, &asks(INTEREST_KEYS), 0);
    let other = typing.drawn(PAGE, 1);
    assert_eq!(told_input(&other), Some((false, false)), "another sub-code on screen: {other:?}");
    // A table for a sub-code that is not on screen is not taken.
    let (not_open, open) = typing.rules_told(PAGE.channel, &RulesOpen { page: protocol::rules::PageSub::new(PAGE.page, 3), ..asks(INTEREST_KEYS) }, MS);
    assert!(!open);
    assert_ne!(told_input(&not_open), Some((true, true)), "{not_open:?}");
    assert_eq!(typing.state().map(|state| state.keys), Some(false), "a table for sub-code 3");
    // The page left: its table closes and keys go to it no more.
    let mut typing = untold();
    typing.rules_told(PAGE.channel, &asks(INTEREST_KEYS), 0);
    let left = typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), MS);
    assert_eq!(told_input(&left), Some((false, false)), "another page opened: {left:?}");
    // The connection gone: `on` is false, and so is `keys`.
    let mut typing = untold();
    typing.rules_told(PAGE.channel, &asks(INTEREST_KEYS), 0);
    let gone = typing.input(PAGE.channel, false, false, MS);
    assert!(
        matches!(gone.iter().rev().find_map(|send| if let Send::Interface(state) = send { Some(state) } else { None }), Some(state) if !state.on && !state.keys),
        "{gone:?}"
    );
}

/// TODO-7038, the gap REQ-7037 leaves for the owner: a table's interest is
/// by class, so a table whose only key rule names one key asks for the keys'
/// class, the interface is told `keys`, and every key the viewer presses
/// is given to the page's decoder, which drops those no rule names; none
/// leaves for the channel unless a rule reports it. This holds today's
/// behaviour so that a change to it is not unnoticed.
#[test]
fn a_table_naming_one_key_is_given_every_key() {
    let mut typing = untold();
    let (taken, _) = typing.rules_told(PAGE.channel, &asks(INTEREST_KEYS), 0);
    assert_eq!(told_input(&taken), Some((true, true)));
    for (index, character) in ['a', 'r', '1'].into_iter().enumerate() {
        let sends = typing.input_intent(&key(character), (u64::try_from(index).unwrap() + 1) * 20 * MS);
        assert!(matches!(sends.as_slice(), [Send::Decoder { input: RuleInput::Key { .. }, .. }]), "{character} is the table's: {sends:?}");
    }
}

/// RULE-46, RULE-52 item 1 (REQ-6534): the trusted core gives the control
/// under a press, and a control's enter and leave as the HOVER that names
/// it; a pointer event's PICK follows it inside its report.
#[test]
fn a_hover_names_its_control_and_a_pick_follows_its_pointer_event() {
    let mut typing = ready(SELECT_CAP);
    let inputs = |sends: &[Send]| sends.iter().filter_map(|send| if let Send::Decoder { input, .. } = send { Some(*input) } else { None }).collect::<Vec<_>>();
    typing.pointer_context(PAGE.channel, None, 0);
    let moved = typing.input_intent(&intent(Intent::Mouse { action: MouseAction::Move, button: 0, at: point(4, 4) }), MS);
    assert!(matches!(inputs(&moved).as_slice(), [RuleInput::Pointer { action: PointerAction::Move, control: 0, x: 51, y: 84, .. }]), "{moved:?}");
    typing.pointer_context(PAGE.channel, None, 2);
    let entered = typing.input_intent(&intent(Intent::Hover { leave: false, column: 4, row: 4 }), 2 * MS);
    assert!(matches!(inputs(&entered).as_slice(), [RuleInput::Pointer { action: PointerAction::Move, control: 2, x: 51, y: 84, .. }]), "{entered:?}");
    assert_eq!(rule_events(&reported(&mut typing, &entered, 2 * MS)), [(0, 0, "Hover".to_owned())], "carried as HOVER");
    let pressed = typing.input_intent(&intent(Intent::Mouse { action: MouseAction::Press, button: 0, at: point(4, 4) }), 3 * MS);
    assert!(matches!(inputs(&pressed).as_slice(), [RuleInput::Pointer { action: PointerAction::Press, control: 2, .. }]), "{pressed:?}");
    // The page is drawn by the ReGIS renderer: what lay under the pointer.
    let pick = event::Pick { on: Stamp { page: PAGE.page, sub: 0 }, drawing: 5, x: 1, y: 2, picture_x: 300, picture_y: 200, commits: 9 };
    assert!(typing.pick(PAGE, pick));
    assert_eq!(rule_events(&reported(&mut typing, &pressed, 3 * MS)), [(0, 0, "Mouse+Pick".to_owned())]);
    typing.pointer_context(PAGE.channel, None, 0);
    let left = typing.input_intent(&intent(Intent::Hover { leave: true, column: 4, row: 4 }), 4 * MS);
    assert!(matches!(inputs(&left).as_slice(), [RuleInput::Pointer { action: PointerAction::Move, control: 0, .. }]), "{left:?}");
}

/// RULE-48 (REQ-6534): a fault its decoder reports for the open page is
/// passed to the page's channel at most once a second for each code, and
/// not at all for a page that is not open.
#[test]
fn a_fault_is_passed_once_a_second_for_each_code() {
    let mut typing = ready(SELECT_CAP);
    let faults = |sends: &[Send]| items(sends).into_iter().filter(|item| item.contains("RulesFault")).collect::<Vec<_>>();
    let first = faults(&typing.rules_fault(PAGE.channel, OPEN, 1, 5, 3, MS));
    assert!(matches!(first.as_slice(), [fault] if fault.contains("CommandsSkipped") && fault.contains("count: 3")), "{first:?}");
    assert_eq!(faults(&typing.rules_fault(PAGE.channel, OPEN, 1, 5, 1, 2 * MS)).len(), 0, "the same code within the second");
    assert_eq!(faults(&typing.rules_fault(PAGE.channel, OPEN, 1, 4, 1, 2 * MS)).len(), 1, "another code");
    assert_eq!(faults(&typing.rules_fault(PAGE.channel, OPEN, 1, 5, 1, MS + SECOND)).len(), 1, "a second on");
    assert_eq!(faults(&typing.rules_fault(PAGE.channel, protocol::rules::PageSub::new(0x201, 0), 1, 4, 1, 3 * SECOND)).len(), 0, "a page that is not open");
    assert_eq!(faults(&typing.rules_fault(PAGE.channel, OPEN, 1, 2, 1, 3 * SECOND)).len(), 0, "the withdrawn code 2");
    assert_eq!(faults(&typing.rules_fault(PAGE.channel, OPEN, 1, 11, 1, 3 * SECOND)).len(), 0, "no such fault");
}
