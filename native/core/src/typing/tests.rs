//! Tests of typing into a page (`interaction.md`, INPUT.md).

use super::*;
use neoteletext_input::control::PAGE_MODES;
use neoteletext_input::event::{KeyCode, MouseAction, Point, read_message};
use neoteletext_input::modes::{KEY_RELEASE, MOUSE_BUTTONS};
use neoteletext_input::{VERSION, WELCOME_TIMEOUT_MS};

const PAGE: PageKey = PageKey { channel: 7, page: 0x200 };

fn key(character: char, local: u32) -> InputIntent {
    let intent = Intent::Key { action: Action::Press, key: KeyCode::Character(character), mods: 0, text: Some(character.to_string()), base: None };
    InputIntent { key: PAGE, local, intent }
}

/// Every item the sends gave the fetcher, as the channel reads them.
fn items(sends: &[Send]) -> Vec<String> {
    let mut out = Vec::new();
    for send in sends {
        if let Send::Fetcher(_, bytes) = send {
            read_message(bytes, &mut |item| out.push(format!("{item:?}"))).unwrap();
        }
    }
    out
}

fn welcome(caps: u32) -> Vec<u8> {
    let mut body = [0_u8; neoteletext_input::control::MAX_BODY];
    let length = neoteletext_input::control::write_body(&Control::Welcome { version: VERSION, caps }, &mut body).unwrap();
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
    assert_eq!(items(&opened), [format!("Event(Hello {{ version: 2, caps: {OFFERED}, agent: None }})")]);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "nothing before WELCOME");
    let welcomed = typing.control(PAGE.channel, &welcome(PAGE_CAP | CURSOR_CAP), 5);
    assert_eq!(
        items(&welcomed),
        ["Event(Resize { columns: 40, rows: 25, width: 0, height: 0 })".to_owned(), "Event(Page(512))".to_owned(), "Event(FocusIn)".to_owned(),]
    );
    assert!(welcomed.iter().any(|send| matches!(send, Send::Interface(state) if state.on && state.secure)));
    assert_eq!(items(&typing.input_intent(&key('a', 1), 6)), ["Text(\"a\")"]);
}

#[test]
fn a_channel_that_does_not_answer_is_refused_and_the_strip_says_so() {
    let mut typing = Typing::default();
    typing.input(PAGE.channel, true, false, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert_eq!(typing.tick(u64::from(WELCOME_TIMEOUT_MS) - 1), []);
    let sends = typing.tick(u64::from(WELCOME_TIMEOUT_MS));
    assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.refused && !state.on));
    assert_eq!(items(&typing.control(PAGE.channel, &welcome(PAGE_CAP), 10_001)), [] as [String; 0], "too late");
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
    assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "no socket, no page");
    typing.input(PAGE.channel, true, false, 0);
    typing.control(PAGE.channel, &welcome(PAGE_CAP), 0);
    assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "no page open");
    typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 0);
    assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "another page open");
    typing.opened(Some(8), None, 0);
    assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "another tab");
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), ["Text(\"a\")"]);
    let away = typing.focused(false, 0);
    assert_eq!(items(&away), ["Event(FocusOut)"]);
    assert_eq!(items(&typing.input_intent(&key('b', 2), 0)), [] as [String; 0], "the window has no focus");
    assert_eq!(items(&typing.focused(true, 0)), ["Event(FocusIn)"]);
    typing.input(PAGE.channel, false, false, 0);
    assert_eq!(items(&typing.input_intent(&key('c', 3), 0)), [] as [String; 0], "the socket closed");
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
    assert_eq!(items(&typing.input_intent(&key('b', 2), 2)), [] as [String; 0], "the keys are the strip's");
    // The window losing the focus while editing changes nothing for the
    // page, nor does it coming back.
    assert_eq!(items(&typing.focused(false, 3)), [] as [String; 0]);
    assert_eq!(items(&typing.focused(true, 4)), [] as [String; 0]);
    assert_eq!(items(&typing.editing(false, 5)), ["Event(FocusIn)"]);
    assert_eq!(items(&typing.input_intent(&key('c', 3), 6)).len(), 1);
}

/// REQ-INT-106: a reconnection is a new session.
#[test]
fn a_new_connection_is_a_new_session() {
    let mut typing = ready(PAGE_CAP | MOUSE_BUTTONS);
    typing.input(PAGE.channel, false, false, 1);
    let again = typing.input(PAGE.channel, true, true, 2);
    assert_eq!(items(&again).len(), 1);
    assert!(items(&again)[0].starts_with("Event(Hello"));
    assert_eq!(items(&typing.input_intent(&key('a', 1), 3)), [] as [String; 0], "waiting for WELCOME again");
}

/// Section 5.3: a key that types text is text; with a command
/// modifier, or in `ALL_KEYS`, it is a KEY.
#[test]
fn text_goes_as_text_and_commands_as_keys() {
    let mut typing = ready(PAGE_CAP | ALL_KEYS);
    let control_c = Intent::Key { action: Action::Press, key: KeyCode::Character('c'), mods: CONTROL, text: Some("c".into()), base: None };
    let sends = typing.input_intent(&InputIntent { key: PAGE, local: 1, intent: control_c }, 0);
    assert_eq!(items(&sends), ["Event(Key(Key { action: Press, key: Character('c'), mods: 4, text: None, base: None }))"]);
    let release = Intent::Key { action: Action::Release, key: KeyCode::Character('c'), mods: 0, text: None, base: None };
    assert_eq!(items(&typing.input_intent(&InputIntent { key: PAGE, local: 2, intent: release }, 0)), [] as [String; 0], "no KEY_RELEASE");
    typing.control(PAGE.channel, &[0xFF, 0x02, u8::try_from(ALL_KEYS).unwrap()], 0);
    assert_eq!(
        items(&typing.input_intent(&key('a', 3), 0)),
        ["Event(Key(Key { action: Press, key: Character('a'), mods: 0, text: Some(\"a\"), base: None }))"],
        "every key a KEY in ALL_KEYS"
    );
}

/// REQ-INT-116 rule 3: a decomposed `e` and combining accent go as
/// U+00E9, as text and as a KEY's text.
#[test]
fn text_goes_in_normalization_form_c() {
    let mut typing = ready(PAGE_CAP | ALL_KEYS);
    let text = InputIntent { key: PAGE, local: 1, intent: Intent::Text("e\u{301}".into()) };
    assert_eq!(items(&typing.input_intent(&text, 0)), ["Text(\"\u{e9}\")"]);
    typing.control(PAGE.channel, &[0xFF, 0x02, u8::try_from(ALL_KEYS).unwrap()], 0);
    let key = Intent::Key { action: Action::Press, key: KeyCode::Character('e'), mods: 0, text: Some("e\u{301}".into()), base: None };
    let sent = items(&typing.input_intent(&InputIntent { key: PAGE, local: 2, intent: key }, 0));
    assert_eq!(sent, ["Event(Key(Key { action: Press, key: Character('e'), mods: 0, text: Some(\"\u{e9}\"), base: None }))"]);
    let paste = InputIntent { key: PAGE, local: 3, intent: Intent::Paste("e\u{301}".into()) };
    assert_eq!(items(&typing.input_intent(&paste, 0)), [r#"Event(Paste("e\u{301}"))"#], "a paste as copied");
}

/// REQ-INT-115: the mouse goes only in a mouse mode, and the page's own
/// modes come with the page (REQ-INT-114).
#[test]
fn the_mouse_goes_only_when_the_page_or_channel_asks() {
    let mut typing = ready(PAGE_CAP | MOUSE_BUTTONS | KEY_RELEASE);
    let click = InputIntent {
        key: PAGE,
        local: 1,
        intent: Intent::Mouse { action: MouseAction::Press, button: 0, at: Point { column: 10, row: 5, pixels: None, mods: 0 } },
    };
    assert_eq!(items(&typing.input_intent(&click, 0)), [] as [String; 0]);
    let page_modes = [0xFF, PAGE_MODES, 0x80, 0x04, u8::try_from(MOUSE_BUTTONS).unwrap()];
    let told = typing.control(PAGE.channel, &page_modes, 0);
    assert!(matches!(told.last(), Some(Send::Interface(state)) if state.modes == MOUSE_BUTTONS), "the interface knows");
    assert_eq!(
        items(&typing.input_intent(&click, 0)),
        ["Event(Mouse(Mouse { action: Press, button: 0, at: Point { column: 10, row: 5, pixels: None, mods: 0 } }))"]
    );
}

/// REQ-INT-110: the acknowledgement is the input count; it clears
/// echo by the interface's numbers and gives the round trip
/// (REQ-INT-137).
#[test]
fn the_acknowledgement_clears_echo_and_measures_the_round_trip() {
    let mut typing = ready(PAGE_CAP);
    // HELLO, RESIZE, PAGE and FOCUS_IN were 4.
    typing.input_intent(&key('a', 11), 100);
    typing.input_intent(&key('b', 12), 110);
    let body = |row: u8, acked: u8| [0x02, 0x00, 0, acked, row, 4, 10, cursor::ECHO];
    assert_eq!(typing.cursor(PAGE.channel, body(24, 5), 150), [], "row 24 is refused");
    let sends = typing.cursor(PAGE.channel, body(5, 5), 150);
    let Some(Send::Interface(state)) = sends.last() else { panic!("{sends:?}") };
    assert_eq!(state.cursor, Some(CursorState { row: 5, column: 4, width: 10, flags: cursor::ECHO }));
    assert_eq!((state.acked, state.round_trip_ms), (11, Some(50)));
    assert_eq!(state.show, None, "the open page's own record");
    let elsewhere = |acked: u8| [0x02, 0x01, 0, acked, 255, 0, 0, 0];
    let sends = typing.cursor(PAGE.channel, elsewhere(6), 155);
    assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.show == Some(0x201) && state.acked == 12), "the host went to 201");
    let sends = typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 156);
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
        ("interface-core/src/browser/terminal_ui.rs", include_str!("../../../interface-core/src/browser/terminal_ui.rs")),
        ("interface-core/src/input.rs", include_str!("../../../interface-core/src/input.rs")),
        ("interface/src/keys.rs", include_str!("../../../interface/src/keys.rs")),
        ("protocol/src/interaction.rs", include_str!("../../../protocol/src/interaction.rs")),
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
        sent += items(&typing.input_intent(&key(char::from(b'a' + u8::try_from(n % 26).unwrap()), n), 0)).len();
    }
    assert_eq!(sent, usize::try_from(neoteletext_input::MAX_PER_SECOND).unwrap() - 3, "RESIZE, PAGE and FOCUS_IN counted");
}
