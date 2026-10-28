use super::*;
use alloc::vec::Vec;

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

fn typing(cursor: Option<CursorView>) -> TypingView {
    TypingView { channel: KEY.channel, page: KEY.page, on: true, secure: false, refused: false, cursor, round_trip_us: None, show: None }
}

fn intents(outputs: &[Output]) -> Vec<Intent> {
    outputs.iter().filter_map(|output| if let Output::Terminal { intent, .. } = output { Some(intent.clone()) } else { None }).collect()
}

fn press(browser: &mut Browser, character: char, mods: u8) {
    let mut text = [0_u8; 4];
    let text = character.encode_utf8(&mut text);
    browser.terminal_key(Action::Press, KeyCode::Character(character), mods, Some(text), None);
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
    press(&mut browser, 'a', 0);
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "the core has not said");
    browser.set_typing(typing(None), 0);
    assert_eq!(browser.typing_target(), Some(KEY));
    press(&mut browser, 'a', 0);
    browser.terminal_key(Action::Release, KeyCode::Character('a'), 0, None, None);
    let sent = intents(&browser.take_outputs());
    assert_eq!(sent.len(), 2);
    assert!(matches!(&sent[0], Intent::Key { action: Action::Press, key: KeyCode::Character('a'), text: Some(text), .. } if text == "a"));
    assert!(matches!(&sent[1], Intent::Key { action: Action::Release, .. }));
    browser.set_typing(TypingView { on: false, ..typing(None) }, 0);
    press(&mut browser, 'b', 0);
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "typing is off");
}

/// A panel, popover or list of the strip, while open, takes the keys:
/// none go to the page's channel. Before, Return in the star's popover
/// went to a channel that takes keys, and the popover never closed.
#[test]
fn the_strips_panels_take_the_keys_from_the_channel() {
    let mut browser = browser();
    browser.set_typing(typing(None), 0);
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

/// REQ-INT-115, REQ-INT-136 (decision E.125): a plain click on a page
/// that takes input, off its links, is the host's, with no request; with
/// the browser's modifier it is the browser's.
#[test]
fn a_click_is_the_hosts_unless_the_modifier_is_held() {
    let mut browser = browser();
    let mut canvas = Recorder::new();
    browser.set_typing(typing(None), 0);
    pointer_at(&mut browser, 10.5, 5.5);
    assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Mouse { action: MouseAction::Move, button: 0, .. }]), "a move, no button held");
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

/// REQ-RCH-005: the middle, right, back and forward buttons go over the
/// page, pressed and released, as the main button does, and every one of
/// the mouse's events carries the modifiers held; a release whose press
/// did not go does not go, and with the browser's modifier no button goes.
/// Before, only the main button went, and the modifiers were always 0.
#[test]
fn every_button_and_modifier_goes_over_the_page() {
    use crate::input::mouse_button::{BACK, FORWARD, LEFT, MIDDLE, RIGHT};
    use neoteletext_input::event::{ALT, SHIFT};
    let mut browser = browser();
    let mut canvas = Recorder::new();
    browser.set_typing(typing(None), 0);
    pointer_at(&mut browser, 10.5, 5.5);
    browser.take_outputs();
    browser.set_mouse_modifiers(SHIFT | CONTROL);
    browser.pointer_pressed(&mut canvas, false, 10_000);
    browser.pointer_released();
    assert!(browser.middle_pressed(), "the middle button");
    assert!(browser.other_button_released(MIDDLE));
    assert!(browser.secondary_pressed(), "the right button");
    assert!(browser.other_button_released(RIGHT));
    for button in [BACK, FORWARD] {
        assert!(browser.other_button_pressed(button), "button {button}");
        assert!(browser.other_button_released(button), "button {button}");
    }
    let sent = intents(&browser.take_outputs());
    let mut buttons = Vec::new();
    for intent in &sent {
        let Intent::Mouse { action, button, at } = intent else { panic!("only the buttons: {sent:?}") };
        assert_eq!(at.mods, SHIFT | CONTROL, "the modifiers held: {intent:?}");
        assert_eq!((at.column, at.row), (10, 5));
        buttons.push((*action, *button));
    }
    let expected: Vec<(MouseAction, u8)> =
        [LEFT, MIDDLE, RIGHT, BACK, FORWARD].into_iter().flat_map(|button| [(MouseAction::Press, button), (MouseAction::Release, button)]).collect();
    assert_eq!(buttons, expected);
    assert!(!browser.other_button_released(BACK), "no press went");
    browser.set_mouse_modifiers(ALT);
    browser.terminal_wheel(0.0, 1.0);
    assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Wheel { at: Point { mods: ALT, .. }, .. }]), "the wheel's modifiers");
    browser.set_browser_modifier(true);
    assert!(!browser.other_button_pressed(BACK));
    assert!(!browser.middle_pressed());
    assert!(!browser.secondary_pressed());
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "the browser's modifier keeps every button the browser's");
}

/// Decision 122, the owner's revision of decision 118 on 28 October 2026
/// ("links must be acted upon by the client browser"): on a page that
/// takes input a link the page declares is the browser's. A plain click on
/// it follows it and a middle click is the browser's, as on any page, and
/// neither sends the channel a press or a release; a click on a control,
/// or on a cell beside the link, still goes to the channel. Before, the
/// plain click and the middle click on the link went to the channel.
#[cfg(feature = "links")]
#[test]
fn a_link_on_a_page_that_takes_input_is_the_browsers() {
    use crate::input::mouse_button::{LEFT, MIDDLE};
    use crate::links::{Area, LinkKind, LinkRun, PageLinks};
    let mut browser = browser();
    let mut canvas = Recorder::new();
    browser.set_typing(typing(None), 0);
    let link = LinkRun { row: 5, column: 10, length: 3, double: false, light: false, kind: LinkKind::Page };
    let control = Area { row: 20, column: 1, width: 9, height: 1 };
    browser.set_links(KEY, PageLinks { runs: alloc::vec![link], controls: alloc::vec![control], ..PageLinks::default() });
    pointer_at(&mut browser, 11.5, 5.5);
    browser.take_outputs();
    browser.pointer_pressed(&mut canvas, false, 10_000);
    browser.pointer_released();
    let outputs = browser.take_outputs();
    assert!(outputs.iter().any(|output| matches!(output, Output::Cell(click) if (click.column, click.row) == (11, 5))), "a plain click follows the link");
    assert_eq!(intents(&outputs), [] as [Intent; 0], "no press or release of the click reaches the channel");
    browser.middle_pressed();
    browser.other_button_released(MIDDLE);
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "nor of the middle button");
    for (column, row, what) in [(13.5, 5.5, "beside the link"), (4.5, 20.5, "on a control")] {
        pointer_at(&mut browser, column, row);
        browser.take_outputs();
        browser.pointer_pressed(&mut canvas, false, 20_000);
        browser.pointer_released();
        let sent = intents(&browser.take_outputs());
        assert!(
            matches!(
                sent.as_slice(),
                [Intent::Mouse { action: MouseAction::Press, button: LEFT, .. }, Intent::Mouse { action: MouseAction::Release, button: LEFT, .. }]
            ),
            "a click {what} is the channel's: {sent:?}"
        );
        assert!(browser.middle_pressed(), "a middle click {what} is the channel's");
        assert!(browser.other_button_released(MIDDLE));
        browser.take_outputs();
    }
}

/// REQ-RCH-001: the interface knows the page under the pointer, whose
/// frames it draws as they arrive.
#[test]
fn the_page_under_the_pointer_is_known() {
    let mut browser = browser();
    pointer_at(&mut browser, 3.0, 3.0);
    assert_eq!(browser.pointed_page(), Some(KEY));
    pointer_at(&mut browser, -2.0, 3.0);
    assert_eq!(browser.pointed_page(), None, "beside the page");
}

/// Decision 107 and INPUT.md section 6.3: the wheel and the pointer's
/// moves over the page are the channel's; beside the page, or with the
/// browser's modifier, the wheel is the browser's.
#[test]
fn the_wheel_and_moves_go_over_the_page() {
    let mut browser = browser();
    browser.set_typing(typing(None), 0);
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
    assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Mouse { action: MouseAction::Move, button: 0, .. }]), "every move");
}

/// REQ-RCH-030, REQ-RCH-034 (`interaction.md` section 6): every change of
/// the dot over the open page goes as a move, buttons held or not, with
/// the dot inside its cell; the same dot does not go again; leaving the
/// page, onto its border or out of the window, is gone, once; a drag let
/// go off the page ends with gone, and its release does not go. On the
/// code before, a move with no button outside a pointer area did not go,
/// and nothing told the channel the pointer had left.
#[test]
fn every_move_goes_and_leaving_the_page_is_gone() {
    let mut browser = browser();
    browser.set_typing(typing(None), 0);
    pointer_at(&mut browser, 5.5, 10.5);
    let sent = intents(&browser.take_outputs());
    let [Intent::Mouse { action: MouseAction::Move, button: 0, at }] = sent.as_slice() else { panic!("{sent:?}") };
    let (x, y) = at.pixels.unwrap();
    assert_eq!((x.checked_div(12), y.checked_div(20), at.column, at.row), (Some(5), Some(10), 5, 10), "the dot inside its cell");
    pointer_at(&mut browser, 5.5, 10.5);
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "the same dot");
    pointer_at(&mut browser, 25.5, 10.5);
    assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Mouse { action: MouseAction::Move, button: 0, .. }]), "anywhere");
    let mut canvas = Recorder::new();
    browser.pointer_pressed(&mut canvas, false, 10_000);
    browser.take_outputs();
    pointer_at(&mut browser, 30.5, 3.5);
    assert!(matches!(intents(&browser.take_outputs()).as_slice(), [Intent::Mouse { action: MouseAction::Move, button: 1, .. }]), "a drag");
    pointer_at(&mut browser, -2.0, 3.5);
    assert_eq!(intents(&browser.take_outputs()), [Intent::Gone], "off the page: gone");
    pointer_at(&mut browser, -3.0, 3.5);
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "gone once");
    browser.pointer_released();
    assert_eq!(intents(&browser.take_outputs()), [] as [Intent; 0], "the drag let go off the page: gone released it");
    // Out of the window, as the interface reports winit's `CursorLeft`.
    pointer_at(&mut browser, 3.5, 3.5);
    browser.take_outputs();
    browser.pointer_moved(-1.0, -1.0, 20_000);
    assert_eq!(intents(&browser.take_outputs()), [Intent::Gone]);
}

/// The page's middle, at `column`, `row`, in logical pixels.
fn spot(browser: &Browser, column: f64, row: f64) -> (f64, f64) {
    let (_, page) = browser.wall.open().unwrap();
    (page.x0 + page.width() * (column / 40.0), page.y0 + page.height() * (row / 25.0))
}

/// REQ-RCH-041 to REQ-RCH-046: while keys go to the page's channel a
/// contact on the page goes as an intent for the trusted core to number,
/// each change of its dot a move; one that leaves the page is cancelled;
/// a lift is a lift; while keys do not go, no contact is taken. On the
/// code before, the browser took no contact at all.
#[test]
fn a_contact_on_the_page_goes_to_the_channel() {
    use crate::input::TouchPhase;
    let mut browser = browser();
    let (x, y) = spot(&browser, 4.5, 4.5);
    assert!(!browser.touch(TouchPhase::Began, 7, x, y), "keys do not go: not taken");
    browser.set_typing(typing(None), 0);
    assert!(browser.touch(TouchPhase::Began, 7, x, y));
    assert!(browser.touch(TouchPhase::Moved, 7, x, y));
    let (x2, y2) = spot(&browser, 6.5, 4.5);
    assert!(browser.touch(TouchPhase::Moved, 7, x2, y2));
    assert!(browser.touch(TouchPhase::Ended, 7, x2, y2));
    let sent = intents(&browser.take_outputs());
    let phases: Vec<_> =
        sent.iter().map(|intent| if let Intent::Touch { phase, contact: 7, at } = intent { (*phase, at.column) } else { panic!("{sent:?}") }).collect();
    assert_eq!(phases, [(TouchPhase::Began, 4), (TouchPhase::Moved, 6), (TouchPhase::Ended, 6)], "the same dot not again");
    assert!(browser.touch(TouchPhase::Began, 8, x, y));
    assert!(browser.touch(TouchPhase::Moved, 8, -5.0, y), "off the page");
    assert!(!browser.touch(TouchPhase::Moved, 8, x, y), "no longer the channel's");
    let sent = intents(&browser.take_outputs());
    assert!(
        matches!(sent.as_slice(), [Intent::Touch { phase: TouchPhase::Began, .. }, Intent::Touch { phase: TouchPhase::Cancelled, contact: 8, .. }]),
        "{sent:?}"
    );
}

/// REQ-RCH-048 (decision 122): a contact that begins on a link the page
/// declares is the browser's: nothing goes to the channel, and lifting it
/// on the link follows the link; lifting it elsewhere follows nothing.
#[cfg(feature = "links")]
#[test]
fn a_contact_on_a_link_follows_it_and_never_reaches_the_channel() {
    use crate::input::TouchPhase;
    use crate::links::{LinkKind, LinkRun, PageLinks};
    let mut browser = browser();
    browser.set_typing(typing(None), 0);
    let link = LinkRun { row: 5, column: 10, length: 3, double: false, light: false, kind: LinkKind::Page };
    browser.set_links(KEY, PageLinks { runs: alloc::vec![link], ..PageLinks::default() });
    browser.take_outputs();
    let (x, y) = spot(&browser, 11.5, 5.5);
    assert!(browser.touch(TouchPhase::Began, 3, x, y));
    assert!(browser.touch(TouchPhase::Moved, 3, x + 1.0, y));
    assert!(browser.touch(TouchPhase::Ended, 3, x, y));
    let outputs = browser.take_outputs();
    assert_eq!(intents(&outputs), [] as [Intent; 0], "the channel is never told");
    assert!(outputs.iter().any(|output| matches!(output, Output::Cell(click) if (click.column, click.row) == (11, 5))), "followed on the lift");
    let (away_x, away_y) = spot(&browser, 30.5, 15.5);
    browser.touch(TouchPhase::Began, 4, x, y);
    browser.touch(TouchPhase::Ended, 4, away_x, away_y);
    let outputs = browser.take_outputs();
    assert!(!outputs.iter().any(|output| matches!(output, Output::Cell(_))), "lifted off the link: nothing followed");
    assert_eq!(intents(&outputs), [] as [Intent; 0]);
}

#[test]
fn paste_and_composition_each_go_as_one_intent() {
    let mut browser = browser();
    browser.set_typing(typing(None), 0);
    assert!(browser.terminal_paste("line\nline"));
    assert!(browser.terminal_compose(9, "に"), "composition goes whenever keys go");
    let sent = intents(&browser.take_outputs());
    assert_eq!(sent, [Intent::Paste("line\nline".into()), Intent::Compose { cursor: 1, text: "に".into() }], "the offset kept within the text");
}

#[test]
fn long_text_goes_as_one_intent_within_the_paste_bound() {
    let mut browser = browser();
    browser.set_typing(typing(None), 0);
    browser.terminal_text(&"é".repeat(2_100));
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
    browser.set_typing(TypingView { round_trip_us: Some(3_250), ..typing(None) }, 0);
    let said = badge(&browser);
    assert!(said.contains("Keys go to") && said.contains("3.2 ms") && said.contains("Not secure"), "{said}");
    // REQ-RCH-016: a ping over a near path, a fraction of a millisecond.
    browser.set_typing(TypingView { round_trip_us: Some(190), ..typing(None) }, 0);
    assert!(badge(&browser).contains("0.1 ms"), "{}", badge(&browser));
    browser.set_typing(TypingView { secure: true, ..typing(None) }, 0);
    assert!(!badge(&browser).contains("Not secure"));
    browser.set_typing(TypingView { on: false, refused: true, ..typing(None) }, 0);
    assert!(badge(&browser).contains("did not answer"), "{}", badge(&browser));
}

#[test]
fn the_cursor_is_drawn_only_while_typing() {
    let mut browser = browser();
    let mut canvas = Recorder::new();
    browser.draw_terminal(&mut canvas, 0);
    let before = canvas.fills.len();
    browser.set_typing(typing(Some(CursorView { row: 3, column: 0, flags: 0 })), 0);
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
