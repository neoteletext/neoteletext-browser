//! The web client pipe's tests: every kind round trips, each side refuses
//! the other's kinds, and what is out of its bounds is refused both ways.

use super::*;

/// A change that puts a message out of its bounds.
type Change<T> = Box<dyn Fn(&mut T)>;

fn start() -> Start {
    Start {
        id: 7,
        tab: 3,
        jar: 0,
        channel: "https://a.example/news/".into(),
        kind: RequestKind::Fetch,
        method: "POST".into(),
        url: "https://a.example/api?x=1".into(),
        client: "https://a.example/100".into(),
        client_space: AddressSpace::Public,
        headers: vec![("content-type".into(), b"application/json".to_vec()), ("x-note".into(), vec![0xE9, b' ', b'a'])],
        mode: Mode::Cors,
        credentials: Credentials::Include,
        cache: CacheMode::NoStore,
        redirect: RedirectMode::Manual,
        referrer: Referrer::Url("https://a.example/".into()),
        referrer_policy: ReferrerPolicy::StrictOrigin,
        integrity: "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=".into(),
        keepalive: true,
        body: BodySource::Stream,
    }
}

fn head() -> Head {
    Head {
        id: 7,
        response_type: ResponseType::Cors,
        status: 200,
        status_text: "OK".into(),
        url: "https://b.example/data".into(),
        redirected: true,
        headers: vec![("content-type".into(), b"text/plain".to_vec()), ("x-empty".into(), Vec::new())],
    }
}

fn to_client() -> Vec<ToWebClient> {
    let mut plain = start();
    plain.referrer = Referrer::Client;
    plain.body = BodySource::None;
    plain.client = String::new();
    plain.integrity = String::new();
    let mut every = vec![
        ToWebClient::Start(Box::new(start())),
        ToWebClient::Start(Box::new(plain)),
        ToWebClient::RequestBody { id: 7, bytes: vec![0; MAX_BODY_PIECE] },
        ToWebClient::RequestBodyEnd(7),
        ToWebClient::Abort(u32::MAX),
        ToWebClient::More { id: 7, bytes: MAX_ROOM },
        ToWebClient::TabCloses(3),
        ToWebClient::JarKept(JarPiece { session: false, last: false, bytes: vec![7; MAX_BODY_PIECE] }),
        ToWebClient::JarKept(JarPiece { session: true, last: true, bytes: Vec::new() }),
        ToWebClient::SocketSend { id: 7, binary: true, bytes: vec![1, 2, 3] },
        ToWebClient::SocketClose { id: 7, code: 1000, reason: "done".into() },
    ];
    // Every value of every option crosses.
    for &kind in RequestKind::ALL {
        for &mode in Mode::ALL {
            let mut varied = start();
            (varied.kind, varied.mode, varied.referrer) = (kind, mode, Referrer::None);
            every.push(ToWebClient::Start(Box::new(varied)));
        }
    }
    for &credentials in Credentials::ALL {
        for &cache in CacheMode::ALL {
            for &redirect in RedirectMode::ALL {
                let mut varied = start();
                (varied.credentials, varied.cache, varied.redirect, varied.body) = (credentials, cache, redirect, BodySource::Bytes);
                every.push(ToWebClient::Start(Box::new(varied)));
            }
        }
    }
    for &space in AddressSpace::ALL {
        let mut varied = start();
        varied.client_space = space;
        every.push(ToWebClient::Start(Box::new(varied)));
    }
    for &policy in ReferrerPolicy::ALL {
        let mut varied = start();
        varied.referrer_policy = policy;
        every.push(ToWebClient::Start(Box::new(varied)));
    }
    every
}

fn from_client() -> Vec<FromWebClient> {
    let mut opaque = head();
    (opaque.response_type, opaque.status, opaque.status_text, opaque.url, opaque.headers) = (ResponseType::Opaque, 0, String::new(), String::new(), Vec::new());
    let mut every = vec![
        FromWebClient::Confined,
        FromWebClient::Head(Box::new(head())),
        FromWebClient::Head(Box::new(opaque)),
        FromWebClient::Body { id: 7, bytes: vec![0xFF; MAX_BODY_PIECE] },
        FromWebClient::End(7),
        FromWebClient::Opened { id: 7, text: "https://a.example/events".into() },
        FromWebClient::Event { id: 7, event: "message".into(), data: "line one\nline two \u{1F600}".into(), last_id: "42".into() },
        FromWebClient::Event { id: 7, event: "x".repeat(MAX_EVENT_TYPE), data: "d".repeat(MAX_EVENT_DATA), last_id: "i".repeat(MAX_EVENT_ID) },
        FromWebClient::Retrying(7),
        FromWebClient::Jar(JarPiece { session: true, last: false, bytes: vec![1] }),
        FromWebClient::Jar(JarPiece { session: false, last: true, bytes: Vec::new() }),
        FromWebClient::SocketMessage { id: 7, binary: false, bytes: b"hello".to_vec() },
        FromWebClient::SocketClosed { id: 7, code: 1006, reason: String::new(), clean: false },
    ];
    for &error in ErrorKind::ALL {
        for &reason in Reason::ALL {
            every.push(FromWebClient::Failed { id: 7, error, reason });
        }
    }
    every
}

#[test]
fn every_kind_round_trips() {
    let mut pipe = Vec::new();
    for message in to_client() {
        write_to_client(&mut pipe, &message).unwrap();
    }
    let mut input = pipe.as_slice();
    for message in to_client() {
        assert_eq!(read_to_client(&mut input).unwrap(), Some(message));
    }
    assert_eq!(read_to_client(&mut input).unwrap(), None);

    let mut pipe = Vec::new();
    for message in from_client() {
        write_to_core(&mut pipe, &message).unwrap();
    }
    let mut input = pipe.as_slice();
    for message in from_client() {
        assert_eq!(read_from_client(&mut input).unwrap(), Some(message));
    }
    assert_eq!(read_from_client(&mut input).unwrap(), None);
}

/// Each side reads only what the other may say: a web client's message is
/// refused by the web client, and the trusted core's by the trusted core.
#[test]
fn each_side_refuses_the_others_kinds() {
    for message in to_client() {
        let mut pipe = Vec::new();
        write_to_client(&mut pipe, &message).unwrap();
        assert!(matches!(read_from_client(&mut pipe.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
    }
    for message in from_client() {
        let mut pipe = Vec::new();
        write_to_core(&mut pipe, &message).unwrap();
        assert!(matches!(read_to_client(&mut pipe.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
    }
}

/// A start out of its bounds is written by no one and read by no one.
#[test]
fn a_start_out_of_bounds_is_refused() {
    let wrong: Vec<(&str, Change<Start>)> = vec![
        ("no method", Box::new(|start| start.method = String::new())),
        ("a method with a space", Box::new(|start| start.method = "GE T".into())),
        ("a long method", Box::new(|start| start.method = "M".repeat(MAX_METHOD + 1))),
        ("no address", Box::new(|start| start.url = String::new())),
        ("an address with a line feed", Box::new(|start| start.url = "https://a.example/\n".into())),
        ("an address with a space", Box::new(|start| start.url = "https://a.example/ a".into())),
        ("a long address", Box::new(|start| start.url = format!("https://a.example/{}", "a".repeat(MAX_URL)))),
        ("a client address that is not ASCII", Box::new(|start| start.client = "https://\u{e9}.example/".into())),
        ("no channel identity", Box::new(|start| start.channel = String::new())),
        ("a channel identity that is no folder", Box::new(|start| start.channel = "https://a.example/news".into())),
        ("a channel identity with a space", Box::new(|start| start.channel = "https://a.example/ne ws/".into())),
        ("a referrer with a control", Box::new(|start| start.referrer = Referrer::Url("https://a.example/\u{7}".into()))),
        ("a header name with a colon", Box::new(|start| start.headers.push(("a:b".into(), Vec::new())))),
        ("an empty header name", Box::new(|start| start.headers.push((String::new(), Vec::new())))),
        ("a header value with a line feed", Box::new(|start| start.headers.push(("a".into(), b"x\r\ny: z".to_vec())))),
        ("a header value with a NUL", Box::new(|start| start.headers.push(("a".into(), vec![b'x', 0])))),
        ("a header value that starts with a space", Box::new(|start| start.headers.push(("a".into(), b" x".to_vec())))),
        ("a long header value", Box::new(|start| start.headers.push(("a".into(), vec![b'x'; MAX_HEADER_VALUE + 1])))),
        ("too many headers", Box::new(|start| start.headers = vec![("a".into(), Vec::new()); MAX_REQUEST_HEADERS + 1])),
        (
            "headers too long together",
            Box::new(|start| start.headers = vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE]); MAX_HEADER_BYTES / MAX_HEADER_VALUE + 1]),
        ),
        ("long integrity metadata", Box::new(|start| start.integrity = "a".repeat(MAX_INTEGRITY + 1))),
        ("integrity metadata with a control", Box::new(|start| start.integrity = "sha256-a\n".into())),
    ];
    for (what, change) in wrong {
        let mut changed = start();
        change(&mut changed);
        assert!(changed.check().is_err(), "{what}");
        assert!(write_to_client(&mut Vec::new(), &ToWebClient::Start(Box::new(changed))).is_err(), "{what}");
    }
}

/// What a web client sends is held to its bounds as it is read: the
/// trusted core trusts none of it.
#[test]
fn a_head_out_of_bounds_is_refused() {
    let wrong: Vec<(&str, Change<Head>)> = vec![
        ("a status of 99", Box::new(|head| head.status = 99)),
        ("an interim status", Box::new(|head| head.status = 101)),
        ("a status of 600", Box::new(|head| head.status = 600)),
        ("a readable answer with no status", Box::new(|head| head.status = 0)),
        ("an unreadable answer with a status", Box::new(|head| head.response_type = ResponseType::Opaque)),
        ("a status text with a line feed", Box::new(|head| head.status_text = "OK\r\nSet-Cookie: a=b".into())),
        ("a long status text", Box::new(|head| head.status_text = "a".repeat(MAX_STATUS_TEXT + 1))),
        ("an address with a control", Box::new(|head| head.url = "https://a.example/\u{1b}".into())),
        ("too many headers", Box::new(|head| head.headers = vec![("a".into(), Vec::new()); MAX_RESPONSE_HEADERS + 1])),
        ("a header value with a carriage return", Box::new(|head| head.headers.push(("a".into(), b"x\ry".to_vec())))),
        ("a header name that is not a token", Box::new(|head| head.headers.push(("a b".into(), Vec::new())))),
    ];
    for (what, change) in wrong {
        let mut changed = head();
        change(&mut changed);
        assert!(changed.check().is_err(), "{what}");
        assert!(write_to_core(&mut Vec::new(), &FromWebClient::Head(Box::new(changed))).is_err(), "{what}");
    }
}

/// A frame of the given kind and payload.
fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![kind];
    frame.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// Bytes that are no message are refused, whichever side reads them: a
/// payload that ends early or runs on, an unknown value of an option, an
/// empty or oversized piece, room of nothing or too much.
#[test]
fn malformed_payloads_are_refused() {
    let mut whole = Vec::new();
    write_to_client(&mut whole, &ToWebClient::Start(Box::new(start()))).unwrap();
    let payload = &whole[5..];
    for cut in 0..payload.len() {
        assert!(read_to_client(&mut frame(KIND_START, &payload[..cut]).as_slice()).is_err(), "a start cut at {cut}");
    }
    let mut longer = payload.to_vec();
    longer.push(0);
    assert!(read_to_client(&mut frame(KIND_START, &longer).as_slice()).is_err(), "a start that runs on");
    // The option bytes follow the three numbers.
    for option in 12..21 {
        let mut changed = payload.to_vec();
        changed[option] = 200;
        assert!(read_to_client(&mut frame(KIND_START, &changed).as_slice()).is_err(), "option byte {option}");
    }
    assert!(read_to_client(&mut frame(KIND_REQUEST_BODY, &[0, 0, 0, 7]).as_slice()).is_err(), "an empty piece");
    assert!(read_to_client(&mut frame(KIND_REQUEST_BODY, &vec![0; 4 + MAX_BODY_PIECE + 1]).as_slice()).is_err(), "a piece too long");
    assert!(read_to_client(&mut frame(KIND_MORE, &[0, 0, 0, 7, 0, 0, 0, 0]).as_slice()).is_err(), "room for nothing");
    assert!(read_to_client(&mut frame(KIND_MORE, &[0, 0, 0, 7, 0xFF, 0xFF, 0xFF, 0xFF]).as_slice()).is_err(), "too much room");
    assert!(read_to_client(&mut frame(KIND_ABORT, &[0, 0, 7]).as_slice()).is_err(), "a short number");
    assert!(read_to_client(&mut frame(KIND_JAR_KEPT, &[0, 0]).as_slice()).is_err(), "an empty piece that is not the last");
    assert!(read_to_client(&mut frame(KIND_JAR_KEPT, &[0, 2, 1]).as_slice()).is_err(), "a flag that is neither");
    assert!(read_from_client(&mut frame(KIND_JAR, &vec![1; 2 + MAX_BODY_PIECE + 1]).as_slice()).is_err(), "a jar piece too long");
    assert!(read_to_client(&mut frame(KIND_ABORT, &[0, 0, 0, 7, 0]).as_slice()).is_err(), "a long number");

    assert!(read_from_client(&mut frame(KIND_BODY, &[0, 0, 0, 7]).as_slice()).is_err(), "an empty piece");
    assert!(read_from_client(&mut frame(KIND_FAILED, &[0, 0, 0, 7, 9, 0]).as_slice()).is_err(), "an unknown error kind");
    assert!(read_from_client(&mut frame(KIND_FAILED, &[0, 0, 0, 7, 0, 200]).as_slice()).is_err(), "an unknown reason");
    assert!(read_from_client(&mut frame(KIND_CONFINED, &[0]).as_slice()).is_err(), "confined with a payload");
    assert!(read_from_client(&mut frame(KIND_OPENED, &[0, 0, 0, 7, b'a', b'\n']).as_slice()).is_err(), "an opening with a line feed");
    let mut event = vec![0, 0, 0, 7];
    for text in [&b"message"[..], b"1", &[0xFF, 0xFE]] {
        event.extend_from_slice(&u32::try_from(text.len()).unwrap().to_be_bytes());
        event.extend_from_slice(text);
    }
    assert!(read_from_client(&mut frame(KIND_EVENT, &event).as_slice()).is_err(), "event data that is not UTF-8");
    let mut head_bytes = Vec::new();
    write_to_core(&mut head_bytes, &FromWebClient::Head(Box::new(head()))).unwrap();
    let payload = &head_bytes[5..];
    for cut in 0..payload.len() {
        assert!(read_from_client(&mut frame(KIND_HEAD, &payload[..cut]).as_slice()).is_err(), "a head cut at {cut}");
    }
}
