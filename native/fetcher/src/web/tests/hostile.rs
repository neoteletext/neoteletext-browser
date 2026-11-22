//! Hostile servers, in this process: heads and bodies that are too large
//! or never end, drips of a byte against each time limit, decompression
//! bombs in each coding, redirect loops and chains to other partitions
//! and to local addresses, a name that resolves elsewhere the second
//! time, `Set-Cookie` floods, malformed preflight answers, and the HTTP/2
//! abuse the h2 crate leaves to its caller as far as a server written
//! frame by frame can make it. Each must end within its named bound, with
//! no panic and with the process's memory bounded (TODO-7323 item 5).
//!
//! Memory is the process's resident size, sampled while a test runs, so
//! it counts every test running beside it: the bound asserted is loose
//! ([`MEMORY_SLACK`]), enough to catch a buffer that grows with what a
//! server sends, which over this computer's loopback would pass it in
//! well under a second.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use protocol::web::{
    Credentials, ErrorKind, FETCH_WINDOW_BYTES, FromWebClient, MAX_CHUNK_BYTES, MAX_JAR_BYTES, MAX_NETWORK_BUFFER_BYTES, Reason, RequestKind, ToWebClient,
};

use super::support::{Client, HERE, Outcome, Reply, certificate, header, serve};
use crate::web::coding::tests::gzip;
use crate::web::limits::{BODY_IDLE_TIMEOUT, DECODED_RATIO_FLOOR, MAX_COOKIES_PER_DOMAIN, MAX_DECODED_BODY, MAX_DECOMPRESSED_RATIO, MAX_REDIRECTS, PIECE};

/// The time limits a hostile test's client is given in place of the
/// engine's 10 and 30 seconds, so that each test ends soon.
const SHORT: Duration = Duration::from_secs(2);
/// What a failure may take past the limit that ends it: the scheduling
/// of a loaded test run.
const LATE: Duration = Duration::from_secs(3);
/// The growth of the process's resident memory a test may cause, loose
/// because other tests run beside it.
const MEMORY_SLACK: u64 = 256 * 1024 * 1024;
/// How long a hostile server keeps writing at most, so that none of its
/// threads outlives the test run by long.
const SERVER_LIFETIME: Duration = Duration::from_secs(40);
/// A piece of what a flood writes at a time.
const FLOOD_PIECE: usize = 16 * 1024;

/// The process's resident memory in bytes, where the system says it.
fn resident() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kib| kib * 1024)
}

/// The most resident memory seen while it lives, sampled every few
/// milliseconds on a thread of its own.
struct Peak {
    before: Option<u64>,
    most: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl Peak {
    fn start() -> Self {
        let (most, stop) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
        let (seen, stopped) = (Arc::clone(&most), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                if let Some(now) = resident() {
                    seen.fetch_max(now, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        Self { before: resident(), most, stop }
    }

    /// Asserts the memory grew by no more than the slack, and says by how
    /// much.
    fn check(self, what: &str) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(before) = self.before {
            let grew = self.most.load(Ordering::SeqCst).saturating_sub(before);
            eprintln!("{what}: resident memory grew by at most {} KiB", grew / 1024);
            assert!(grew <= MEMORY_SLACK, "{what}: the process grew by {grew} bytes");
        }
    }
}

/// A server on this computer that runs `script` on each connection, on a
/// thread of its own.
fn raw_server(script: impl Fn(TcpStream) + Send + Sync + 'static) -> u16 {
    raw_server_at(HERE, script)
}

fn raw_server_at(address: IpAddr, script: impl Fn(TcpStream) + Send + Sync + 'static) -> u16 {
    let listener = TcpListener::bind((address, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = Arc::new(script);
    std::thread::spawn(move || {
        for socket in listener.incoming().flatten() {
            let script = Arc::clone(&script);
            std::thread::spawn(move || script(socket));
        }
    });
    port
}

/// Reads a request's head, to its blank line or the connection's end.
fn read_head(socket: &mut impl Read) {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap_or(0) == 1 {
        head.push(byte[0]);
    }
}

/// Writes `first`, then `again` over and over until the connection fails
/// or [`SERVER_LIFETIME`] passes, waiting `pause` between writes.
fn flood(socket: &mut dyn Write, first: &[u8], again: &[u8], pause: Duration) {
    let began = Instant::now();
    if socket.write_all(first).is_err() {
        return;
    }
    while began.elapsed() < SERVER_LIFETIME && socket.write_all(again).is_ok() {
        if !pause.is_zero() {
            std::thread::sleep(pause);
        }
    }
}

/// A web client for the channel `/a/` at `origin`, its limits short.
fn short_client(origin: &str) -> Client {
    Client::with(&format!("{origin}/a/"), |setup| (setup.settings.open_deadline, setup.settings.head_timeout) = (SHORT, SHORT))
}

/// A request's events to its end, whatever they are, the body's bytes
/// counted; how long it took.
fn ended(client: &mut Client, id: u32) -> (Outcome, Duration) {
    let asked = Instant::now();
    let outcome = client.outcome(id);
    (outcome, asked.elapsed())
}

/// The failure of a request that ended in failure, with or without a head.
fn failure(outcome: &Outcome) -> Reason {
    outcome.failed.unwrap_or_else(|| panic!("a failure, not {:?} and {} bytes", outcome.head, outcome.body.len())).1
}

/// A head that never ends: its name, what the server writes first, what
/// it writes over and over, the pause between, the limit it must end
/// within, and the reasons it may fail with.
type Endless = (&'static str, Vec<u8>, Vec<u8>, Duration, Duration, &'static [Reason]);

/// HTTP/1.1 heads that never end, or end only after too much, fail by the
/// head's bound long before its time limit, and a head dripped a byte at
/// a time, or a flood of interim answers, fails at the time limit.
#[test]
fn heads_that_never_end_fail_within_their_bounds() {
    let cases: Vec<Endless> = vec![
        (
            "endless header lines",
            b"HTTP/1.1 200 OK\r\n".to_vec(),
            b"X-Flood: aaaaaaaaaaaaaaaa\r\n".to_vec(),
            Duration::ZERO,
            Duration::ZERO,
            &[Reason::Protocol, Reason::TooLarge],
        ),
        ("an endless status line", b"HTTP/1.1 200 ".to_vec(), vec![b'A'; FLOOD_PIECE], Duration::ZERO, Duration::ZERO, &[Reason::Protocol, Reason::TooLarge]),
        (
            "one header without end",
            b"HTTP/1.1 200 OK\r\nX-Long: ".to_vec(),
            vec![b'v'; FLOOD_PIECE],
            Duration::ZERO,
            Duration::ZERO,
            &[Reason::Protocol, Reason::TooLarge],
        ),
        (
            "endless interim answers",
            Vec::new(),
            b"HTTP/1.1 103 Early Hints\r\nLink: </a>\r\n\r\n".to_vec(),
            Duration::from_millis(1),
            SHORT,
            &[Reason::Timeout, Reason::Protocol],
        ),
        ("a head dripped a byte at a time", b"HTTP/1.1 200 OK\r\n".to_vec(), b"x".to_vec(), Duration::from_millis(50), SHORT, &[Reason::Timeout]),
    ];
    for (what, first, again, pause, limit, reasons) in cases {
        let peak = Peak::start();
        let port = raw_server(move |mut socket| {
            read_head(&mut socket);
            flood(&mut socket, &first, &again, pause);
        });
        let mut client = short_client(&format!("http://site.test:{port}"));
        let id = client.start(&format!("http://site.test:{port}/a/x"), |_| {});
        let (outcome, took) = ended(&mut client, id);
        let reason = outcome.reason();
        eprintln!("{what}: {reason:?} after {took:?}");
        assert!(reasons.contains(&reason), "{what}: {reason:?}");
        assert!(took <= limit + LATE, "{what}: ended after {took:?}");
        client.finish().unwrap();
        peak.check(what);
    }
}

/// The deadline over a connection's whole opening holds against a TLS
/// handshake answered one byte at a time: a record's header promising
/// 16 KiB, its bytes dripped, so that no single read waits long.
#[test]
fn a_handshake_dripped_a_byte_at_a_time_fails_at_the_opening_deadline() {
    let port = raw_server(|mut socket| {
        let mut hello = [0_u8; 512];
        let _ = socket.read(&mut hello);
        // A handshake record (RFC 8446 section 5.1): type 22, version 3.3,
        // a length of 16,384, then its bytes slowly.
        flood(&mut socket, &[0x16, 0x03, 0x03, 0x40, 0x00], &[0x02], Duration::from_millis(50));
    });
    let mut client = short_client(&format!("https://localhost:{port}"));
    let id = client.start(&format!("https://localhost:{port}/a/x"), |_| {});
    let (outcome, took) = ended(&mut client, id);
    assert_eq!(outcome.reason(), Reason::Timeout);
    assert!(took >= SHORT && took <= SHORT + LATE, "ended after {took:?}");
    client.finish().unwrap();
}

/// A body cut short: its name, the whole answer, and the reasons it may
/// fail with, or none when it is whole.
type Early = (&'static str, &'static [u8], Option<&'static [Reason]>);

/// A body that ends before its framing says it does, in each framing of
/// HTTP/1.1, fails after its head; a body that ends with the connection
/// and no framing is whole.
#[test]
fn a_body_that_ends_early_fails_in_each_framing() {
    let cases: [Early; 4] = [
        ("a chunk cut short", b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n10\r\nabc", Some(&[Reason::Protocol, Reason::Connection])),
        ("no last chunk", b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n", Some(&[Reason::Protocol, Reason::Connection])),
        (
            "a chunk size that is no number",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nhello\r\n0\r\n\r\n",
            // A chunk size that is no number breaks a rule of HTTP/1.1 (RFC
            // 9112 section 7.1), and the developer log says so.
            Some(&[Reason::Protocol]),
        ),
        ("a body to the connection's end", b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nhello", None),
    ];
    for (what, answer, reasons) in cases {
        let answer = answer.to_vec();
        let port = raw_server(move |mut socket| {
            read_head(&mut socket);
            let _ = socket.write_all(&answer);
        });
        let mut client = Client::new(&format!("http://site.test:{port}/a/"));
        let outcome = client.get(&format!("http://site.test:{port}/a/x"));
        match reasons {
            Some(reasons) => {
                let reason = failure(&outcome);
                assert!(reasons.contains(&reason), "{what}: {reason:?}");
                if what != "a chunk size that is no number" {
                    assert!(outcome.head.is_some(), "{what}: the head came first");
                }
            }
            None => assert_eq!(outcome.text(), "hello", "{what}"),
        }
        client.finish().unwrap();
    }
}

/// A body that never ends, read by a script that makes no room: no more
/// than the first room crosses the pipe, the memory stays flat, and an
/// abort ends it at once. All the body's events are on the pipe before
/// the abort's answer, so what came before it is all that came.
#[test]
fn an_endless_body_fills_no_more_than_its_room() {
    let peak = Peak::start();
    let port = raw_server(|mut socket| {
        read_head(&mut socket);
        let mut chunk = format!("{FLOOD_PIECE:x}\r\n").into_bytes();
        chunk.extend_from_slice(&[b'x'; FLOOD_PIECE]);
        chunk.extend_from_slice(b"\r\n");
        flood(&mut socket, b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n", &chunk, Duration::ZERO);
    });
    let mut client = Client::new(&format!("http://site.test:{port}/a/"));
    client.makes_room = false;
    let id = client.start(&format!("http://site.test:{port}/a/endless"), |_| {});
    let received = room_then_abort(&mut client, id, usize::try_from(FETCH_WINDOW_BYTES).unwrap());
    let window = usize::try_from(FETCH_WINDOW_BYTES).unwrap();
    assert!(received <= window && received > window - MAX_CHUNK_BYTES, "{received} bytes came into room for {window}");
    client.finish().unwrap();
    peak.check("an endless body");
}

/// Reads the request `id`'s body until `room` bytes less a piece have
/// come, waits, aborts, and counts every byte that came before the
/// abort's answer.
fn room_then_abort(client: &mut Client, id: u32, room: usize) -> usize {
    let mut received = 0;
    let mut aborted = false;
    loop {
        match client.next_event() {
            FromWebClient::Head(head) => assert_eq!(head.id, id),
            FromWebClient::Body { id: of, bytes } => {
                assert!(of == id && bytes.len() <= MAX_CHUNK_BYTES);
                received += bytes.len();
            }
            FromWebClient::Failed { id: of, error: ErrorKind::Abort, reason: Reason::Aborted } if of == id && aborted => return received,
            other => panic!("{other:?}"),
        }
        if !aborted && received > room - MAX_CHUNK_BYTES {
            // Time for anything past the room to come, if it would.
            std::thread::sleep(Duration::from_millis(300));
            client.send(&ToWebClient::Abort(id));
            aborted = true;
        }
    }
}

/// The room granted never passes its cap, end to end (`protocol::web`'s
/// `MAX_NETWORK_BUFFER_BYTES`, DEC-7318 item 4): a script host that gives
/// room for far more than it has read, before the answer's body begins,
/// is sent no more than the cap. This is the end-to-end form of the unit
/// test `client::tests::room_and_starts_are_bounded`: the room is given
/// before the server answers, and the bytes are counted up to the abort's
/// answer, after which the pipe carries nothing more of the request, so
/// that nothing in it waits on a quiet spell.
#[test]
fn granted_room_never_passes_its_cap_end_to_end() {
    let port = raw_server(|mut socket| {
        read_head(&mut socket);
        // The room is given while the server waits.
        std::thread::sleep(Duration::from_millis(300));
        let mut chunk = format!("{FLOOD_PIECE:x}\r\n").into_bytes();
        chunk.extend_from_slice(&[b'y'; FLOOD_PIECE]);
        chunk.extend_from_slice(b"\r\n");
        flood(&mut socket, b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n", &chunk, Duration::ZERO);
    });
    let mut client = Client::new(&format!("http://site.test:{port}/a/"));
    client.makes_room = false;
    let id = client.start(&format!("http://site.test:{port}/a/endless"), |_| {});
    for _ in 0..3 {
        client.send(&ToWebClient::More { id, bytes: MAX_NETWORK_BUFFER_BYTES });
    }
    let cap = usize::try_from(MAX_NETWORK_BUFFER_BYTES).unwrap();
    let received = room_then_abort(&mut client, id, cap);
    assert!(received <= cap && received > cap - MAX_CHUNK_BYTES, "{received} bytes came into room capped at {cap}");
    client.finish().unwrap();
}

/// A fetch's body that falls silent fails at [`BODY_IDLE_TIMEOUT`], two
/// minutes, which no test can shorten.
#[test]
#[ignore = "takes BODY_IDLE_TIMEOUT, 120 seconds, which has no seam to shorten it: run by hand with --ignored"]
fn a_body_that_falls_silent_fails_at_its_idle_limit() {
    let port = raw_server(|mut socket| {
        read_head(&mut socket);
        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nsome");
        std::thread::sleep(BODY_IDLE_TIMEOUT + Duration::from_secs(30));
    });
    let mut client = Client::new(&format!("http://site.test:{port}/a/"));
    let id = client.start(&format!("http://site.test:{port}/a/x"), |_| {});
    let asked = Instant::now();
    let mut outcome = None;
    while outcome.is_none() {
        match client.next_within(BODY_IDLE_TIMEOUT + LATE) {
            Some(FromWebClient::Failed { id: of, error, reason }) if of == id => outcome = Some((error, reason)),
            Some(FromWebClient::Head(_) | FromWebClient::Body { .. }) => {}
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(outcome, Some((ErrorKind::Timeout, Reason::Timeout)));
    assert!(asked.elapsed() >= BODY_IDLE_TIMEOUT && asked.elapsed() <= BODY_IDLE_TIMEOUT + LATE, "{:?}", asked.elapsed());
    client.finish().unwrap();
}

/// "Hello" at 65 MiB of zeros, as Google's brotli encoder 1.2.1 writes it
/// at quality 11 with a 16 MiB window (made on 22 November 2026 with its
/// Python binding run by `uvx`; the browser has no brotli encoder):
/// 66 bytes, SHA-256 fc955f9f…31be.
const BROTLI_BOMB: [u8; 66] = [
    0xCF, 0xFF, 0xFF, 0x7F, 0xF8, 0x27, 0x00, 0xE2, 0xB1, 0x40, 0x20, 0xF7, 0xFE, 0x9F, 0xFF, 0xFF, 0xFF, 0xF0, 0x4F, 0x00, 0xC4, 0x61, 0x01, 0x80, 0xEE, 0xFD,
    0x3F, 0xFF, 0xFF, 0xFF, 0xE1, 0x9F, 0x00, 0x88, 0xC3, 0x22, 0x00, 0xDD, 0xFB, 0x7F, 0xFE, 0xFF, 0xFF, 0xC3, 0x3F, 0x01, 0x10, 0x87, 0x05, 0x00, 0xBA, 0xF7,
    0xFF, 0xF5, 0xFF, 0xFF, 0xF8, 0x27, 0x00, 0xE2, 0xB0, 0x00, 0x40, 0xF7, 0xFE, 0x01,
];
/// What [`BROTLI_BOMB`] decodes to: 65 MiB.
const BROTLI_BOMB_BYTES: u64 = 65 * 1024 * 1024;

/// `body` as a zlib stream, the `deflate` coding as servers write it.
fn zlib(body: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(body).unwrap();
    encoder.finish().unwrap()
}

/// TODO-7321 item 7, `MAX_DECOMPRESSED_RATIO` of `docs/script-api.md`
/// section 10.6: a coded body that decodes to more than a hundred times
/// its coded length fails as too large once it is past the floor, having
/// given little more than the floor; a short one of the same ratio, and
/// a long one of an ordinary ratio, are read whole.
#[test]
fn a_body_that_decodes_past_the_ratio_fails_past_the_floor() {
    let floor = usize::try_from(DECODED_RATIO_FLOOR).unwrap();
    let dense = gzip(&vec![0_u8; 4 * floor]);
    assert!(u64::try_from(dense.len()).unwrap() * MAX_DECOMPRESSED_RATIO < DECODED_RATIO_FLOOR, "{} bytes coded: past the ratio at the floor", dense.len());
    let short = gzip(&vec![0_u8; floor / 2]);
    // Bytes that code to about a third: no two runs alike for long.
    let ordinary: Vec<u8> = (0..4 * floor).map(|index| b"0123456789abcdef"[((index * 2_654_435_761) >> 13) % 16]).collect();
    let coded = gzip(&ordinary);
    assert!(
        u64::try_from(coded.len()).unwrap() * MAX_DECOMPRESSED_RATIO > u64::try_from(ordinary.len()).unwrap(),
        "{} bytes coded: within the ratio",
        coded.len()
    );
    let server = serve(move |request| {
        let body = match request.target.as_str() {
            "/a/dense" => &dense,
            "/a/short" => &short,
            _ => &coded,
        };
        Reply::new(200).header("content-encoding", "gzip").body(body)
    });
    let mut client = Client::new(&server.url("/a/"));
    let outcome = client.get(&server.url("/a/dense"));
    assert_eq!(outcome.failed, Some((ErrorKind::Network, Reason::TooLarge)));
    assert!(outcome.body.len() <= floor + PIECE, "{} bytes given", outcome.body.len());
    let outcome = client.get(&server.url("/a/short"));
    assert_eq!((outcome.failed, outcome.body.len()), (None, floor / 2));
    let outcome = client.get(&server.url("/a/ordinary"));
    assert_eq!(outcome.failed, None);
    assert_eq!(outcome.body.len(), ordinary.len());
    assert!(outcome.body.iter().eq(ordinary.iter()), "the bytes as they were coded");
    client.finish().unwrap();
}

/// A small body that decodes past [`MAX_DECODED_BODY`] fails as too
/// large in each coding, having given no more than the bound, with the
/// memory flat: gzip, deflate and brotli.
#[test]
fn a_decompression_bomb_fails_in_each_coding() {
    const { assert!(BROTLI_BOMB_BYTES > MAX_DECODED_BODY, "the bomb decodes past the bound") };
    let past = vec![0_u8; usize::try_from(MAX_DECODED_BODY).unwrap() + 1024 * 1024];
    let bombs = [("gzip", gzip(&past)), ("deflate", zlib(&past)), ("br", BROTLI_BOMB.to_vec())];
    for (coding, coded) in bombs {
        assert!(coded.len() < 128 * 1024, "{coding}: {} bytes coded", coded.len());
        let peak = Peak::start();
        let server = serve(move |_| Reply::new(200).header("content-encoding", coding).body(&coded));
        let mut client = Client::new(&server.url("/a/"));
        let asked = Instant::now();
        let outcome = client.get(&server.url("/a/bomb"));
        eprintln!("{coding} bomb: {:?} after {:?}, {} bytes given", outcome.failed, asked.elapsed(), outcome.body.len());
        assert_eq!(outcome.failed, Some((ErrorKind::Network, Reason::TooLarge)), "{coding}");
        assert!(u64::try_from(outcome.body.len()).unwrap() <= MAX_DECODED_BODY, "{coding}");
        assert!(asked.elapsed() < Duration::from_secs(20), "{coding}: {:?}", asked.elapsed());
        client.finish().unwrap();
        peak.check(coding);
    }
}

/// An event stream that never ends a line, or never ends an event, fails
/// as too large at its bound and is not opened again.
#[test]
fn an_event_stream_without_end_fails_at_its_bound() {
    let flooded = [("a line without end", vec![b'a'; FLOOD_PIECE]), ("data without a blank line", b"data: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n".repeat(512))];
    for (what, again) in flooded {
        let peak = Peak::start();
        let port = raw_server(move |mut socket| {
            read_head(&mut socket);
            flood(&mut socket, b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n", &again, Duration::ZERO);
        });
        let mut client = Client::new(&format!("http://site.test:{port}/a/"));
        let id = client.start(&format!("http://site.test:{port}/a/events"), |start| start.kind = RequestKind::EventSource);
        assert_eq!(client.next_event(), FromWebClient::Opened { id, protocol: String::new(), url: format!("http://site.test:{port}/a/events") }, "{what}");
        assert_eq!(client.next_event(), FromWebClient::Failed { id, error: ErrorKind::Network, reason: Reason::TooLarge }, "{what}");
        assert_eq!(client.next_within(Duration::from_millis(1500)), None, "{what}: not opened again");
        client.finish().unwrap();
        peak.check(what);
    }
}

/// A redirect loop between two origins ends at the standard's limit, each
/// hop asked once; a redirect into another channel of the origin carries
/// none of this channel's cookies and is read only by that channel's
/// leave.
#[test]
fn redirects_across_origins_and_partitions_are_bounded() {
    let server = serve(|asked| {
        let host = asked.header("host").unwrap_or_default();
        let other = if host.starts_with("site.test") { "other.test" } else { "site.test" };
        match asked.target.as_str() {
            "/a/loop" => Reply::to(302, &format!("http://{other}:{}/a/loop", host.rsplit(':').next().unwrap())).header("access-control-allow-origin", "*"),
            "/a/login" => Reply::ok("in").header("set-cookie", "sid=secret; Path=/"),
            "/a/go" => Reply::to(302, "/b/inside"),
            _ => Reply::ok("other channel's"),
        }
    });
    let mut client = Client::new(&server.url("/a/"));
    assert_eq!(client.get(&server.url("/a/loop")).reason(), Reason::RedirectLimit);
    assert_eq!(server.count("GET", "/a/loop"), MAX_REDIRECTS + 1, "the first request and twenty redirects");
    assert_eq!(client.get(&server.url("/a/login")).text(), "in");
    assert_eq!(client.fetch(&server.url("/a/go"), |start| start.credentials = Credentials::SameOrigin).reason(), Reason::Cors);
    let inside = server.the("/b/inside");
    assert_eq!(inside.header("cookie"), None, "no cookie of this channel goes to another");
    assert_eq!(inside.header("origin"), Some(format!("http://site.test:{}", server.port)));
    client.finish().unwrap();
}

/// A channel from the internet that is no secure context, its server's
/// address taken as public by `--allow-local`, is not led to this
/// computer by a redirect, to an address or to a name; nor by a name that
/// resolves to the public address first and to this computer the second
/// time (DNS rebinding), each connection's address being checked anew.
#[test]
fn a_redirect_or_a_second_resolution_does_not_lead_to_this_computer() {
    let local = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
    let reached = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&reached);
    let inside = raw_server_at(local, move |mut socket| {
        counted.fetch_add(1, Ordering::SeqCst);
        read_head(&mut socket);
        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nrouter");
    });
    let server = serve(move |asked| match asked.target.as_str() {
        "/a/to-address" => Reply::to(302, &format!("http://127.0.0.2:{inside}/admin")),
        "/a/to-name" => Reply::to(302, &format!("http://router.test:{inside}/admin")),
        _ => Reply::ok("public").header("connection", "close"),
    });
    let port = server.port;
    let resolutions = Arc::new(Mutex::new(0_usize));
    let resolved = Arc::clone(&resolutions);
    let mut client = Client::with(&server.url("/a/"), move |setup| {
        setup.space = protocol::web::AddressSpace::Public;
        setup.settings.allowed = vec![HERE];
        setup.settings.names = Some(Rc::new(move |name: &str, asked: u16| {
            let mut count = resolved.lock().unwrap();
            *count += 1;
            match name {
                "router.test" => vec![SocketAddr::new(local, asked)],
                // Public the first time, this computer after.
                "rebind.test" if *count > 1 => vec![SocketAddr::new(local, inside)],
                _ => vec![SocketAddr::new(HERE, asked)],
            }
        }));
    });
    assert_eq!(client.get(&server.url("/a/to-address")).reason(), Reason::LocalRefused);
    assert_eq!(client.get(&server.url("/a/to-name")).reason(), Reason::LocalRefused);
    *resolutions.lock().unwrap() = 0;
    let rebound = format!("http://rebind.test:{port}/a/x");
    let first = client.fetch(&rebound, |start| start.mode = protocol::web::Mode::Cors);
    assert_eq!(first.reason(), Reason::Cors, "the public server answered, and gave no leave to read it");
    assert_eq!(client.get(&rebound).reason(), Reason::LocalRefused, "the second resolution's address is refused");
    assert_eq!(reached.load(Ordering::SeqCst), 0, "nothing on this computer's other address was ever reached");
    client.finish().unwrap();
}

/// The records of a jar as written out: how many cookies it holds.
fn records(mut jar: &[u8]) -> usize {
    let mut count = 0;
    while !jar.is_empty() {
        let origin = usize::from(u16::from_be_bytes([jar[0], jar[1]]));
        jar = &jar[2 + origin..];
        let text = usize::try_from(u32::from_be_bytes([jar[0], jar[1], jar[2], jar[3]])).unwrap();
        jar = &jar[4 + text + 16..];
        count += 1;
    }
    count
}

/// A flood of `Set-Cookie` headers, malformed and well formed, within the
/// head's bound: the jar keeps to its count for a domain, what is written
/// out keeps to the pipe's bound, and the request after carries no more
/// than the jar holds.
#[test]
fn a_set_cookie_flood_keeps_the_jar_to_its_bounds() {
    let peak = Peak::start();
    let server = serve(|asked| {
        let round: usize = asked.target.rsplit('/').next().unwrap().parse().unwrap_or(0);
        let mut reply = Reply::ok("fed");
        for index in 0..60 {
            reply = match index % 4 {
                0 => reply.header("set-cookie", &format!("c{round}x{index}={}; Path=/; Max-Age=600", "v".repeat(200))),
                1 => reply.header("set-cookie", &format!("=;;;===; Domain=;;{}", ";".repeat(300))),
                2 => reply.header("set-cookie", &format!("__Host-{index}=1; Domain=site.test; {}", "Secure; ".repeat(40))),
                _ => reply.header("set-cookie", &format!("s{round}x{index}=1; Expires={}; SameSite=None", "x".repeat(1100))),
            };
        }
        reply
    });
    let mut client = Client::new(&server.url("/a/"));
    for round in 1..=5 {
        assert_eq!(client.get(&server.url(&format!("/a/feed/{round}"))).text(), "fed");
    }
    let (lasting, session) = (records(&client.jar[0]), records(&client.jar[1]));
    assert!(client.jar[0].len() + client.jar[1].len() <= MAX_JAR_BYTES);
    assert_eq!(lasting + session, MAX_COOKIES_PER_DOMAIN, "{lasting} lasting and {session} session cookies");
    client.get(&server.url("/a/after"));
    let sent = server.the("/a/after").header("cookie").unwrap_or_default();
    assert_eq!(sent.split("; ").count(), MAX_COOKIES_PER_DOMAIN);
    client.finish().unwrap();
    peak.check("a Set-Cookie flood");
}

/// The Fetch standard's CORS-preflight fetch: an answer of each malformed
/// kind is no leave, and the request itself is never made.
#[test]
fn a_malformed_preflight_answer_is_no_leave() {
    let server = serve(|asked| {
        let origin = asked.header("origin").unwrap_or_default();
        let fine = |reply: Reply| reply.header("access-control-allow-methods", "PUT").header("access-control-allow-headers", "x-token");
        if asked.method != "OPTIONS" {
            return Reply::ok("done").header("access-control-allow-origin", &origin);
        }
        match asked.target.as_str() {
            "/no-origin" => fine(Reply::new(204)),
            "/wrong-origin" => fine(Reply::new(204).header("access-control-allow-origin", "http://elsewhere.test")),
            "/two-origins" => fine(Reply::new(204).header("access-control-allow-origin", &origin).header("access-control-allow-origin", &origin)),
            "/methods-not-a-list" => Reply::new(204).header("access-control-allow-origin", &origin).header("access-control-allow-methods", "PUT PATCH"),
            "/headers-not-a-list" => Reply::new(204)
                .header("access-control-allow-origin", &origin)
                .header("access-control-allow-methods", "PUT")
                .header("access-control-allow-headers", "x-token;x"),
            "/server-error" => fine(Reply::new(500).header("access-control-allow-origin", &origin)),
            "/a-redirect" => fine(Reply::to(307, "/fine").header("access-control-allow-origin", &origin)),
            "/an-interim-like-status" => fine(Reply::new(199).header("access-control-allow-origin", &origin)),
            _ => fine(Reply::new(204).header("access-control-allow-origin", &origin)).header("access-control-max-age", "-5"),
        }
    });
    let mut client = Client::new(&server.url("/a/"));
    let put = |start: &mut protocol::web::Start| (start.method, start.headers) = ("PUT".into(), vec![header("x-token", "t")]);
    for path in ["/no-origin", "/wrong-origin", "/two-origins", "/methods-not-a-list", "/headers-not-a-list", "/server-error", "/a-redirect"] {
        let outcome = client.fetch(&server.at("http", "other.test", path), put);
        assert_eq!(outcome.reason(), Reason::Preflight, "{path}");
        assert_eq!(server.count("PUT", path), 0, "{path}: the request was never made");
    }
    // A max-age that is no number is the default, and the leave stands.
    assert_eq!(client.fetch(&server.at("http", "other.test", "/fine"), put).text(), "done");
    client.finish().unwrap();
}

/// An HTTP/2 frame (RFC 9113 section 4.1).
fn frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(payload.len()).unwrap().to_be_bytes();
    let mut frame = vec![length[1], length[2], length[3], kind, flags];
    frame.extend_from_slice(&stream.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// RFC 9113 section 6: the frame types and flags a hostile server writes.
const DATA: u8 = 0x0;
const HEADERS: u8 = 0x1;
const RST_STREAM: u8 = 0x3;
const SETTINGS: u8 = 0x4;
const PING: u8 = 0x6;
const CONTINUATION: u8 = 0x9;
const END_HEADERS: u8 = 0x4;
/// `SETTINGS_INITIAL_WINDOW_SIZE` (RFC 9113 section 6.5.2).
const INITIAL_WINDOW_SIZE: u16 = 0x4;
/// HPACK's static table entry 8, `:status: 200` (RFC 7541 appendix A),
/// as an indexed field (section 6.1).
const STATUS_200: u8 = 0x88;
/// The client preface (RFC 9113 section 3.4).
const PREFACE_BYTES: usize = 24;

/// An HPACK integer with an `prefix`-bit prefix after `flags` (RFC 7541
/// section 5.1).
fn hpack_integer(flags: u8, prefix: u32, mut value: usize) -> Vec<u8> {
    let most = (1_usize << prefix) - 1;
    if value < most {
        return vec![flags | u8::try_from(value).unwrap()];
    }
    let mut out = vec![flags | u8::try_from(most).unwrap()];
    value -= most;
    while value >= 128 {
        out.push(u8::try_from(value % 128).unwrap() | 0x80);
        value /= 128;
    }
    out.push(u8::try_from(value).unwrap());
    out
}

/// A field as a literal with a new name, `indexed` into the dynamic table
/// or not (RFC 7541 sections 6.2.1 and 6.2.2), its strings plain.
fn literal(name: &str, value: &[u8], indexed: bool) -> Vec<u8> {
    let mut out = if indexed { vec![0x40] } else { vec![0x00] };
    out.extend(hpack_integer(0, 7, name.len()));
    out.extend_from_slice(name.as_bytes());
    out.extend(hpack_integer(0, 7, value.len()));
    out.extend_from_slice(value);
    out
}

/// An HTTP/2 server over TLS that reads the client's preface, writes its
/// own `settings` frame, then runs `script` on the connection.
fn h2_server(settings: Vec<u8>, script: impl Fn(&mut dyn Write) + Send + Sync + 'static) -> u16 {
    let (certificate, key) = certificate();
    let mut config = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![certificate], key).unwrap();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let config = Arc::new(config);
    let script = Arc::new(script);
    raw_server(move |socket| {
        let connection = rustls::ServerConnection::new(Arc::clone(&config)).unwrap();
        let mut stream = rustls::StreamOwned::new(connection, socket);
        let mut preface = [0_u8; PREFACE_BYTES];
        if stream.read_exact(&mut preface).is_err() || stream.write_all(&frame(SETTINGS, 0, 0, &settings)).is_err() {
            return;
        }
        script(&mut stream);
    })
}

/// What one HTTP/2 abuse is: its name, the settings the server opens
/// with, what it then writes once and over and over, and the reasons the
/// request may fail with.
struct Abuse {
    what: &'static str,
    settings: Vec<u8>,
    first: Vec<u8>,
    again: Vec<u8>,
    reasons: &'static [Reason],
    /// Whether the request carries a body larger than any window.
    upload: bool,
    /// Whether the script makes room only after a while.
    reads_late: bool,
}

/// The abuses of HTTP/2 a server can make that the h2 crate leaves its
/// caller to bound.
fn abuses() -> Vec<Abuse> {
    let failing: &[Reason] = &[Reason::Timeout, Reason::Protocol, Reason::Connection];
    let status = frame(HEADERS, END_HEADERS, 1, &[STATUS_200]);
    let mut huge = vec![STATUS_200];
    huge.extend(literal("x-huge", &vec![b'h'; 70 * 1024], false));
    let mut split = Vec::new();
    for (place, piece) in huge.chunks(FLOOD_PIECE).enumerate() {
        let last = (place + 1) * FLOOD_PIECE >= huge.len();
        split.extend(frame(if place == 0 { HEADERS } else { CONTINUATION }, if last { END_HEADERS } else { 0 }, 1, piece));
    }
    let mut bomb = vec![STATUS_200];
    bomb.extend(literal("x-a", &[b'b'; 4000], true));
    // The dynamic table's first entry, index 62 (RFC 7541 section 2.3.3),
    // named again and again.
    bomb.extend(std::iter::repeat_n(0xBE, 200));
    let mut window = INITIAL_WINDOW_SIZE.to_be_bytes().to_vec();
    window.extend_from_slice(&0_u32.to_be_bytes());
    vec![
        Abuse {
            what: "a settings flood",
            settings: Vec::new(),
            first: Vec::new(),
            again: frame(SETTINGS, 0, 0, &[0, 3, 0, 0, 0, 100]),
            reasons: failing,
            upload: false,
            reads_late: false,
        },
        Abuse {
            what: "a ping flood",
            settings: Vec::new(),
            first: Vec::new(),
            again: frame(PING, 0, 0, &[7; 8]),
            reasons: failing,
            upload: false,
            reads_late: false,
        },
        Abuse {
            what: "a reset of the request",
            settings: Vec::new(),
            first: Vec::new(),
            again: frame(RST_STREAM, 0, 1, &[0, 0, 0, 2]),
            reasons: failing,
            upload: false,
            reads_late: false,
        },
        Abuse {
            what: "resets of streams never opened",
            settings: Vec::new(),
            first: Vec::new(),
            again: frame(RST_STREAM, 0, 4, &[0, 0, 0, 8]),
            reasons: failing,
            upload: false,
            reads_late: false,
        },
        Abuse {
            what: "a continuation flood",
            settings: Vec::new(),
            first: frame(HEADERS, 0, 1, &[STATUS_200]),
            again: frame(CONTINUATION, 0, 1, &literal("x-c", &[b'c'; 1000], false)),
            reasons: &[Reason::Timeout, Reason::Protocol, Reason::Connection, Reason::TooLarge],
            upload: false,
            reads_late: false,
        },
        Abuse {
            what: "a header list past the bound",
            settings: Vec::new(),
            first: split,
            again: Vec::new(),
            reasons: &[Reason::Protocol, Reason::Connection, Reason::TooLarge],
            upload: false,
            reads_late: false,
        },
        Abuse {
            what: "an HPACK bomb",
            settings: Vec::new(),
            first: frame(HEADERS, END_HEADERS, 1, &bomb),
            again: Vec::new(),
            reasons: &[Reason::Protocol, Reason::Connection, Reason::TooLarge],
            upload: false,
            reads_late: false,
        },
        Abuse {
            what: "a window never opened",
            settings: window,
            first: Vec::new(),
            again: Vec::new(),
            reasons: &[Reason::Timeout],
            upload: true,
            reads_late: false,
        },
        Abuse {
            what: "data past the window",
            settings: Vec::new(),
            first: status,
            again: frame(DATA, 0, 1, &[b'd'; FLOOD_PIECE]),
            reasons: &[Reason::Protocol, Reason::Connection],
            upload: false,
            reads_late: true,
        },
    ]
}

/// Each abuse ends the request within the head's time limit, or at once,
/// with the memory flat. "Data past the window" is read by a script that
/// makes no room, so the client stops opening its window; a server that
/// sends on regardless breaks flow control (RFC 9113 section 6.9.1).
#[test]
fn http2_abuse_ends_within_its_bounds() {
    for abuse in abuses() {
        let peak = Peak::start();
        let (first, again) = (abuse.first.clone(), abuse.again.clone());
        let port = h2_server(abuse.settings.clone(), move |stream| {
            if again.is_empty() {
                let _ = stream.write_all(&first);
                let _ = stream.flush();
                std::thread::sleep(SERVER_LIFETIME);
            } else {
                flood(stream, &first, &again, Duration::ZERO);
            }
        });
        let mut client = short_client(&format!("https://localhost:{port}"));
        client.makes_room = false;
        let id = client.start(&format!("https://localhost:{port}/a/x"), |start| {
            if abuse.upload {
                (start.method, start.body) = ("POST".into(), protocol::web::BodySource::Bytes(256 * 1024));
            }
        });
        if abuse.upload {
            for _ in 0..4 {
                client.send(&ToWebClient::RequestBody { id, bytes: vec![b'u'; MAX_CHUNK_BYTES] });
            }
            client.send(&ToWebClient::RequestBodyEnd(id));
        }
        let asked = Instant::now();
        let mut reason = None;
        while reason.is_none() {
            match client.next_event() {
                FromWebClient::Failed { id: of, reason: why, .. } if of == id => reason = Some(why),
                // Room once the server has had time to pass the window, so
                // the body is read on to the fault.
                FromWebClient::Body { .. } if abuse.reads_late => {
                    std::thread::sleep(Duration::from_millis(500));
                    client.send(&ToWebClient::More { id, bytes: MAX_NETWORK_BUFFER_BYTES });
                }
                FromWebClient::Head(_) | FromWebClient::Body { .. } | FromWebClient::Sent { .. } => {}
                FromWebClient::End(of) if of == id => panic!("{}: the request ended well", abuse.what),
                other => panic!("{}: {other:?}", abuse.what),
            }
        }
        let (reason, took) = (reason.unwrap(), asked.elapsed());
        eprintln!("{}: {reason:?} after {took:?}", abuse.what);
        assert!(abuse.reasons.contains(&reason), "{}: {reason:?}", abuse.what);
        assert!(took <= SHORT + LATE, "{}: ended after {took:?}", abuse.what);
        client.finish().unwrap();
        peak.check(abuse.what);
    }
}
