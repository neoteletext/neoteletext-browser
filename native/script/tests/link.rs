//! The link (script-api.md section 11.1): one interface for whatever
//! runs a tab's scripts, its in-process implementation, and the bounds a
//! link holds its messages to, which are the protocol crate's own.
mod common;

use common::{claim, engine, key, lines, opening, plain_event, text_event, tick, with_events};
use neoteletext_script::limits::{MAX_SCRIPT_BYTES, MAX_STEP_EVENTS};
use neoteletext_script::{InProcess, LinkError, ScriptLink, Settings, TimeLimits};
use protocol::script::{ContextId, ContextOpen, EventKind, MAX_KEY_ASKS, Step};
use protocol::web::{FromWebClient, MAX_URL_BYTES};

fn link() -> InProcess {
    InProcess::new(engine().clone(), Settings { time: TimeLimits::unhurried(), watchdog: None })
}

/// An opening whose length is its script's.
fn opening_of(context: ContextId, source: &[u8]) -> ContextOpen {
    ContextOpen { source_bytes: u32::try_from(source.len()).unwrap(), ..opening(context) }
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
            let source = "const who = nt.level; nt.addEventListener('tick', (e) => { if (e.tick === 1) console.log(who + ' tick'); });
                          setTimeout(() => console.log(who + ' timer'), 0);
                          nt.addEventListener('keydown', (e) => console.log(who + ' key ' + e.key));
                          nt.addEventListener('close', () => console.log(who + ' close'));";
            let mut open = opening_of(context, source.as_bytes());
            claim(&mut open, &["Enter"]);
            link.open(open, source.as_bytes()).unwrap();
        }
        log.extend(lines(&link.step(tick(1, 0)).unwrap()));
        log.extend(lines(&link.step(with_events(2, 1, vec![key(EventKind::KeyDown, "Enter", 0)])).unwrap()));
        // An answer from the web client has nothing to hear it until the
        // web APIs are built: it is dropped, and the log says so once.
        link.network(FromWebClient::End(7));
        link.network(FromWebClient::End(8));
        assert_eq!(link.take_requests(), Vec::new());
        link.close(ContextId::Page);
        log.extend(lines(&link.step(tick(3, 2)).unwrap()));
        link.close(ContextId::Channel);
        assert!(matches!(link.step(tick(4, 3)), Err(LinkError::Context(_))), "a step after every context closed");
        log
    }
    let mut link = link();
    assert_eq!(
        drive(&mut link),
        vec![
            "page key Enter",
            "channel key Enter",
            "page timer",
            "channel timer",
            "page tick",
            "channel tick",
            "page close",
            "an answer from the web client was dropped: this script library has no web API yet"
        ]
    );
    assert!(link.container().is_none(), "the container outlived its contexts");
}

/// Section 11.2: the link refuses what the message's own reader refuses,
/// naming the field as that reader does, and a step that carries an event
/// only the host makes, before anything runs.
#[test]
fn a_message_out_of_its_bounds_is_refused_before_anything_runs() {
    let mut link = link();
    let refused = |link: &mut InProcess, change: &dyn Fn(&mut ContextOpen), source: &[u8]| {
        let mut open = opening_of(ContextId::Page, source);
        change(&mut open);
        link.open(open, source)
    };
    let script = b"1";
    assert_eq!(refused(&mut link, &|_| {}, &vec![b' '; MAX_SCRIPT_BYTES + 1]), Err(LinkError::Refused("source length")));
    assert_eq!(refused(&mut link, &|_| {}, b""), Err(LinkError::Refused("source length")));
    // The script given is not the length the opening says follows.
    assert_eq!(refused(&mut link, &|open| open.source_bytes = 2, script), Err(LinkError::Refused("source length")));
    assert_eq!(refused(&mut link, &|open| open.keys = vec!["k".into(); MAX_KEY_ASKS + 1], script), Err(LinkError::Refused("too many keys")));
    assert_eq!(refused(&mut link, &|open| open.keys = vec!["k".repeat(33)], script), Err(LinkError::Refused("key name")));
    assert_eq!(refused(&mut link, &|open| open.tick_divisor = 61, script), Err(LinkError::Refused("tick divisor")));
    assert_eq!(refused(&mut link, &|open| open.language = "x".repeat(17), script), Err(LinkError::Refused("language")));
    assert_eq!(refused(&mut link, &|open| open.page_url = "x".repeat(MAX_URL_BYTES + 1), script), Err(LinkError::Refused("page address")));
    assert_eq!(refused(&mut link, &|open| open.channel_title = "x".repeat(65), script), Err(LinkError::Refused("channel title")));
    assert_eq!(refused(&mut link, &|open| open.page = 0x900, script), Err(LinkError::Refused("page number")));
    assert!(link.container().is_none(), "a refused opening made a container");
    let source = b"nt.addEventListener('tick', () => console.log('ran'));";
    let mut open = opening_of(ContextId::Page, source);
    claim(&mut open, &[]);
    link.open(open, source).unwrap();
    let many = vec![plain_event(EventKind::Focus, [0; 4]); MAX_STEP_EVENTS + 1];
    assert_eq!(link.step(with_events(1, 0, many)).err(), Some(LinkError::Refused("too many events")));
    assert_eq!(link.step(with_events(1, 0, vec![text_event(EventKind::Text, [0; 4], &"x".repeat(4097))])).err(), Some(LinkError::Refused("event payload")));
    assert_eq!(link.step(Step { painted: vec![0; 33], ..tick(1, 0) }).err(), Some(LinkError::Refused("too many painted")));
    // Events only the host makes.
    for kind in [EventKind::Tick, EventKind::Timer, EventKind::Close, EventKind::Shared, EventKind::Visibility, EventKind::Painted, EventKind::SoundCue] {
        assert_eq!(
            link.step(with_events(1, 0, vec![plain_event(kind, [0; 4])])).err(),
            Some(LinkError::Refused("event kind: only the host makes it")),
            "{kind:?}"
        );
    }
    // Events without the payload their kind has: a key with no name or too
    // long a one, a pointer with too few numbers or a part of one, text
    // that is not Unicode, a wheel with anything.
    for event in [
        plain_event(EventKind::KeyDown, [0; 4]),
        text_event(EventKind::KeyDown, [0; 4], &"k".repeat(33)),
        common::ints_event(EventKind::Pointer, [0; 4], &[0; 3]),
        protocol::script::Event { kind: EventKind::Pointer, numbers: [0; 4], payload: vec![0; 17] },
        protocol::script::Event { kind: EventKind::Text, numbers: [0; 4], payload: vec![0xFF] },
        text_event(EventKind::Wheel, [0; 4], "x"),
    ] {
        assert_eq!(link.step(with_events(1, 0, vec![event.clone()])).err(), Some(LinkError::Refused("event payload")), "{event:?}");
    }
    // Nothing ran for any refused step: this is the first tick heard.
    assert_eq!(lines(&link.step(tick(2, 0)).unwrap()), vec!["ran"]);
}
