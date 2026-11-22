//! The link (script-api.md section 11.1): one interface for whatever
//! runs a tab's scripts, its in-process implementation, and the bounds a
//! link holds its messages to.
mod common;

use common::{claiming, engine, lines, opening, tick, with_events};
use neoteletext_script::limits::{MAX_KEY_ASKS, MAX_SCRIPT_BYTES, MAX_STEP_EVENTS, MAX_URL_BYTES};
use neoteletext_script::ops::EventKind;
use neoteletext_script::{ContextId, EventPayload, FromFetcher, InProcess, LinkError, ScriptLink, Settings, Step, StepEvent, TimeLimits};

fn link() -> InProcess {
    InProcess::new(engine().clone(), Settings { time: TimeLimits::unhurried(), watchdog: None })
}

/// TODO-7289's "done": a script runs in either context through the link,
/// hears ticks, timers and injected events, and ends when it is closed.
#[test]
fn a_script_runs_in_either_context_through_the_link() {
    // Through the trait alone, as the browser and the runner use it.
    fn drive(link: &mut dyn ScriptLink) -> Vec<String> {
        let mut log = Vec::new();
        assert!(matches!(link.step(tick(0, 0)), Err(LinkError::Context(_))), "a step before any context");
        for context in [ContextId::Channel, ContextId::Page] {
            let mut open = opening(context);
            open.claims = claiming(&["Enter"]);
            let source = "const who = nt.level; nt.addEventListener('tick', (e) => { if (e.tick === 1) console.log(who + ' tick'); });
                          setTimeout(() => console.log(who + ' timer'), 0);
                          nt.addEventListener('keydown', (e) => console.log(who + ' key ' + e.key));
                          nt.addEventListener('close', () => console.log(who + ' close'));";
            link.open(open, source.as_bytes()).unwrap();
        }
        log.extend(lines(&link.step(tick(1, 0)).unwrap()));
        let key = StepEvent { kind: EventKind::KEY_DOWN, ints: [0; 4], payload: EventPayload::Text("Enter".into()) };
        log.extend(lines(&link.step(with_events(2, 1, vec![key])).unwrap()));
        link.network(FromFetcher { context: ContextId::Page, kind: 30, ints: [0; 4], payload: EventPayload::None });
        assert_eq!(link.take_requests(), Vec::new());
        link.close(ContextId::Page);
        log.extend(lines(&link.step(tick(3, 2)).unwrap()));
        link.close(ContextId::Channel);
        assert!(matches!(link.step(tick(4, 3)), Err(LinkError::Context(_))), "a step after every context closed");
        log
    }
    let mut link = link();
    assert_eq!(drive(&mut link), vec!["page key Enter", "channel key Enter", "page timer", "channel timer", "page tick", "channel tick", "page close"]);
    assert!(link.container().is_none(), "the container outlived its contexts");
}

/// Section 11.2: every count and length of a message is held to its
/// bound, and a message that names what only the host makes is refused,
/// before anything runs.
#[test]
fn a_message_past_a_bound_is_refused_before_anything_runs() {
    let mut link = link();
    let refused = |link: &mut InProcess, change: &dyn Fn(&mut neoteletext_script::ContextOpen), source: &[u8]| {
        let mut open = opening(ContextId::Page);
        change(&mut open);
        link.open(open, source)
    };
    assert_eq!(refused(&mut link, &|_| {}, &vec![b' '; MAX_SCRIPT_BYTES + 1]), Err(LinkError::Bound("MAX_SCRIPT_BYTES")));
    assert_eq!(refused(&mut link, &|open| open.claims.keys = vec!["k".into(); MAX_KEY_ASKS + 1], b""), Err(LinkError::Bound("MAX_KEY_ASKS")));
    assert_eq!(refused(&mut link, &|open| open.claims.keys = vec!["k".repeat(33)], b""), Err(LinkError::Bound("MAX_KEY_NAME_BYTES")));
    assert_eq!(refused(&mut link, &|open| open.claims.tick_divisor = 61, b""), Err(LinkError::Bound("MAX_TICK_DIVISOR")));
    assert_eq!(refused(&mut link, &|open| open.language = "x".repeat(17), b""), Err(LinkError::Bound("MAX_LANGUAGE_BYTES")));
    assert_eq!(refused(&mut link, &|open| open.page_url = "x".repeat(MAX_URL_BYTES + 1), b""), Err(LinkError::Bound("MAX_URL_BYTES")));
    assert_eq!(refused(&mut link, &|open| open.channel_title = "x".repeat(65), b""), Err(LinkError::Bound("MAX_TITLE_BYTES")));
    assert!(link.container().is_none(), "a refused opening made a container");
    let mut open = opening(ContextId::Page);
    open.claims = claiming(&[]);
    link.open(open, b"nt.addEventListener('tick', () => console.log('ran'));").unwrap();
    let event = |kind, payload| StepEvent { kind, ints: [0; 4], payload };
    let many = vec![event(EventKind::FOCUS, EventPayload::None); MAX_STEP_EVENTS + 1];
    assert_eq!(link.step(with_events(1, 0, many)).err(), Some(LinkError::Bound("MAX_STEP_EVENTS")));
    assert_eq!(
        link.step(with_events(1, 0, vec![event(EventKind::TEXT, EventPayload::Text("x".repeat(4097)))])).err(),
        Some(LinkError::Bound("MAX_EVENT_PAYLOAD_BYTES"))
    );
    assert_eq!(link.step(Step { painted: vec![0; 33], ..tick(1, 0) }).err(), Some(LinkError::Bound("MAX_STEP_PAINTED")));
    // Events only the host makes, and events without their payload.
    for kind in [EventKind::TICK, EventKind::TIMER, EventKind::CLOSE, EventKind::SHARED, EventKind::VISIBILITY, EventKind::SOUND_ALLOWED, EventKind::PAINTED] {
        assert!(matches!(link.step(with_events(1, 0, vec![event(kind, EventPayload::None)])), Err(LinkError::Refused(_))), "{kind:?}");
    }
    assert!(matches!(link.step(with_events(1, 0, vec![event(EventKind::KEY_DOWN, EventPayload::None)])), Err(LinkError::Refused(_))));
    assert!(matches!(link.step(with_events(1, 0, vec![event(EventKind::POINTER, EventPayload::Ints(vec![0; 3]))])), Err(LinkError::Refused(_))));
    assert!(matches!(link.step(with_events(1, 0, vec![event(EventKind::KEY_DOWN, EventPayload::Text("k".repeat(33)))])), Err(LinkError::Refused(_))));
    // Nothing ran for any refused step: this is the first tick heard.
    assert_eq!(lines(&link.step(tick(2, 0)).unwrap()), vec!["ran"]);
}
