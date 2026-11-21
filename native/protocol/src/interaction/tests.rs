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
    let input = InputIntent { key: KEY, intent };
    let mut payload = Vec::new();
    push_input(&mut payload, &input)?;
    let back = read_input(&mut Fields::new(&payload))?;
    assert_eq!(back, input);
    Ok(back)
}

/// A position in the cell at `column`, `row`, with a dot inside it.
fn at(column: u32, row: u32) -> Point {
    Point { column, row, pixels: Some((column.wrapping_mul(12).wrapping_add(5), row.wrapping_mul(20).wrapping_add(7))), mods: CONTROL }
}

/// REQ-INT-132: the interface sends intents, each read whole.
#[test]
fn every_intent_round_trips() {
    for intent in [
        Intent::Key { action: Action::Press, key: KeyCode::Character('a'), mods: 0, text: Some("a".into()), base: Some('q') },
        Intent::Key { action: Action::Release, key: KeyCode::Functional(2), mods: CONTROL | ALT, text: None, base: None },
        Intent::Text("Grüße, мир".into()),
        Intent::Mouse { action: MouseAction::Press, button: 0, at: at(10, 5) },
        Intent::Mouse { action: MouseAction::Move, button: 1, at: Point { pixels: None, ..at(39, 24) } },
        Intent::Wheel { dx: 0, dy: -120, at: at(0, 0) },
        Intent::Paste("line one\nline two\t".into()),
        Intent::Compose { cursor: 1, text: "にほ".into() },
        Intent::Hover { leave: true, column: 2, row: 20 },
        Intent::Compose { cursor: 0, text: String::new() },
        Intent::Gone,
        Intent::Touch { phase: TouchPhase::Began, contact: u64::MAX, at: at(39, 24) },
        Intent::Touch { phase: TouchPhase::Moved, contact: 7, at: Point { pixels: None, ..at(3, 4) } },
        Intent::Touch { phase: TouchPhase::Ended, contact: 7, at: at(3, 4) },
        Intent::Touch { phase: TouchPhase::Cancelled, contact: 0, at: at(0, 0) },
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
        // REQ-1645: a page is 40 columns, so column 39 is the last.
        Intent::Mouse { action: MouseAction::Press, button: 0, at: at(40, 0) },
        Intent::Mouse { action: MouseAction::Press, button: 0, at: at(0, 25) },
        Intent::Compose { cursor: 0, text: "a".repeat(MAX_COMPOSE + 1) },
        Intent::Paste("a".repeat(MAX_PASTE + 1)),
        Intent::Mouse { action: MouseAction::Gone, button: 0, at: at(1, 1) },
        Intent::Touch { phase: TouchPhase::Began, contact: 1, at: at(40, 0) },
    ] {
        assert!(round_trip(refused.clone()).is_err(), "{refused:?}");
    }
    // Bytes the interface cannot send: an unknown tag, a bad character.
    let mut payload = Vec::new();
    super::super::push_key(&mut payload, KEY);
    payload.extend_from_slice(&1_u32.to_be_bytes());
    for tail in [
        vec![0xEE_u8],
        vec![8, 4, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        vec![TAG_KEY, 0, KEY_CHARACTER, 0, 0, 0xD8, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF],
    ] {
        let mut bytes = payload.clone();
        bytes.extend_from_slice(&tail);
        assert!(read_input(&mut Fields::new(&bytes)).is_err(), "{tail:?}");
    }
}

// REQ-1554: the wall of pages with none zoomed, told as a message of no
// fields.
#[test]
fn the_wall_without_a_page_round_trips() {
    use super::super::{ToCore, read_from_interface, write_to_core};
    let mut bytes = Vec::new();
    write_to_core(&mut bytes, &ToCore::WallWithoutPage).unwrap();
    assert_eq!(read_from_interface(&mut bytes.as_slice()).unwrap().unwrap(), ToCore::WallWithoutPage);
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
fn typing_round_trips_and_a_bad_cursor_is_refused() {
    let typing = Typing {
        channel: 3,
        page: 0x200,
        on: true,
        secure: false,
        refused: false,
        cursor: Some(CursorState { row: 5, column: 6, flags: cursor::BLOCK }),
        round_trip_us: Some(12_345),
        wheel: true,
        asks: true,
        keys: true,
        asked: crate::rules::KeyAsks::default(),
    };
    let mut payload = Vec::new();
    push_typing(&mut payload, &typing).unwrap();
    assert_eq!(read_typing(&mut Fields::new(&payload)).unwrap(), typing);
    let refused = Typing { on: false, refused: true, cursor: None, asks: false, keys: false, ..typing.clone() };
    let mut payload = Vec::new();
    push_typing(&mut payload, &refused).unwrap();
    assert_eq!(read_typing(&mut Fields::new(&payload)).unwrap(), refused);
    let bad = Typing { cursor: Some(CursorState { row: 24, column: 0, flags: 0 }), ..typing.clone() };
    assert!(push_typing(&mut Vec::new(), &bad).is_err());
    // The browser draws no echo (decision E.134): no flag but block is told.
    let echo = Typing { cursor: Some(CursorState { row: 5, column: 6, flags: 1 }), ..typing.clone() };
    assert!(push_typing(&mut Vec::new(), &echo).is_err());
}

/// REQ-7037: whether the open page's table asks for the keyboard is flag
/// bit 128 of the typing flags byte, alone, and round-trips by itself; with
/// every bit of the byte now a fact, a byte of every flag reads, each true.
/// On the code before, bit 128 was refused as no flag.
#[test]
fn the_keyboard_flag_is_bit_128_and_every_flags_byte_reads() {
    /// Where the flags byte is: after the channel's four bytes and the page's two.
    const FLAGS_AT: usize = 6;
    const KEYS_BIT: u8 = 128;
    let bare = Typing {
        channel: 3,
        page: 0x200,
        on: false,
        secure: false,
        refused: false,
        cursor: None,
        round_trip_us: None,
        wheel: false,
        asks: false,
        keys: false,
        asked: crate::rules::KeyAsks::default(),
    };
    for keys in [false, true] {
        let typing = Typing { keys, ..bare.clone() };
        let mut payload = Vec::new();
        push_typing(&mut payload, &typing).unwrap();
        assert_eq!(payload[FLAGS_AT], if keys { KEYS_BIT } else { 0 }, "keys {keys}: the bit alone");
        assert_eq!(read_typing(&mut Fields::new(&payload)).unwrap(), typing, "keys {keys}");
    }
    let every = Typing {
        on: true,
        secure: true,
        refused: true,
        cursor: Some(CursorState { row: 5, column: 6, flags: cursor::BLOCK }),
        round_trip_us: Some(7),
        wheel: true,
        asks: true,
        keys: true,
        ..bare
    };
    let mut payload = Vec::new();
    push_typing(&mut payload, &every).unwrap();
    assert_eq!(payload[FLAGS_AT], u8::MAX, "every flag");
    assert_eq!(read_typing(&mut Fields::new(&payload)).unwrap(), every);
}

/// REQ-INT-133 (REQ-1381): the fetcher takes whole items of the stream, at
/// most the input protocol's `MAX_MESSAGE`, 8,192 bytes (DEC-1715).
#[test]
fn a_message_for_the_fetcher_is_whole_and_bounded() {
    // RULE-46 (REQ-6534): a viewer's input goes only inside a RULE_EVENT.
    assert!(check_message(b"abc\xFF\x01\x80\x04\x00\x04\x02").is_err(), "bare text and a bare KEY");
    assert!(check_message(&rule_event()).is_ok());
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

/// REQ-INT-127: the trusted core's reader refuses a dot outside the
/// 480 by 500 page image or outside its cell (`px` / 12 must be the
/// column and `py` / 20 the row), for the mouse, the wheel and a contact;
/// a position with no dot, and a dot on a cell's last dots, are taken. On
/// the code before, every one of these dots was taken.
#[test]
fn a_dot_outside_its_cell_or_the_image_is_refused() {
    let cell = |column, row, x, y| Point { column, row, pixels: Some((x, y)), mods: 0 };
    let taken = [cell(0, 0, 0, 0), cell(39, 24, 479, 499), cell(10, 5, 131, 119), Point { column: 39, row: 24, pixels: None, mods: 0 }];
    let refused = [
        cell(10, 5, 132, 110),
        cell(10, 5, 125, 120),
        cell(10, 5, 119, 110),
        cell(39, 24, 480, 499),
        cell(39, 24, 479, 500),
        cell(0, 0, u32::MAX, u32::MAX),
        // REQ-1645: past the last column, with or without a dot in it.
        cell(40, 0, 480, 0),
        cell(41, 0, 492, 0),
    ];
    for at in taken {
        for intent in [
            Intent::Mouse { action: MouseAction::Move, button: 0, at },
            Intent::Wheel { dx: 0, dy: 120, at },
            Intent::Touch { phase: TouchPhase::Moved, contact: 1, at },
        ] {
            round_trip(intent).unwrap();
        }
        assert!(dot_in_cell(&at), "{at:?}");
    }
    for at in refused {
        for intent in [
            Intent::Mouse { action: MouseAction::Move, button: 0, at },
            Intent::Wheel { dx: 0, dy: 120, at },
            Intent::Touch { phase: TouchPhase::Moved, contact: 1, at },
        ] {
            let input = InputIntent { key: KEY, intent };
            let mut payload = Vec::new();
            assert!(push_input(&mut payload, &input).is_err(), "the interface's writer refuses {at:?}");
            // Written by hand, as a hostile interface would, the reader refuses it.
            let mut payload = Vec::new();
            super::super::push_key(&mut payload, KEY);
            payload.extend_from_slice(&[3, 2, 0]);
            payload.extend_from_slice(&at.column.to_be_bytes());
            payload.extend_from_slice(&at.row.to_be_bytes());
            payload.push(1);
            let (x, y) = at.pixels.unwrap();
            payload.extend_from_slice(&x.to_be_bytes());
            payload.extend_from_slice(&y.to_be_bytes());
            payload.push(0);
            assert!(read_input(&mut Fields::new(&payload)).is_err(), "{at:?}");
        }
        assert!(!dot_in_cell(&at), "{at:?}");
    }
}

/// A `RULE_EVENT` carrying raw text, as the engine writes it (RULE-46).
pub(crate) fn rule_event() -> Vec<u8> {
    use neoteletext_input::event::{Event, Item, Stamp, write};
    use neoteletext_input::rules::{RuleBody, RuleEvent};
    let on = Stamp::new(0x100, 0).unwrap();
    let event = RuleEvent { on, generation: 1, rule: 2, row: 0, tick: 3, target: neoteletext_rules::table::Target::NONE, body: RuleBody::Input(b"abc") };
    let mut out = [0_u8; MAX_MESSAGE];
    let length = write(&Item::Event(Event::RuleEvent(event)), &mut out).unwrap();
    out[..length].to_vec()
}

/// REQ-7037 item 1 (TODO-7158): what of the keyboard the open page's table
/// asks for rides on the typing message, at its bound, and is told only of
/// a table that asks for the keyboard.
#[test]
fn the_keys_asked_ride_on_the_typing_message_and_only_with_the_keyboard() {
    use crate::rules::{Key, KeyAsk, KeyAsks, KeyKind, MAX_KEY_ASKS};
    /// The bytes of one key ask: its kind, its 32-bit code, the mask and value.
    const ASK_BYTES: usize = 7;
    /// Where the flags byte is: after the channel's four bytes and the page's two.
    const FLAGS_AT: usize = 6;
    const KEYS_BIT: u8 = 128;
    let first = u32::from('A');
    let keys: Vec<KeyAsk> = (first..first + u32::try_from(MAX_KEY_ASKS).unwrap())
        .map(|code| KeyAsk { key: Some(Key::of(if code % 2 == 0 { KeyKind::Character } else { KeyKind::Physical }, code)), mods_mask: 1, mods_value: 1 })
        .collect();
    let typing = Typing {
        channel: 3,
        page: 0x200,
        on: true,
        secure: false,
        refused: false,
        cursor: None,
        round_trip_us: None,
        wheel: false,
        asks: true,
        keys: true,
        asked: KeyAsks { text: true, keys: keys.clone() },
    };
    let mut payload = Vec::new();
    push_typing(&mut payload, &typing).unwrap();
    assert_eq!(read_typing(&mut Fields::new(&payload)).unwrap(), typing, "at the bound");
    let any = Typing { asked: KeyAsks { text: false, keys: vec![KeyAsk::EVERY_KEY] }, ..typing.clone() };
    let mut any_payload = Vec::new();
    push_typing(&mut any_payload, &any).unwrap();
    assert_eq!(read_typing(&mut Fields::new(&any_payload)).unwrap(), any, "any key");

    // Nothing is asked of no keyboard, on sending and on arrival.
    let off = Typing { keys: false, ..typing.clone() };
    assert!(push_typing(&mut Vec::new(), &off).is_err(), "keys asked with the keys flag off");
    let text_off = Typing { keys: false, asked: KeyAsks { text: true, keys: Vec::new() }, ..typing.clone() };
    assert!(push_typing(&mut Vec::new(), &text_off).is_err(), "text asked with the keys flag off");
    let mut flag_off = payload.clone();
    flag_off[FLAGS_AT] &= !KEYS_BIT;
    assert!(read_typing(&mut Fields::new(&flag_off)).is_err(), "the keys flag cleared under a list");

    // One more than the bound, on sending and on arrival.
    let over = Typing { asked: KeyAsks { text: true, keys: [keys.clone(), vec![KeyAsk::EVERY_KEY]].concat() }, ..typing.clone() };
    assert!(push_typing(&mut Vec::new(), &over).is_err());
    let count_at = payload.len() - MAX_KEY_ASKS * ASK_BYTES - 1;
    assert_eq!(usize::from(payload[count_at]), MAX_KEY_ASKS);
    let mut longer = payload.clone();
    longer[count_at] = u8::try_from(MAX_KEY_ASKS + 1).unwrap();
    longer.extend_from_slice(&payload[payload.len() - ASK_BYTES..]);
    assert!(read_typing(&mut Fields::new(&longer)).is_err(), "129 entries");

    // A malformed ask, on arrival.
    let last = payload.len() - ASK_BYTES;
    for (at, byte, what) in [
        (last, 4, "a kind above 3"),
        (last, 0, "any key with a code"),
        (last + 1, 0xD8, "no Unicode scalar value"),
        (last + 6, 3, "a value bit outside its mask"),
    ] {
        let mut bad = payload.clone();
        bad[at] = byte;
        assert!(read_typing(&mut Fields::new(&bad)).is_err(), "{what}");
    }
    // The text flag is a flag: neither false nor true is refused.
    let text_at = count_at - 1;
    let mut text = payload.clone();
    text[text_at] = 2;
    assert!(read_typing(&mut Fields::new(&text)).is_err(), "a text flag of 2");
}
