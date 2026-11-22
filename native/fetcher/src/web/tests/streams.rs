//! Bodies as streams: an answer's body in bounded pieces as far as there
//! is room for it, HTTP/2 by ALPN, connections kept, an abort, and
//! server-sent events.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use protocol::web::{ErrorKind, FETCH_WINDOW_BYTES, FromWebClient, MAX_CHUNK_BYTES, MAX_NETWORK_BUFFER_BYTES, Reason, RequestKind, ToWebClient};

use super::support::{Client, Piece, Reply, serve, serve_tls};
use crate::web::coding::tests::gzip;
use crate::web::limits::{MAX_DECODED_BODY, MIN_RETRY};

/// An answer's body comes in pieces of at most `MAX_CHUNK_BYTES` bytes,
/// and only as far as the trusted core has made room: a server that sends
/// without end fills no memory here or there.
#[test]
fn a_body_is_sent_only_as_far_as_there_is_room() {
    const BODY: usize = 6 * 1024 * 1024;
    let server = serve(|_| Reply::new(200).body(&vec![b'x'; BODY]));
    let mut client = Client::new(&server.url("/a/"));
    client.makes_room = false;
    let id = client.start(&server.url("/a/big"), |_| {});
    assert!(matches!(client.next_event(), FromWebClient::Head(_)));
    let mut received = 0;
    let mut take = |client: &mut Client, wait: Duration| {
        let mut ended = false;
        while let Some(event) = client.next_within(wait) {
            match event {
                FromWebClient::Body { id: of, bytes } => {
                    assert!(of == id && !bytes.is_empty() && bytes.len() <= MAX_CHUNK_BYTES);
                    received += bytes.len();
                }
                FromWebClient::End(of) => {
                    assert_eq!(of, id);
                    ended = true;
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        (received, ended)
    };
    let initial = usize::try_from(FETCH_WINDOW_BYTES).unwrap();
    let (so_far, ended) = take(&mut client, Duration::from_millis(500));
    assert!(!ended && so_far <= initial && so_far > initial - MAX_CHUNK_BYTES, "{so_far} bytes came into room for {initial}");
    client.send(&ToWebClient::More { id, bytes: 100_000 });
    let (so_far, ended) = take(&mut client, Duration::from_millis(500));
    assert!(!ended && so_far <= initial + 100_000 && so_far > initial + 100_000 - MAX_CHUNK_BYTES, "{so_far}");
    client.makes_room = true;
    client.send(&ToWebClient::More { id, bytes: MAX_NETWORK_BUFFER_BYTES });
    assert_eq!(client.outcome(id).body.len() + so_far, BODY);
    client.finish().unwrap();
}

/// A coded body that decodes to more than its bound fails, having sent
/// no more than the bound: a small answer cannot be made a flood.
#[test]
fn a_coded_body_decodes_to_no_more_than_its_bound() {
    let over = usize::try_from(MAX_DECODED_BODY).unwrap() + 1024 * 1024;
    let coded = gzip(&vec![0_u8; over]);
    assert!(coded.len() < 100 * 1024, "{} bytes coded", coded.len());
    let server = serve(move |_| Reply::new(200).header("content-encoding", "gzip").body(&coded));
    let mut client = Client::new(&server.url("/a/"));
    let outcome = client.get(&server.url("/a/bomb"));
    assert_eq!(outcome.failed, Some((ErrorKind::Network, Reason::TooLarge)));
    assert!(outcome.body.len() <= usize::try_from(MAX_DECODED_BODY).unwrap());
    assert!(outcome.pieces.iter().all(|&piece| piece <= MAX_CHUNK_BYTES));
    client.finish().unwrap();
}

/// DEC-7260 item 7: HTTP/2 where the server chooses it in the TLS
/// handshake, its requests sharing one connection; HTTP/1.1 otherwise,
/// its connection kept for the next request.
#[test]
fn http2_is_chosen_by_alpn_and_connections_are_kept() {
    let handler = |asked: &super::support::Asked| {
        if asked.target == "/a/slow" {
            Reply::new(200).then(Piece::Wait(Duration::from_millis(300))).body(b"slow")
        } else {
            Reply::ok(if asked.http2 { "HTTP/2" } else { "HTTP/1.1" })
        }
    };
    let both = serve_tls(&["h2", "http/1.1"], handler);
    let mut client = Client::new(&both.at("https", "localhost", "/a/"));
    assert_eq!(client.get(&both.at("https", "localhost", "/a/x")).text(), "HTTP/2");
    // Eight at once, on the one connection.
    let asked = Instant::now();
    let ids: Vec<u32> = (0..8).map(|_| client.start(&both.at("https", "localhost", "/a/slow"), |_| {})).collect();
    let mut ended = 0;
    while ended < ids.len() {
        if matches!(client.next_event(), FromWebClient::End(_)) {
            ended += 1;
        }
    }
    assert!(asked.elapsed() < Duration::from_millis(1500), "together, not one after another");
    assert_eq!(both.connections(), 1);
    let post = client.fetch(&both.at("https", "localhost", "/a/post"), |start| start.method = "POST".into());
    assert_eq!((post.ok().status, post.ok().status_text.as_str()), (200, ""), "HTTP/2 has no status text");
    client.finish().unwrap();

    let old = serve_tls(&["http/1.1"], handler);
    let mut client = Client::new(&old.at("https", "localhost", "/a/"));
    for _ in 0..3 {
        assert_eq!(client.get(&old.at("https", "localhost", "/a/x")).text(), "HTTP/1.1");
    }
    assert_eq!(old.connections(), 1, "one connection, kept between requests");
    // Requests with credentials and without do not share one (the Fetch
    // standard's connection pool key).
    client.fetch(&old.at("https", "localhost", "/a/x"), |start| start.credentials = protocol::web::Credentials::Omit);
    assert_eq!(old.connections(), 2);
    client.finish().unwrap();

    let plain = serve(handler);
    let mut client = Client::new(&plain.url("/a/"));
    for _ in 0..3 {
        assert_eq!(client.get(&plain.url("/a/x")).text(), "HTTP/1.1");
    }
    assert_eq!(plain.connections(), 1);
    client.finish().unwrap();
}

/// An abort ends a request wherever it is, once, with the standard's
/// `AbortError`; an abort that crosses the request's own end is answered
/// by nothing more.
#[test]
fn an_abort_ends_a_request_once() {
    let server = serve(|asked| if asked.target == "/a/hang" { Reply::new(200).body(b"start").then(Piece::Hang) } else { Reply::ok("done") });
    let mut client = Client::new(&server.url("/a/"));
    let id = client.start(&server.url("/a/hang"), |_| {});
    assert!(matches!(client.next_event(), FromWebClient::Head(_)));
    assert!(matches!(client.next_event(), FromWebClient::Body { .. }));
    client.send(&ToWebClient::Abort(id));
    assert_eq!(client.next_event(), FromWebClient::Failed { id, error: ErrorKind::Abort, reason: Reason::Aborted });
    // While its body is still being given.
    let uploading = client.start(&server.url("/a/upload"), |start| (start.method, start.body) = ("POST".into(), protocol::web::BodySource::Bytes(8)));
    client.send(&ToWebClient::RequestBody { id: uploading, bytes: b"part".to_vec() });
    client.send(&ToWebClient::Abort(uploading));
    assert_eq!(client.next_event(), FromWebClient::Failed { id: uploading, error: ErrorKind::Abort, reason: Reason::Aborted });
    client.send(&ToWebClient::RequestBody { id: uploading, bytes: b"late".to_vec() });
    client.send(&ToWebClient::RequestBodyEnd(uploading));
    assert_eq!(server.count("POST", "/a/upload"), 0);
    // After its end, and for a request that never was: nothing.
    let done = client.start(&server.url("/a/done"), |_| {});
    assert_eq!(client.outcome(done).text(), "done");
    client.send(&ToWebClient::Abort(done));
    client.send(&ToWebClient::Abort(9999));
    client.send(&ToWebClient::More { id: 9999, bytes: 1 });
    assert_eq!(client.next_within(Duration::from_millis(200)), None);
    // A request body past its stated length fails as too large, a stream
    // past the bound on any body likewise, and one that ends short of its
    // stated length is no request.
    let large = client.start(&server.url("/a/large"), |start| (start.method, start.body) = ("POST".into(), protocol::web::BodySource::Bytes(4)));
    client.send(&ToWebClient::RequestBody { id: large, bytes: b"12345".to_vec() });
    assert_eq!(client.next_event(), FromWebClient::Failed { id: large, error: ErrorKind::Network, reason: Reason::TooLarge });
    let short = client.start(&server.url("/a/large"), |start| (start.method, start.body) = ("POST".into(), protocol::web::BodySource::Bytes(4)));
    client.send(&ToWebClient::RequestBody { id: short, bytes: b"123".to_vec() });
    client.send(&ToWebClient::RequestBodyEnd(short));
    assert_eq!(client.next_event(), FromWebClient::Failed { id: short, error: ErrorKind::Network, reason: Reason::Request });
    let large = client.start(&server.url("/a/large"), |start| (start.method, start.body) = ("POST".into(), protocol::web::BodySource::Stream));
    for _ in 0..=protocol::web::MAX_REQUEST_BODY_BYTES / MAX_CHUNK_BYTES {
        client.send(&ToWebClient::RequestBody { id: large, bytes: vec![0; MAX_CHUNK_BYTES] });
    }
    assert_eq!(client.next_event(), FromWebClient::Failed { id: large, error: ErrorKind::Network, reason: Reason::TooLarge });
    client.finish().unwrap();
}

/// The HTML standard's `EventSource` (section 9.2): the stream is opened
/// as the standard asks, its events come as they are sent, and when the
/// connection ends it is made again with the last event's ID, after the
/// wait the stream asked for and no sooner than the least.
#[test]
fn an_event_stream_is_read_and_made_again() {
    static OPENED: AtomicUsize = AtomicUsize::new(0);
    let server = serve(|asked| {
        let stream = Reply::new(200).header("content-type", "text/event-stream; charset=utf-8");
        match (asked.target.as_str(), OPENED.fetch_add(1, Ordering::SeqCst)) {
            ("/a/events", 0) => {
                stream.body(b"retry: 10\nid: 7\nevent: tick\ndata: one\n\n").then(Piece::Wait(Duration::from_millis(50))).body(b"data: two\n\ndata: cut")
            }
            ("/a/events", _) => {
                stream.body(format!("data: again after {}\n\n", asked.header("last-event-id").unwrap_or_default()).as_bytes()).then(Piece::Hang)
            }
            ("/a/moved", _) => Reply::to(302, "/a/events"),
            ("/a/page", _) => Reply::ok("<html>"),
            ("/a/gone", _) => Reply::new(204),
            _ => Reply::new(404).header("content-type", "text/event-stream"),
        }
    });
    let mut client = Client::new(&server.url("/a/"));
    let source = |start: &mut protocol::web::Start| start.kind = RequestKind::EventSource;
    let id = client.start(&server.url("/a/events"), source);
    assert_eq!(client.next_event(), FromWebClient::Opened { id, protocol: String::new(), url: server.url("/a/events") });
    assert_eq!(client.next_event(), FromWebClient::Event { id, event: "tick".into(), data: "one".into(), last_id: "7".into() });
    assert_eq!(client.next_event(), FromWebClient::Event { id, event: "message".into(), data: "two".into(), last_id: "7".into() });
    let lost = Instant::now();
    assert_eq!(client.next_event(), FromWebClient::Retrying(id));
    assert_eq!(client.next_event(), FromWebClient::Opened { id, protocol: String::new(), url: server.url("/a/events") });
    assert!(lost.elapsed() >= MIN_RETRY.checked_sub(Duration::from_millis(50)).unwrap(), "a stream cannot ask to be made again at once");
    assert_eq!(
        client.next_event(),
        FromWebClient::Event { id, event: "message".into(), data: "again after 7".into(), last_id: "7".into() },
        "the event cut short was discarded"
    );
    let asked = server.asked();
    assert_eq!(asked[0].header("accept").as_deref(), Some("text/event-stream"));
    assert_eq!(asked[0].header("cache-control").as_deref(), Some("no-cache"), "never cached");
    assert_eq!((asked[0].header("last-event-id"), asked[1].header("last-event-id").as_deref()), (None, Some("7")));
    // The script closes it by aborting.
    client.send(&ToWebClient::Abort(id));
    assert_eq!(client.next_event(), FromWebClient::Failed { id, error: ErrorKind::Abort, reason: Reason::Aborted });
    // A stream a redirect led to says the address that answered, whose
    // origin is its events' (TODO-7371 step 1).
    let moved = client.start(&server.url("/a/moved#part"), source);
    assert_eq!(client.next_event(), FromWebClient::Opened { id: moved, protocol: String::new(), url: server.url("/a/events") });
    client.send(&ToWebClient::Abort(moved));
    while client.next_event() != (FromWebClient::Failed { id: moved, error: ErrorKind::Abort, reason: Reason::Aborted }) {}
    // What is not an event stream fails for good, with no retrying.
    for path in ["/a/page", "/a/gone", "/a/missing"] {
        assert_eq!(client.fetch(&server.url(path), source).reason(), Reason::NotEventStream, "{path}");
    }
    assert_eq!(client.fetch(&server.at("http", "other.test", "/a/events"), source).reason(), Reason::Cors, "another origin's, by its leave alone");
    assert_eq!(client.next_within(Duration::from_millis(200)), None);
    client.finish().unwrap();
}

/// A server gone when the stream is first asked for is asked again, the
/// script told each time.
#[test]
fn an_event_stream_that_cannot_connect_is_tried_again() {
    let closed = std::net::TcpListener::bind((super::support::HERE, 0)).unwrap().local_addr().unwrap().port();
    let mut client = Client::new(&format!("http://site.test:{closed}/a/"));
    let id = client.start(&format!("http://site.test:{closed}/a/events"), |start| start.kind = RequestKind::EventSource);
    assert_eq!(client.next_event(), FromWebClient::Retrying(id));
    assert_eq!(client.next_event(), FromWebClient::Retrying(id));
    client.send(&ToWebClient::Abort(id));
    assert_eq!(client.next_event(), FromWebClient::Failed { id, error: ErrorKind::Abort, reason: Reason::Aborted });
    client.finish().unwrap();
}

/// The Fetch standard's keepalive flag (the script API's departure D9,
/// TODO-7371 step 4): a request a page starts as it closes goes on when
/// its context closes, its answer told to no one; a request without the
/// flag ends with its context. The bodies of the keepalive requests under
/// way are held to the standard's 64 kibibytes together.
#[test]
fn a_keepalive_request_outlives_its_context() {
    let server = serve(|asked| match asked.target.as_str() {
        // Each answers late, so the context has closed by then.
        "/a/beacon" | "/a/plain" => {
            std::thread::sleep(Duration::from_millis(250));
            Reply::to(302, &format!("{}-landed", asked.target))
        }
        "/a/slow" => {
            std::thread::sleep(Duration::from_millis(600));
            Reply::ok("slow")
        }
        _ => Reply::ok("landed"),
    });
    let mut client = Client::new(&server.url("/a/"));
    client.start(&server.url("/a/beacon"), |start| start.keepalive = true);
    client.start(&server.url("/a/plain"), |_| {});
    client.send(&ToWebClient::ContextCloses(protocol::script::ContextId::Page));
    assert_eq!(client.next_within(Duration::from_millis(1200)), None, "neither says anything more");
    assert_eq!((server.count("GET", "/a/beacon-landed"), server.count("GET", "/a/plain-landed")), (1, 0), "the keepalive request went on, the other ended");
    // The quota: 60 kibibytes under way, and 8 more are too many; once
    // the first has ended there is room again.
    let post = |bytes: u32| {
        move |start: &mut protocol::web::Start| (start.method, start.keepalive, start.body) = ("POST".into(), true, protocol::web::BodySource::Bytes(bytes))
    };
    let send = |client: &mut Client, id: u32, bytes: usize| {
        for piece in vec![0_u8; bytes].chunks(MAX_CHUNK_BYTES) {
            client.send(&ToWebClient::RequestBody { id, bytes: piece.to_vec() });
        }
        client.send(&ToWebClient::RequestBodyEnd(id));
    };
    let first = client.start(&server.url("/a/slow"), post(60 * 1024));
    send(&mut client, first, 60 * 1024);
    let second = client.start(&server.url("/a/over"), post(8 * 1024));
    send(&mut client, second, 8 * 1024);
    assert_eq!(client.next_event(), FromWebClient::Failed { id: second, error: ErrorKind::Network, reason: Reason::TooLarge });
    assert_eq!(client.outcome(first).text(), "slow");
    let third = client.start(&server.url("/a/after"), post(8 * 1024));
    send(&mut client, third, 8 * 1024);
    assert_eq!(client.outcome(third).text(), "landed");
    assert_eq!(server.count("POST", "/a/over"), 0);
    client.finish().unwrap();
}
