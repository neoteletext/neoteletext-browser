//! Tests of typing into a page (`interaction.md`, DOC-1781).

use super::*;
use neoteletext_input::control::Control;
use neoteletext_input::event::{Action, CONTROL, KeyCode, MouseAction, Point, read_message};
use neoteletext_input::{VERSION, WELCOME_TIMEOUT_MS};

const PAGE: PageKey = PageKey { channel: 7, page: 0x200 };
/// SELECT of the page, with no sub-page named.
const SELECT_PAGE: &str = "Event(Select(Select { page: 512, sub: 16255 }, \"\"))";
/// The key `a` as the channel reads it: a KEY carrying its text (DOC-1935).
const KEY_A: &str =
    "Event(Key(Key { on: Stamp { page: 512, sub: 0 }, action: Press, key: Character('a'), mods: 0, text: Some(\"a\"), base: None, lost: false }))";

fn key(character: char) -> InputIntent {
    let intent = Intent::Key { action: Action::Press, key: KeyCode::Character(character), mods: 0, text: Some(character.to_string()), base: None };
    InputIntent { key: PAGE, intent }
}

/// Every item the sends gave the fetcher, as the channel reads them.
fn items(sends: &[Send]) -> Vec<String> {
    let mut out = Vec::new();
    for send in sends {
        if let Send::Fetcher { bytes, .. } = send {
            read_message(bytes, &mut |item| out.push(format!("{item:?}"))).unwrap();
        }
    }
    out
}

/// Microseconds in a millisecond, for times written in milliseconds.
const MS: u64 = 1_000;

fn welcome(caps: u32) -> Vec<u8> {
    welcome_with(caps, None)
}

/// A WELCOME with the session RESUME gives, when it is agreed.
fn welcome_with(caps: u32, resume: Option<neoteletext_input::control::Resumption>) -> Vec<u8> {
    let mut body = [0_u8; neoteletext_input::control::MAX_BODY];
    let length = neoteletext_input::control::write_body(&Control::Welcome { version: VERSION, caps, resume }, &mut body).unwrap();
    body[..length].to_vec()
}

fn ready(caps: u32) -> Typing {
    let mut typing = Typing::default();
    typing.input(PAGE.channel, true, false, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    typing.control(PAGE.channel, &welcome(caps), 0);
    typing.drawn(PAGE, 0);
    typing
}

/// REQ-INT-106: HELLO first, nothing before WELCOME, then RESIZE, the
/// focus and the selection, and no PAGE (REQ-1556).
#[test]
fn a_session_starts_with_hello_and_waits_for_welcome() {
    let mut typing = Typing::default();
    let opened = typing.input(PAGE.channel, true, true, 0);
    let offered = OFFERED | RESUME_CAP;
    assert_eq!(items(&opened), [format!("Event(Hello {{ version: {VERSION}, caps: {offered}, agent: None }})")]);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "nothing before WELCOME");
    let welcomed = typing.control(PAGE.channel, &welcome(SELECT_CAP | CURSOR_CAP), 5);
    assert_eq!(items(&welcomed), ["Event(Resize { columns: 40, rows: 25, width: 0, height: 0 })", "Event(FocusIn)", SELECT_PAGE]);
    assert!(welcomed.iter().any(|send| matches!(send, Send::Interface(state) if state.on && state.secure)));
    assert_eq!(items(&typing.input_intent(&key('a'), 5)), [] as [String; 0], "nothing on screen yet (REQ-1554)");
    typing.drawn(PAGE, 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 6)), [KEY_A]);
}

#[test]
fn a_channel_that_does_not_answer_is_refused_and_the_strip_says_so() {
    let mut typing = Typing::default();
    typing.input(PAGE.channel, true, false, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert_eq!(typing.tick(u64::from(WELCOME_TIMEOUT_MS) * MS - 1), []);
    let sends = typing.tick(u64::from(WELCOME_TIMEOUT_MS) * MS);
    assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.refused && !state.on));
    assert_eq!(items(&typing.control(PAGE.channel, &welcome(SELECT_CAP), 10_001 * MS)), [] as [String; 0], "too late");
}

/// Decision 99: a page takes a phone's keys only while it is open for
/// typing.
#[test]
fn a_page_takes_keys_only_while_open_for_typing() {
    let mut typing = Typing::default();
    assert!(!typing.page_takes_keys(), "nothing open");
    typing.input(PAGE.channel, true, false, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert!(!typing.page_takes_keys(), "not yet welcomed");
    assert!(ready(SELECT_CAP).page_takes_keys());
    let mut away = ready(SELECT_CAP);
    away.opened(Some(PAGE.channel), None, 0);
    assert!(!away.page_takes_keys(), "the page closed");
}

/// REQ-INT-130: input goes only under every condition.
#[test]
fn input_goes_only_under_every_condition() {
    let mut typing = Typing::default();
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "no socket, no page");
    typing.input(PAGE.channel, true, false, 0);
    typing.control(PAGE.channel, &welcome(SELECT_CAP), 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "no page open");
    typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "another page open");
    typing.opened(Some(8), None, 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "another tab");
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "nothing on screen");
    typing.drawn(PAGE, 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [KEY_A]);
    let away = typing.focused(false, 0);
    let release = KEY_A.replace("Press", "Release").replace("Some(\"a\")", "None").replace("lost: false", "lost: true");
    assert_eq!(items(&away), [release, "Event(FocusOut)".to_owned()], "the key held goes up, lost");
    assert_eq!(items(&typing.input_intent(&key('b'), 0)), [] as [String; 0], "the window has no focus");
    assert_eq!(items(&typing.focused(true, 0)), ["Event(FocusIn)"]);
    typing.input(PAGE.channel, false, false, 0);
    assert_eq!(items(&typing.input_intent(&key('c'), 0)), [] as [String; 0], "the socket closed");
}

/// REQ-1554 item 4, REQ-1553: the page losing the focus says `FOCUS_OUT`
/// and never touches the selection, so its updates go on.
#[test]
fn losing_the_focus_never_changes_the_selection() {
    let mut typing = ready(SELECT_CAP);
    let leaving = typing.opened(Some(PAGE.channel), None, 5);
    assert_eq!(items(&leaving), ["Event(FocusOut)"], "no SELECT");
    assert!(leaving.iter().any(|send| matches!(send, Send::Interface(state) if !state.on)));
    assert_eq!(typing.selection(PAGE.channel), Some((PAGE.page, SUBCODE_ANY)));
    let back = typing.opened(Some(PAGE.channel), Some(PAGE), 6);
    assert_eq!(items(&back), ["Event(FocusIn)"], "no SELECT");
    // Another tab in front: the same.
    assert_eq!(items(&typing.opened(Some(8), None, 7)), ["Event(FocusOut)"]);
    assert_eq!(typing.selection(PAGE.channel), Some((PAGE.page, SUBCODE_ANY)));
}

/// REQ-1554: the wall of pages with none zoomed selects nothing.
#[test]
fn the_wall_selects_nothing_and_a_page_after_it_is_selected_again() {
    let mut typing = ready(SELECT_CAP);
    typing.opened(Some(PAGE.channel), None, 5);
    assert_eq!(items(&typing.selects_nothing(PAGE.channel, 6)), ["Event(Select(Select { page: 0, sub: 16255 }, \"\"))"]);
    let back = typing.opened(Some(PAGE.channel), Some(PAGE), 7);
    assert_eq!(items(&back), [SELECT_PAGE, "Event(FocusIn)"]);
    assert_eq!(items(&typing.input_intent(&key('a'), 8)), [] as [String; 0], "nothing on screen until the page is drawn again");
}

/// REQ-1555: input is stamped with the page on screen, not the selection:
/// a key after a new SELECT, before its frame, carries the old page.
#[test]
fn input_is_stamped_with_the_page_on_screen() {
    let mut typing = ready(SELECT_CAP);
    let next = PageKey { page: 0x201, ..PAGE };
    assert_eq!(items(&typing.opened(Some(PAGE.channel), Some(next), 5)), ["Event(Select(Select { page: 513, sub: 16255 }, \"\"))"]);
    let sent = items(&typing.input_intent(&InputIntent { key: next, intent: key('a').intent }, 6));
    assert_eq!(sent, [KEY_A], "the old page's stamp, which the channel drops");
    typing.drawn(next, 0x0003);
    let sent = items(&typing.input_intent(&InputIntent { key: next, intent: key('a').intent }, 7));
    assert_eq!(sent, [KEY_A.replace("page: 512, sub: 0", "page: 513, sub: 3")]);
}

/// REQ-207: a key held when the page loses the input goes up marked lost,
/// stamped with the page that held it, before `FOCUS_OUT` or SELECT.
#[test]
fn a_held_key_is_released_lost_when_the_input_ends() {
    let lost = "Event(Key(Key { on: Stamp { page: 512, sub: 0 }, action: Release, key: Character('a'), mods: 0, text: None, base: None, lost: true }))";
    let mut typing = ready(SELECT_CAP);
    typing.input_intent(&key('a'), 5);
    assert_eq!(items(&typing.focused(false, 6)), [lost, "Event(FocusOut)"]);
    let mut typing = ready(SELECT_CAP);
    typing.input_intent(&key('a'), 5);
    let next = PageKey { page: 0x201, ..PAGE };
    assert_eq!(items(&typing.opened(Some(PAGE.channel), Some(next), 6)), [lost, "Event(Select(Select { page: 513, sub: 16255 }, \"\"))"]);
    // A key that came up is not let go again.
    let mut typing = ready(SELECT_CAP);
    typing.input_intent(&key('a'), 5);
    let up = Intent::Key { action: Action::Release, key: KeyCode::Character('a'), mods: 0, text: None, base: None };
    typing.input_intent(&InputIntent { key: PAGE, intent: up }, 6);
    assert_eq!(items(&typing.focused(false, 7)), ["Event(FocusOut)"]);
}

/// A part of the browser taking the keys, not only the address field,
/// takes the page's focus (REQ-1554 item 1); the selection stays.
#[test]
fn the_keys_going_elsewhere_keep_the_selection() {
    let mut typing = ready(SELECT_CAP);
    assert_eq!(items(&typing.editing(true, 1)), ["Event(FocusOut)"]);
    assert_eq!(typing.selection(PAGE.channel), Some((PAGE.page, SUBCODE_ANY)));
}

// DEC-1707: a redirect the core did not follow leaves the server with a
// selection the tab does not have, so the tab says its own again.
#[test]
fn a_selection_is_said_again_when_asked() {
    let mut typing = ready(SELECT_CAP);
    assert_eq!(items(&typing.select_again(PAGE.channel, MS)), [SELECT_PAGE]);
    assert_eq!(items(&typing.select_again(0, MS)), [] as [String; 0], "no such channel");
}

#[test]
fn editing_the_address_takes_the_keys_from_the_page_and_says_so() {
    let mut typing = ready(SELECT_CAP);
    assert_eq!(items(&typing.editing(true, 1)), ["Event(FocusOut)"]);
    assert_eq!(items(&typing.input_intent(&key('b'), 2)), [] as [String; 0], "the keys are the strip's");
    // The window losing the focus while editing changes nothing for the
    // page, nor does it coming back.
    assert_eq!(items(&typing.focused(false, 3)), [] as [String; 0]);
    assert_eq!(items(&typing.focused(true, 4)), [] as [String; 0]);
    assert_eq!(items(&typing.editing(false, 5)), ["Event(FocusIn)"]);
    assert_eq!(items(&typing.input_intent(&key('c'), 6)).len(), 1);
}

/// REQ-INT-106: a reconnection is a new session.
#[test]
fn a_new_connection_is_a_new_session() {
    let mut typing = ready(SELECT_CAP);
    typing.input(PAGE.channel, false, false, 1);
    let again = typing.input(PAGE.channel, true, true, 2);
    assert_eq!(items(&again).len(), 1);
    assert!(items(&again)[0].starts_with("Event(Hello"));
    assert_eq!(items(&typing.input_intent(&key('a'), 3)), [] as [String; 0], "waiting for WELCOME again");
}

/// DOC-1935 (decision E.125): every key goes as KEY with its
/// text, a command modifier's key with none, and every release.
#[test]
fn every_key_goes_as_a_key_with_its_text() {
    let mut typing = ready(SELECT_CAP);
    let control_c = Intent::Key { action: Action::Press, key: KeyCode::Character('c'), mods: CONTROL, text: None, base: None };
    let sends = typing.input_intent(&InputIntent { key: PAGE, intent: control_c }, 0);
    assert_eq!(
        items(&sends),
        ["Event(Key(Key { on: Stamp { page: 512, sub: 0 }, action: Press, key: Character('c'), mods: 4, text: None, base: None, lost: false }))"]
    );
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [KEY_A]);
    let release = Intent::Key { action: Action::Release, key: KeyCode::Character('a'), mods: 0, text: None, base: None };
    assert_eq!(items(&typing.input_intent(&InputIntent { key: PAGE, intent: release }, 0)).len(), 1, "a release");
}

/// REQ-INT-116 rule 3: a decomposed `e` and combining accent go as
/// U+00E9, as text and as a KEY's text.
#[test]
fn text_goes_in_normalization_form_c() {
    let mut typing = ready(SELECT_CAP);
    let text = InputIntent { key: PAGE, intent: Intent::Text("e\u{301}".into()) };
    assert_eq!(items(&typing.input_intent(&text, 0)), ["Text(\"\u{e9}\")"]);
    let key = Intent::Key { action: Action::Press, key: KeyCode::Character('e'), mods: 0, text: Some("e\u{301}".into()), base: None };
    let sent = items(&typing.input_intent(&InputIntent { key: PAGE, intent: key }, 0));
    assert_eq!(
        sent,
        ["Event(Key(Key { on: Stamp { page: 512, sub: 0 }, action: Press, key: Character('e'), mods: 0, text: Some(\"\u{e9}\"), base: None, lost: false }))"]
    );
    let paste = InputIntent { key: PAGE, intent: Intent::Paste("e\u{301}".into()) };
    assert_eq!(items(&typing.input_intent(&paste, 0)), [r#"Event(Paste { on: Stamp { page: 512, sub: 0 }, text: "e\u{301}" })"#], "a paste as copied");
}

/// REQ-INT-115 (decision E.125): a click goes with no request, and a
/// withdrawn `PAGE_MODES` record changes nothing.
#[test]
fn the_mouse_always_goes() {
    let mut typing = ready(SELECT_CAP);
    let click = InputIntent {
        key: PAGE,
        intent: Intent::Mouse { action: MouseAction::Press, button: 0, at: Point { column: 10, row: 5, pixels: Some((125, 110)), mods: 0 } },
    };
    let expected = [
        "Event(Mouse(Mouse { on: Stamp { page: 512, sub: 0 }, action: Press, button: 0, at: Point { column: 10, row: 5, pixels: Some((125, 110)), mods: 0 }, device: Mouse }))",
    ];
    assert_eq!(items(&typing.input_intent(&click, 0)), expected);
    let release = InputIntent {
        intent: Intent::Mouse { action: MouseAction::Release, button: 0, at: Point { column: 10, row: 5, pixels: Some((125, 110)), mods: 0 } },
        ..click.clone()
    };
    typing.input_intent(&release, 0);
    typing.control(PAGE.channel, &[0xFF, 0x04, 0x80, 0x04, 0x08], 0);
    assert_eq!(items(&typing.input_intent(&click, 0)), expected, "an older channel's PAGE_MODES");
}

/// REQ-RCH-005: every button and the modifiers go as DOC-1938
/// lays MOUSE out: `0xFF`, the type, the flags (the action, bit 2 for the
/// dot, bit 3 for the modifiers), the button, the cell, the dot and the
/// modifiers, each press and its release.
#[test]
fn every_button_and_modifier_goes_as_input_md_lays_it_out() {
    use interface_core::input::mouse_button::{BACK, FORWARD, LEFT, MIDDLE, RIGHT};
    use neoteletext_input::event::{ALT, SHIFT};
    /// MOUSE's flags: a release, the dot present, the modifiers present.
    const RELEASE: u8 = 1;
    const DOT: u8 = 1 << 2;
    const MODS: u8 = 1 << 3;
    /// Page 200 and sub-code 0 as two varints.
    const STAMP: [u8; 3] = [0x80, 0x04, 0x00];
    let mut typing = ready(SELECT_CAP);
    let at = Point { column: 10, row: 5, pixels: Some((125, 110)), mods: SHIFT | ALT | CONTROL };
    for button in [LEFT, MIDDLE, RIGHT, BACK, FORWARD] {
        for (action, flags) in [(MouseAction::Press, DOT | MODS), (MouseAction::Release, RELEASE | DOT | MODS)] {
            let intent = InputIntent { key: PAGE, intent: Intent::Mouse { action, button, at } };
            let sent: Vec<Vec<u8>> = typing
                .input_intent(&intent, 0)
                .into_iter()
                .filter_map(|send| if let Send::Fetcher { bytes, .. } = send { Some(bytes) } else { None })
                .collect();
            assert_eq!(
                sent,
                [vec![0xFF, 0x02, STAMP[0], STAMP[1], STAMP[2], flags, button, 10, 5, 125, 110, SHIFT | ALT | CONTROL]],
                "button {button}, {action:?}"
            );
        }
    }
}

/// REQ-INT-110: a cursor record places the cursor, with its block flag
/// alone, since the browser echoes nothing; it gives no round trip, which
/// is the push connection's own (REQ-RCH-016); and one naming another
/// page is the host going there (REQ-INT-113).
#[test]
fn a_cursor_record_places_the_cursor() {
    let mut typing = ready(SELECT_CAP);
    // HELLO, RESIZE and FOCUS_IN were 3.
    typing.input_intent(&key('a'), 100 * MS);
    typing.input_intent(&key('b'), 110 * MS);
    // A channel written before the engine's decision E.134 still sends an
    // echo width and the echo flag (1): read, and not told.
    let body = |row: u8, acked: u8| [0x02, 0x00, 0, acked, row, 4, 10, 1 | BLOCK];
    assert_eq!(typing.cursor(PAGE.channel, body(24, 5)), [], "row 24 is refused");
    let sends = typing.cursor(PAGE.channel, body(5, 5));
    let Some(Send::Interface(state)) = sends.last() else { panic!("{sends:?}") };
    assert_eq!(state.cursor, Some(CursorState { row: 5, column: 4, flags: BLOCK }), "no echo is told");
    assert_eq!(state.round_trip_us, None);
}

/// REQ-INT-137: no code that handles the viewer's input prints or logs
/// anything, so no key can reach a log. The files are the trusted
/// core's typing, the interface's keys and intents, and the protocol's
/// intents; the keyboard process prints only why it stopped.
#[test]
fn no_input_code_logs() {
    let sources = [
        ("core/src/typing.rs", include_str!("../typing.rs")),
        ("core/src/typing/pointer.rs", include_str!("pointer.rs")),
        ("core/src/trusted_core/typing.rs", include_str!("../trusted_core/typing.rs")),
        ("interface-core/src/browser/typing.rs", include_str!("../../../interface-core/src/browser/typing.rs")),
        ("interface-core/src/browser/typing/draw.rs", include_str!("../../../interface-core/src/browser/typing/draw.rs")),
        ("interface-core/src/input.rs", include_str!("../../../interface-core/src/input.rs")),
        ("interface/src/keys.rs", include_str!("../../../interface/src/keys.rs")),
        ("protocol/src/interaction.rs", include_str!("../../../protocol/src/interaction.rs")),
        ("protocol/src/interaction/intent.rs", include_str!("../../../protocol/src/interaction/intent.rs")),
        ("protocol/src/interaction/typing.rs", include_str!("../../../protocol/src/interaction/typing.rs")),
    ];
    // The keyboard process prints one thing: why it stopped, which is
    // the relay's or the pipe's error, never a key.
    let keyboard = include_str!("../../../keyboard/src/keyboard.rs");
    assert_eq!(keyboard.matches(concat!("eprint", "ln!")).count(), 1);
    assert!(keyboard.contains(concat!("eprint", "ln!(\"neoteletext-keyboard: {error}\")")));
    // Spelled in pieces, so this test's own text does not match.
    let calls =
        [concat!("print", "ln!"), concat!("eprint", "ln!"), concat!("print", "!("), concat!("eprint", "!("), concat!("dbg", "!("), concat!("log", "::")];
    for (path, source) in sources {
        for call in calls {
            assert!(!source.contains(call), "{path} has {call}");
        }
    }
}

// DEC-1526: an adopted sub-page is the selection, so a session started
// again selects it and not 3F7F. DEC-1527: and a tab's SELECT goes only on
// its own connection.
#[test]
fn the_selection_is_each_tabs_own_and_survives_a_new_session() {
    let mut typing = ready(SELECT_CAP);
    typing.adopt(PAGE.channel, PAGE.page, 1);
    typing.input(8, true, false, 0);
    typing.control(8, &welcome(SELECT_CAP), 0);
    let other = typing.select(8, 0x300, 2, "", 0);
    assert!(other.iter().all(|send| matches!(send, Send::Fetcher { channel: 8, .. })));
    assert_eq!(typing.selection(PAGE.channel), Some((PAGE.page, 1)), "the other tab's selection is its own");
    assert_eq!(typing.selection(8), Some((0x300, 2)));
    typing.input(PAGE.channel, false, false, 0);
    typing.input(PAGE.channel, true, false, 0);
    let again = items(&typing.control(PAGE.channel, &welcome(SELECT_CAP), 0));
    assert!(again.contains(&"Event(Select(Select { page: 512, sub: 1 }, \"\"))".to_owned()), "{again:?}");
}

/// REQ-INT-109: 120 units a second.
#[test]
fn the_rate_is_limited() {
    let mut typing = ready(SELECT_CAP);
    let mut sent = 0;
    for n in 0..200_u32 {
        sent += items(&typing.input_intent(&key(char::from(b'a' + u8::try_from(n % 26).unwrap())), 0)).len();
    }
    assert_eq!(sent, usize::try_from(neoteletext_input::MAX_PER_SECOND).unwrap() - 2, "RESIZE and FOCUS_IN counted");
}

mod pointer;
mod resume;

/// REQ-1693 item 6, DEC-1717 item 11: page parameters go in the SELECT of
/// their selection. The same page with others is a new selection, and any
/// other selection leaves them behind.
#[test]
fn select_carries_the_parameters_of_its_selection() {
    let mut typing = ready(SELECT_CAP);
    let channel = PAGE.channel;
    let with = |sub: u32, parameters: &str| format!("Event(Select(Select {{ page: 513, sub: {sub} }}, {parameters:?}))");
    let told = typing.select(channel, 0x201, SUBCODE_ANY, "a=1", 1);
    assert_eq!(items(&told), [with(u32::from(SUBCODE_ANY), "a=1")]);
    assert!(items(&typing.select(channel, 0x201, SUBCODE_ANY, "a=1", 2)).is_empty(), "said once");
    let other = typing.select(channel, 0x201, SUBCODE_ANY, "b=2", 3);
    assert_eq!(items(&other), [with(u32::from(SUBCODE_ANY), "b=2")]);
    assert_eq!(items(&typing.select_again(channel, 4)), [with(u32::from(SUBCODE_ANY), "b=2")], "said again, whole");
    let left = typing.select(channel, 0x201, 1, "", 5);
    assert_eq!(items(&left), [with(1, "")], "a sub-page of it leaves them behind");
    typing.select(channel, 0x201, SUBCODE_ANY, "a=1", 6);
    let next = PageKey { channel, page: 0x202 };
    assert_eq!(items(&typing.opened(Some(channel), Some(next), 7)), ["Event(Select(Select { page: 514, sub: 16255 }, \"\"))"], "so does another page");
    assert_eq!(typing.parameters(channel), Some(""));
}

/// A session that starts late, or starts again, says them.
#[test]
fn a_sessions_start_says_the_parameters() {
    let mut typing = Typing::default();
    typing.input(PAGE.channel, true, false, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    typing.select(PAGE.channel, PAGE.page, SUBCODE_ANY, "a=1", 0);
    let started = typing.control(PAGE.channel, &welcome(SELECT_CAP), 0);
    let said = items(&started).into_iter().find(|item| item.contains("Select")).unwrap();
    assert_eq!(said, "Event(Select(Select { page: 512, sub: 16255 }, \"a=1\"))");
}
