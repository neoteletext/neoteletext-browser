//! Tests of viewer state in the running browser (neoteletext-common's
//! `docs/viewer-state.md` VS-103 to VS-119).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::arithmetic_side_effects, reason = "a test")]

use super::super::test_support::Test;
use super::*;
use neoteletext_web::viewer_state::{Lifetime, MAX_SIGNED_BYTES, PUBLIC_KEY_BYTES, signed_bytes};

const CHALLENGE: [u8; CHALLENGE_BYTES] = [0x42; CHALLENGE_BYTES];

fn challenge(test: &mut Test, channel: u32, challenge: [u8; CHALLENGE_BYTES], create: bool) -> StateReply {
    let message = FromFetcher::StateChallenged { request: 7, challenge, create };
    let Some(ToFetcher::StateAnswer { request: 7, reply }) = test.core.state_reply(channel, &message).unwrap() else { panic!("no answer") };
    reply
}

fn public_key(reply: StateReply) -> [u8; PUBLIC_KEY_BYTES] {
    match reply {
        StateReply::Proof { public_key, .. } => public_key,
        StateReply::None(reason) => panic!("no proof: {reason:?}"),
    }
}

/// What the core told each fetcher, since the last call.
fn told(test: &mut Test) -> Vec<(u32, ToFetcher)> {
    std::mem::take(&mut test.core.controls.told_fetchers)
}

/// VS-103 and VS-104: a fetcher's challenge is signed with its own tab's
/// channel's key over bytes naming that channel, whatever it asks; a key
/// is made only after the viewer acted on the tab; a channel without a
/// key and a fetcher of no tab are refused.
#[test]
fn a_fetcher_is_answered_for_its_own_tabs_channel_after_a_gesture() {
    let mut test = Test::new("state-own");
    let news = test.tab("https://news.example/teletext/302", false);
    assert_eq!(challenge(&mut test, news, CHALLENGE, true), StateReply::None(NoneReason::NoGesture));
    assert_eq!(challenge(&mut test, news, CHALLENGE, false), StateReply::None(NoneReason::NoKey));
    test.core.viewer_gesture(news);
    let reply = challenge(&mut test, news, CHALLENGE, true);
    let StateReply::Proof { algorithm, lifetime, public_key, signature } = reply else { panic!("a proof") };
    assert_eq!((algorithm, lifetime), (Algorithm::Ed25519, Lifetime::Visit));
    let identity = identity_of("https://news.example/teletext/").unwrap();
    let mut bytes = [0_u8; MAX_SIGNED_BYTES];
    let length = signed_bytes(neoteletext_input::VERSION, Algorithm::Ed25519, Lifetime::Visit, &CHALLENGE, &identity, &mut bytes).unwrap();
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key).verify(&bytes[..length], &signature).unwrap();
    assert_eq!(challenge(&mut test, 999, CHALLENGE, true), StateReply::None(NoneReason::NoKey), "no such tab");
    let file = test.tab("file:///tmp/pages/", false);
    test.core.viewer_gesture(file);
    assert_eq!(challenge(&mut test, file, [1; CHALLENGE_BYTES], true), StateReply::None(NoneReason::NoKey), "VS-210: a file channel has none");
}

/// VS-119: one challenge is signed once on a fetcher, and a fetcher is
/// given at most 60 signatures a minute.
#[test]
fn a_challenge_is_signed_once_and_signatures_are_bounded() {
    let mut test = Test::new("state-bounds");
    let news = test.tab("https://news.example/teletext/", false);
    test.core.viewer_gesture(news);
    public_key(challenge(&mut test, news, CHALLENGE, true));
    assert_eq!(challenge(&mut test, news, CHALLENGE, false), StateReply::None(NoneReason::NoKey), "the same challenge again");
    for index in 1..MOST_PROOFS_PER_FETCHER_PER_MINUTE {
        let mut fresh = [0_u8; CHALLENGE_BYTES];
        fresh[..4].copy_from_slice(&index.to_be_bytes());
        public_key(challenge(&mut test, news, fresh, false));
    }
    assert_eq!(challenge(&mut test, news, [0xEE; CHALLENGE_BYTES], false), StateReply::None(NoneReason::NoKey), "the 61st in a minute");
}

/// VS-111: tabs of one channel share its key, and when a key is made
/// through one, the others offer; a private tab has its own key.
#[test]
fn tabs_of_a_channel_share_its_key_and_learn_of_it() {
    let mut test = Test::new("state-tabs");
    let first = test.tab("https://news.example/teletext/", false);
    let second = test.tab("https://news.example/teletext/302", false);
    let private = test.tab("https://news.example/teletext/", true);
    test.core.viewer_gesture(first);
    let key = public_key(challenge(&mut test, first, CHALLENGE, true));
    assert_eq!(told(&mut test), [(second, ToFetcher::StateSendOffer)], "the other ordinary tab, alone");
    assert_eq!(public_key(challenge(&mut test, second, [2; CHALLENGE_BYTES], false)), key);
    test.core.viewer_gesture(private);
    assert_ne!(public_key(challenge(&mut test, private, [3; CHALLENGE_BYTES], true)), key, "VS-110: the private jar's own");
}

/// VS-112 and VS-113: the list shows the channel without any part of its
/// key; Forget deletes it and each of its connections is told
/// `forgotten`; the next key is new.
#[test]
fn the_list_forgets_and_tells_each_connection() {
    let mut test = Test::new("state-list");
    let first = test.tab("https://news.example/teletext/", false);
    let second = test.tab("https://news.example/teletext/302", false);
    test.core.viewer_gesture(first);
    let key = public_key(challenge(&mut test, first, CHALLENGE, true));
    told(&mut test);
    test.core.known_request(KnownRequest::Panel(true)).unwrap();
    let view = test.last_known();
    assert_eq!(view.rows.len(), 1);
    assert_eq!(view.rows[0].label, "https://news.example/teletext/");
    assert!(view.rows[0].has_key && !view.rows[0].remembered);
    let mut written = Vec::new();
    protocol::write_to_interface(&mut written, &ToInterface::Known(view.clone())).unwrap();
    assert!(!written.windows(PUBLIC_KEY_BYTES).any(|window| window == key), "VS-116: no key reaches the interface");
    test.core.known_request(KnownRequest::Forget(view.rows[0].number)).unwrap();
    let forgotten = ToFetcher::StateSendNone(NoneReason::Forgotten);
    assert_eq!(told(&mut test), [(first, forgotten.clone()), (second, forgotten)]);
    test.core.viewer_gesture(first);
    assert_ne!(public_key(challenge(&mut test, first, [9; CHALLENGE_BYTES], true)), key);
}

/// VS-112: Block forgets the channel, declines its later requests and is
/// kept in the settings; "Never" declines every channel's.
#[test]
fn blocking_and_never_decline() {
    let mut test = Test::new("state-block");
    let news = test.tab("https://news.example/teletext/", false);
    test.core.viewer_gesture(news);
    public_key(challenge(&mut test, news, CHALLENGE, true));
    test.core.known_request(KnownRequest::Panel(true)).unwrap();
    let number = test.last_known().rows[0].number;
    test.core.known_request(KnownRequest::Block { number, blocked: true }).unwrap();
    assert_eq!(test.core.settings.state_block, ["https://news.example/teletext/"]);
    test.core.viewer_gesture(news);
    assert_eq!(challenge(&mut test, news, [5; CHALLENGE_BYTES], true), StateReply::None(NoneReason::Declined));
    let view = test.last_known();
    assert!(view.rows[0].blocked && !view.rows[0].has_key, "listed blocked, to unblock");
    let other = test.tab("https://other.example/", false);
    test.core.channels_may_know_changed(false);
    test.core.viewer_gesture(other);
    assert_eq!(challenge(&mut test, other, [6; CHALLENGE_BYTES], true), StateReply::None(NoneReason::Declined));
}

/// VS-208 and Q15: a channel's offer to remember is shown once, and not
/// at all without the keychain, which leaves "Remember me" unavailable
/// with its reason.
#[test]
fn an_offer_is_shown_only_where_remembering_can_be() {
    let mut test = Test::new("state-offer");
    let news = test.tab("https://news.example/teletext/", false);
    test.core.viewer_gesture(news);
    public_key(challenge(&mut test, news, CHALLENGE, true));
    test.core.state_reply(news, &FromFetcher::StateKeepOffered).unwrap();
    let strip = test.core.strip();
    assert_eq!(strip.known, Known::Visit);
    assert!(strip.can_remember && strip.keep_offer, "offered");
    test.core.known_request(KnownRequest::DismissOffer).unwrap();
    test.core.state_reply(news, &FromFetcher::StateKeepOffered).unwrap();
    assert!(!test.core.strip().keep_offer, "once a run");
    test.core.viewer_state.no_keychain = true;
    test.core.viewer_state_at_start();
    let strip = test.core.strip();
    assert!(!strip.can_remember && !strip.keep_offer);
    test.core.known_request(KnownRequest::Panel(true)).unwrap();
    let view = test.last_known();
    assert!(!view.can_remember && view.notice.contains(NO_KEYCHAIN), "{}", view.notice);
}

/// VS-204: a `STATE_END` deletes the key only when it names it.
#[test]
fn an_end_deletes_only_the_key_it_names() {
    let mut test = Test::new("state-end");
    let news = test.tab("https://news.example/teletext/", false);
    test.core.viewer_gesture(news);
    let key = public_key(challenge(&mut test, news, CHALLENGE, true));
    test.core.state_reply(news, &FromFetcher::StateEndReceived { key_id: [0; 32] }).unwrap();
    assert_eq!(test.core.strip().known, Known::Visit);
    let key_id = neoteletext_web::viewer_state::key_id(Algorithm::Ed25519, &key);
    test.core.state_reply(news, &FromFetcher::StateEndReceived { key_id }).unwrap();
    assert_eq!(test.core.strip().known, Known::Not);
}
