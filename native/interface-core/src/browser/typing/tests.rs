use super::*;
use alloc::string::String;
use alloc::vec::Vec;

use neoteletext_input::cursor::{ECHO, MASKED};
use neoteletext_input::event::{Action, CONTROL, KeyCode, MouseAction, Point};

use crate::browser::{Browser, Frame, Output};
use crate::canvas::PageKey;
use crate::channel::{Channel, PageInfo, Status};
use crate::geometry::Rect;
use crate::input::Intent;
use crate::test_canvas::Recorder;

const KEY: PageKey = PageKey { channel: 4, page: 0x100 };

fn browser() -> Browser {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    let mut channel = Channel::empty();
    channel.channel = KEY.channel;
    channel.pages[0] = Some(PageInfo { title: "Terminal".into(), status: Status::Loaded });
    browser.set_channel(channel);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.wall.zoom(KEY, 0);
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 10_000);
    browser.take_outputs();
    browser
}

fn typing(cursor: Option<CursorView>, acked: u32) -> TypingView {
    TypingView { channel: KEY.channel, page: KEY.page, on: true, secure: false, refused: false, cursor, acked, round_trip_ms: None, show: None }
}

fn intents(outputs: &[Output]) -> Vec<Intent> {
    outputs.iter().filter_map(|output| if let Output::Terminal { intent, .. } = output { Some(intent.clone()) } else { None }).collect()
}

fn press(browser: &mut Browser, character: char, mods: u8) -> bool {
    let mut text = [0_u8; 4];
    let text = character.encode_utf8(&mut text);
    browser.terminal_key(Action::Press, KeyCode::Character(character), mods, Some(text), None, 0)
}

fn pointer_at(browser: &mut Browser, column: f64, row: f64) {
    let (_, page) = browser.wall.open().unwrap();
    let x = page.x0 + page.width() * (column / 40.0);
    let y = page.y0 + page.height() * (row / 25.0);
    browser.pointer_moved(x, y, 10_000);
}

/// REQ-INT-130, REQ-INT-132: intents only while the core says so.
#[test]
fn keys_go_only_while_the_core_says_so() {
    let mut browser = browser();
    assert!(!press(&mut browser, 'a', 0));
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "the core has not said");
    browser.set_typing(typing(None, 0), 0);
    assert_eq!(browser.typing_target(), Some(KEY));
    press(&mut browser, 'a', 0);
    browser.terminal_key(Action::Release, KeyCode::Character('a'), 0, None, None, 0);
    let sent = intents(&browser.take_outputs());
    assert_eq!(sent.len(), 2);
    assert!(matches!(&sent[0], Intent::Key { action: Action::Press, key: KeyCode::Character('a'), text: Some(text), .. } if text == "a"));
    assert!(matches!(&sent[1], Intent::Key { action: Action::Release, .. }));
    browser.set_typing(TypingView { on: false, ..typing(None, 0) }, 0);
    press(&mut browser, 'b', 0);
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "typing is off");
}

/// A panel, popover or list of the strip, while open, takes the keys:
/// none go to the page's channel. Before, Return in the star's popover
/// went to a channel that takes keys, and the popover never closed.
#[test]
fn the_strips_panels_take_the_keys_from_the_channel() {
    let mut browser = browser();
    browser.set_typing(typing(None, 0), 0);
    assert_eq!(browser.typing_target(), Some(KEY));
    browser.set_star(crate::library::StarView { open: true, ..crate::library::StarView::default() });
    assert_eq!(browser.typing_target(), None, "the star's popover");
    browser.set_star(crate::library::StarView::default());
    browser.toggle_history();
    assert_eq!(browser.typing_target(), None, "the history panel");
    browser.toggle_history();
    browser.toggle_menu();
    assert_eq!(browser.typing_target(), None, "the menu");
    browser.toggle_menu();
    assert_eq!(browser.typing_target(), Some(KEY));
}

/// REQ-INT-134: echo of the viewer's own printable keys, within the
/// width, masked when asked, gone on acknowledgement or after 2 s.
#[test]
fn echo_stays_in_its_width_is_masked_and_clears_on_acknowledgement_or_time() {
    let mut browser = browser();
    let cursor = CursorView { row: 5, column: 10, width: 3, flags: ECHO };
    browser.set_typing(typing(Some(cursor), 0), 0);
    for character in "abcd".chars() {
        press(&mut browser, character, 0);
    }
    let echoed: String = browser.typing.echoes.iter().map(|echo| echo.character).collect();
    assert_eq!(echoed, "abc", "never past the width");
    browser.terminal_key(Action::Press, KeyCode::Functional(BACKSPACE), 0, None, None, 0);
    assert_eq!(browser.typing.echoes.len(), 2);
    press(&mut browser, 'x', CONTROL);
    assert_eq!(browser.typing.echoes.len(), 2, "Control-x is a command, not text");
    // The first key's acknowledgement removes its echo.
    browser.set_typing(typing(Some(cursor), 1), 10);
    assert_eq!(browser.typing.echoes.len(), 1);
    assert!(browser.settle_echoes(ECHO_LIFETIME_MILLISECONDS));
    assert!(browser.typing.echoes.is_empty());
    browser.set_typing(typing(Some(CursorView { flags: ECHO | MASKED, ..cursor }), 99), 0);
    browser.terminal_text("pw", 0);
    assert!(browser.typing.echoes.iter().all(|echo| echo.character == MASK));
    // Without echo, nothing shows.
    browser.set_typing(typing(Some(CursorView { flags: 0, ..cursor }), 99), 0);
    press(&mut browser, 'q', 0);
    assert!(browser.typing.echoes.is_empty());
}

/// REQ-INT-115, REQ-INT-136 (decision E.125): a plain click on a page
/// that takes input is the host's, with no request; with the browser's
/// modifier it is the browser's, to follow a link.
#[test]
fn a_click_is_the_hosts_unless_the_modifier_is_held() {
    let mut browser = browser();
    let mut canvas = Recorder::new();
    browser.set_typing(typing(None, 0), 0);
    pointer_at(&mut browser, 10.5, 5.5);
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "no move without a button or a pointer area");
    browser.pointer_pressed(&mut canvas, false, 10_000);
    browser.pointer_released();
    let outputs = browser.take_outputs();
    let sent = intents(&outputs);
    assert!(
        matches!(
            sent.as_slice(),
            [
                Intent::Mouse { action: MouseAction::Press, button: 0, at: Point { column: 10, row: 5, pixels: Some(_), .. } },
                Intent::Mouse { action: MouseAction::Release, button: 0, .. },
            ]
        ),
        "{sent:?}"
    );
    assert!(!outputs.iter().any(|output| matches!(output, Output::Cell(_))));
    browser.set_browser_modifier(true);
    browser.pointer_pressed(&mut canvas, false, 11_000);
    let outputs = browser.take_outputs();
    assert!(outputs.iter().any(|output| matches!(output, Output::Cell(_))), "the modifier keeps a link's click");
    assert_eq!(intents(&outputs), [] as [Intent; 0]);
}

/// Decision 107 and INPUT.md section 6.3: the wheel and the pointer's
/// moves over the page are the channel's; beside the page, or with the
/// browser's modifier, the wheel is the browser's.
#[test]
fn the_wheel_and_moves_go_over_the_page() {
    let mut browser = browser();
    browser.set_typing(typing(None, 0), 0);
    pointer_at(&mut browser, 3.0, 3.0);
    browser.take_outputs();
    assert_eq!(browser.terminal_wheel(0.0, 1.0), Some(false), "no mode needed");
    assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Wheel { dx: 0, dy: 120, .. }]));
    browser.set_browser_modifier(true);
    assert_eq!(browser.terminal_wheel(0.0, 1.0), None, "the modifier keeps it the browser's");
    browser.set_browser_modifier(false);
    pointer_at(&mut browser, -2.0, 3.0);
    assert_eq!(browser.terminal_wheel(0.0, 1.0), None, "beside the page the browser scrolls");
    pointer_at(&mut browser, 3.0, 3.0);
    browser.take_outputs();
    pointer_at(&mut browser, 5.0, 3.0);
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "no move with no button outside a pointer area");
}

/// INPUT.md section 7.4 (decision E.128): a move with no button held goes
/// only within a pointer area of the open page; a drag goes anywhere on it.
#[cfg(feature = "links")]
#[test]
fn a_move_goes_in_a_pointer_area_and_a_drag_anywhere() {
    use crate::links::{Area, PageLinks};
    let mut browser = browser();
    browser.set_typing(typing(None, 0), 0);
    let area = Area { row: 10, column: 0, width: 20, height: 2 };
    browser.set_links(KEY, PageLinks { pointer_areas: alloc::vec![area], ..PageLinks::default() });
    pointer_at(&mut browser, 5.5, 10.5);
    assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Mouse { action: MouseAction::Move, button: 0, .. }]), "in the area");
    pointer_at(&mut browser, 25.5, 10.5);
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "beside it");
    let mut canvas = Recorder::new();
    browser.pointer_pressed(&mut canvas, false, 10_000);
    browser.take_outputs();
    pointer_at(&mut browser, 30.5, 3.5);
    assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Mouse { action: MouseAction::Move, button: 1, .. }]), "a drag goes anywhere");
}

#[test]
fn paste_and_composition_each_go_as_one_intent() {
    let mut browser = browser();
    browser.set_typing(typing(None, 0), 0);
    assert!(browser.terminal_paste("line\nline"));
    assert!(browser.terminal_compose(9, "に"), "composition goes whenever keys go");
    let sent = intents(&browser.take_outputs());
    assert_eq!(sent, [Intent::Paste("line\nline".into()), Intent::Compose { cursor: 1, text: "に".into() }], "the offset kept within the text");
}

#[test]
fn long_text_goes_as_one_intent_within_the_paste_bound() {
    let mut browser = browser();
    browser.set_typing(typing(None, 0), 0);
    browser.terminal_text(&"é".repeat(2_100), 0);
    let sent = intents(&browser.take_outputs());
    assert!(matches!(sent.as_slice(), [Intent::Text(text)] if text.len() == 4_000), "cut at a character within 4 000 bytes");
}

/// REQ-INT-131: the strip says where keys go, "Not secure" on plain
/// `ws://`, and the round trip; or that the channel did not answer.
#[test]
fn the_strip_says_where_keys_go() {
    let area = Rect::new(0.0, 0.0, 900.0, 40.0);
    let badge = |browser: &Browser| {
        let mut canvas = Recorder::new();
        browser.draw_typing_badge(&mut canvas, area);
        canvas.texts.iter().map(|(_, text)| text.as_str()).collect::<String>()
    };
    let mut browser = browser();
    assert_eq!(badge(&browser), "", "nothing while keys do not go");
    browser.set_typing(TypingView { round_trip_ms: Some(3), ..typing(None, 0) }, 0);
    let said = badge(&browser);
    assert!(said.contains("Keys go to") && said.contains("3 ms") && said.contains("Not secure"), "{said}");
    browser.set_typing(TypingView { secure: true, ..typing(None, 0) }, 0);
    assert!(!badge(&browser).contains("Not secure"));
    browser.set_typing(TypingView { on: false, refused: true, ..typing(None, 0) }, 0);
    assert!(badge(&browser).contains("did not answer"), "{}", badge(&browser));
}

#[test]
fn the_cursor_is_drawn_only_while_typing() {
    let mut browser = browser();
    let mut canvas = Recorder::new();
    browser.draw_terminal(&mut canvas, 0);
    let before = canvas.fills.len();
    browser.set_typing(typing(Some(CursorView { row: 3, column: 0, width: 0, flags: 0 }), 0), 0);
    browser.draw_terminal(&mut canvas, 0);
    assert!(canvas.fills.len() > before, "a bar is drawn");
}

/// INPUT.md section 5.4: a wheel's notch is at least a whole line, in
/// either direction, so one slow notch moves a list a line; a faster
/// notch keeps its acceleration, and no turn stays no turn.
#[test]
fn a_wheel_notch_is_at_least_a_line() {
    use crate::browser::notched;
    assert!((notched(0.1) - 1.0).abs() < f64::EPSILON, "a slow notch on macOS");
    assert!((notched(-0.1) + 1.0).abs() < f64::EPSILON);
    assert!((notched(2.5) - 2.5).abs() < f64::EPSILON, "a fast one keeps its acceleration");
    assert!(notched(0.0).abs() < f64::EPSILON);
}
