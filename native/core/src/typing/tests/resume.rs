//! Tests of resuming a lost connection's session in the trusted core
//! (DOC-1923, REQ-INT-125; `RETURN-CHANNEL.md` REQ-RCH-013).

use super::*;
use neoteletext_input::caps::RESUME_CAP;
use neoteletext_input::control::Resumption;

const TOKEN: [u8; neoteletext_input::TOKEN_BYTES] = [7; neoteletext_input::TOKEN_BYTES];
const NEXT_TOKEN: [u8; neoteletext_input::TOKEN_BYTES] = [9; neoteletext_input::TOKEN_BYTES];

/// A session whose channel set RESUME, its start sent, its page with no
/// table: no input was ever given, so no `RULE_EVENT` was sent (RULE-50).
fn resumable_plain() -> Typing {
    let mut typing = Typing::default();
    typing.input(PAGE.channel, true, true, 0);
    typing.opened(Some(PAGE.channel), Some(PAGE), 0);
    typing.control(PAGE.channel, &welcome_with(SELECT_CAP | RESUME_CAP, Some(Resumption { token: TOKEN, taken: None })), 0);
    typing.drawn(PAGE, 0);
    typing
}

/// The same, with a table that sends everything: the focus it is given is
/// reported, a `RULE_EVENT`.
fn resumable() -> Typing {
    let mut typing = resumable_plain();
    let focus = typing.rules_told(PAGE.channel, &send_all(), 0).0;
    reported(&mut typing, &focus, 0);
    typing
}

/// The cursor record acknowledging `acked`.
fn acknowledge(typing: &mut Typing, acked: u16) {
    let [high, low] = acked.to_be_bytes();
    typing.cursor(PAGE.channel, [0x02, 0x00, high, low, 255, 0, 0, 0]);
}

/// REQ-RCH-013, REQ-INT-125: after a lost connection of a session that
/// sent no input, the next asks to resume, with the token and the last
/// acknowledgement; resumed, RESIZE and SELECT go before anything new.
#[test]
fn a_session_that_sent_no_input_is_resumed() {
    let mut typing = resumable_plain();
    typing.input(PAGE.channel, false, true, 3 * MS);
    let hello = typing.input(PAGE.channel, true, true, 5 * MS);
    let offered = OFFERED | RESUME_CAP;
    let resume = format!("Event(Resume(Resume {{ token: {TOKEN:?}, acked: 0 }}))");
    assert_eq!(items(&hello), [format!("Event(Hello {{ version: {VERSION}, caps: {offered}, agent: None }})"), resume], "HELLO, then RESUME, in one message");
    let resumed = typing.control(PAGE.channel, &welcome_with(SELECT_CAP | RESUME_CAP, Some(Resumption { token: NEXT_TOKEN, taken: Some(2) })), 6 * MS);
    assert_eq!(
        items(&resumed),
        ["Event(Resize { columns: 40, rows: 25, width: 0, height: 0 })".to_owned(), "Event(Select(Select { page: 512, sub: 16255 }, \"\"))".to_owned()]
    );
}

/// RULE-46, RULE-111 case 2 (REQ-6534, REQ-6615): a `RULE_EVENT` not
/// acknowledged when its connection ends is discarded and never sent
/// again: the session that gave a page's table input is not resumed, so
/// the next connection begins a new one, from HELLO alone, and the page is
/// a new selection. The decoder is told the connection ended. On the code
/// before, the session was resumed and the unacknowledged key sent again.
#[test]
fn an_unacknowledged_report_is_never_sent_again() {
    let mut typing = resumable();
    typed(&mut typing, &key('a'), MS);
    typed(&mut typing, &key('b'), 2 * MS);
    acknowledge(&mut typing, 4);
    let ended = typing.input(PAGE.channel, false, true, 3 * MS);
    assert!(ended.contains(&Send::ConnectionEnded { channel: PAGE.channel }), "{ended:?}");
    assert_eq!(items(&typing.input_intent(&key('c'), 4 * MS)), [] as [String; 0], "nothing while the connection is down");
    let hello = typing.input(PAGE.channel, true, true, 5 * MS);
    assert_eq!(items(&hello).len(), 1, "HELLO alone: {hello:?}");
    let fresh = typing.control(PAGE.channel, &welcome_with(SELECT_CAP | RESUME_CAP, Some(Resumption { token: NEXT_TOKEN, taken: None })), 6 * MS);
    let sent = items(&fresh);
    assert!(!sent.iter().any(|item| item.contains("RuleEvent")), "no report is sent again: {sent:?}");
    assert_eq!(items(&typing.input_intent(&key('d'), 7 * MS)), [] as [String; 0], "no input until the decoder tells its table afresh");
}

/// REQ-INT-125: a channel that no longer holds the session answers new:
/// the session begins again from HELLO, nothing of the old sent again.
#[test]
fn a_session_not_held_begins_again() {
    let mut typing = resumable_plain();
    typing.input(PAGE.channel, false, true, 2 * MS);
    typing.input(PAGE.channel, true, true, 3 * MS);
    let fresh = typing.control(PAGE.channel, &welcome_with(SELECT_CAP | RESUME_CAP, Some(Resumption { token: NEXT_TOKEN, taken: None })), 4 * MS);
    assert_eq!(items(&fresh), ["Event(Resize { columns: 40, rows: 25, width: 0, height: 0 })", "Event(Select(Select { page: 512, sub: 16255 }, \"\"))"]);
}

/// REQ-INT-106: a channel that did not set RESUME, and a channel the
/// viewer left, begin a new session at the next connection: HELLO alone.
#[test]
fn without_resume_or_after_leaving_a_reconnection_is_new() {
    let mut plain = ready(SELECT_CAP);
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
    typed(&mut typing, &key('a'), MS);
    typing.input(PAGE.channel, false, true, 2 * MS);
    typing.input(PAGE.channel, true, true, 3 * MS);
    let refused = typing.control(PAGE.channel, &welcome_with(SELECT_CAP | RESUME_CAP, Some(Resumption { token: NEXT_TOKEN, taken: Some(900) })), 4 * MS);
    assert_eq!(items(&refused), [] as [String; 0]);
    assert_eq!(items(&typing.input_intent(&key('b'), 5 * MS)), [] as [String; 0]);
}

/// DEC-6358, fault 9: a channel whose decoder is put to sleep while its
/// tab stays keeps the tab's selection, where a channel that goes forgets
/// everything; its session is forgotten either way (REQ-INT-125).
#[test]
fn a_channel_asleep_keeps_its_tabs_selection() {
    let mut asleep = resumable();
    let _ = asleep.select(PAGE.channel, PAGE.page, 3, "", MS);
    asleep.channel_asleep(PAGE.channel);
    assert_eq!(asleep.selection(PAGE.channel), Some((PAGE.page, 3)), "the sub-page the tab showed");
    assert_eq!(items(&asleep.input(PAGE.channel, true, false, 2 * MS)).len(), 1, "a new session: HELLO alone");
    let mut gone = resumable();
    let _ = gone.select(PAGE.channel, PAGE.page, 3, "", MS);
    gone.channel_gone(PAGE.channel);
    assert_eq!(gone.selection(PAGE.channel), None, "a channel no tab shows keeps nothing");
}
