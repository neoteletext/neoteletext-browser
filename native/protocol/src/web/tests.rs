//! The web client's messages' tests: every kind round trips in both
//! forms, each side refuses the other's kinds, a script host can say only
//! what is a script host's, the relay adds the tab and the token without
//! reading a payload, a start for another partition is refused, and every
//! bound refuses the value one past it, written or read.

use super::*;

/// A change that puts a message out of its bounds.
type Change<T> = Box<dyn Fn(&mut T)>;

const TAB: u32 = 3;

fn token() -> PartitionToken {
    PartitionToken::new([0xA5; PARTITION_TOKEN_BYTES])
}

fn start() -> Start {
    Start {
        id: 7,
        context: ContextId::Page,
        kind: RequestKind::Fetch,
        method: "POST".into(),
        url: "https://a.example/api?x=1".into(),
        client: "https://a.example/news/100".into(),
        headers: vec![("content-type".into(), b"application/json".to_vec()), ("x-note".into(), vec![0xE9, b' ', b'a'])],
        mode: Mode::Cors,
        credentials: Credentials::Include,
        cache: CacheMode::NoStore,
        redirect: RedirectMode::Manual,
        referrer: Referrer::Url("https://a.example/".into()),
        referrer_policy: ReferrerPolicy::StrictOrigin,
        integrity: "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=".into(),
        keepalive: true,
        body: BodySource::Bytes(12),
        protocols: Vec::new(),
        last_event_id: String::new(),
    }
}

fn socket() -> Start {
    Start {
        kind: RequestKind::WebSocket,
        method: "GET".into(),
        url: "wss://a.example/live".into(),
        body: BodySource::None,
        protocols: vec!["chat".into(), "p".repeat(MAX_SUBPROTOCOL_BYTES)],
        ..start()
    }
}

fn stream() -> Start {
    Start { kind: RequestKind::EventSource, method: "GET".into(), body: BodySource::None, last_event_id: "i".repeat(MAX_EVENT_ID_BYTES), ..start() }
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

/// Every message a script host may write.
fn requests() -> Vec<ToWebClient> {
    let mut plain = start();
    (plain.referrer, plain.body, plain.client, plain.integrity) = (Referrer::Client, BodySource::None, String::new(), String::new());
    let mut every = vec![
        ToWebClient::Start(Box::new(start())),
        ToWebClient::Start(Box::new(plain)),
        ToWebClient::Start(Box::new(socket())),
        ToWebClient::Start(Box::new(stream())),
        ToWebClient::RequestBody { id: 7, bytes: vec![0; MAX_CHUNK_BYTES] },
        ToWebClient::RequestBodyEnd(7),
        ToWebClient::Abort(u32::MAX),
        ToWebClient::More { id: 7, bytes: MAX_NETWORK_BUFFER_BYTES },
        ToWebClient::SocketSend { id: 7, binary: true, last: false, bytes: vec![1; MAX_CHUNK_BYTES] },
        ToWebClient::SocketSend { id: 7, binary: false, last: true, bytes: Vec::new() },
        ToWebClient::SocketClose { id: 7, code: 1000, reason: "r".repeat(MAX_CLOSE_REASON_BYTES) },
        ToWebClient::ContextCloses(ContextId::Page),
        ToWebClient::ContextCloses(ContextId::Channel),
    ];
    // Every value of every option crosses.
    for &mode in Mode::ALL {
        for &context in ContextId::ALL {
            let mut varied = start();
            (varied.mode, varied.context, varied.referrer, varied.body) = (mode, context, Referrer::None, BodySource::Stream);
            every.push(ToWebClient::Start(Box::new(varied)));
        }
    }
    for &credentials in Credentials::ALL {
        for &cache in CacheMode::ALL {
            for &redirect in RedirectMode::ALL {
                let mut varied = start();
                (varied.credentials, varied.cache, varied.redirect) = (credentials, cache, redirect);
                every.push(ToWebClient::Start(Box::new(varied)));
            }
        }
    }
    for &policy in ReferrerPolicy::ALL {
        let mut varied = start();
        varied.referrer_policy = policy;
        every.push(ToWebClient::Start(Box::new(varied)));
    }
    every
}

/// The messages only the trusted core writes, each with the tab it names.
fn core_only() -> Vec<(u32, ToWebClient)> {
    vec![
        (TAB, ToWebClient::TabCloses),
        (PROCESS_TAB, ToWebClient::JarKept(JarPiece { session: false, last: false, bytes: vec![7; MAX_CHUNK_BYTES] })),
        (PROCESS_TAB, ToWebClient::JarKept(JarPiece { session: true, last: true, bytes: Vec::new() })),
        (PROCESS_TAB, ToWebClient::ForgetJar),
    ]
}

/// Every message a script host is given.
fn answers() -> Vec<FromWebClient> {
    let mut unreadable = head();
    (unreadable.response_type, unreadable.status, unreadable.status_text, unreadable.url, unreadable.headers) =
        (ResponseType::OpaqueRedirect, 0, String::new(), String::new(), Vec::new());
    let mut every = vec![
        FromWebClient::Head(Box::new(head())),
        FromWebClient::Head(Box::new(unreadable)),
        FromWebClient::Body { id: 7, bytes: vec![0xFF; MAX_CHUNK_BYTES] },
        FromWebClient::End(7),
        FromWebClient::Opened { id: 7, protocol: String::new() },
        FromWebClient::Opened { id: 7, protocol: "p".repeat(MAX_SUBPROTOCOL_BYTES) },
        FromWebClient::Event { id: 7, event: "message".into(), data: "line one\nline two \u{1F600}".into(), last_id: "42".into() },
        FromWebClient::Event {
            id: 7,
            event: "x".repeat(MAX_EVENT_TYPE_BYTES),
            data: "d".repeat(MAX_EVENT_DATA_BYTES),
            last_id: "i".repeat(MAX_EVENT_ID_BYTES),
        },
        FromWebClient::Retrying(7),
        FromWebClient::SocketMessage { id: 7, binary: false, last: true, bytes: b"hello".to_vec() },
        FromWebClient::SocketMessage { id: 7, binary: true, last: true, bytes: Vec::new() },
        FromWebClient::SocketClosed { id: 7, code: 1006, reason: String::new(), clean: false },
        FromWebClient::Sent { id: 7, bytes: 1 },
        FromWebClient::Sent { id: 7, bytes: u32::try_from(MAX_REQUEST_BODY_BYTES).unwrap() },
    ];
    for &error in ErrorKind::ALL {
        for &reason in Reason::ALL {
            every.push(FromWebClient::Failed { id: 7, error, reason });
        }
    }
    every
}

/// The messages for the trusted core alone.
fn process_only() -> Vec<FromWebClient> {
    vec![
        FromWebClient::Confined,
        FromWebClient::Jar(JarPiece { session: true, last: false, bytes: vec![1] }),
        FromWebClient::Jar(JarPiece { session: false, last: true, bytes: Vec::new() }),
    ]
}

/// A frame of the given kind and payload.
fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![kind];
    frame.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// A frame after a tab's number: the client form.
fn tabbed(tab: u32, kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = tab.to_be_bytes().to_vec();
    bytes.extend_from_slice(&frame(kind, payload));
    bytes
}

/// A text or byte field as the pipe carries it.
fn field(bytes: &[u8]) -> Vec<u8> {
    let mut field = u32::try_from(bytes.len()).unwrap().to_be_bytes().to_vec();
    field.extend_from_slice(bytes);
    field
}

#[test]
fn every_kind_round_trips_in_the_host_form() {
    let mut pipe = Vec::new();
    for message in requests() {
        write_request(&mut pipe, &message).unwrap();
    }
    let mut input = pipe.as_slice();
    for message in requests() {
        assert_eq!(read_request(&mut input).unwrap(), Some(message));
    }
    assert_eq!(read_request(&mut input).unwrap(), None);

    let mut pipe = Vec::new();
    for message in answers() {
        write_answer(&mut pipe, &message).unwrap();
    }
    let mut input = pipe.as_slice();
    for message in answers() {
        assert_eq!(read_answer(&mut input).unwrap(), Some(message));
    }
    assert_eq!(read_answer(&mut input).unwrap(), None);
}

#[test]
fn every_kind_round_trips_in_the_client_form() {
    let to_client: Vec<(u32, ToWebClient)> = requests().into_iter().map(|message| (TAB, message)).chain(core_only()).collect();
    let mut pipe = Vec::new();
    for (tab, message) in &to_client {
        write_to_client(&mut pipe, *tab, &token(), message).unwrap();
    }
    let mut input = pipe.as_slice();
    for expected in to_client {
        assert_eq!(read_to_client(&mut input, &token()).unwrap(), Some(expected));
    }
    assert_eq!(read_to_client(&mut input, &token()).unwrap(), None);

    let from_client: Vec<(u32, FromWebClient)> =
        answers().into_iter().map(|message| (TAB, message)).chain(process_only().into_iter().map(|message| (PROCESS_TAB, message))).collect();
    let mut pipe = Vec::new();
    for (tab, message) in &from_client {
        write_from_client(&mut pipe, *tab, message).unwrap();
    }
    let mut input = pipe.as_slice();
    for expected in from_client {
        assert_eq!(read_from_client(&mut input).unwrap(), Some(expected));
    }
    assert_eq!(read_from_client(&mut input).unwrap(), None);
}

/// Each side reads only what the other may say.
#[test]
fn each_side_refuses_the_others_kinds() {
    for message in requests() {
        let mut host = Vec::new();
        write_request(&mut host, &message).unwrap();
        assert!(matches!(read_answer(&mut host.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
        let mut client = Vec::new();
        write_to_client(&mut client, TAB, &token(), &message).unwrap();
        assert!(matches!(read_from_client(&mut client.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
        assert!(matches!(relay_from_client(&mut client.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
    }
    for message in answers() {
        let mut host = Vec::new();
        write_answer(&mut host, &message).unwrap();
        assert!(matches!(read_request(&mut host.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
        let mut client = Vec::new();
        write_from_client(&mut client, TAB, &message).unwrap();
        assert!(matches!(read_to_client(&mut client.as_slice(), &token()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
    }
}

/// A script host says only what is a script host's: it cannot close a
/// tab, hand over a jar or forget one, and it is never given the web
/// client's `Confined` or its jar. Neither the writers, the readers nor
/// the relay pass them.
#[test]
fn a_script_host_has_only_its_own_kinds() {
    for (tab, message) in core_only() {
        assert!(write_request(&mut Vec::new(), &message).is_err(), "{message:?}");
        let mut client = Vec::new();
        write_to_client(&mut client, tab, &token(), &message).unwrap();
        // The same frame without its tab is what a script host would have
        // to write.
        let mut forged = client[4..].to_vec();
        assert!(matches!(read_request(&mut forged.as_slice()), Err(ProtocolError::UnknownKind(_))), "{message:?}");
        let mut out = Vec::new();
        assert!(matches!(relay_to_client(TAB, &token(), &mut forged, &mut out), Err(ProtocolError::UnknownKind(_))), "{message:?}");
        assert!(out.is_empty(), "nothing of it is relayed: {message:?}");
    }
    for message in process_only() {
        assert!(write_answer(&mut Vec::new(), &message).is_err(), "{message:?}");
        let mut client = Vec::new();
        write_from_client(&mut client, PROCESS_TAB, &message).unwrap();
        assert!(matches!(read_answer(&mut &client[4..]), Err(ProtocolError::UnknownKind(_))), "{message:?}");
        assert_eq!(relay_from_client(&mut client.as_slice()).unwrap(), Some(FromClientFrame::Core(message)));
    }
}

/// The relay turns what a script host wrote into what the web client
/// reads, with the tab and the token, and what the web client wrote into
/// what the script host reads.
#[test]
fn the_relay_carries_every_message_both_ways() {
    let mut written = Vec::new();
    for message in requests() {
        write_request(&mut written, &message).unwrap();
    }
    let mut relayed = Vec::new();
    relay_to_client(TAB, &token(), &mut written, &mut relayed).unwrap();
    assert_eq!(written, Vec::<u8>::new());
    let mut input = relayed.as_slice();
    for message in requests() {
        assert_eq!(read_to_client(&mut input, &token()).unwrap(), Some((TAB, message)));
    }
    assert_eq!(input, &[] as &[u8]);

    let mut pipe = Vec::new();
    for message in answers() {
        write_from_client(&mut pipe, TAB, &message).unwrap();
    }
    let mut input = pipe.as_slice();
    for message in answers() {
        let Some(FromClientFrame::Answer { tab, frame }) = relay_from_client(&mut input).unwrap() else { panic!("an answer: {message:?}") };
        assert_eq!(tab, TAB);
        assert_eq!(read_answer(&mut frame.as_slice()).unwrap(), Some(message));
    }
    assert_eq!(relay_from_client(&mut input).unwrap(), None);
}

/// The relay moves whole messages only, and stops at the first that is
/// not a script host's, keeping it and what follows.
#[test]
fn the_relay_keeps_what_is_not_whole_and_stops_at_a_fault() {
    let mut whole = Vec::new();
    write_request(&mut whole, &ToWebClient::Abort(7)).unwrap();
    let mut second = Vec::new();
    write_request(&mut second, &ToWebClient::More { id: 7, bytes: 1 }).unwrap();
    for cut in 0..second.len() {
        let mut written = [whole.as_slice(), &second[..cut]].concat();
        let mut out = Vec::new();
        relay_to_client(TAB, &token(), &mut written, &mut out).unwrap();
        assert_eq!(written, &second[..cut], "what is not whole stays, cut at {cut}");
        assert_eq!(read_to_client(&mut out.as_slice(), &token()).unwrap(), Some((TAB, ToWebClient::Abort(7))));
    }
    let foreign = frame(KIND_JAR_KEPT, &[0, 1]);
    let mut written = [whole.as_slice(), &foreign, &second].concat();
    let mut out = Vec::new();
    assert!(matches!(relay_to_client(TAB, &token(), &mut written, &mut out), Err(ProtocolError::UnknownKind(KIND_JAR_KEPT))));
    assert_eq!(written, [foreign.as_slice(), &second].concat(), "the fault and what follows stay");
    assert_eq!(out, tabbed(TAB, KIND_ABORT, &7_u32.to_be_bytes()), "what came before it was moved");
    // A length no message has is refused before its bytes are waited for.
    let mut long = vec![KIND_ABORT];
    long.extend_from_slice(&u32::try_from(MAX_PAYLOAD + 1).unwrap().to_be_bytes());
    assert!(matches!(relay_to_client(TAB, &token(), &mut long, &mut Vec::new()), Err(ProtocolError::TooLong(_))));
    // A start leaves room for the token the relay adds.
    let mut start = vec![KIND_START];
    start.extend_from_slice(&u32::try_from(MAX_HOST_START_BYTES + 1).unwrap().to_be_bytes());
    assert!(matches!(relay_to_client(TAB, &token(), &mut start, &mut Vec::new()), Err(ProtocolError::TooLong(_))));
}

/// One process, one partition (DEC-7268): a start with any token but the
/// process's own is refused, whatever else it says, and so is a start
/// with no token, as a script host writes it.
#[test]
fn a_start_for_another_partition_is_refused() {
    let message = ToWebClient::Start(Box::new(start()));
    let mut pipe = Vec::new();
    write_to_client(&mut pipe, TAB, &token(), &message).unwrap();
    assert_eq!(read_to_client(&mut pipe.as_slice(), &token()).unwrap(), Some((TAB, message.clone())));
    for changed in 0..PARTITION_TOKEN_BYTES {
        let mut other = *token().bytes();
        other[changed] ^= 1;
        let refused = read_to_client(&mut pipe.as_slice(), &PartitionToken::new(other));
        assert!(matches!(refused, Err(ProtocolError::BadValue("partition token"))), "a token that differs at byte {changed}");
    }
    let mut host = Vec::new();
    write_request(&mut host, &message).unwrap();
    let mut bare = TAB.to_be_bytes().to_vec();
    bare.extend_from_slice(&host);
    assert!(read_to_client(&mut bare.as_slice(), &token()).is_err(), "a start the trusted core did not relay");
    // A token is never written out.
    assert_eq!(format!("{:?}", token()), "PartitionToken(..)");
}

/// A tab's message names a tab, and the process's own names none.
#[test]
fn a_message_names_the_tab_its_kind_may() {
    for message in requests() {
        assert!(write_to_client(&mut Vec::new(), PROCESS_TAB, &token(), &message).is_err(), "{message:?}");
    }
    assert!(write_to_client(&mut Vec::new(), PROCESS_TAB, &token(), &ToWebClient::TabCloses).is_err());
    assert!(write_to_client(&mut Vec::new(), TAB, &token(), &ToWebClient::ForgetJar).is_err());
    assert!(read_to_client(&mut tabbed(PROCESS_TAB, KIND_ABORT, &7_u32.to_be_bytes()).as_slice(), &token()).is_err());
    assert!(read_to_client(&mut tabbed(TAB, KIND_FORGET_JAR, &[]).as_slice(), &token()).is_err());
    for message in answers() {
        assert!(write_from_client(&mut Vec::new(), PROCESS_TAB, &message).is_err(), "{message:?}");
    }
    assert!(write_from_client(&mut Vec::new(), TAB, &FromWebClient::Confined).is_err());
    assert!(read_from_client(&mut tabbed(TAB, KIND_CONFINED, &[]).as_slice()).is_err());
    assert!(relay_from_client(&mut tabbed(TAB, KIND_CONFINED, &[]).as_slice()).is_err());
    assert!(read_from_client(&mut tabbed(PROCESS_TAB, KIND_END, &7_u32.to_be_bytes()).as_slice()).is_err());
    assert!(relay_from_client(&mut tabbed(PROCESS_TAB, KIND_END, &7_u32.to_be_bytes()).as_slice()).is_err());
    // A pipe that ends after a tab's number is cut short, not ended.
    assert!(read_to_client(&mut TAB.to_be_bytes().as_slice(), &token()).is_err());
    assert!(read_from_client(&mut TAB.to_be_bytes().as_slice()).is_err());
    assert!(relay_from_client(&mut TAB.to_be_bytes().as_slice()).is_err());
}

/// A start out of its bounds is written by no one and read by no one;
/// each bound holds the value at it and refuses the value one past.
#[test]
fn a_start_out_of_bounds_is_refused() {
    let at_bound: Vec<(&str, Change<Start>)> = vec![
        ("the longest method", Box::new(|start| start.method = "M".repeat(MAX_METHOD_BYTES))),
        ("the longest address", Box::new(|start| start.url = format!("https://a.example/{}", "a".repeat(MAX_URL_BYTES - 18)))),
        ("the longest client address", Box::new(|start| start.client = "a".repeat(MAX_URL_BYTES))),
        ("the longest referrer", Box::new(|start| start.referrer = Referrer::Url("a".repeat(MAX_URL_BYTES)))),
        ("the longest header name", Box::new(|start| start.headers = vec![("n".repeat(MAX_HEADER_NAME_BYTES), Vec::new())])),
        ("the longest header value", Box::new(|start| start.headers = vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES])])),
        ("the most headers", Box::new(|start| start.headers = vec![("a".into(), Vec::new()); MAX_REQUEST_HEADERS])),
        (
            "the most header bytes",
            Box::new(|start| start.headers = vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES - 1]); MAX_HEADER_BYTES / MAX_HEADER_VALUE_BYTES]),
        ),
        ("the longest integrity metadata", Box::new(|start| start.integrity = "a".repeat(MAX_INTEGRITY_BYTES))),
        ("the largest body", Box::new(|start| start.body = BodySource::Bytes(u32::try_from(MAX_REQUEST_BODY_BYTES).unwrap()))),
    ];
    for (what, change) in at_bound {
        let mut changed = start();
        change(&mut changed);
        let message = ToWebClient::Start(Box::new(changed));
        let mut pipe = Vec::new();
        write_request(&mut pipe, &message).unwrap_or_else(|error| panic!("{what}: {error}"));
        assert_eq!(read_request(&mut pipe.as_slice()).unwrap(), Some(message), "{what}");
    }
    let mut most = socket();
    most.protocols = (0..MAX_WS_PROTOCOLS).map(|number| format!("p{number}")).collect();
    assert!(most.check().is_ok(), "the most subprotocols");

    let wrong: Vec<(&str, Change<Start>)> = vec![
        ("no method", Box::new(|start| start.method = String::new())),
        ("a method with a space", Box::new(|start| start.method = "GE T".into())),
        ("a method one byte too long", Box::new(|start| start.method = "M".repeat(MAX_METHOD_BYTES + 1))),
        ("CONNECT", Box::new(|start| start.method = "CONNECT".into())),
        ("trace, in any case", Box::new(|start| start.method = "tRaCe".into())),
        ("TRACK", Box::new(|start| start.method = "TRACK".into())),
        ("no address", Box::new(|start| start.url = String::new())),
        ("an address with a line feed", Box::new(|start| start.url = "https://a.example/\n".into())),
        ("an address with a space", Box::new(|start| start.url = "https://a.example/ a".into())),
        ("an address one byte too long", Box::new(|start| start.url = format!("https://a.example/{}", "a".repeat(MAX_URL_BYTES - 17)))),
        ("an address of another scheme", Box::new(|start| start.url = "file:///etc/passwd".into())),
        ("a socket's address for a fetch", Box::new(|start| start.url = "wss://a.example/".into())),
        ("a client address that is not ASCII", Box::new(|start| start.client = "https://\u{e9}.example/".into())),
        ("a client address one byte too long", Box::new(|start| start.client = "a".repeat(MAX_URL_BYTES + 1))),
        ("a referrer with a control", Box::new(|start| start.referrer = Referrer::Url("https://a.example/\u{7}".into()))),
        ("a referrer one byte too long", Box::new(|start| start.referrer = Referrer::Url("a".repeat(MAX_URL_BYTES + 1)))),
        ("a header name with a colon", Box::new(|start| start.headers.push(("a:b".into(), Vec::new())))),
        ("an empty header name", Box::new(|start| start.headers.push((String::new(), Vec::new())))),
        ("a header name one byte too long", Box::new(|start| start.headers.push(("n".repeat(MAX_HEADER_NAME_BYTES + 1), Vec::new())))),
        ("a header value with a line feed", Box::new(|start| start.headers.push(("a".into(), b"x\r\ny: z".to_vec())))),
        ("a header value with a NUL", Box::new(|start| start.headers.push(("a".into(), vec![b'x', 0])))),
        ("a header value that starts with a space", Box::new(|start| start.headers.push(("a".into(), b" x".to_vec())))),
        ("a header value one byte too long", Box::new(|start| start.headers.push(("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES + 1])))),
        ("one header too many", Box::new(|start| start.headers = vec![("a".into(), Vec::new()); MAX_REQUEST_HEADERS + 1])),
        (
            "headers one byte too long together",
            Box::new(|start| {
                start.headers = vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES - 1]); MAX_HEADER_BYTES / MAX_HEADER_VALUE_BYTES];
                start.headers.push(("b".into(), Vec::new()));
            }),
        ),
        ("integrity metadata one byte too long", Box::new(|start| start.integrity = "a".repeat(MAX_INTEGRITY_BYTES + 1))),
        ("integrity metadata with a control", Box::new(|start| start.integrity = "sha256-a\n".into())),
        ("a body one byte too large", Box::new(|start| start.body = BodySource::Bytes(u32::try_from(MAX_REQUEST_BODY_BYTES + 1).unwrap()))),
        ("subprotocols for a fetch", Box::new(|start| start.protocols = vec!["chat".into()])),
        ("a last event ID for a fetch", Box::new(|start| start.last_event_id = "1".into())),
    ];
    for (what, change) in wrong {
        let mut changed = start();
        change(&mut changed);
        assert!(changed.check().is_err(), "{what}");
        assert!(write_request(&mut Vec::new(), &ToWebClient::Start(Box::new(changed.clone()))).is_err(), "{what}");
        assert!(write_to_client(&mut Vec::new(), TAB, &token(), &ToWebClient::Start(Box::new(changed))).is_err(), "{what}");
    }
    let wrong_socket: Vec<(&str, Change<Start>)> = vec![
        ("an http address for a socket", Box::new(|start| start.url = "https://a.example/live".into())),
        ("one subprotocol too many", Box::new(|start| start.protocols = (0..=MAX_WS_PROTOCOLS).map(|number| format!("p{number}")).collect())),
        ("a subprotocol one byte too long", Box::new(|start| start.protocols = vec!["p".repeat(MAX_SUBPROTOCOL_BYTES + 1)])),
        ("a subprotocol that is no token", Box::new(|start| start.protocols = vec!["a b".into()])),
        ("an empty subprotocol", Box::new(|start| start.protocols = vec![String::new()])),
        ("a subprotocol twice", Box::new(|start| start.protocols = vec!["chat".into(), "chat".into()])),
    ];
    for (what, change) in wrong_socket {
        let mut changed = socket();
        change(&mut changed);
        assert!(changed.check().is_err(), "{what}");
    }
    let wrong_stream: Vec<(&str, Change<Start>)> = vec![
        ("a last event ID one byte too long", Box::new(|start| start.last_event_id = "i".repeat(MAX_EVENT_ID_BYTES + 1))),
        ("a last event ID with a line feed", Box::new(|start| start.last_event_id = "1\r\nCookie: a=b".into())),
        ("a last event ID with a NUL", Box::new(|start| start.last_event_id = "1\0".into())),
    ];
    for (what, change) in wrong_stream {
        let mut changed = stream();
        change(&mut changed);
        assert!(changed.check().is_err(), "{what}");
    }
}

/// What a web client sends is held to its bounds as it is read: neither
/// the trusted core nor a script host trusts any of it.
#[test]
fn a_head_out_of_bounds_is_refused() {
    let mut most = head();
    most.status_text = "a".repeat(MAX_STATUS_TEXT_BYTES);
    most.url = "a".repeat(MAX_URL_BYTES);
    most.headers = vec![("a".into(), Vec::new()); MAX_RESPONSE_HEADERS];
    let message = FromWebClient::Head(Box::new(most));
    let mut pipe = Vec::new();
    write_answer(&mut pipe, &message).unwrap();
    assert_eq!(read_answer(&mut pipe.as_slice()).unwrap(), Some(message), "a head at its bounds");

    let wrong: Vec<(&str, Change<Head>)> = vec![
        ("a status of 99", Box::new(|head| head.status = 99)),
        ("an interim status", Box::new(|head| head.status = 101)),
        ("a status of 600", Box::new(|head| head.status = 600)),
        ("a readable answer with no status", Box::new(|head| head.status = 0)),
        ("an unreadable answer with a status", Box::new(|head| head.response_type = ResponseType::OpaqueRedirect)),
        ("a status text with a line feed", Box::new(|head| head.status_text = "OK\r\nSet-Cookie: a=b".into())),
        ("a status text one byte too long", Box::new(|head| head.status_text = "a".repeat(MAX_STATUS_TEXT_BYTES + 1))),
        ("an address with a control", Box::new(|head| head.url = "https://a.example/\u{1b}".into())),
        ("an address one byte too long", Box::new(|head| head.url = "a".repeat(MAX_URL_BYTES + 1))),
        ("one header too many", Box::new(|head| head.headers = vec![("a".into(), Vec::new()); MAX_RESPONSE_HEADERS + 1])),
        ("a header value with a carriage return", Box::new(|head| head.headers.push(("a".into(), b"x\ry".to_vec())))),
        ("a header name that is not a token", Box::new(|head| head.headers.push(("a b".into(), Vec::new())))),
    ];
    for (what, change) in wrong {
        let mut changed = head();
        change(&mut changed);
        assert!(changed.check().is_err(), "{what}");
        assert!(write_answer(&mut Vec::new(), &FromWebClient::Head(Box::new(changed.clone()))).is_err(), "{what}");
        assert!(write_from_client(&mut Vec::new(), TAB, &FromWebClient::Head(Box::new(changed))).is_err(), "{what}");
    }
}

/// The smaller messages' bounds, written: the value one past each is
/// refused.
#[test]
fn a_message_one_past_its_bound_is_not_written() {
    let past_request = [
        ToWebClient::RequestBody { id: 7, bytes: Vec::new() },
        ToWebClient::RequestBody { id: 7, bytes: vec![0; MAX_CHUNK_BYTES + 1] },
        ToWebClient::More { id: 7, bytes: 0 },
        ToWebClient::More { id: 7, bytes: MAX_NETWORK_BUFFER_BYTES + 1 },
        ToWebClient::SocketSend { id: 7, binary: true, last: false, bytes: Vec::new() },
        ToWebClient::SocketSend { id: 7, binary: true, last: true, bytes: vec![0; MAX_CHUNK_BYTES + 1] },
        ToWebClient::SocketClose { id: 7, code: 1000, reason: "r".repeat(MAX_CLOSE_REASON_BYTES + 1) },
    ];
    for message in past_request {
        assert!(write_request(&mut Vec::new(), &message).is_err(), "{message:?}");
        assert!(write_to_client(&mut Vec::new(), TAB, &token(), &message).is_err(), "{message:?}");
    }
    let jar_past = [JarPiece { session: false, last: false, bytes: Vec::new() }, JarPiece { session: false, last: true, bytes: vec![0; MAX_CHUNK_BYTES + 1] }];
    for piece in jar_past {
        assert!(write_to_client(&mut Vec::new(), PROCESS_TAB, &token(), &ToWebClient::JarKept(piece.clone())).is_err());
        assert!(write_from_client(&mut Vec::new(), PROCESS_TAB, &FromWebClient::Jar(piece)).is_err());
    }
    let past_answer = [
        FromWebClient::Body { id: 7, bytes: Vec::new() },
        FromWebClient::Body { id: 7, bytes: vec![0; MAX_CHUNK_BYTES + 1] },
        FromWebClient::Opened { id: 7, protocol: "p".repeat(MAX_SUBPROTOCOL_BYTES + 1) },
        FromWebClient::Opened { id: 7, protocol: "a b".into() },
        FromWebClient::Event { id: 7, event: "x".repeat(MAX_EVENT_TYPE_BYTES + 1), data: String::new(), last_id: String::new() },
        FromWebClient::Event { id: 7, event: String::new(), data: "d".repeat(MAX_EVENT_DATA_BYTES + 1), last_id: String::new() },
        FromWebClient::Event { id: 7, event: String::new(), data: String::new(), last_id: "i".repeat(MAX_EVENT_ID_BYTES + 1) },
        FromWebClient::SocketMessage { id: 7, binary: true, last: false, bytes: Vec::new() },
        FromWebClient::SocketMessage { id: 7, binary: true, last: true, bytes: vec![0; MAX_CHUNK_BYTES + 1] },
        FromWebClient::SocketClosed { id: 7, code: 1000, reason: "r".repeat(MAX_CLOSE_REASON_BYTES + 1), clean: true },
        FromWebClient::Sent { id: 7, bytes: 0 },
        FromWebClient::Sent { id: 7, bytes: u32::try_from(MAX_REQUEST_BODY_BYTES + 1).unwrap() },
    ];
    for message in past_answer {
        assert!(write_answer(&mut Vec::new(), &message).is_err(), "{message:?}");
        assert!(write_from_client(&mut Vec::new(), TAB, &message).is_err(), "{message:?}");
    }
}

/// The same bounds, read: bytes that say one past a bound are refused
/// whole by every reader, whoever wrote them.
#[test]
fn a_message_one_past_its_bound_is_not_read() {
    let id = 7_u32.to_be_bytes();
    let with_id = |rest: &[u8]| [id.as_slice(), rest].concat();
    let past_request: Vec<(&str, u8, Vec<u8>)> = vec![
        ("an empty piece", KIND_REQUEST_BODY, with_id(&[])),
        ("a piece one byte too long", KIND_REQUEST_BODY, with_id(&vec![0; MAX_CHUNK_BYTES + 1])),
        ("room for nothing", KIND_MORE, with_id(&0_u32.to_be_bytes())),
        ("one byte too much room", KIND_MORE, with_id(&(MAX_NETWORK_BUFFER_BYTES + 1).to_be_bytes())),
        ("an empty socket piece that is not the last", KIND_SOCKET_SEND, with_id(&[1, 0])),
        ("a socket piece one byte too long", KIND_SOCKET_SEND, with_id(&[vec![1, 1], vec![0; MAX_CHUNK_BYTES + 1]].concat())),
        ("a close reason one byte too long", KIND_SOCKET_CLOSE, with_id(&[vec![3, 232], vec![b'r'; MAX_CLOSE_REASON_BYTES + 1]].concat())),
        ("a close reason that is not UTF-8", KIND_SOCKET_CLOSE, with_id(&[3, 232, 0xFF])),
        ("a short number", KIND_ABORT, vec![0, 0, 7]),
        ("a long number", KIND_ABORT, with_id(&[0])),
        ("an unknown context", KIND_CONTEXT_CLOSES, vec![3]),
        ("a flag that is neither", KIND_SOCKET_SEND, with_id(&[2, 1, 0])),
    ];
    for (what, kind, payload) in past_request {
        assert!(read_request(&mut frame(kind, &payload).as_slice()).is_err(), "{what}");
        assert!(read_to_client(&mut tabbed(TAB, kind, &payload).as_slice(), &token()).is_err(), "{what}");
    }
    assert!(read_to_client(&mut tabbed(PROCESS_TAB, KIND_JAR_KEPT, &[0, 0]).as_slice(), &token()).is_err(), "an empty jar piece that is not the last");
    assert!(read_to_client(&mut tabbed(PROCESS_TAB, KIND_JAR_KEPT, &[0, 2, 1]).as_slice(), &token()).is_err(), "a flag that is neither");
    assert!(read_to_client(&mut tabbed(PROCESS_TAB, KIND_FORGET_JAR, &[0]).as_slice(), &token()).is_err(), "a forgetting with a payload");
    assert!(read_to_client(&mut tabbed(TAB, KIND_TAB_CLOSES, &[0]).as_slice(), &token()).is_err(), "a closing with a payload");
    let long_jar = [vec![1, 1], vec![1; MAX_CHUNK_BYTES + 1]].concat();
    assert!(read_from_client(&mut tabbed(PROCESS_TAB, KIND_JAR, &long_jar).as_slice()).is_err(), "a jar piece one byte too long");
    assert!(relay_from_client(&mut tabbed(PROCESS_TAB, KIND_JAR, &long_jar).as_slice()).is_err(), "a jar piece one byte too long, relayed");
    assert!(read_from_client(&mut tabbed(PROCESS_TAB, KIND_CONFINED, &[0]).as_slice()).is_err(), "confined with a payload");

    let event = |event: &[u8], last_id: &[u8], data: &[u8]| with_id(&[field(event), field(last_id), field(data)].concat());
    let past_answer: Vec<(&str, u8, Vec<u8>)> = vec![
        ("an empty piece", KIND_BODY, with_id(&[])),
        ("a piece one byte too long", KIND_BODY, with_id(&vec![0; MAX_CHUNK_BYTES + 1])),
        ("an unknown error kind", KIND_FAILED, with_id(&[9, 0])),
        ("an unknown reason", KIND_FAILED, with_id(&[0, 200])),
        ("a subprotocol one byte too long", KIND_OPENED, with_id(&[b'p'; MAX_SUBPROTOCOL_BYTES + 1])),
        ("a subprotocol with a line feed", KIND_OPENED, with_id(b"a\n")),
        ("an event type one byte too long", KIND_EVENT, event(&vec![b'x'; MAX_EVENT_TYPE_BYTES + 1], b"", b"")),
        ("an event ID one byte too long", KIND_EVENT, event(b"", &vec![b'i'; MAX_EVENT_ID_BYTES + 1], b"")),
        ("event data one byte too long", KIND_EVENT, event(b"", b"", &vec![b'd'; MAX_EVENT_DATA_BYTES + 1])),
        ("event data that is not UTF-8", KIND_EVENT, event(b"message", b"1", &[0xFF, 0xFE])),
        ("an empty socket piece that is not the last", KIND_SOCKET_MESSAGE, with_id(&[1, 0])),
        ("a socket piece one byte too long", KIND_SOCKET_MESSAGE, with_id(&[vec![1, 1], vec![0; MAX_CHUNK_BYTES + 1]].concat())),
        ("a close reason one byte too long", KIND_SOCKET_CLOSED, with_id(&[vec![3, 232, 1], vec![b'r'; MAX_CLOSE_REASON_BYTES + 1]].concat())),
        ("nothing sent", KIND_SENT, with_id(&0_u32.to_be_bytes())),
        ("one byte too many sent", KIND_SENT, with_id(&u32::try_from(MAX_REQUEST_BODY_BYTES + 1).unwrap().to_be_bytes())),
    ];
    for (what, kind, payload) in past_answer {
        assert!(read_answer(&mut frame(kind, &payload).as_slice()).is_err(), "{what}");
        assert!(read_from_client(&mut tabbed(TAB, kind, &payload).as_slice()).is_err(), "{what}");
    }
}

/// A start or a head cut anywhere, or with a byte after it, or with an
/// option no value has, is refused.
#[test]
fn malformed_payloads_are_refused() {
    for start in [start(), socket(), stream()] {
        let mut whole = Vec::new();
        write_request(&mut whole, &ToWebClient::Start(Box::new(start))).unwrap();
        let payload = &whole[HEADER_BYTES..];
        for cut in 0..payload.len() {
            assert!(read_request(&mut frame(KIND_START, &payload[..cut]).as_slice()).is_err(), "a start cut at {cut}");
        }
        let mut longer = payload.to_vec();
        longer.push(0);
        assert!(read_request(&mut frame(KIND_START, &longer).as_slice()).is_err(), "a start that runs on");
        // The option bytes follow the request's number: the context, the
        // kind, five options, the keepalive flag and the body's source.
        for option in 4..13 {
            let mut changed = payload.to_vec();
            changed[option] = 200;
            assert!(read_request(&mut frame(KIND_START, &changed).as_slice()).is_err(), "option byte {option}");
        }
    }
    // The codes the script API refuses have no value here: `no-cors` (2),
    // `navigate` (3), a page's own load (3) and an opaque answer (2).
    assert!(Mode::of(2).is_err() && Mode::of(3).is_err() && RequestKind::of(3).is_err() && ResponseType::of(2).is_err());
    let mut whole = Vec::new();
    write_answer(&mut whole, &FromWebClient::Head(Box::new(head()))).unwrap();
    let payload = &whole[HEADER_BYTES..];
    for cut in 0..payload.len() {
        assert!(read_answer(&mut frame(KIND_HEAD, &payload[..cut]).as_slice()).is_err(), "a head cut at {cut}");
    }
    let mut longer = payload.to_vec();
    longer.push(0);
    assert!(read_answer(&mut frame(KIND_HEAD, &longer).as_slice()).is_err(), "a head that runs on");
}

/// The address bound is the engine's, and no kind of this pipe is one of
/// the script link's: a script host reads and writes both on its pipes,
/// and the trusted core tells them apart by kind alone.
#[test]
fn the_kinds_are_the_pipes_own() {
    assert_eq!(MAX_URL_BYTES, neoteletext_core::limits::MAX_URL_LENGTH);
    let all = [KINDS, crate::script::KINDS].concat();
    for (place, kind) in all.iter().enumerate() {
        assert!(!all[..place].contains(kind), "kind {kind} is used twice");
    }
    for &kind in KINDS {
        assert!(!(is_request_kind(kind) && is_answer_kind(kind)), "kind {kind} goes one way");
        assert!(!crate::script::is_to_host_kind(kind) && !crate::script::is_to_decoder_kind(kind), "kind {kind} is not the script link's");
    }
}

/// neoteletext-common's DEC-7315: the three reasons a request to this
/// computer or the local network fails with cross the pipe each as
/// itself, so the trusted core can tell a refusal from each of the two
/// permissions to ask for; and an address space's name reads back.
#[test]
fn the_local_network_reasons_and_spaces_are_told_apart() {
    for reason in [Reason::LocalRefused, Reason::LocalPermission, Reason::DevicePermission] {
        let message = FromWebClient::Failed { id: 7, error: ErrorKind::Network, reason };
        let mut pipe = Vec::new();
        write_from_client(&mut pipe, 3, &message).unwrap();
        assert_eq!(read_from_client(&mut pipe.as_slice()).unwrap(), Some((3, message)));
    }
    assert!(Reason::ALL.contains(&Reason::LocalRefused) && Reason::ALL.contains(&Reason::LocalPermission) && Reason::ALL.contains(&Reason::DevicePermission));
    for &space in AddressSpace::ALL {
        assert_eq!(AddressSpace::named(space.name()), Some(space));
    }
    assert_eq!(AddressSpace::named("Public"), None);
    assert_eq!(AddressSpace::named(""), None);
}

/// A start's payload written field by field with no check, in the order
/// `Start::write` writes it, so that bytes no writer would make can be
/// read: the reader is the bound a script host that has been subverted
/// meets.
fn raw_start(start: &Start) -> Vec<u8> {
    let mut payload = start.id.to_be_bytes().to_vec();
    payload.extend_from_slice(&[
        start.context.code(),
        start.kind.code(),
        start.mode.code(),
        start.credentials.code(),
        start.cache.code(),
        start.redirect.code(),
        start.referrer_policy.code(),
        u8::from(start.keepalive),
    ]);
    match start.body {
        BodySource::None => payload.push(BODY_NONE),
        BodySource::Bytes(length) => {
            payload.push(BODY_BYTES);
            payload.extend_from_slice(&length.to_be_bytes());
        }
        BodySource::Stream => payload.push(BODY_STREAM),
    }
    for text in [&start.method, &start.url, &start.client] {
        payload.extend(field(text.as_bytes()));
    }
    match &start.referrer {
        Referrer::None => payload.push(REFERRER_NONE),
        Referrer::Client => payload.push(REFERRER_CLIENT),
        Referrer::Url(url) => {
            payload.push(REFERRER_URL);
            payload.extend(field(url.as_bytes()));
        }
    }
    payload.extend(field(start.integrity.as_bytes()));
    payload.extend_from_slice(&u16::try_from(start.headers.len()).unwrap().to_be_bytes());
    for (name, value) in &start.headers {
        payload.extend(field(name.as_bytes()));
        payload.extend(field(value));
    }
    payload.extend_from_slice(&u16::try_from(start.protocols.len()).unwrap().to_be_bytes());
    for protocol in &start.protocols {
        payload.extend(field(protocol.as_bytes()));
    }
    payload.extend(field(start.last_event_id.as_bytes()));
    payload
}

/// The writer of `raw_start` is the protocol's own, byte for byte, for
/// every start the writer accepts.
#[test]
fn the_raw_start_is_the_writers() {
    for start in [start(), socket(), stream()] {
        let mut written = Vec::new();
        write_request(&mut written, &ToWebClient::Start(Box::new(start.clone()))).unwrap();
        assert_eq!(written, frame(KIND_START, &raw_start(&start)));
    }
}

/// `docs/script-api.md` section 11.3: a start one past each bound, or
/// malformed, written by hand as a subverted script host would write it,
/// is refused by the script host's reader and by the web client's, each
/// time naming the field that broke its bound.
#[test]
fn a_start_one_past_a_bound_is_refused_when_read_by_name() {
    let cases: Vec<(&str, Start)> = vec![
        ("method", Start { method: "M".repeat(MAX_METHOD_BYTES + 1), ..start() }),
        ("method", Start { method: String::new(), ..start() }),
        ("method", Start { method: "GE T".into(), ..start() }),
        ("method", Start { method: "Track".into(), ..start() }),
        ("address", Start { url: format!("https://a.example/{}", "a".repeat(MAX_URL_BYTES - 17)), ..start() }),
        ("address", Start { url: "file:///etc/passwd".into(), ..start() }),
        ("address", Start { url: "wss://a.example/".into(), ..start() }),
        ("address", Start { url: "https://a.example/\n".into(), ..start() }),
        ("address", Start { url: "https://a.example/live".into(), ..socket() }),
        ("client address", Start { client: "a".repeat(MAX_URL_BYTES + 1), ..start() }),
        ("client address", Start { client: "https://a.example/\u{7f}".into(), ..start() }),
        ("referrer", Start { referrer: Referrer::Url("a".repeat(MAX_URL_BYTES + 1)), ..start() }),
        ("referrer", Start { referrer: Referrer::Url("https://a.example/ x".into()), ..start() }),
        ("integrity", Start { integrity: "a".repeat(MAX_INTEGRITY_BYTES + 1), ..start() }),
        ("integrity", Start { integrity: "sha256-\t".into(), ..start() }),
        ("too many headers", Start { headers: vec![("a".into(), Vec::new()); MAX_REQUEST_HEADERS + 1], ..start() }),
        ("header name", Start { headers: vec![("n".repeat(MAX_HEADER_NAME_BYTES + 1), Vec::new())], ..start() }),
        ("header name", Start { headers: vec![(String::new(), Vec::new())], ..start() }),
        ("header name", Start { headers: vec![("a:b".into(), Vec::new())], ..start() }),
        ("header value", Start { headers: vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES + 1])], ..start() }),
        ("header value", Start { headers: vec![("a".into(), b"x\r\nInjected: 1".to_vec())], ..start() }),
        ("header value", Start { headers: vec![("a".into(), b"x ".to_vec())], ..start() }),
        (
            "headers too long",
            Start {
                headers: [
                    vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES - 1]); MAX_HEADER_BYTES / MAX_HEADER_VALUE_BYTES],
                    vec![("b".into(), Vec::new())],
                ]
                .concat(),
                ..start()
            },
        ),
        ("body length", Start { body: BodySource::Bytes(u32::try_from(MAX_REQUEST_BODY_BYTES + 1).unwrap()), ..start() }),
        ("subprotocols", Start { protocols: (0..=MAX_WS_PROTOCOLS).map(|number| format!("p{number}")).collect(), ..socket() }),
        ("subprotocols", Start { protocols: vec!["chat".into()], ..start() }),
        ("subprotocols", Start { protocols: vec!["chat".into(), "chat".into()], ..socket() }),
        ("subprotocol", Start { protocols: vec!["p".repeat(MAX_SUBPROTOCOL_BYTES + 1)], ..socket() }),
        ("subprotocol", Start { protocols: vec!["a b".into()], ..socket() }),
        ("last event ID", Start { last_event_id: "i".repeat(MAX_EVENT_ID_BYTES + 1), ..stream() }),
        ("last event ID", Start { last_event_id: "1\r\nCookie: a=b".into(), ..stream() }),
        ("last event ID", Start { last_event_id: "1".into(), ..start() }),
    ];
    for (name, start) in cases {
        let payload = raw_start(&start);
        let host = read_request(&mut frame(KIND_START, &payload).as_slice());
        assert!(matches!(host, Err(ProtocolError::BadValue(what)) if what == name), "{name}: the script host's reader gave {host:?}");
        let relayed = [token().bytes().as_slice(), &payload].concat();
        let client = read_to_client(&mut tabbed(TAB, KIND_START, &relayed).as_slice(), &token());
        assert!(matches!(client, Err(ProtocolError::BadValue(what)) if what == name), "{name}: the web client's reader gave {client:?}");
    }
}

/// A head's header bounds, the name, the value and all of them together,
/// at the bound and one past, written and read; the reader names each.
#[test]
fn a_head_one_past_a_header_bound_is_refused_by_name() {
    let headers = |headers: Vec<(String, Vec<u8>)>| Head { headers, ..head() };
    let at = [
        headers(vec![("n".repeat(MAX_HEADER_NAME_BYTES), Vec::new())]),
        headers(vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES])]),
        headers(vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES - 1]); MAX_HEADER_BYTES / MAX_HEADER_VALUE_BYTES]),
    ];
    for head in at {
        let message = FromWebClient::Head(Box::new(head));
        let mut pipe = Vec::new();
        write_answer(&mut pipe, &message).unwrap();
        assert_eq!(read_answer(&mut pipe.as_slice()).unwrap(), Some(message));
    }
    let past = [
        ("header name", headers(vec![("n".repeat(MAX_HEADER_NAME_BYTES + 1), Vec::new())])),
        ("header value", headers(vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES + 1])])),
        (
            "headers too long",
            headers(
                [vec![("a".into(), vec![b'x'; MAX_HEADER_VALUE_BYTES - 1]); MAX_HEADER_BYTES / MAX_HEADER_VALUE_BYTES], vec![("b".into(), Vec::new())]]
                    .concat(),
            ),
        ),
    ];
    for (name, head) in past {
        assert!(matches!(head.check(), Err(ProtocolError::BadValue(what)) if what == name), "{name}");
        assert!(write_answer(&mut Vec::new(), &FromWebClient::Head(Box::new(head.clone()))).is_err(), "{name}");
        // Written by hand, as a web client that has been subverted would.
        let mut payload = head.id.to_be_bytes().to_vec();
        payload.push(head.response_type.code());
        payload.extend_from_slice(&head.status.to_be_bytes());
        payload.push(u8::from(head.redirected));
        payload.extend(field(head.status_text.as_bytes()));
        payload.extend(field(head.url.as_bytes()));
        payload.extend_from_slice(&u16::try_from(head.headers.len()).unwrap().to_be_bytes());
        for (header, value) in &head.headers {
            payload.extend(field(header.as_bytes()));
            payload.extend(field(value));
        }
        let read = read_answer(&mut frame(KIND_HEAD, &payload).as_slice());
        assert!(matches!(read, Err(ProtocolError::BadValue(what)) if what == name), "{name}: {read:?}");
        let read = read_from_client(&mut tabbed(TAB, KIND_HEAD, &payload).as_slice());
        assert!(matches!(read, Err(ProtocolError::BadValue(what)) if what == name), "{name}: {read:?}");
    }
}
