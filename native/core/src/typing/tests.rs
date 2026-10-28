//! Tests of typing into a page (`interaction.md`, INPUT.md).

use super::*;
use neoteletext_input::caps::PAGE_CAP;
use neoteletext_input::control::Control;
use neoteletext_input::event::{Action, CONTROL, KeyCode, MouseAction, Point, read_message};
use neoteletext_input::{VERSION, WELCOME_TIMEOUT_MS};

const PAGE: PageKey = PageKey { channel: 7, page: 0x200 };
/// The key `a` as the channel reads it: a KEY carrying its text (INPUT.md
/// section 6.3).
const KEY_A: &str = "Event(Key(Key { action: Press, key: Character('a'), mods: 0, text: Some(\"a\"), base: None }))";

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
    typing
}

/// REQ-INT-106: HELLO first, nothing before WELCOME, then RESIZE, the
/// focus and the page.
#[test]
fn a_session_starts_with_hello_and_waits_for_welcome() {
    let mut typing = Typing::default();
    let opened = typing.input(PAGE.channel, true, true, 0);
    let offered = OFFERED | RESUME_CAP;
    assert_eq!(items(&opened), [format!("Event(Hello {{ version: 2, caps: {offered}, agent: None }})")]);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "nothing before WELCOME");
    let welcomed = typing.control(PAGE.channel, &welcome(PAGE_CAP | CURSOR_CAP), 5);
    assert_eq!(
        items(&welcomed),
        ["Event(Resize { columns: 40, rows: 25, width: 0, height: 0 })".to_owned(), "Event(Page(512))".to_owned(), "Event(FocusIn)".to_owned(),]
    );
    assert!(welcomed.iter().any(|send| matches!(send, Send::Interface(state) if state.on && state.secure)));
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
    assert_eq!(items(&typing.control(PAGE.channel, &welcome(PAGE_CAP), 10_001 * MS)), [] as [String; 0], "too late");
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
    assert!(ready(PAGE_CAP).page_takes_keys());
    let mut away = ready(PAGE_CAP);
    away.opened(Some(PAGE.channel), None, 0);
    assert!(!away.page_takes_keys(), "the page closed");
}

/// REQ-INT-130: input goes only under every condition.
#[test]
fn input_goes_only_under_every_condition() {
    let mut typing = Typing::default();
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "no socket, no page");
    typing.input(PAGE.channel, true, false, 0);
    typing.control(PAGE.channel, &welcome(PAGE_CAP), 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "no page open");
    typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "another page open");
    typing.opened(Some(8), None, 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [] as [String; 0], "another tab");
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [KEY_A]);
    let away = typing.focused(false, 0);
    assert_eq!(items(&away), ["Event(FocusOut)"]);
    assert_eq!(items(&typing.input_intent(&key('b'), 0)), [] as [String; 0], "the window has no focus");
    assert_eq!(items(&typing.focused(true, 0)), ["Event(FocusIn)"]);
    typing.input(PAGE.channel, false, false, 0);
    assert_eq!(items(&typing.input_intent(&key('c'), 0)), [] as [String; 0], "the socket closed");
}

/// Section 9.1: leaving the page tells the channel the focus went and
/// no page is open, so it lets go of what is held (REQ-INT-108).
#[test]
fn leaving_the_page_says_focus_out_and_page_zero() {
    let mut typing = ready(PAGE_CAP);
    let leaving = typing.opened(Some(PAGE.channel), None, 5);
    assert_eq!(items(&leaving), ["Event(FocusOut)", "Event(Page(0))"]);
    assert!(leaving.iter().any(|send| matches!(send, Send::Interface(state) if !state.on)));
    let back = typing.opened(Some(PAGE.channel), Some(PAGE), 6);
    assert_eq!(items(&back), ["Event(Page(512))", "Event(FocusIn)"]);
}

#[test]
fn editing_the_address_takes_the_keys_from_the_page_and_says_so() {
    let mut typing = ready(PAGE_CAP);
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
    let mut typing = ready(PAGE_CAP);
    typing.input(PAGE.channel, false, false, 1);
    let again = typing.input(PAGE.channel, true, true, 2);
    assert_eq!(items(&again).len(), 1);
    assert!(items(&again)[0].starts_with("Event(Hello"));
    assert_eq!(items(&typing.input_intent(&key('a'), 3)), [] as [String; 0], "waiting for WELCOME again");
}

/// INPUT.md section 6.3 (decision E.125): every key goes as KEY with its
/// text, a command modifier's key with none, and every release.
#[test]
fn every_key_goes_as_a_key_with_its_text() {
    let mut typing = ready(PAGE_CAP);
    let control_c = Intent::Key { action: Action::Press, key: KeyCode::Character('c'), mods: CONTROL, text: None, base: None };
    let sends = typing.input_intent(&InputIntent { key: PAGE, intent: control_c }, 0);
    assert_eq!(items(&sends), ["Event(Key(Key { action: Press, key: Character('c'), mods: 4, text: None, base: None }))"]);
    assert_eq!(items(&typing.input_intent(&key('a'), 0)), [KEY_A]);
    let release = Intent::Key { action: Action::Release, key: KeyCode::Character('a'), mods: 0, text: None, base: None };
    assert_eq!(items(&typing.input_intent(&InputIntent { key: PAGE, intent: release }, 0)).len(), 1, "a release");
}

/// REQ-INT-116 rule 3: a decomposed `e` and combining accent go as
/// U+00E9, as text and as a KEY's text.
#[test]
fn text_goes_in_normalization_form_c() {
    let mut typing = ready(PAGE_CAP);
    let text = InputIntent { key: PAGE, intent: Intent::Text("e\u{301}".into()) };
    assert_eq!(items(&typing.input_intent(&text, 0)), ["Text(\"\u{e9}\")"]);
    let key = Intent::Key { action: Action::Press, key: KeyCode::Character('e'), mods: 0, text: Some("e\u{301}".into()), base: None };
    let sent = items(&typing.input_intent(&InputIntent { key: PAGE, intent: key }, 0));
    assert_eq!(sent, ["Event(Key(Key { action: Press, key: Character('e'), mods: 0, text: Some(\"\u{e9}\"), base: None }))"]);
    let paste = InputIntent { key: PAGE, intent: Intent::Paste("e\u{301}".into()) };
    assert_eq!(items(&typing.input_intent(&paste, 0)), [r#"Event(Paste("e\u{301}"))"#], "a paste as copied");
}

/// REQ-INT-115 (decision E.125): a click goes with no request, and a
/// withdrawn `PAGE_MODES` record changes nothing.
#[test]
fn the_mouse_always_goes() {
    let mut typing = ready(PAGE_CAP);
    let click = InputIntent {
        key: PAGE,
        intent: Intent::Mouse { action: MouseAction::Press, button: 0, at: Point { column: 10, row: 5, pixels: Some((125, 110)), mods: 0 } },
    };
    let expected = ["Event(Mouse(Mouse { action: Press, button: 0, at: Point { column: 10, row: 5, pixels: Some((125, 110)), mods: 0 }, device: Mouse }))"];
    assert_eq!(items(&typing.input_intent(&click, 0)), expected);
    let release = InputIntent {
        intent: Intent::Mouse { action: MouseAction::Release, button: 0, at: Point { column: 10, row: 5, pixels: Some((125, 110)), mods: 0 } },
        ..click.clone()
    };
    typing.input_intent(&release, 0);
    typing.control(PAGE.channel, &[0xFF, 0x04, 0x80, 0x04, 0x08], 0);
    assert_eq!(items(&typing.input_intent(&click, 0)), expected, "an older channel's PAGE_MODES");
}

/// REQ-RCH-005: every button and the modifiers go as INPUT.md section 5.4
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
    let mut typing = ready(PAGE_CAP);
    let at = Point { column: 10, row: 5, pixels: Some((125, 110)), mods: SHIFT | ALT | CONTROL };
    for button in [LEFT, MIDDLE, RIGHT, BACK, FORWARD] {
        for (action, flags) in [(MouseAction::Press, DOT | MODS), (MouseAction::Release, RELEASE | DOT | MODS)] {
            let intent = InputIntent { key: PAGE, intent: Intent::Mouse { action, button, at } };
            let sent: Vec<Vec<u8>> = typing
                .input_intent(&intent, 0)
                .into_iter()
                .filter_map(|send| if let Send::Fetcher { bytes, .. } = send { Some(bytes) } else { None })
                .collect();
            assert_eq!(sent, [vec![0xFF, 0x02, flags, button, 10, 5, 125, 110, SHIFT | ALT | CONTROL]], "button {button}, {action:?}");
        }
    }
}

/// REQ-INT-110: a cursor record places the cursor, with its block flag
/// alone, since the browser echoes nothing; it gives no round trip, which
/// is the push connection's own (REQ-RCH-016); and one naming another
/// page is the host going there (REQ-INT-113).
#[test]
fn a_cursor_record_places_the_cursor() {
    let mut typing = ready(PAGE_CAP);
    // HELLO, RESIZE, PAGE and FOCUS_IN were 4.
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
    assert_eq!(state.show, None, "the open page's own record");
    let elsewhere = |acked: u8| [0x02, 0x01, 0, acked, 255, 0, 0, 0];
    let sends = typing.cursor(PAGE.channel, elsewhere(6));
    assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.show == Some(0x201)), "the host went to 201");
    let sends = typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 156 * MS);
    assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.show.is_none()), "and the browser followed");
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

/// REQ-INT-109: 120 units a second.
#[test]
fn the_rate_is_limited() {
    let mut typing = ready(PAGE_CAP);
    let mut sent = 0;
    for n in 0..200_u32 {
        sent += items(&typing.input_intent(&key(char::from(b'a' + u8::try_from(n % 26).unwrap())), 0)).len();
    }
    assert_eq!(sent, usize::try_from(neoteletext_input::MAX_PER_SECOND).unwrap() - 3, "RESIZE, PAGE and FOCUS_IN counted");
}

mod pointer;
mod resume;
