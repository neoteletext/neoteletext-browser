//! Every operation of the shared core at its bounds (script-api.md
//! sections 10.1, 10.2, 10.5 and 10.6): each refusal is a `RangeError`
//! whose message begins with the bound's name, or a `TypeError`, and
//! leaves the host exactly as it was.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, reason = "tests")]

use super::*;
use crate::link::{Claims, PointerClaim, Renderer};
use crate::random::SEED_BYTES;

const CHANNEL: &str = "https://example.test/chan/";
const PAGE: i32 = 2;
const CHANNEL_CONTEXT: i32 = 1;

fn opening(context: ContextId) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer: Some(Renderer::Regis),
        page: 0x1A0,
        subcode: Some(2),
        claims: Claims { keys: vec!["Enter".into()], text: true, pointer: PointerClaim::Press, wheel: false, tick_divisor: 1, network: false },
        language: "en".into(),
        channel_url: CHANNEL.into(),
        page_url: format!("{CHANNEL}1A0.tti"),
        script_url: format!("{CHANNEL}page.js"),
        channel_title: "Example".into(),
        seed: [9; SEED_BYTES],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 600,
        sounds: vec![1, 7],
        sound_allowed: true,
    }
}

/// A host with both contexts open, at tick 0 of a step, in a callback of the page context.
fn host() -> Host {
    let mut host = Host::new();
    host.open(opening(ContextId::Channel), 100);
    host.open(opening(ContextId::Page), 200);
    host.begin_step(0, true);
    host.begin_callback(ContextId::Page, Some(EventKind::TICK), 0);
    host
}

fn call(host: &mut Host, op: Op, ints: &[i32]) -> Reply {
    host.call(PAGE, op.number(), ints, Payload::None)
}

fn call_text(host: &mut Host, op: Op, ints: &[i32], text: &str) -> Reply {
    host.call(PAGE, op.number(), ints, Payload::Text(text.as_bytes()))
}

/// Everything a script could tell of the host's state, and the bytes it holds.
fn fingerprint(host: &mut Host) -> String {
    let page = host.budget(ContextId::Page);
    let channel = host.budget(ContextId::Channel);
    let keys = host.call(PAGE, Op::SHARED_KEYS.number(), &[], Payload::None);
    format!("{page}{channel}{keys:?}{}:{}:{:?}:{:?}", host.state_bytes(), host.batch_bytes(), host.title, host.navigate)
}

/// Asserts `reply` is a `RangeError` naming `bound`.
#[track_caller]
fn assert_range(reply: &Reply, bound: &str) {
    match reply {
        Reply::Range(message) => assert!(message.starts_with(&format!("{bound}: ")), "the RangeError does not begin with {bound}: {message}"),
        other => panic!("expected a RangeError naming {bound}, got {other:?}"),
    }
}

/// Asserts that `refused`, called on a host, answers a `RangeError`
/// naming `bound` and changes nothing.
#[track_caller]
fn refuses(host: &mut Host, bound: &str, refused: impl FnOnce(&mut Host) -> Reply) {
    let before = fingerprint(host);
    let reply = refused(host);
    assert_range(&reply, bound);
    assert_eq!(fingerprint(host), before, "a refused call changed the host");
}

/// Asserts that `refused` answers a `TypeError` and changes nothing.
#[track_caller]
fn mistypes(host: &mut Host, refused: impl FnOnce(&mut Host) -> Reply) {
    let before = fingerprint(host);
    let reply = refused(host);
    assert!(matches!(reply, Reply::Type(_)), "expected a TypeError, got {reply:?}");
    assert_eq!(fingerprint(host), before, "a refused call changed the host");
}

#[test]
fn a_call_is_refused_for_an_unknown_operation_context_or_shape() {
    let mut host = host();
    mistypes(&mut host, |host| host.call(PAGE, 9999, &[], Payload::None));
    assert!(matches!(host.call(3, Op::NOW.number(), &[], Payload::None), Reply::Type(_)));
    assert!(matches!(host.call(0, Op::NOW.number(), &[], Payload::None), Reply::Type(_)));
    // Each operation takes exactly its whole numbers and its payload.
    mistypes(&mut host, |host| call(host, Op::TIMER, &[1]));
    mistypes(&mut host, |host| call(host, Op::TIMER, &[1, 0, 0]));
    mistypes(&mut host, |host| call_text(host, Op::TIMER, &[1, 0], "x"));
    mistypes(&mut host, |host| call(host, Op::LOG, &[1]));
    mistypes(&mut host, |host| host.call(PAGE, Op::LOG.number(), &[1], Payload::Bytes(b"x")));
    // A string that is not well-formed Unicode is not taken.
    mistypes(&mut host, |host| host.call(PAGE, Op::LOG.number(), &[1], Payload::Text(&[0xFF, 0xFE])));
    // A context that is not open has no host.
    host.close(ContextId::Page);
    assert!(matches!(call(&mut host, Op::NOW, &[]), Reply::Type(_)));
}

#[test]
fn log_keeps_bounded_lines_and_counts_the_rest() {
    let mut host = host();
    refuses(&mut host, "MAX_LOG_LEVEL", |host| call_text(host, Op::LOG, &[5], "x"));
    refuses(&mut host, "MAX_LOG_LEVEL", |host| call_text(host, Op::LOG, &[-1], "x"));
    let long = "é".repeat(MAX_LOG_CHARS + 50);
    assert_eq!(call_text(&mut host, Op::LOG, &[3], &format!("\u{7}\n{long}")), Reply::Undefined);
    for _ in 0..MAX_LOG_LINES + 9 {
        assert_eq!(call_text(&mut host, Op::LOG, &[1], "more"), Reply::Undefined);
    }
    let batch = host.take_batch(1, Vec::new());
    assert_eq!(batch.log.len(), MAX_LOG_LINES);
    assert_eq!(batch.log_dropped, 10);
    assert_eq!(batch.log[0].text.chars().count(), MAX_LOG_CHARS);
    assert_eq!((batch.log[0].level, batch.log[0].context), (3, Some(ContextId::Page)));
    assert!(batch.log.iter().all(|line| !line.text.chars().any(char::is_control)));
}

#[test]
fn timers_are_bounded_in_delay_and_number() {
    let mut host = host();
    refuses(&mut host, "MAX_TIMER_TICKS", |host| call(host, Op::TIMER, &[0, 0]));
    refuses(&mut host, "MAX_TIMER_TICKS", |host| call(host, Op::TIMER, &[-5, 0]));
    refuses(&mut host, "MAX_TIMER_TICKS", |host| call(host, Op::TIMER, &[MAX_TIMER_TICKS + 1, 0]));
    mistypes(&mut host, |host| call(host, Op::TIMER, &[1, 2]));
    let mut ids = Vec::new();
    for _ in 0..MAX_TIMERS {
        let Reply::Int(id) = call(&mut host, Op::TIMER, &[MAX_TIMER_TICKS, 1]) else { panic!("no timer") };
        assert!(!ids.contains(&id));
        ids.push(id);
    }
    refuses(&mut host, "MAX_TIMERS", |host| call(host, Op::TIMER, &[1, 0]));
    // The channel context's table is its own.
    assert!(matches!(host.call(CHANNEL_CONTEXT, Op::TIMER.number(), &[1, 0], Payload::None), Reply::Int(_)));
    // Cancelling makes room; cancelling a number that names nothing is nothing.
    assert_eq!(call(&mut host, Op::CANCEL, &[ids[0]]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::CANCEL, &[123_456]), Reply::Undefined);
    assert!(matches!(call(&mut host, Op::TIMER, &[1, 0]), Reply::Int(_)));
}

#[test]
fn timers_fire_in_order_of_due_tick_then_creation_and_repeat_once() {
    let mut host = host();
    let id = |reply: Reply| match reply {
        Reply::Int(id) => id,
        other => panic!("{other:?}"),
    };
    let late = id(call(&mut host, Op::TIMER, &[3, 0]));
    let first = id(call(&mut host, Op::TIMER, &[2, 0]));
    let second = id(call(&mut host, Op::TIMER, &[2, 0]));
    let every = id(call(&mut host, Op::TIMER, &[1, 1]));
    assert_eq!(host.next_due(ContextId::Page), Some(1));
    host.begin_step(1, true);
    assert_eq!(host.due_timers(ContextId::Page), vec![every]);
    assert!(host.fire_timer(ContextId::Page, every));
    // Five ticks later the repeating timer is due once, not five times,
    // and the others in order of due tick and then of making.
    host.begin_step(6, true);
    assert_eq!(host.due_timers(ContextId::Page), vec![first, second, every, late]);
    for timer in [first, second, every, late] {
        assert!(host.fire_timer(ContextId::Page, timer));
    }
    assert_eq!(host.due_timers(ContextId::Page), Vec::<i32>::new());
    assert!(!host.fire_timer(ContextId::Page, first), "a one-shot timer fired twice");
    assert_eq!(host.next_due(ContextId::Page), Some(7));
}

#[test]
fn key_and_pointer_state_follow_the_claimed_events_only() {
    let mut host = host();
    refuses(&mut host, "MAX_KEY_NAME_BYTES", |host| call_text(host, Op::KEY_STATE, &[0], &"k".repeat(MAX_KEY_NAME_BYTES + 1)));
    mistypes(&mut host, |host| call_text(host, Op::KEY_STATE, &[2], "Enter"));
    let key = |kind, name: &str, bits| StepEvent { kind, ints: [bits, 0, 0, 0], payload: EventPayload::Text(name.into()) };
    assert!(host.take_in(ContextId::Page, &key(EventKind::KEY_DOWN, "Enter", 0)));
    // A key the manifest did not claim is never given, and is never down.
    assert!(!host.take_in(ContextId::Page, &key(EventKind::KEY_DOWN, "q", 0)));
    assert_eq!(call_text(&mut host, Op::KEY_STATE, &[0], "Enter"), Reply::Int(1));
    assert_eq!(call_text(&mut host, Op::KEY_STATE, &[1], "Enter"), Reply::Int(1));
    assert_eq!(call_text(&mut host, Op::KEY_STATE, &[0], "q"), Reply::Int(0));
    // Pressed is of this step alone; down lasts until the release.
    host.begin_step(1, true);
    assert_eq!(call_text(&mut host, Op::KEY_STATE, &[1], "Enter"), Reply::Int(0));
    assert_eq!(call_text(&mut host, Op::KEY_STATE, &[0], "Enter"), Reply::Int(1));
    assert!(host.take_in(ContextId::Page, &key(EventKind::KEY_UP, "Enter", 0)));
    assert_eq!(call_text(&mut host, Op::KEY_STATE, &[0], "Enter"), Reply::Int(0));
    // The pointer: a press is claimed, a move is not (the manifest says "press").
    let pointer = |action: PointerAction, x, y, handle| StepEvent {
        kind: EventKind::POINTER,
        ints: [action.number(), x, y, handle],
        payload: EventPayload::Ints(vec![0, 1, 0, 0]),
    };
    assert!(host.take_in(ContextId::Page, &pointer(PointerAction::DOWN, 30, 40, 0)));
    assert!(!host.take_in(ContextId::Page, &pointer(PointerAction::MOVE, 31, 41, 0)));
    assert_eq!(call(&mut host, Op::POINTER_STATE, &[]), Reply::Text("30,40,1,1,0".into()));
    // Text was claimed; the wheel was not.
    assert!(host.take_in(ContextId::Page, &StepEvent { kind: EventKind::TEXT, ints: [0; 4], payload: EventPayload::Text("hi".into()) }));
    assert!(!host.take_in(ContextId::Page, &StepEvent { kind: EventKind::WHEEL, ints: [0; 4], payload: EventPayload::None }));
}

#[test]
fn the_title_is_the_page_contexts_and_bounded() {
    let mut host = host();
    refuses(&mut host, "MAX_TITLE_BYTES", |host| call_text(host, Op::TITLE, &[], &"t".repeat(MAX_TITLE_BYTES + 1)));
    mistypes(&mut host, |host| call_text(host, Op::TITLE, &[], "two\nlines"));
    mistypes(&mut host, |host| host.call(CHANNEL_CONTEXT, Op::TITLE.number(), &[], Payload::Text(b"mine")));
    assert_eq!(call_text(&mut host, Op::TITLE, &[], "First"), Reply::Undefined);
    assert_eq!(call_text(&mut host, Op::TITLE, &[], "Scores"), Reply::Undefined);
    assert_eq!(call(&mut host, Op::INFO, &[Info::PAGE_TITLE.number()]), Reply::Text("Scores".into()));
    assert_eq!(host.take_batch(1, Vec::new()).title.as_deref(), Some("Scores"));
}

/// A navigation refused for want of activation: a `TypeError` that stages nothing.
#[track_caller]
fn refused_without_activation(host: &mut Host, target: &str) {
    assert!(matches!(call_text(host, Op::NAVIGATE, &[], target), Reply::Type(_)));
    assert_eq!(host.navigate, None);
}

#[test]
fn navigation_is_rate_limited_and_leaving_the_channel_needs_activation() {
    let mut host = host();
    refuses(&mut host, "MAX_URL_BYTES", |host| call_text(host, Op::NAVIGATE, &[], &format!("https://example.test/{}", "a".repeat(MAX_URL_BYTES))));
    // Not an address and not a page: a TypeError; the try is still
    // counted (rule 6), so the third in a second is refused by the rate.
    assert!(matches!(call_text(&mut host, Op::NAVIGATE, &[], "ftp://example.test/x"), Reply::Type(_)));
    assert!(matches!(call_text(&mut host, Op::NAVIGATE, &[], "9FF"), Reply::Type(_)));
    refuses(&mut host, "MAX_NAVIGATIONS_PER_SECOND", |host| call_text(host, Op::NAVIGATE, &[], "100"));
    // A second later, inside the channel: by page and sub-page, by address.
    host.begin_step(60, true);
    assert_eq!(call_text(&mut host, Op::NAVIGATE, &[], "1A0/0002"), Reply::Undefined);
    assert_eq!(host.take_batch(1, Vec::new()).navigate.as_deref(), Some("1A0/0002"));
    assert_eq!(call_text(&mut host, Op::NAVIGATE, &[], &format!("{CHANNEL}200.tti")), Reply::Undefined);
    let _ = host.take_batch(2, Vec::new());
    // Another channel: only while a key press, a pointer release or a
    // component's activation is being handled.
    host.begin_step(120, true);
    host.begin_callback(ContextId::Page, Some(EventKind::TICK), 0);
    refused_without_activation(&mut host, "https://elsewhere.test/");
    host.begin_callback(ContextId::Page, Some(EventKind::KEY_DOWN), 0);
    assert_eq!(call_text(&mut host, Op::NAVIGATE, &[], "https://elsewhere.test/"), Reply::Undefined);
    assert_eq!(host.take_batch(3, Vec::new()).navigate.as_deref(), Some("https://elsewhere.test/"));
    host.begin_step(180, true);
    host.begin_callback(ContextId::Page, Some(EventKind::POINTER), PointerAction::DOWN.number());
    refused_without_activation(&mut host, "https://elsewhere.test/");
    host.begin_callback(ContextId::Page, Some(EventKind::POINTER), PointerAction::UP.number());
    assert_eq!(call_text(&mut host, Op::NAVIGATE, &[], "https://elsewhere.test/"), Reply::Undefined);
}

#[test]
fn scopes_nest_to_a_bound_and_end_with_their_timers() {
    let mut host = host();
    let begin = |host: &mut Host| match call(host, Op::SCOPE_BEGIN, &[]) {
        Reply::Int(id) => id,
        other => panic!("{other:?}"),
    };
    let mut open = Vec::new();
    for _ in 0..MAX_SCOPE_DEPTH {
        open.push(begin(&mut host));
    }
    refuses(&mut host, "MAX_SCOPE_DEPTH", |host| call(host, Op::SCOPE_BEGIN, &[]));
    // A timer made now belongs to the innermost scope.
    let Reply::Int(timer) = call(&mut host, Op::TIMER, &[1, 1]) else { panic!("no timer") };
    let innermost = *open.last().unwrap();
    mistypes(&mut host, |host| call(host, Op::SCOPE_LEAVE, &[open[0]]));
    mistypes(&mut host, |host| call(host, Op::SCOPE_LEAVE, &[9999]));
    mistypes(&mut host, |host| call(host, Op::SCOPE_END, &[9999]));
    for scope in open.iter().rev() {
        assert_eq!(call(&mut host, Op::SCOPE_LEAVE, &[*scope]), Reply::Undefined);
    }
    // Left scopes still exist until they are ended, up to the bound.
    for _ in MAX_SCOPE_DEPTH..MAX_SCOPES {
        let scope = begin(&mut host);
        assert_eq!(call(&mut host, Op::SCOPE_LEAVE, &[scope]), Reply::Undefined);
    }
    refuses(&mut host, "MAX_SCOPES", |host| call(host, Op::SCOPE_BEGIN, &[]));
    host.begin_step(1, true);
    assert_eq!(host.due_timers(ContextId::Page), vec![timer]);
    assert_eq!(call(&mut host, Op::SCOPE_END, &[innermost]), Reply::Undefined);
    assert_eq!(host.due_timers(ContextId::Page), Vec::<i32>::new(), "ending the scope left its timer");
    mistypes(&mut host, |host| call(host, Op::SCOPE_END, &[innermost]));
    assert!(matches!(call(&mut host, Op::SCOPE_BEGIN, &[]), Reply::Int(_)));
}

#[test]
fn the_shared_store_is_bounded_and_tells_the_other_context() {
    let mut host = host();
    let set = |host: &mut Host, key: &str, value: &str| call_text(host, Op::SHARED_SET, &[i32::try_from(key.len()).unwrap()], &format!("{key}{value}"));
    refuses(&mut host, "MAX_SHARED_KEY_BYTES", |host| set(host, &"k".repeat(MAX_SHARED_KEY_BYTES + 1), "1"));
    refuses(&mut host, "MAX_SHARED_VALUE_BYTES", |host| set(host, "k", &"1".repeat(MAX_SHARED_VALUE_BYTES + 1)));
    refuses(&mut host, "MAX_SHARED_KEY_BYTES", |host| call_text(host, Op::SHARED_GET, &[], &"k".repeat(MAX_SHARED_KEY_BYTES + 1)));
    // The key's length must fall between two characters of the payload.
    mistypes(&mut host, |host| call_text(host, Op::SHARED_SET, &[1], "é1"));
    mistypes(&mut host, |host| call_text(host, Op::SHARED_SET, &[9], "k1"));
    assert_eq!(set(&mut host, "score", "{\"n\":1}"), Reply::Undefined);
    assert_eq!(set(&mut host, "score", "{\"n\":2}"), Reply::Undefined);
    assert_eq!(call_text(&mut host, Op::SHARED_GET, &[], "score"), Reply::Text("{\"n\":2}".into()));
    assert_eq!(call_text(&mut host, Op::SHARED_GET, &[], "none"), Reply::Undefined);
    assert_eq!(host.call(CHANNEL_CONTEXT, Op::SHARED_GET.number(), &[], Payload::Text(b"score")), Reply::Text("{\"n\":2}".into()));
    // The other context is told once of the key, however often it changed.
    assert_eq!(host.take_shared_notice(), Some((ContextId::Channel, "score".into())));
    assert_eq!(host.take_shared_notice(), None);
    // As many bytes as the store may hold, then no more.
    let big = "1".repeat(MAX_SHARED_VALUE_BYTES);
    let mut filled = 0;
    while matches!(set(&mut host, &format!("b{filled}"), &big), Reply::Undefined) {
        filled += 1;
        assert!(filled <= MAX_SHARED_BYTES / MAX_SHARED_VALUE_BYTES);
    }
    refuses(&mut host, "MAX_SHARED_BYTES", |host| set(host, "last", &big));
    for index in 0..filled {
        assert_eq!(call_text(&mut host, Op::SHARED_DELETE, &[], &format!("b{index}")), Reply::Undefined);
    }
    // As many keys as it may hold, then no more.
    let mut made = 1;
    while matches!(set(&mut host, &format!("k{made}"), "0"), Reply::Undefined) {
        made += 1;
        assert!(made <= MAX_SHARED_KEYS);
    }
    assert_eq!(made, MAX_SHARED_KEYS);
    refuses(&mut host, "MAX_SHARED_KEYS", |host| set(host, "one-more", "0"));
    let Reply::Text(keys) = call(&mut host, Op::SHARED_KEYS, &[]) else { panic!("no keys") };
    assert_eq!(keys.matches("\",\"").count() + 1, MAX_SHARED_KEYS);
    assert!(keys.starts_with("[\"score\","));
    // The store goes with the channel context.
    host.close(ContextId::Channel);
    assert_eq!(call(&mut host, Op::SHARED_KEYS, &[]), Reply::Text("[]".into()));
}

#[test]
fn random_bytes_are_bounded_and_differ() {
    let mut host = host();
    refuses(&mut host, "MAX_RANDOM_BYTES", |host| call(host, Op::RANDOM, &[i32::try_from(MAX_RANDOM_BYTES).unwrap() + 1]));
    refuses(&mut host, "MAX_RANDOM_BYTES", |host| call(host, Op::RANDOM, &[-1]));
    let Reply::Bytes(first) = call(&mut host, Op::RANDOM, &[32]) else { panic!("no bytes") };
    let Reply::Bytes(second) = call(&mut host, Op::RANDOM, &[32]) else { panic!("no bytes") };
    assert_eq!((first.len(), second.len()), (32, 32));
    assert_ne!(first, second);
    assert!(matches!(call(&mut host, Op::RANDOM, &[i32::try_from(MAX_RANDOM_BYTES).unwrap()]), Reply::Bytes(bytes) if bytes.len() == MAX_RANDOM_BYTES));
    assert_eq!(call(&mut host, Op::RANDOM, &[0]), Reply::Bytes(Vec::new()));
}

#[test]
fn the_clock_moves_only_with_the_tick() {
    let mut host = host();
    assert_eq!(call(&mut host, Op::CLOCK, &[]), Reply::Text("1800000000000".into()));
    assert_eq!(call(&mut host, Op::ZONE, &[]), Reply::Int(600));
    assert_eq!(call(&mut host, Op::NOW, &[]), Reply::Int(0));
    host.begin_step(90, true);
    // Ninety ticks are a second and a half, to the tick and no finer.
    assert_eq!(call(&mut host, Op::CLOCK, &[]), Reply::Text("1800000001500".into()));
    assert_eq!(call(&mut host, Op::NOW, &[]), Reply::Int(90));
    assert_eq!(call(&mut host, Op::NOW, &[1]), Reply::Int(0));
    assert_eq!(call(&mut host, Op::INFO, &[Info::ORIGIN.number()]), Reply::Text("1800000000000".into()));
    mistypes(&mut host, |host| call(host, Op::NOW, &[2]));
}

#[test]
fn info_gives_each_context_its_own_page() {
    let mut host = host();
    let info = |host: &mut Host, context: i32, field: Info| host.call(context, Op::INFO.number(), &[field.number()], Payload::None);
    assert_eq!(info(&mut host, PAGE, Info::VERSION), Reply::Int(1));
    assert_eq!(info(&mut host, PAGE, Info::LEVEL), Reply::Int(2));
    assert_eq!(info(&mut host, CHANNEL_CONTEXT, Info::LEVEL), Reply::Int(1));
    assert_eq!(info(&mut host, PAGE, Info::PAGE_NUMBER), Reply::Text("1A0".into()));
    assert_eq!(info(&mut host, PAGE, Info::PAGE_SUBPAGE), Reply::Text("0002".into()));
    assert_eq!(info(&mut host, PAGE, Info::PAGE_RENDERER), Reply::Text("regis".into()));
    assert_eq!(info(&mut host, PAGE, Info::CHANNEL_URL), Reply::Text(CHANNEL.into()));
    // The channel context has no page until it is shown one.
    assert_eq!(info(&mut host, CHANNEL_CONTEXT, Info::PAGE_NUMBER), Reply::Undefined);
    assert_eq!(info(&mut host, CHANNEL_CONTEXT, Info::PAGE_URL), Reply::Undefined);
    let shown = StepEvent { kind: EventKind::PAGE_SHOW, ints: [0x2B0, -1, 2, 0], payload: EventPayload::Text(format!("{CHANNEL}2B0.nap")) };
    assert!(host.take_in(ContextId::Channel, &shown));
    assert!(!host.take_in(ContextId::Page, &shown), "a page context was told of a page shown");
    assert_eq!(info(&mut host, CHANNEL_CONTEXT, Info::PAGE_NUMBER), Reply::Text("2B0".into()));
    assert_eq!(info(&mut host, CHANNEL_CONTEXT, Info::PAGE_SUBPAGE), Reply::Undefined);
    assert_eq!(info(&mut host, CHANNEL_CONTEXT, Info::PAGE_RENDERER), Reply::Text("naplps".into()));
    mistypes(&mut host, |host| call(host, Op::INFO, &[999]));
}

#[test]
fn listening_names_a_real_thing_and_kind() {
    let mut host = host();
    mistypes(&mut host, |host| call(host, Op::LISTEN, &[7, EventKind::TICK.number(), 1]));
    mistypes(&mut host, |host| call(host, Op::LISTEN, &[0, 0, 1]));
    mistypes(&mut host, |host| call(host, Op::LISTEN, &[0, 9999, 1]));
    mistypes(&mut host, |host| call(host, Op::LISTEN, &[0, EventKind::TICK.number(), 2]));
    assert!(!host.listens(ContextId::Page, EventKind::TICK));
    assert_eq!(call(&mut host, Op::LISTEN, &[0, EventKind::TICK.number(), 1]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::LISTEN, &[0, EventKind::TICK.number(), 1]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::LISTEN, &[Handle::SOUND.number(), EventKind::SOUND_ENDED.number(), 1]), Reply::Undefined);
    assert!(host.listens(ContextId::Page, EventKind::TICK) && !host.listens(ContextId::Channel, EventKind::TICK));
    assert!(host.listens_to(ContextId::Page, Handle::SOUND.number(), EventKind::SOUND_ENDED));
    let watch = host.take_batch(1, Vec::new()).watch.unwrap();
    assert_eq!(watch.len(), 2);
    assert_eq!(call(&mut host, Op::LISTEN, &[0, EventKind::TICK.number(), 0]), Reply::Undefined);
    assert!(!host.listens(ContextId::Page, EventKind::TICK));
    // Told again only when it changed.
    assert_eq!(host.take_batch(2, Vec::new()).watch.map(|watch| watch.len()), Some(1));
    assert_eq!(host.take_batch(3, Vec::new()).watch, None);
}

#[test]
fn sound_operations_are_checked() {
    let mut host = host();
    let voices = AUDIO_CHANNELS;
    refuses(&mut host, "MAX_SOUND_NUMBER", |host| call(host, Op::SOUND_PLAY, &[0, 0, 100]));
    refuses(&mut host, "MAX_SOUND_NUMBER", |host| call(host, Op::SOUND_PLAY, &[256, 0, 100]));
    refuses(&mut host, "AUDIO_CHANNELS", |host| call(host, Op::SOUND_PLAY, &[1, voices, 100]));
    refuses(&mut host, "MAX_GAIN", |host| call(host, Op::SOUND_PLAY, &[1, 0, 101]));
    // A sound the channel did not announce is a forged handle.
    mistypes(&mut host, |host| call(host, Op::SOUND_PLAY, &[2, 0, 100]));
    refuses(&mut host, "AUDIO_CHANNELS", |host| call(host, Op::SOUND_STOP, &[-1]));
    refuses(&mut host, "AUDIO_CHANNELS", |host| call(host, Op::SYNTH_START, &[voices, 13, 0, 100]));
    mistypes(&mut host, |host| call(host, Op::SYNTH_START, &[0, 2, 0, 100]));
    refuses(&mut host, "MAX_SYNTH_VARIANT", |host| call(host, Op::SYNTH_START, &[0, 13, 256, 100]));
    mistypes(&mut host, |host| call(host, Op::SYNTH_MIDI, &[0, 0xF0, 0, 0]));
    mistypes(&mut host, |host| call(host, Op::SYNTH_MIDI, &[0, 0xA0, 0, 0]));
    refuses(&mut host, "MAX_MIDI_DATA", |host| call(host, Op::SYNTH_MIDI, &[0, 0x90, 128, 0]));
    refuses(&mut host, "MAX_MIDI_DATA", |host| call(host, Op::SYNTH_MIDI, &[0, 0x90, 60, -1]));
    refuses(&mut host, "MAX_NOTE_TICKS", |host| call(host, Op::SYNTH_NOTE, &[0, 60, 100, 0]));
    refuses(&mut host, "MAX_NOTE_TICKS", |host| call(host, Op::SYNTH_NOTE, &[0, 60, 100, MAX_NOTE_TICKS + 1]));
    assert_eq!(call(&mut host, Op::SOUND_PLAY, &[7, 3, 50]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::SYNTH_START, &[1, 12, 0, 80]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::SYNTH_MIDI, &[1, 0xE0, 0x7F, 0x7F]), Reply::Undefined);
    let batch = host.take_batch(1, Vec::new());
    assert_eq!(
        batch.sound,
        vec![
            SoundOperation::Play { voice: 3, sound: 7, gain: 50 },
            SoundOperation::SynthStart { part: 1, family: 12, variant: 0, gain: 80 },
            SoundOperation::Midi { part: 1, status: 0xE0, data1: 0x7F, data2: 0x7F }
        ]
    );
}

/// The security review's lead (section 3.5): stops count as operations,
/// for the callback, the step and the second. The spike let one callback
/// make 2,000 stops.
#[test]
fn sound_stops_are_counted_for_the_callback_the_step_and_the_second() {
    let mut host = host();
    let per_callback = usize::try_from(MAX_SOUND_OPERATIONS_PER_CALLBACK).unwrap();
    for _ in 0..per_callback {
        assert_eq!(call(&mut host, Op::SOUND_STOP, &[0]), Reply::Undefined);
    }
    refuses(&mut host, "MAX_SOUND_OPERATIONS_PER_CALLBACK", |host| call(host, Op::SOUND_STOP, &[0]));
    // Another callback of the same step: the step's count is used up too.
    host.begin_callback(ContextId::Page, Some(EventKind::TIMER), 0);
    refuses(&mut host, "MAX_SOUND_OPERATIONS_PER_STEP", |host| call(host, Op::SOUND_STOP, &[0]));
    assert_eq!(host.take_batch(1, Vec::new()).sound.len(), per_callback);
    // Step after step, the second's count fills.
    let mut made = per_callback;
    let mut tick = 1;
    let per_second = usize::try_from(MAX_SOUND_OPERATIONS_PER_SECOND).unwrap();
    while made < per_second {
        host.begin_step(tick, true);
        host.begin_callback(ContextId::Page, Some(EventKind::TIMER), 0);
        for _ in 0..per_callback.min(per_second - made) {
            assert_eq!(call(&mut host, Op::SOUND_STOP, &[0]), Reply::Undefined);
            made += 1;
        }
        let _ = host.take_batch(2, Vec::new());
        tick += 1;
    }
    host.begin_step(tick, true);
    host.begin_callback(ContextId::Page, Some(EventKind::TIMER), 0);
    refuses(&mut host, "MAX_SOUND_OPERATIONS_PER_SECOND", |host| call(host, Op::SOUND_STOP, &[0]));
    // A second after the first of them, there is room again.
    host.begin_step(TICKS_PER_SECOND, true);
    host.begin_callback(ContextId::Page, Some(EventKind::TIMER), 0);
    assert_eq!(call(&mut host, Op::SOUND_STOP, &[0]), Reply::Undefined);
}

#[test]
fn a_note_is_released_on_its_tick_and_sound_with_its_context() {
    let mut host = host();
    assert_eq!(call(&mut host, Op::SYNTH_START, &[2, 13, 0, 100]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::SYNTH_NOTE, &[2, 60, 90, 5]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::SOUND_PLAY, &[1, 0, 100]), Reply::Undefined);
    assert_eq!(host.next_due(ContextId::Page), Some(5));
    assert_eq!(host.take_batch(1, Vec::new()).sound.len(), 3);
    host.begin_step(4, true);
    assert_eq!(host.take_batch(2, Vec::new()).sound, Vec::new());
    host.begin_step(5, true);
    assert_eq!(host.take_batch(3, Vec::new()).sound, vec![SoundOperation::Midi { part: 2, status: 0x80, data1: 60, data2: 0 }]);
    // A note still held when its context ends is forgotten with the
    // instrument, and the playing sound stopped.
    host.begin_callback(ContextId::Page, Some(EventKind::TIMER), 0);
    assert_eq!(call(&mut host, Op::SYNTH_NOTE, &[2, 62, 90, 50]), Reply::Undefined);
    let _ = host.take_batch(4, Vec::new());
    host.close(ContextId::Page);
    assert_eq!(host.take_batch(5, Vec::new()).sound, vec![SoundOperation::Stop { voice: 0 }, SoundOperation::SynthStop { part: 2 }]);
    host.begin_step(100, true);
    assert_eq!(host.take_batch(6, Vec::new()).sound, Vec::new());
}

/// Section 3.5 (DEC-7282 item 7): until the viewer has pressed a key or
/// the pointer in the tab, sound does nothing and is logged once.
#[test]
fn sound_does_nothing_until_the_viewer_has_acted() {
    let mut host = Host::new();
    let mut open = opening(ContextId::Page);
    open.sound_allowed = false;
    host.open(open, 10);
    host.begin_step(0, false);
    host.begin_callback(ContextId::Page, Some(EventKind::TICK), 0);
    assert_eq!(call(&mut host, Op::INFO, &[Info::SOUND_ALLOWED.number()]), Reply::Int(0));
    assert_eq!(call(&mut host, Op::SOUND_PLAY, &[1, 0, 100]), Reply::Undefined);
    assert_eq!(call(&mut host, Op::SYNTH_NOTE, &[0, 60, 100, 5]), Reply::Undefined);
    let batch = host.take_batch(1, Vec::new());
    assert_eq!(batch.sound, Vec::new());
    assert_eq!(batch.log.len(), 1, "the refusal is logged once");
    assert_eq!(host.next_due(ContextId::Page), None);
    host.begin_step(1, true);
    assert_eq!(call(&mut host, Op::INFO, &[Info::SOUND_ALLOWED.number()]), Reply::Int(1));
    assert_eq!(call(&mut host, Op::SOUND_PLAY, &[1, 0, 100]), Reply::Undefined);
    assert_eq!(host.take_batch(2, Vec::new()).sound.len(), 1);
}

/// Section 10.1 rule 7 (the security review's finding S-06, TODO-7272):
/// with every table of the shared core filled to its own bound, the bytes
/// the host holds for the container and one step's output are within
/// their named bounds.
#[test]
fn with_every_table_full_the_host_holds_no_more_than_its_bounds() {
    let mut host = host();
    for context in [PAGE, CHANNEL_CONTEXT] {
        for _ in 0..MAX_TIMERS {
            host.call(context, Op::TIMER.number(), &[1, 1], Payload::None);
        }
        for kind in EventKind::ALL {
            host.call(context, Op::LISTEN.number(), &[0, kind.1.number(), 1], Payload::None);
            host.call(context, Op::LISTEN.number(), &[-1, kind.1.number(), 1], Payload::None);
        }
        for _ in 0..MAX_SCOPES {
            if let Reply::Int(scope) = host.call(context, Op::SCOPE_BEGIN.number(), &[], Payload::None) {
                host.call(context, Op::SCOPE_LEAVE.number(), &[scope], Payload::None);
            }
        }
    }
    let value = "1".repeat(MAX_SHARED_VALUE_BYTES);
    for index in 0..MAX_SHARED_KEYS {
        let key = format!("{index:064}");
        host.call(PAGE, Op::SHARED_SET.number(), &[64], Payload::Text(format!("{key}{value}").as_bytes()));
    }
    let line = "\u{10FFFF}".repeat(MAX_LOG_CHARS);
    for _ in 0..MAX_LOG_LINES * 2 {
        call_text(&mut host, Op::LOG, &[1], &line);
    }
    call_text(&mut host, Op::TITLE, &[], &"t".repeat(MAX_TITLE_BYTES));
    host.begin_callback(ContextId::Page, Some(EventKind::KEY_DOWN), 0);
    call_text(&mut host, Op::NAVIGATE, &[], &format!("https://e.test/{}", "a".repeat(MAX_URL_BYTES - 15)));
    assert!(host.navigate.is_some() && host.title.is_some() && host.log.len() == MAX_LOG_LINES);
    for tick in 0..TICKS_PER_SECOND {
        host.begin_callback(ContextId::Page, Some(EventKind::TIMER), 0);
        for _ in 0..MAX_SOUND_OPERATIONS_PER_CALLBACK {
            call(&mut host, Op::SYNTH_NOTE, &[0, 60, 100, MAX_NOTE_TICKS]);
        }
        assert!(host.batch_bytes() <= MAX_BATCH_BYTES, "one step's output is {} bytes", host.batch_bytes());
        assert!(host.state_bytes() <= MAX_HOST_STATE_BYTES, "the host holds {} bytes", host.state_bytes());
        if tick + 1 < TICKS_PER_SECOND {
            // The log, title and navigation stay staged; the sound leaves.
            host.sound.clear();
            host.begin_step(tick + 1, true);
        }
    }
    assert_eq!(host.pending_notes.len(), MAX_PENDING_NOTES);
    assert!(host.state_bytes() > MAX_SHARED_BYTES, "the tables were not filled");
}

/// The same rule, at the bounds themselves: lowered so that the shared
/// core's small tables can reach them, a call that would pass
/// `MAX_HOST_STATE_BYTES` or `MAX_BATCH_BYTES` throws naming it and
/// changes nothing.
#[test]
fn a_call_past_the_host_state_or_batch_bound_is_refused_by_name() {
    let mut host = host();
    let held = host.state_bytes();
    host.set_limits_for_test(held + 600, 300);
    let set = |host: &mut Host, key: &str, value: &str| call_text(host, Op::SHARED_SET, &[i32::try_from(key.len()).unwrap()], &format!("{key}{value}"));
    assert_eq!(set(&mut host, "a", &"1".repeat(100)), Reply::Undefined);
    refuses(&mut host, "MAX_HOST_STATE_BYTES", |host| set(host, "b", &"1".repeat(500)));
    refuses(&mut host, "MAX_BATCH_BYTES", |host| call_text(host, Op::NAVIGATE, &[], &format!("{CHANNEL}{}", "a".repeat(300))));
    let mut host = self::host();
    let held = host.state_bytes();
    host.set_limits_for_test(held + 100, MAX_BATCH_BYTES);
    refuses(&mut host, "MAX_HOST_STATE_BYTES", |host| call_text(host, Op::TITLE, &[], &"t".repeat(MAX_TITLE_BYTES)));
    let mut made = 0;
    while matches!(call(&mut host, Op::TIMER, &[1, 0]), Reply::Int(_)) {
        made += 1;
    }
    assert!(made < MAX_TIMERS);
    // With not a byte of room left, nothing that holds anything is taken.
    let held = host.state_bytes();
    host.set_limits_for_test(held, MAX_BATCH_BYTES);
    refuses(&mut host, "MAX_HOST_STATE_BYTES", |host| call(host, Op::TIMER, &[1, 0]));
    refuses(&mut host, "MAX_HOST_STATE_BYTES", |host| call(host, Op::LISTEN, &[0, EventKind::TICK.number(), 1]));
    refuses(&mut host, "MAX_HOST_STATE_BYTES", |host| call(host, Op::SCOPE_BEGIN, &[]));
    refuses(&mut host, "MAX_HOST_STATE_BYTES", |host| call(host, Op::SOUND_STOP, &[0]));
}

#[test]
fn the_budget_report_names_every_entry_with_its_limit() {
    let mut host = host();
    call(&mut host, Op::TIMER, &[5, 0]);
    let Reply::Text(json) = call(&mut host, Op::BUDGET, &[]) else { panic!("no budget") };
    for entry in [
        "\"scriptBytes\":{\"used\":200,\"limit\":1048576}",
        "\"heapBytes\":{\"used\":0,\"limit\":50331648}",
        "\"timers\":{\"used\":1,\"limit\":32}",
        "\"listeners\":{\"used\":0,\"limit\":256}",
        "\"requests\":{\"used\":0,\"limit\":0}",
        "\"sockets\":{\"used\":0,\"limit\":0}",
        "\"soundOperations\":{\"used\":0,\"limit\":256}",
        "\"callbackPercent\":{\"used\":0,\"limit\":100}",
        "\"callbackPercentMax\":{\"used\":0,\"limit\":100}",
        "\"stepPercentMax\":{\"used\":0,\"limit\":100}",
        "\"secondPercent\":{\"used\":0,\"limit\":100}",
        "\"batchBytes\":{\"used\":0,\"limit\":262144}",
        "\"failures\":{\"used\":0,\"limit\":16}",
    ] {
        assert!(json.contains(entry), "{entry} is not in {json}");
    }
    assert!(json.contains(&format!("\"hostBytes\":{{\"used\":{},\"limit\":8388608}}", host.state_bytes())));
}

#[test]
fn failures_in_a_row_stop_the_context_and_a_success_starts_the_count_again() {
    let mut host = host();
    for _ in 0..MAX_CONSECUTIVE_FAILURES - 1 {
        assert!(!host.count_failure(ContextId::Page, true));
    }
    assert!(!host.count_failure(ContextId::Page, false));
    for _ in 0..MAX_CONSECUTIVE_FAILURES - 1 {
        assert!(!host.count_failure(ContextId::Page, true));
    }
    assert!(host.runs(ContextId::Page));
    assert!(host.count_failure(ContextId::Page, true));
    assert!(!host.runs(ContextId::Page) && host.runs(ContextId::Channel));
    let states = host.take_batch(1, Vec::new()).states;
    assert_eq!(
        states.iter().map(|state| (state.context, state.stopped, state.failures)).collect::<Vec<_>>(),
        vec![(ContextId::Channel, None, 0), (ContextId::Page, Some(StopReason::Failures), 16)]
    );
}
