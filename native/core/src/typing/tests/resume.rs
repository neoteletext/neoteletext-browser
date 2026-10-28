//! Tests of resuming a lost connection's session in the trusted core
//! (INPUT.md section 6.4, REQ-INT-125; `RETURN-CHANNEL.md` REQ-RCH-013).

use super::*;
use neoteletext_input::caps::RESUME_CAP;
use neoteletext_input::control::Resumption;

const TOKEN: [u8; neoteletext_input::TOKEN_BYTES] = [7; neoteletext_input::TOKEN_BYTES];
const NEXT_TOKEN: [u8; neoteletext_input::TOKEN_BYTES] = [9; neoteletext_input::TOKEN_BYTES];

/// A session whose channel set RESUME, its start sent: the count is 4,
/// HELLO, RESIZE, PAGE and `FOCUS_IN`.
fn resumable() -> Typing {
    let mut typing = Typing::default();
    typing.input(PAGE.channel, true, true, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    typing.control(PAGE.channel, &welcome_with(PAGE_CAP | RESUME_CAP, Some(Resumption { token: TOKEN, taken: None })), 0);
    typing
}

/// The cursor record acknowledging `acked`.
fn acknowledge(typing: &mut Typing, acked: u16) {
    let [high, low] = acked.to_be_bytes();
    typing.cursor(PAGE.channel, [0x02, 0x00, high, low, 255, 0, 0, 0]);
}

/// REQ-RCH-013, REQ-INT-125: after a lost connection the next asks to
/// resume, with the token and the last acknowledgement; resumed, what the
/// channel had not taken goes again first, in order, then RESIZE, the
/// focus and PAGE, before anything new. On the code before, a reconnection
/// was always a new session and the unacknowledged key was lost.
#[test]
fn a_resumed_session_sends_again_what_the_channel_had_not_taken() {
    let mut typing = resumable();
    typing.input_intent(&key('a'), MS);
    typing.input_intent(&key('b'), 2 * MS);
    acknowledge(&mut typing, 5);
    typing.input(PAGE.channel, false, true, 3 * MS);
    assert_eq!(items(&typing.input_intent(&key('c'), 4 * MS)), [] as [String; 0], "nothing while the connection is down");
    let hello = typing.input(PAGE.channel, true, true, 5 * MS);
    let offered = OFFERED | RESUME_CAP;
    let resume = format!("Event(Resume(Resume {{ token: {TOKEN:?}, acked: 5 }}))");
    assert_eq!(items(&hello), [format!("Event(Hello {{ version: 2, caps: {offered}, agent: None }})"), resume], "HELLO, then RESUME, in one message");
    let resumed = typing.control(PAGE.channel, &welcome_with(PAGE_CAP | RESUME_CAP, Some(Resumption { token: NEXT_TOKEN, taken: Some(5) })), 6 * MS);
    let key_b = KEY_A.replace("'a'", "'b'").replace("\"a\"", "\"b\"");
    assert_eq!(
        items(&resumed),
        [key_b, "Event(Resize { columns: 40, rows: 25, width: 0, height: 0 })".to_owned(), "Event(Page(512))".to_owned(), "Event(FocusIn)".to_owned()]
    );
    assert_eq!(items(&typing.input_intent(&key('a'), 7 * MS)), [KEY_A], "then input goes on");
    // The next connection resumes from the new token.
    typing.input(PAGE.channel, false, true, 8 * MS);
    let again = items(&typing.input(PAGE.channel, true, true, 9 * MS));
    assert!(again[1].contains(&format!("{NEXT_TOKEN:?}")), "{again:?}");
}

/// REQ-INT-125: a channel that no longer holds the session answers new:
/// the session begins again from HELLO, nothing of the old sent again.
#[test]
fn a_session_not_held_begins_again() {
    let mut typing = resumable();
    typing.input_intent(&key('a'), MS);
    typing.input(PAGE.channel, false, true, 2 * MS);
    typing.input(PAGE.channel, true, true, 3 * MS);
    let fresh = typing.control(PAGE.channel, &welcome_with(PAGE_CAP | RESUME_CAP, Some(Resumption { token: NEXT_TOKEN, taken: None })), 4 * MS);
    assert_eq!(items(&fresh), ["Event(Resize { columns: 40, rows: 25, width: 0, height: 0 })", "Event(Page(512))", "Event(FocusIn)"]);
}

/// REQ-INT-106: a channel that did not set RESUME, and a channel the
/// viewer left, begin a new session at the next connection: HELLO alone.
#[test]
fn without_resume_or_after_leaving_a_reconnection_is_new() {
    let mut plain = ready(PAGE_CAP);
    plain.input(PAGE.channel, false, false, MS);
    assert_eq!(items(&plain.input(PAGE.channel, true, false, 2 * MS)).len(), 1, "HELLO alone");
    let mut left = resumable();
    left.channel_gone(PAGE.channel);
    assert_eq!(items(&left.input(PAGE.channel, true, false, MS)).len(), 1, "forgotten: HELLO alone");
}

/// REQ-INT-125: a resumption from a count the receiver never sent up to
/// is refused, and the session is forgotten: no input goes on it.
#[test]
fn a_false_count_is_refused() {
    let mut typing = resumable();
    typing.input_intent(&key('a'), MS);
    typing.input(PAGE.channel, false, true, 2 * MS);
    typing.input(PAGE.channel, true, true, 3 * MS);
    let refused = typing.control(PAGE.channel, &welcome_with(PAGE_CAP | RESUME_CAP, Some(Resumption { token: NEXT_TOKEN, taken: Some(900) })), 4 * MS);
    assert_eq!(items(&refused), [] as [String; 0]);
    assert_eq!(items(&typing.input_intent(&key('b'), 5 * MS)), [] as [String; 0]);
}
