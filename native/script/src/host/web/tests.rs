//! The web APIs' operations at their bounds and against a web client
//! that breaks the protocol (script-api.md sections 10.1, 10.2 and 11.3):
//! each refusal names its bound or is a `TypeError` and changes nothing;
//! a handle is the calling context's own; and an answer that is not one
//! its request may be given fails that request and nothing else.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, reason = "tests")]

use protocol::script::{ContextFlags, ContextOpen, Level, Renderer};
use protocol::web::Reason;

use super::*;
use crate::random::SEED_BYTES;

const PAGE: i32 = 2;
const CHANNEL: i32 = 1;
/// A request's head: GET of an address, the client's referrer, no integrity, no last event ID, no headers.
const HEAD: &str = "GET\0https://example.test/x\0about:client\0\0\0";
/// The flags of a plain fetch: mode `cors`, credentials `same-origin`.
const PLAIN: i32 = (1 << fetch_flag::MODE.0) | (1 << fetch_flag::CREDENTIALS.0);

fn opening(context: ContextId, network: bool) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer: Renderer::Teletext,
        level: Level::TwoFive,
        page: 0x100,
        subcode: 0,
        flags: ContextFlags { network, ..ContextFlags::default() },
        tick_divisor: 0,
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [9; SEED_BYTES],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: Vec::new(),
        source_bytes: 1,
    }
}

/// A host with both contexts open and the network, at a step's start.
fn host() -> Host {
    let mut host = Host::new();
    host.open(opening(ContextId::Channel, true), 100);
    host.open(opening(ContextId::Page, true), 100);
    host.begin_step(0, false);
    host
}

fn text(host: &mut Host, context: i32, op: Op, ints: &[i32], payload: &str) -> Reply {
    host.call(context, op.number(), ints, Payload::Text(payload.as_bytes()))
}

fn bytes(host: &mut Host, context: i32, op: Op, ints: &[i32], payload: &[u8]) -> Reply {
    host.call(context, op.number(), ints, Payload::Bytes(payload))
}

fn plain(host: &mut Host, context: i32, op: Op, ints: &[i32]) -> Reply {
    host.call(context, op.number(), ints, Payload::None)
}

/// Starts a plain fetch of the page context and gives its number.
fn fetch(host: &mut Host) -> u32 {
    let Reply::Int(id) = text(host, PAGE, Op::FETCH, &[PLAIN, 0], HEAD) else { panic!("the fetch did not start") };
    u32::try_from(id).unwrap()
}

fn head(id: u32) -> FromWebClient {
    head_with(id, 200)
}

/// A head with `status`.
fn head_with(id: u32, status: u16) -> FromWebClient {
    FromWebClient::Head(Box::new(Head {
        id,
        response_type: ResponseType::Basic,
        status,
        status_text: "OK".into(),
        url: "https://example.test/x".into(),
        redirected: false,
        headers: Vec::new(),
    }))
}

/// Every event the host has for its contexts now.
fn events(host: &mut Host) -> Vec<NetworkEvent> {
    core::iter::from_fn(|| host.next_network_event()).collect()
}

fn kinds(events: &[NetworkEvent]) -> Vec<EventKind> {
    events.iter().map(|event| event.kind).collect()
}

#[track_caller]
fn assert_range(reply: &Reply, bound: &str) {
    assert!(matches!(reply, Reply::Range(text) if text.starts_with(bound)), "{reply:?} is not a RangeError naming {bound}");
}

#[track_caller]
fn assert_type(reply: &Reply) {
    assert!(matches!(reply, Reply::Type(_)), "{reply:?} is not a TypeError");
}

/// Section 10.1 rule 3 and section 11.3's `Start`: a request that is not
/// one the web client takes is refused whole, with nothing counted,
/// nothing held and nothing sent.
#[test]
fn a_request_the_web_client_would_refuse_changes_nothing() {
    let mut host = host();
    let before = host.state_bytes();
    for head in [
        "CONNECT\0https://example.test/x\0about:client\0\0\0",
        "TRACE\0https://example.test/x\0about:client\0\0\0",
        "GE T\0https://example.test/x\0about:client\0\0\0",
        "GET\0ftp://example.test/x\0about:client\0\0\0",
        "GET\0https://example.test/x\0about:client\0\0\0Bad Name:v\n",
        "GET\0https://example.test/x\0about:client\0\0\0no colon\n",
        "GET\0https://example.test/x\0about:client\0\0\0A:b\u{20ac}\n",
        "GET\0https://example.test/x\0about:client\0\0id\n",
        "GET\0https://example.test/x",
    ] {
        assert_type(&text(&mut host, PAGE, Op::FETCH, &[PLAIN, 0], head));
    }
    // Flags that name no mode, a body with no body flag, a body too long, the wrong numbers.
    assert_type(&text(&mut host, PAGE, Op::FETCH, &[6 << fetch_flag::CACHE.0, 0], HEAD));
    assert_type(&text(&mut host, PAGE, Op::FETCH, &[3 << fetch_flag::CREDENTIALS.0, 0], HEAD));
    assert_type(&text(&mut host, PAGE, Op::FETCH, &[1 << fetch_flag::BITS, 0], HEAD));
    assert_type(&text(&mut host, PAGE, Op::FETCH, &[-1, 0], HEAD));
    assert_type(&text(&mut host, PAGE, Op::FETCH, &[PLAIN, 5], HEAD));
    assert_type(&text(&mut host, PAGE, Op::FETCH, &[PLAIN, -1], HEAD));
    assert_type(&text(&mut host, PAGE, Op::FETCH, &[PLAIN], HEAD));
    assert_type(&bytes(&mut host, PAGE, Op::FETCH, &[PLAIN, 0], HEAD.as_bytes()));
    let too_long = i32::try_from(MAX_REQUEST_BODY_BYTES + 1).unwrap();
    assert_range(&text(&mut host, PAGE, Op::FETCH, &[PLAIN | (1 << fetch_flag::BODY.0), too_long], &HEAD.replace("GET", "POST")), "MAX_REQUEST_BODY_BYTES");
    // An event stream with a body; a socket that is no socket's address.
    assert_type(&text(&mut host, PAGE, Op::FETCH, &[PLAIN | 1 | (1 << fetch_flag::BODY.0), 1], HEAD));
    assert_type(&text(&mut host, PAGE, Op::WS_OPEN, &[22], "https://example.test/x"));
    assert_type(&text(&mut host, PAGE, Op::WS_OPEN, &[19], "wss://example.test/a b"));
    assert_type(&text(&mut host, PAGE, Op::WS_OPEN, &[99], "wss://example.test/"));
    assert_eq!(host.web().take_requests(), Vec::new());
    assert_eq!(host.state_bytes(), before);
    // And the next request is the first: no number was spent.
    assert_eq!(fetch(&mut host), 1);
}

/// Section 10.1 rule 4: a request is the context's that made it. Another
/// context's number is a stale handle, and giving one up does nothing.
#[test]
fn a_request_is_its_own_contexts() {
    let mut host = host();
    let id = i32::try_from(fetch(&mut host)).unwrap();
    let Reply::Int(body) = text(&mut host, PAGE, Op::FETCH, &[PLAIN | (1 << fetch_flag::BODY.0), 3], &HEAD.replace("GET", "POST")) else { panic!() };
    host.web().take_requests();
    assert_type(&bytes(&mut host, CHANNEL, Op::FETCH_BODY, &[body, 1], b"abc"));
    assert_eq!(plain(&mut host, CHANNEL, Op::FETCH_ABORT, &[id]), Reply::Undefined);
    assert_eq!(plain(&mut host, CHANNEL, Op::FETCH_CREDIT, &[id, 10]), Reply::Undefined);
    assert_eq!(plain(&mut host, PAGE, Op::FETCH_ABORT, &[999]), Reply::Undefined);
    assert_eq!(plain(&mut host, PAGE, Op::FETCH_ABORT, &[-1]), Reply::Undefined);
    assert_eq!(host.web().take_requests(), Vec::new(), "nothing was sent for another context's request");
    assert_eq!(bytes(&mut host, PAGE, Op::FETCH_BODY, &[body, 1], b"abc"), Reply::Int(1));
    assert_type(&bytes(&mut host, PAGE, Op::FETCH_BODY, &[body, 1], b"more"));
    assert_eq!(host.web().take_requests(), vec![ToWebClient::RequestBody { id: 2, bytes: b"abc".to_vec() }, ToWebClient::RequestBodyEnd(2)]);
}

/// Section 11.3: a request's body is the length its start declared. One
/// that is not fails the request at the web client.
#[test]
fn a_body_that_is_not_the_length_declared_fails_its_request() {
    for (pieces, last) in [(&[&b"abcd"[..]][..], 1), (&[&b"ab"[..]][..], 1), (&[&b"abc"[..]][..], 0), (&[&b""[..]][..], 0)] {
        let mut host = host();
        let Reply::Int(id) = text(&mut host, PAGE, Op::FETCH, &[PLAIN | (1 << fetch_flag::BODY.0), 3], &HEAD.replace("GET", "POST")) else { panic!() };
        host.web().take_requests();
        let reply = pieces.iter().map(|piece| bytes(&mut host, PAGE, Op::FETCH_BODY, &[id, last], piece)).last().unwrap();
        assert_type(&reply);
        assert_eq!(host.web().take_requests(), vec![ToWebClient::Abort(1)], "{pieces:?} {last}");
        assert_eq!(kinds(&events(&mut host)), vec![EventKind::FETCH_FAILED]);
    }
}

/// Section 11.3, `More`: room is made a byte or more at a time, and a
/// request never has more than `MAX_NETWORK_BUFFER_BYTES` of it.
#[test]
fn room_is_bounded() {
    let mut host = host();
    let id = i32::try_from(fetch(&mut host)).unwrap();
    assert_type(&plain(&mut host, PAGE, Op::FETCH_CREDIT, &[id, 0]));
    assert_type(&plain(&mut host, PAGE, Op::FETCH_CREDIT, &[id, -5]));
    let most = i32::try_from(MAX_NETWORK_BUFFER_BYTES).unwrap() - i32::try_from(FETCH_WINDOW_BYTES).unwrap();
    assert_range(&plain(&mut host, PAGE, Op::FETCH_CREDIT, &[id, most + 1]), "MAX_NETWORK_BUFFER_BYTES");
    assert_eq!(plain(&mut host, PAGE, Op::FETCH_CREDIT, &[id, most]), Reply::Undefined);
    assert_range(&plain(&mut host, PAGE, Op::FETCH_CREDIT, &[id, 1]), "MAX_NETWORK_BUFFER_BYTES");
    let sent = host.web().take_requests();
    assert_eq!(sent.last(), Some(&ToWebClient::More { id: 1, bytes: u32::try_from(most).unwrap() }));
    assert_eq!(sent.len(), 2);
}

/// Section 11.3: "the script host checks everything arriving as hostile
/// too". An answer for no request is dropped; one a request may not be
/// given fails that request, tells the web client to give it up, and
/// leaves every other request as it was.
#[test]
fn an_answer_that_breaks_the_protocol_fails_its_request_alone() {
    let body = |id: u32, length: usize| FromWebClient::Body { id, bytes: vec![1; length] };
    for (what, answers) in [
        ("a body before the head", vec![body(1, 10)]),
        ("an end before the head", vec![FromWebClient::End(1)]),
        ("two heads", vec![head(1), head(1)]),
        ("a body past the room", vec![head(1), body(1, 65_536), body(1, 65_536), body(1, 65_536), body(1, 65_536), body(1, 1)]),
        ("an event for a fetch", vec![FromWebClient::Event { id: 1, event: "x".into(), data: "y".into(), last_id: String::new() }]),
        ("a socket's opening for a fetch", vec![FromWebClient::Opened { id: 1, protocol: "chat".into(), url: String::new() }]),
        ("a socket's message for a fetch", vec![FromWebClient::SocketMessage { id: 1, binary: true, last: true, bytes: vec![1] }]),
        ("a head with a status that is none", vec![head_with(1, 99)]),
        ("a body piece of no bytes", vec![head(1), body(1, 0)]),
    ] {
        let mut host = host();
        assert_eq!((fetch(&mut host), fetch(&mut host)), (1, 2));
        host.web().take_requests();
        // Answers for no request of this container, and the trusted core's own.
        host.web().arrive(head(77));
        host.web().arrive(FromWebClient::Confined);
        for answer in answers {
            host.web().arrive(answer);
        }
        host.web().arrive(head(2));
        let events = events(&mut host);
        let failed: Vec<&NetworkEvent> = events.iter().filter(|event| event.kind == EventKind::FETCH_FAILED).collect();
        assert_eq!(failed.len(), 1, "{what}: {events:?}");
        assert_eq!(
            (failed[0].ints[0], failed[0].ints[1], &failed[0].payload),
            (1, 0, &InputPayload::None),
            "{what}: the reason is the log's, not the script's"
        );
        assert!(events.iter().any(|event| event.kind == EventKind::FETCH_HEAD && event.ints[0] == 2), "{what}: the other request was answered");
        assert_eq!(host.web().take_requests(), vec![ToWebClient::Abort(1)], "{what}");
        // Nothing more is heard of the failed request.
        host.web().arrive(body(1, 5));
        assert!(host.next_network_event().is_none(), "{what}");
        let batch = host.take_batch(1, &[]);
        assert!(batch.log.unwrap_or_default().iter().any(|line| line.text.starts_with("request 1 failed: ")), "{what}");
    }
}

/// Section 10.1 rule 7: what the host holds of the web client's answers
/// is bounded. A web client that sends without end, against the room it
/// was given, fails its request; the host's bytes stay within their bound.
#[test]
fn a_web_client_that_floods_is_held_to_the_networks_bound() {
    let mut host = host();
    let id = fetch(&mut host);
    host.web().arrive(head(id));
    for _ in 0..1_000 {
        host.web().arrive(FromWebClient::Body { id, bytes: vec![0; MAX_CHUNK_BYTES] });
        assert!(host.state_bytes() <= crate::limits::MAX_HOST_STATE_BYTES);
        assert!(host.web().state_bytes() <= MAX_NETWORK_BUFFER_BYTES + MAX_CHUNK_BYTES);
    }
    let events = events(&mut host);
    assert_eq!(events.last().map(|event| event.kind), Some(EventKind::FETCH_FAILED));
    // What was within the room was given; what was past it never is.
    let given: usize = events.iter().map(|event| if let InputPayload::Bytes(bytes) = &event.payload { bytes.len() } else { 0 }).sum();
    assert!(given <= usize::try_from(FETCH_WINDOW_BYTES).unwrap(), "{given} bytes were given against a window of {FETCH_WINDOW_BYTES}");
    // A socket's messages have no window: past the bound on answers waiting the socket fails.
    let Reply::Int(socket) = text(&mut host, PAGE, Op::WS_OPEN, &[19], "wss://example.test/") else { panic!() };
    let socket = u32::try_from(socket).unwrap();
    host.web().arrive(FromWebClient::Opened { id: socket, protocol: String::new(), url: String::new() });
    for _ in 0..MAX_ANSWERS_WAITING + 10 {
        host.web().arrive(FromWebClient::SocketMessage { id: socket, binary: true, last: true, bytes: vec![1] });
    }
    assert!(host.web().state_bytes() <= MAX_NETWORK_BUFFER_BYTES);
    assert_eq!(kinds(&self::events(&mut host)).last(), Some(&EventKind::FETCH_FAILED));
}

/// The developer log is given a failure's reason, and the script's event
/// carries the error's kind alone (section 9.2; DEC-7315's three reasons
/// among them).
#[test]
fn a_failures_reason_is_logged_and_not_given() {
    for reason in [Reason::LocalRefused, Reason::LocalPermission, Reason::DevicePermission, Reason::Cors, Reason::Certificate] {
        let mut host = host();
        let id = fetch(&mut host);
        host.web().arrive(FromWebClient::Failed { id, error: ErrorKind::Network, reason });
        let events = events(&mut host);
        assert_eq!(events, vec![NetworkEvent { context: ContextId::Page, kind: EventKind::FETCH_FAILED, ints: [1, 0, 0, 0], payload: InputPayload::None }]);
        let log = host.take_batch(1, &[]).log.unwrap();
        assert_eq!(log.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(), vec![format!("request 1 failed: {reason:?}")]);
    }
}

/// WebSocket standard section 3.1 as the host holds it: a close code is
/// 1000 or 3000 to 4999 (or the host's own two), a reason 123 bytes; a
/// send needs an open socket; and a socket is full past
/// `MAX_WS_BUFFERED_BYTES` or `MAX_WS_MESSAGE_BYTES`.
#[test]
fn a_sockets_operations_are_held_to_the_standard_and_the_bounds() {
    let mut host = host();
    let Reply::Int(id) = text(&mut host, PAGE, Op::WS_OPEN, &[19], "wss://example.test/chat,other") else { panic!() };
    assert_type(&bytes(&mut host, PAGE, Op::WS_SEND, &[id, 1, 1], b"early"));
    host.web().arrive(FromWebClient::Opened { id: 1, protocol: "chat".into(), url: String::new() });
    assert_eq!(kinds(&events(&mut host)), vec![EventKind::WS_OPENED]);
    for code in [0, 999, 1001, 1006, 2999, 5000, 70_000, -1] {
        assert_type(&text(&mut host, PAGE, Op::WS_CLOSE, &[id, code], ""));
    }
    assert_range(&text(&mut host, PAGE, Op::WS_CLOSE, &[id, 1000], &"x".repeat(124)), "MAX_CLOSE_REASON_BYTES");
    assert_type(&bytes(&mut host, PAGE, Op::WS_SEND, &[id, 2, 1], b"x"));
    assert_type(&bytes(&mut host, PAGE, Op::WS_SEND, &[id, 1, 0], b""));
    assert_type(&bytes(&mut host, CHANNEL, Op::WS_SEND, &[id, 1, 1], b"x"));
    host.web().take_requests();
    // A message in pieces: past MAX_WS_MESSAGE_BYTES the socket is full.
    let piece = vec![0_u8; MAX_HOST_BYTES];
    let pieces = MAX_WS_MESSAGE_BYTES / MAX_HOST_BYTES;
    for _ in 0..pieces {
        assert_eq!(bytes(&mut host, PAGE, Op::WS_SEND, &[id, 1, 0], &piece), Reply::Int(1));
    }
    assert_eq!(bytes(&mut host, PAGE, Op::WS_SEND, &[id, 1, 1], b"x"), Reply::Int(0));
    let sent = host.web().take_requests();
    assert!(sent.iter().all(|message| matches!(message, ToWebClient::SocketSend { bytes, last: false, .. } if bytes.len() <= MAX_CHUNK_BYTES)));
    assert_eq!(sent.len(), MAX_WS_MESSAGE_BYTES / MAX_CHUNK_BYTES);
    // Nothing was reported written, so the socket is full for any more.
    assert_eq!(bytes(&mut host, PAGE, Op::WS_SEND, &[id, 1, 1], b""), Reply::Int(1), "an empty last piece ends the message");
    assert_eq!(bytes(&mut host, PAGE, Op::WS_SEND, &[id, 1, 1], b"x"), Reply::Int(0));
    host.web().arrive(FromWebClient::Sent { id: 1, bytes: 10 });
    assert_eq!(kinds(&events(&mut host)), vec![EventKind::WS_SENT]);
    assert_eq!(bytes(&mut host, PAGE, Op::WS_SEND, &[id, 0, 1], b"0123456789"), Reply::Int(1));
    assert_eq!(text(&mut host, PAGE, Op::WS_CLOSE, &[id, 3000], "done"), Reply::Undefined);
    assert_type(&bytes(&mut host, PAGE, Op::WS_SEND, &[id, 1, 1], b"late"));
    assert_eq!(host.web().take_requests().last(), Some(&ToWebClient::SocketClose { id: 1, code: 3000, reason: "done".into() }));
}

/// Section 3.1: a context that ends drops its requests and the web
/// client is told, when it was ever asked anything; and the manifest's
/// `network`, without which nothing is asked.
#[test]
fn a_context_that_ends_or_has_no_network_asks_nothing_more() {
    let mut host = host();
    fetch(&mut host);
    host.web().take_requests();
    host.close(ContextId::Channel);
    assert_eq!(host.web().take_requests(), Vec::new(), "the channel context asked nothing");
    host.close(ContextId::Page);
    assert_eq!(host.web().take_requests(), vec![ToWebClient::ContextCloses(ContextId::Page)]);
    host.web().arrive(head(1));
    assert!(host.next_network_event().is_none());
    let mut offline = Host::new();
    offline.open(opening(ContextId::Page, false), 100);
    assert_type(&text(&mut offline, PAGE, Op::FETCH, &[PLAIN, 0], HEAD));
    assert_type(&text(&mut offline, PAGE, Op::WS_OPEN, &[19], "wss://example.test/"));
    assert_eq!(offline.web().take_requests(), Vec::new());
    // The address and the encodings need no network.
    assert!(matches!(text(&mut offline, PAGE, Op::URL_PARSE, &[-1], "https://example.test/"), Reply::Text(_)));
    assert!(matches!(bytes(&mut offline, PAGE, Op::DECODE, &[0, 0, 0], b"a"), Reply::Text(_)));
}
