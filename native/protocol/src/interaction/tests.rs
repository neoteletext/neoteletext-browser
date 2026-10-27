//! Typing's tests: every intent and the typing state round trip, what
//! breaks a bound is refused, and a message for the fetcher is whole.

use neoteletext_input::cursor;
use neoteletext_input::event::{Action, KeyCode, MouseAction, Point};
use neoteletext_input::{MAX_COMPOSE, MAX_PASTE};

use super::intent::{KEY_CHARACTER, TAG_KEY};
use super::*;
use crate::Fields;
use neoteletext_input::event::{ALT, CONTROL};

const KEY: PageKey = PageKey { channel: 1, page: 0x100 };

fn round_trip(intent: Intent) -> Result<InputIntent, ProtocolError> {
    let input = InputIntent { key: KEY, local: 9, intent };
    let mut payload = Vec::new();
    push_input(&mut payload, &input)?;
    let back = read_input(&mut Fields::new(&payload))?;
    assert_eq!(back, input);
    Ok(back)
}

fn at(column: u32, row: u32) -> Point {
    Point { column, row, pixels: Some((120, 100)), mods: CONTROL }
}

/// REQ-INT-132: the interface sends intents, each read whole.
#[test]
fn every_intent_round_trips() {
    for intent in [
        Intent::Key { action: Action::Press, key: KeyCode::Character('a'), mods: 0, text: Some("a".into()), base: Some('q') },
        Intent::Key { action: Action::Release, key: KeyCode::Functional(2), mods: CONTROL | ALT, text: None, base: None },
        Intent::Text("Grüße, мир".into()),
        Intent::Mouse { action: MouseAction::Press, button: 0, at: at(10, 5) },
        Intent::Mouse { action: MouseAction::Move, button: 1, at: Point { pixels: None, ..at(71, 24) } },
        Intent::Wheel { dx: 0, dy: -120, at: at(0, 0) },
        Intent::Paste("line one\nline two\t".into()),
        Intent::Compose { cursor: 1, text: "にほ".into() },
        Intent::Compose { cursor: 0, text: String::new() },
    ] {
        round_trip(intent).unwrap();
    }
}

#[test]
fn what_breaks_a_bound_is_refused() {
    for refused in [
        Intent::Text(String::new()),
        Intent::Text("\u{7}".into()),
        Intent::Text("a".repeat(MAX_PASTE + 1)),
        Intent::Key { action: Action::Press, key: KeyCode::Functional(56), mods: 0, text: None, base: None },
        Intent::Key { action: Action::Press, key: KeyCode::Character('a'), mods: 0, text: Some("a".repeat(33)), base: None },
        Intent::Mouse { action: MouseAction::Press, button: 0, at: at(72, 0) },
        Intent::Mouse { action: MouseAction::Press, button: 0, at: at(0, 25) },
        Intent::Compose { cursor: 0, text: "a".repeat(MAX_COMPOSE + 1) },
        Intent::Paste("a".repeat(MAX_PASTE + 1)),
    ] {
        assert!(round_trip(refused.clone()).is_err(), "{refused:?}");
    }
    // Bytes the interface cannot send: an unknown tag, a bad character.
    let mut payload = Vec::new();
    super::super::push_key(&mut payload, KEY);
    payload.extend_from_slice(&1_u32.to_be_bytes());
    for tail in [vec![9_u8], vec![TAG_KEY, 0, KEY_CHARACTER, 0, 0, 0xD8, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]] {
        let mut bytes = payload.clone();
        bytes.extend_from_slice(&tail);
        assert!(read_input(&mut Fields::new(&bytes)).is_err(), "{tail:?}");
    }
}

#[test]
fn editing_round_trips_and_only_as_a_flag() {
    use super::super::{ToCore, read_from_interface, write_to_core};
    for editing in [true, false] {
        let mut bytes = Vec::new();
        write_to_core(&mut bytes, &ToCore::Editing(editing)).unwrap();
        assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap().unwrap(), ToCore::Editing(editing));
        // The framing: the kind, a 32-bit length, the flag.
        bytes[5] = 2;
        assert!(read_from_interface(&mut bytes.as_slice()).is_err(), "neither false nor true");
    }
}

#[test]
fn typing_round_trips_and_a_bad_cursor_or_mode_is_refused() {
    let typing = Typing {
        channel: 3,
        page: 0x200,
        on: true,
        secure: false,
        refused: false,
        modes: neoteletext_input::modes::MOUSE_BUTTONS,
        cursor: Some(CursorState { row: 5, column: 6, width: 10, flags: cursor::ECHO }),
        acked: 40,
        round_trip_ms: Some(12),
        show: Some(0x201),
    };
    let mut payload = Vec::new();
    push_typing(&mut payload, &typing).unwrap();
    assert_eq!(read_typing(&mut Fields::new(&payload)).unwrap(), typing);
    let refused = Typing { on: false, refused: true, cursor: None, show: None, ..typing.clone() };
    let mut payload = Vec::new();
    push_typing(&mut payload, &refused).unwrap();
    assert_eq!(read_typing(&mut Fields::new(&payload)).unwrap(), refused);
    let bad = Typing { cursor: Some(CursorState { row: 24, column: 0, width: 0, flags: 0 }), ..typing.clone() };
    assert!(push_typing(&mut Vec::new(), &bad).is_err());
    assert!(push_typing(&mut Vec::new(), &Typing { modes: 256, ..typing }).is_err());
}

/// REQ-INT-133: the fetcher takes whole items of the stream, at most
/// 4 096 bytes.
#[test]
fn a_message_for_the_fetcher_is_whole_and_bounded() {
    assert!(check_message(b"abc\xFF\x01\x04\x02").is_ok());
    assert!(check_message(b"\xFF\x01").is_err(), "an item cut short");
    assert!(check_message(b"\xFF\x00").is_err(), "a reserved type");
    assert!(check_message(&[b'a'; MAX_MESSAGE + 1]).is_err());
}

/// The input trace's line (the owner, 27 October 2026) round trips, its
/// text within a row and cleaned.
#[test]
fn a_trace_line_round_trips_within_a_row() {
    let line = TraceLine { text: "MOUSE move b0 12,5".into(), repeats: 17, sent: 1_234 };
    let mut payload = Vec::new();
    push_trace(&mut payload, &line).unwrap();
    assert_eq!(read_trace(&mut Fields::new(&payload)).unwrap(), line);
    assert!(push_trace(&mut Vec::new(), &TraceLine { text: "x".repeat(MAX_TRACE + 1), ..line.clone() }).is_err());
    assert!(push_trace(&mut Vec::new(), &TraceLine { text: "a\u{202e}b".into(), ..line }).is_err(), "only cleaned text");
}
