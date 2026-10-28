//! Tests of the pointer's moves, gone and touch in the trusted core
//! (`RETURN-CHANNEL.md` sections 4.1 and 4.4; INPUT.md section 9.8,
//! REQ-INT-122 to REQ-INT-124), each also a hostile interface's: what it
//! sends cannot raise the rate, set the buttons a move holds, or invent,
//! move or end a contact.

use super::*;
use neoteletext_input::caps::TOUCH_CAP;
use neoteletext_input::event::{Device, Mouse, Touch};
use neoteletext_input::pace::PERIOD_WHOLE_US;
use protocol::interaction::TouchPhase;

/// A position in cell (`column`, `row`), `dot` dots in from its top left.
fn at(column: u32, row: u32, dot: u32) -> Point {
    let (across, down) = (dot.checked_rem(12).unwrap(), dot.checked_div(12).unwrap().checked_rem(20).unwrap());
    Point {
        column,
        row,
        pixels: Some((column.checked_mul(12).unwrap().checked_add(across).unwrap(), row.checked_mul(20).unwrap().checked_add(down).unwrap())),
        mods: 0,
    }
}

fn intent(intent: Intent) -> InputIntent {
    InputIntent { key: PAGE, intent }
}

fn moved(at: Point) -> InputIntent {
    intent(Intent::Mouse { action: MouseAction::Move, button: 0, at })
}

fn mouse(action: MouseAction, button: u8, at: Point, device: Device) -> String {
    format!("{:?}", Item::Event(Event::Mouse(Mouse { action, button, at, device })))
}

fn touch(action: MouseAction, id: u8, at: Point) -> String {
    format!("{:?}", Item::Event(Event::Touch(Touch { action, id, column: at.column, row: at.row, pixels: at.pixels })))
}

fn contact(phase: TouchPhase, contact: u64, at: Point) -> InputIntent {
    intent(Intent::Touch { phase, contact, at })
}

/// The period's whole microseconds and its last third: 16,667.
const PERIOD_US: u64 = PERIOD_WHOLE_US + 1;

/// REQ-RCH-030: every change of the dot over the open page goes, buttons
/// held or not, and the buttons a move holds are those the trusted core
/// sent pressed, whatever the interface says. On the code before, a move
/// held the interface's mask.
#[test]
fn every_move_goes_with_the_buttons_the_core_sent_held() {
    let mut typing = ready(PAGE_CAP);
    let here = at(10, 5, 0);
    assert_eq!(items(&typing.input_intent(&moved(here), 0)), [mouse(MouseAction::Move, 0, here, Device::Mouse)], "no button held");
    let press = intent(Intent::Mouse { action: MouseAction::Press, button: 0, at: here });
    typing.input_intent(&press, 20 * MS);
    let there = at(11, 5, 3);
    let lying = intent(Intent::Mouse { action: MouseAction::Move, button: 0b1111_1110, at: there });
    assert_eq!(items(&typing.input_intent(&lying, 40 * MS)), [mouse(MouseAction::Move, 1, there, Device::Mouse)], "button 0 held, and only it");
}

/// REQ-RCH-031, REQ-RCH-036: a hostile interface sending 1,000 moves a
/// second for 10 s, every one to a new dot, gets 60 a second to the
/// channel, with no drift. On the code before, all 10,000 went.
#[test]
fn a_thousand_moves_a_second_go_sixty_a_second() {
    const SECONDS: u64 = 10;
    const OFFERED_PER_SECOND: u64 = 1_000;
    const STEP_US: u64 = 1_000_000 / OFFERED_PER_SECOND;
    /// Dots offered in turn, each other than the one before.
    const DOTS: u64 = 240;
    let mut typing = ready(PAGE_CAP);
    let mut sent = 0_usize;
    for n in 0..SECONDS * OFFERED_PER_SECOND {
        let now = n.checked_mul(STEP_US).unwrap();
        // The core's timer: what fell due before this move goes first.
        if typing.due().is_some_and(|due| due <= now) {
            sent = sent.checked_add(items(&typing.take_due(now)).len()).unwrap();
        }
        let dot = u32::try_from(n.checked_rem(DOTS).unwrap()).unwrap();
        let place = at(dot.checked_rem(40).unwrap(), dot.checked_div(40).unwrap().checked_add(1).unwrap(), dot);
        sent = sent.checked_add(items(&typing.input_intent(&moved(place), now)).len()).unwrap();
    }
    let end = SECONDS * 1_000_000;
    if typing.due().is_some_and(|due| due <= end) {
        sent = sent.checked_add(items(&typing.take_due(end)).len()).unwrap();
    }
    let expected = usize::try_from(SECONDS * neoteletext_input::MOVES_PER_SECOND).unwrap();
    assert!((expected..=expected.checked_add(1).unwrap()).contains(&sent), "{sent} moves in {SECONDS} s");
}

/// REQ-RCH-032: a move 20 ms after the last goes at once; one 5 ms after
/// waits, the latest replacing it, and goes a period after the last; an
/// unchanged position does not go again; and no timer is armed while
/// nothing waits. On the code before, every move went at once.
#[test]
fn a_move_goes_at_once_if_it_can_and_the_latest_is_kept() {
    let mut typing = ready(PAGE_CAP);
    let (first, second, third) = (at(1, 1, 0), at(1, 1, 1), at(1, 1, 2));
    assert_eq!(items(&typing.input_intent(&moved(first), 0)).len(), 1);
    assert_eq!(typing.due(), None, "nothing waits");
    assert_eq!(items(&typing.input_intent(&moved(second), 5 * MS)), [] as [String; 0], "5 ms after: it waits");
    assert_eq!(typing.due(), Some(PERIOD_WHOLE_US), "until a period after the last");
    assert_eq!(items(&typing.input_intent(&moved(third), 10 * MS)), [] as [String; 0]);
    assert_eq!(items(&typing.take_due(PERIOD_WHOLE_US)), [mouse(MouseAction::Move, 0, third, Device::Mouse)], "the latest");
    assert_eq!(typing.due(), None);
    assert_eq!(items(&typing.input_intent(&moved(third), 30 * MS)), [] as [String; 0], "unchanged");
    assert_eq!(items(&typing.input_intent(&moved(first), 50 * MS)).len(), 1, "20 ms after the last: at once");
}

/// REQ-RCH-033: a drag ending in a release within a period: the waiting
/// move is written before the release, in the same write.
#[test]
fn a_waiting_move_goes_before_the_release() {
    let mut typing = ready(PAGE_CAP);
    let (start, end) = (at(2, 2, 0), at(3, 2, 0));
    typing.input_intent(&intent(Intent::Mouse { action: MouseAction::Press, button: 0, at: start }), 0);
    typing.input_intent(&moved(start), MS);
    assert_eq!(items(&typing.input_intent(&moved(end), 2 * MS)), [] as [String; 0]);
    let sends = typing.input_intent(&intent(Intent::Mouse { action: MouseAction::Release, button: 0, at: end }), 3 * MS);
    assert_eq!(items(&sends), [mouse(MouseAction::Move, 1, end, Device::Mouse), mouse(MouseAction::Release, 0, end, Device::Mouse)]);
    let tags: Vec<_> = sends.iter().filter_map(|send| if let Send::Fetcher { move_of, more, .. } = send { Some((*move_of, *more)) } else { None }).collect();
    assert_eq!(tags, [(Some(0), true), (None, false)], "the move tagged as the mouse's, both in one write");
    assert_eq!(typing.due(), None);
}

/// REQ-RCH-034 (`interaction.md` section 6): the pointer leaving the page
/// sends the waiting move and then gone, with the buttons held and the
/// last position; a drag let go off the page so ends, and its release,
/// which the channel has had as gone, is not sent. On the code before,
/// nothing went and the channel kept the button held.
#[test]
fn leaving_the_page_sends_the_waiting_move_then_gone() {
    // Gone goes only to a channel that agreed TOUCH or RESUME.
    let mut typing = ready(PAGE_CAP | TOUCH_CAP);
    let (start, edge) = (at(5, 5, 0), at(0, 5, 0));
    typing.input_intent(&intent(Intent::Mouse { action: MouseAction::Press, button: 0, at: start }), 0);
    typing.input_intent(&moved(start), MS);
    typing.input_intent(&moved(edge), 2 * MS);
    let sends = typing.input_intent(&intent(Intent::Gone), 3 * MS);
    assert_eq!(items(&sends), [mouse(MouseAction::Move, 1, edge, Device::Mouse), mouse(MouseAction::Gone, 1, edge, Device::Mouse)]);
    let release = intent(Intent::Mouse { action: MouseAction::Release, button: 0, at: edge });
    assert_eq!(items(&typing.input_intent(&release, 4 * MS)), [] as [String; 0], "released already, by gone");
    assert_eq!(items(&typing.input_intent(&intent(Intent::Gone), 5 * MS)), [] as [String; 0], "gone once");
    assert_eq!(items(&typing.input_intent(&moved(start), 30 * MS)), [mouse(MouseAction::Move, 0, start, Device::Mouse)], "back: at once");
}

/// REQ-RCH-034, REQ-INT-123: keys stopping going to the channel, as the
/// window loses the focus or another page opens, send the waiting move
/// and gone before `FOCUS_OUT`.
#[test]
fn keys_stopping_send_the_waiting_move_then_gone() {
    // Gone goes only to a channel that agreed TOUCH or RESUME.
    let mut typing = ready(PAGE_CAP | TOUCH_CAP);
    let (here, there) = (at(5, 5, 0), at(6, 5, 0));
    typing.input_intent(&moved(here), 0);
    typing.input_intent(&moved(there), MS);
    let away = typing.focused(false, 2 * MS);
    assert_eq!(
        items(&away),
        [mouse(MouseAction::Move, 0, there, Device::Mouse), mouse(MouseAction::Gone, 0, there, Device::Mouse), "Event(FocusOut)".to_owned()]
    );
    assert_eq!(typing.due(), None, "nothing waits after");
    typing.focused(true, 3 * MS);
    typing.input_intent(&moved(here), 30 * MS);
    let leaving = typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 31 * MS);
    assert_eq!(items(&leaving)[0], mouse(MouseAction::Gone, 0, here, Device::Mouse), "another page opened");
}

fn touch_ready() -> Typing {
    ready(PAGE_CAP | TOUCH_CAP)
}

/// REQ-RCH-041, REQ-RCH-042, REQ-RCH-043, REQ-RCH-049: with TOUCH agreed
/// the first contact goes as MOUSE, device touch, button 0, its moves
/// holding button 0; a second as TOUCH 1; each lift as a release, and a
/// finger moves no more once lifted. On the code before, no contact went.
#[test]
fn the_first_contact_is_the_mouse_and_the_next_touch() {
    let mut typing = touch_ready();
    let (a, b, c) = (at(4, 4, 0), at(20, 10, 0), at(4, 5, 0));
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Began, 77, a), 0)), [mouse(MouseAction::Press, 0, a, Device::Touch)]);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Began, 78, b), MS)), [touch(MouseAction::Press, 1, b)]);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Moved, 77, c), 2 * MS)), [mouse(MouseAction::Move, 1, c, Device::Touch)]);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Ended, 78, b), 3 * MS)), [touch(MouseAction::Release, 1, b)]);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Ended, 77, c), 4 * MS)), [mouse(MouseAction::Release, 0, c, Device::Touch)]);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Moved, 77, a), 40 * MS)), [] as [String; 0], "no hover from a finger");
    assert_eq!(items(&typing.focused(false, 50 * MS)), ["Event(FocusOut)"], "nothing to let go of");
}

/// REQ-RCH-041: without TOUCH agreed the first contact goes as the mouse,
/// device 0, and further contacts send nothing.
#[test]
fn without_touch_a_finger_is_the_mouse_alone() {
    let mut typing = ready(PAGE_CAP);
    let (a, b) = (at(4, 4, 0), at(20, 10, 0));
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Began, 1, a), 0)), [mouse(MouseAction::Press, 0, a, Device::Mouse)]);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Began, 2, b), MS)), [] as [String; 0]);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Moved, 2, a), 2 * MS)), [] as [String; 0]);
}

/// REQ-RCH-046: a cancelled contact goes as gone, from where it was last
/// sent, never as a release; keys stopping send gone for every contact
/// still down.
#[test]
fn a_cancelled_contact_is_gone() {
    let mut typing = touch_ready();
    let (a, b, c) = (at(4, 4, 0), at(20, 10, 0), at(21, 10, 0));
    typing.input_intent(&contact(TouchPhase::Began, 1, a), 0);
    typing.input_intent(&contact(TouchPhase::Began, 2, b), MS);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Cancelled, 2, c), 2 * MS)), [touch(MouseAction::Gone, 1, b)]);
    typing.input_intent(&contact(TouchPhase::Began, 3, c), 3 * MS);
    let away = typing.focused(false, 4 * MS);
    assert_eq!(items(&away), [mouse(MouseAction::Gone, 1, a, Device::Touch), touch(MouseAction::Gone, 1, c), "Event(FocusOut)".to_owned()]);
}

/// REQ-RCH-045: each contact is paced alone, and the moves of two due at
/// one moment leave in one write.
#[test]
fn each_contact_is_paced_alone_and_due_moves_leave_together() {
    let mut typing = touch_ready();
    typing.input_intent(&contact(TouchPhase::Began, 1, at(1, 1, 0)), 0);
    typing.input_intent(&contact(TouchPhase::Began, 2, at(9, 1, 0)), 0);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Moved, 1, at(1, 1, 1)), MS)).len(), 1, "contact 0's first move: at once");
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Moved, 2, at(9, 1, 1)), MS)).len(), 1, "contact 1's own pace");
    typing.input_intent(&contact(TouchPhase::Moved, 1, at(1, 1, 2)), 2 * MS);
    typing.input_intent(&contact(TouchPhase::Moved, 2, at(9, 1, 2)), 2 * MS);
    let due = typing.take_due(MS + PERIOD_US);
    assert_eq!(items(&due), [mouse(MouseAction::Move, 1, at(1, 1, 2), Device::Touch), touch(MouseAction::Move, 1, at(9, 1, 2))]);
    let tags: Vec<_> = due.iter().filter_map(|send| if let Send::Fetcher { move_of, more, .. } = send { Some((*move_of, *more)) } else { None }).collect();
    assert_eq!(tags, [(Some(0), true), (Some(1), false)]);
}

/// REQ-RCH-053, REQ-RCH-043: a hostile interface cannot invent a contact,
/// move or end one not begun, begin one twice, pass `MAX_CONTACTS`, or
/// have the mouse share MOUSE with a finger. On the code before, no
/// contact was numbered at all.
#[test]
fn a_hostile_interface_cannot_invent_a_contact() {
    let mut typing = touch_ready();
    let here = at(3, 3, 0);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Moved, 9, here), 0)), [] as [String; 0], "a move of a contact not begun");
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Ended, 9, here), 0)), [] as [String; 0], "an end of one not begun");
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Cancelled, 9, here), 0)), [] as [String; 0]);
    let mut numbered = 0_usize;
    for system in 0..20_u64 {
        let began = contact(TouchPhase::Began, system.checked_add(1_000).unwrap(), at(u32::try_from(system).unwrap(), 3, 0));
        numbered = numbered.checked_add(items(&typing.input_intent(&began, 0)).len()).unwrap();
    }
    assert_eq!(numbered, MAX_CONTACTS, "ten at most");
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Began, 1_000, here), 0)), [] as [String; 0], "begun twice");
    let click = intent(Intent::Mouse { action: MouseAction::Press, button: 0, at: here });
    assert_eq!(items(&typing.input_intent(&click, 0)), [] as [String; 0], "the mouse never shares MOUSE with a finger");
    assert_eq!(items(&typing.input_intent(&moved(here), 0)), [] as [String; 0]);
}

/// REQ-RCH-041: a contact while a mouse button is held goes as TOUCH, so
/// the two never share MOUSE.
#[test]
fn a_contact_while_a_button_is_held_is_touch() {
    let mut typing = touch_ready();
    let here = at(3, 3, 0);
    typing.input_intent(&intent(Intent::Mouse { action: MouseAction::Press, button: 0, at: here }), 0);
    assert_eq!(items(&typing.input_intent(&contact(TouchPhase::Began, 5, here), MS)), [touch(MouseAction::Press, 1, here)]);
}

/// REQ-RCH-016: the round trip is measured exactly while keys go to the
/// channel, and shown with them.
#[test]
fn the_round_trip_is_measured_while_keys_go() {
    let measures = |sends: &[Send]| sends.iter().filter_map(|send| if let Send::Measure { on, .. } = send { Some(*on) } else { None }).collect::<Vec<_>>();
    let mut typing = Typing::default();
    typing.input(PAGE.channel, true, false, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    assert_eq!(measures(&typing.control(PAGE.channel, &welcome(PAGE_CAP), 0)), [true], "keys go: on");
    let sends = typing.round_trip(PAGE.channel, 190);
    assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.round_trip_us == Some(190)), "{sends:?}");
    assert_eq!(measures(&typing.focused(false, MS)), [false], "keys stop: off");
    assert_eq!(typing.round_trip(PAGE.channel, 200), [], "not measured, not shown");
    assert_eq!(measures(&typing.focused(true, 2 * MS)), [true]);
}

/// REQ-RCH-010: the messages of one moment, a session's start, are marked
/// to leave in one write: all but the last have more after them.
#[test]
fn a_moments_messages_leave_together() {
    let mut typing = Typing::default();
    typing.input(PAGE.channel, true, false, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    let sends = typing.control(PAGE.channel, &welcome(PAGE_CAP), 0);
    let more: Vec<bool> = sends.iter().filter_map(|send| if let Send::Fetcher { more, .. } = send { Some(*more) } else { None }).collect();
    assert_eq!(more, [true, true, false], "RESIZE, PAGE, FOCUS_IN");
}
