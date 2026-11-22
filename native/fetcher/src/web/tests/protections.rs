//! The protections of the channel fetcher's client, rebuilt over hyper:
//! the local network asked only as DEC-7315 allows, no downgrade, the one deadline over a
//! connection's opening, the proxy's tunnel, strict heads, and a page
//! load that says nothing of the viewer; and the web client's own: one
//! partition a process, the channel as the origin, and its limits.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::rc::Rc;
use std::time::{Duration, Instant};

use neoteletext_web::proxy::Proxy;
use protocol::web::{AddressSpace, ErrorKind, FromWebClient, MAX_HEADER_VALUE_BYTES, MAX_RESPONSE_HEADERS, Reason, ResponseType, ToWebClient};

use protocol::script::ContextId;

use super::support::{Client, HERE, OTHER_TOKEN, Piece, Reply, TAB, raw, serve, serve_tls};
use crate::web::limits::{MAX_REQUESTS_PER_TAB, START_BURST};

/// neoteletext-common's DEC-7315, the browser's TODO-7316 items 5 to 7,
/// and finding F-62: this computer and the local network are reached as
/// that decision allows, by the address space the trusted core told the
/// process its channel was loaded from. A channel loaded from this
/// computer or the local network asks them freely. A channel from the
/// internet is refused unless it is a secure context, and then needs the
/// viewer's permission, one for this computer and one for the local
/// network, whatever name resolved to the address; until the browser can
/// ask, that is a refusal by its own name.
#[test]
fn a_public_channel_asks_this_computer_and_the_local_network_only_by_permission() {
    let server = serve(|_| Reply::ok("local"));
    let channel = server.url("/a/");
    let here = [
        server.url("/a/x"),
        server.at("http", "127.0.0.1", "/"),
        server.at("http", "localhost", "/"),
        server.at("http", "[::1]", "/"),
        server.at("http", "[::ffff:127.0.0.1]", "/"),
        server.at("http", "0x7f.1", "/"),
    ];
    let nearby =
        ["http://10.0.0.1/", "http://192.168.1.1/", "http://169.254.169.254/latest/meta-data/", "http://[fe80::1]/", "http://[fd00::1]/", "http://100.64.0.1/"];
    // A channel loaded from this computer: reached, by name and by address.
    let mut client = Client::new(&channel);
    assert_eq!(client.get(&server.url("/a/x")).text(), "local");
    assert_eq!(client.get(&server.at("http", "127.0.0.1", "/a/x")).reason(), Reason::Cors, "reached, and another origin");
    client.finish().unwrap();
    // A channel on the local network asks this computer freely too, and
    // the local network: no permission is wanted, whatever then becomes
    // of the connection.
    let mut local = Client::with(&channel, |setup| (setup.space, setup.settings.open_deadline) = (AddressSpace::Local, Duration::from_millis(300)));
    assert_eq!(local.get(&server.url("/a/x")).text(), "local");
    let outcome = local.get("http://10.255.255.1/");
    assert!(matches!(outcome.reason(), Reason::Timeout | Reason::Connection), "{outcome:?}");
    local.finish().unwrap();
    let before = server.asked().len();
    // A public channel over plain http: refused without asking, for the
    // name that resolves here and for the address in each of its
    // spellings. What the script host says of its page changes nothing:
    // the space is the trusted core's word.
    let mut public = Client::with(&channel, |setup| setup.space = AddressSpace::Public);
    for address in here.iter().map(String::as_str).chain(nearby) {
        assert_eq!(public.get(address).reason(), Reason::LocalRefused, "{address}");
    }
    assert_eq!(public.fetch(&server.url("/a/x"), |start| start.client = "http://127.0.0.1/a/".into()).reason(), Reason::LocalRefused);
    assert_eq!(server.asked().len(), before, "nothing was connected to");
    public.finish().unwrap();
    // A public channel that is a secure context: it needs permission, for
    // this computer and for the local network apart. A plain address is
    // mixed content to it before anything else, so each is asked over
    // https.
    let secure = serve_tls(&["http/1.1"], |_| Reply::ok("secure"));
    let mut secure_public = Client::with(&secure.at("https", "public.test", "/a/"), |setup| setup.space = AddressSpace::Public);
    assert_eq!(secure_public.get(&secure.at("https", "localhost", "/a/x")).reason(), Reason::DevicePermission);
    assert_eq!(secure_public.get(&secure.at("https", "public.test", "/a/x")).reason(), Reason::DevicePermission, "its own name, now resolving here");
    for address in ["https://10.0.0.1/", "https://192.168.1.1/", "https://[fe80::1]/"] {
        assert_eq!(secure_public.get(address).reason(), Reason::LocalPermission, "{address}");
    }
    assert!(secure.asked().is_empty(), "nothing was connected to");
    secure_public.finish().unwrap();
    // An address the viewer named when the browser started is taken as
    // public: open to a public channel, and nothing else is.
    let mut named = Client::with(&channel, |setup| (setup.space, setup.settings.allowed) = (AddressSpace::Public, vec![HERE]));
    assert_eq!(named.get(&server.url("/a/x")).text(), "local");
    assert_eq!(named.get("http://192.168.1.1/").reason(), Reason::LocalRefused, "and nothing else");
    named.finish().unwrap();
}

/// The engine's REQ-SEC-007: no redirect leads from `https` to `http`;
/// and a page loaded securely asks nothing of a plain address.
#[test]
fn nothing_goes_from_https_down_to_http() {
    let plain = serve(|_| Reply::ok("plain"));
    let target = plain.at("http", "localhost", "/a/x");
    let secure = serve_tls(&["http/1.1"], move |asked| if asked.target == "/a/down" { Reply::to(302, &target) } else { Reply::ok("secure") });
    let mut client = Client::new(&secure.at("https", "localhost", "/a/"));
    assert_eq!(client.get(&secure.at("https", "localhost", "/a/x")).text(), "secure");
    assert_eq!(client.get(&secure.at("https", "localhost", "/a/down")).reason(), Reason::Downgrade);
    assert_eq!(client.get("http://elsewhere.test/x").reason(), Reason::MixedContent);
    assert!(plain.asked().is_empty());
    // Up from http to https is followed.
    let up = secure.at("https", "localhost", "/a/x");
    let from_plain = serve(move |_| Reply::to(302, &up).header("access-control-allow-origin", "*"));
    let mut plain_client = Client::new(&from_plain.url("/a/"));
    assert_eq!(plain_client.get(&from_plain.url("/a/up")).reason(), Reason::Cors, "followed, to another origin that gave no leave");
    assert_eq!(secure.count("GET", "/a/x"), 2);
    client.finish().unwrap();
    plain_client.finish().unwrap();
}

/// TLS as the channel fetcher has it: a certificate the roots do not
/// trust, or for another name, is refused and told apart.
#[test]
fn a_certificate_that_fails_is_a_hard_stop() {
    let secure = serve_tls(&["http/1.1"], |_| Reply::ok("secure"));
    let mut wrong_name = Client::new(&secure.at("https", "site.test", "/a/"));
    assert_eq!(wrong_name.get(&secure.at("https", "site.test", "/a/x")).reason(), Reason::Certificate, "the certificate is localhost's");
    wrong_name.finish().unwrap();
    let mut untrusting = Client::with(&secure.at("https", "localhost", "/a/"), |setup| {
        let (allowed, names) = (setup.settings.allowed.clone(), setup.settings.names.take());
        setup.settings = crate::web::connect::Settings::new(&allowed, None, neoteletext_transport::mozilla_roots());
        setup.settings.names = names;
    });
    assert_eq!(untrusting.get(&secure.at("https", "localhost", "/a/x")).reason(), Reason::Certificate);
    assert!(secure.asked().is_empty());
    untrusting.finish().unwrap();
}

/// Finding F-63: one deadline is over a connection's whole opening, and
/// an answer's head has its own time limit.
#[test]
fn a_server_that_says_nothing_fails_in_time() {
    let silent = raw(None);
    let short = Duration::from_millis(300);
    let mut client = Client::with(&format!("http://site.test:{silent}/a/"), move |setup| {
        (setup.settings.open_deadline, setup.settings.head_timeout) = (short, short);
    });
    // The connection opens and no head comes.
    let asked = Instant::now();
    assert_eq!(client.get(&format!("http://site.test:{silent}/a/x")).reason(), Reason::Timeout);
    // The TLS handshake is never answered: the opening's deadline ends it.
    assert_eq!(client.get(&format!("https://localhost:{silent}/a/x")).reason(), Reason::Timeout);
    assert!(asked.elapsed() >= 2 * short && asked.elapsed() < Duration::from_secs(5), "each failed at its limit, not later");
    // Nothing listens: a failure of the connection, at once.
    let closed = TcpListener::bind((HERE, 0)).unwrap().local_addr().unwrap().port();
    assert_eq!(client.get(&format!("http://site.test:{closed}/a/x")).reason(), Reason::Connection);
    client.finish().unwrap();
}

/// The engine's REQ-SEC-011, kept over hyper: an answer's head that is
/// folded, that frames its body twice, or that passes a limit on its
/// size or count, is refused whole.
#[test]
fn a_head_that_breaks_the_rules_is_refused() {
    let lines = |count: usize, value: &str| (0..count).fold(String::new(), |lines, index| lines + &format!("X-H{index}: {value}\r\n"));
    let many = lines(MAX_RESPONSE_HEADERS + 1, "v");
    let long = format!("X-Long: {}\r\n", "v".repeat(MAX_HEADER_VALUE_BYTES + 1));
    let huge = lines(80, &"v".repeat(1000));
    let cases: Vec<(&str, String, &[Reason])> = vec![
        ("a folded header", "HTTP/1.1 200 OK\r\nX-A: one\r\n two\r\nContent-Length: 0\r\n\r\n".into(), &[Reason::Protocol]),
        (
            "a length and a transfer coding",
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".into(),
            &[Reason::Protocol],
        ),
        ("two lengths that differ", "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nhello!".into(), &[Reason::Protocol]),
        ("no status line", "HELLO\r\n\r\n".into(), &[Reason::Protocol]),
        ("a header name with a space", "HTTP/1.1 200 OK\r\nX A: 1\r\nContent-Length: 0\r\n\r\n".into(), &[Reason::Protocol]),
        ("too many headers", format!("HTTP/1.1 200 OK\r\n{many}Content-Length: 0\r\n\r\n"), &[Reason::Protocol, Reason::TooLarge]),
        ("a header value too long", format!("HTTP/1.1 200 OK\r\n{long}Content-Length: 0\r\n\r\n"), &[Reason::TooLarge]),
        ("a head too large", format!("HTTP/1.1 200 OK\r\n{huge}Content-Length: 0\r\n\r\n"), &[Reason::Protocol, Reason::TooLarge]),
        ("a body cut short", "HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nshort".into(), &[Reason::Protocol, Reason::Connection]),
    ];
    for (what, answer, reasons) in cases {
        let port = raw(Some(answer.into_bytes()));
        let mut client = Client::new(&format!("http://site.test:{port}/a/"));
        let outcome = client.get(&format!("http://site.test:{port}/a/x"));
        let reason = outcome.failed.map(|(_, reason)| reason);
        assert!(reason.is_some_and(|reason| reasons.contains(&reason)), "{what}: {reason:?}");
        if what != "a body cut short" {
            assert!(outcome.head.is_none(), "{what}: nothing of a refused answer is given");
        }
        client.finish().unwrap();
    }
    // What is within the limits passes: as many headers as allowed.
    let within = lines(MAX_RESPONSE_HEADERS - 1, "v");
    let port = raw(Some(format!("HTTP/1.1 200 Fine by me\r\n{within}Content-Length: 2\r\n\r\nok").into_bytes()));
    let mut client = Client::new(&format!("http://site.test:{port}/a/"));
    let outcome = client.get(&format!("http://site.test:{port}/a/x"));
    assert_eq!((outcome.ok().headers.len(), outcome.ok().status_text.as_str(), outcome.text().as_str()), (MAX_RESPONSE_HEADERS, "Fine by me", "ok"));
    client.finish().unwrap();
}

/// A proxy on this computer that tunnels one connection to wherever on
/// this computer it is asked; what it was asked.
fn tunnelling_proxy() -> (u16, std::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind((HERE, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (asked, told) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let mut socket = socket.unwrap();
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let line = String::from_utf8_lossy(&head).lines().next().unwrap_or_default().to_owned();
            let target_port: u16 = line.split(' ').nth(1).and_then(|target| target.rsplit(':').next()).and_then(|port| port.parse().ok()).unwrap_or(0);
            let _ = asked.send(line);
            let Ok(mut upstream) = TcpStream::connect((HERE, target_port)) else { continue };
            socket.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n").unwrap();
            let (mut from_client, mut to_upstream) = (socket.try_clone().unwrap(), upstream.try_clone().unwrap());
            std::thread::spawn(move || std::io::copy(&mut from_client, &mut to_upstream));
            std::thread::spawn(move || std::io::copy(&mut upstream, &mut socket));
        }
    });
    (port, told)
}

/// Requirement 19.10 and the engine's REQ-WEB-032: with a proxy every
/// connection is a tunnel through it, TLS end to end inside, and a name
/// that is plainly local is refused before the proxy is asked.
#[test]
fn every_connection_goes_through_the_proxy() {
    let plain = serve(|_| Reply::ok("through the tunnel"));
    let secure = serve_tls(&["h2", "http/1.1"], |_| Reply::ok("secure"));
    let (proxy, asked) = tunnelling_proxy();
    let mut client = Client::with(&plain.url("/a/"), move |setup| {
        setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{proxy}")).unwrap());
        // The proxy resolves names: nothing resolves here.
        setup.settings.names = Some(Rc::new(|name: &str, _: u16| panic!("{name} was resolved beside the proxy")));
    });
    assert_eq!(client.get(&plain.url("/a/x")).text(), "through the tunnel");
    assert_eq!(asked.recv().unwrap(), format!("CONNECT site.test:{} HTTP/1.1", plain.port));
    let mut secure_client = Client::with(&secure.at("https", "localhost", "/a/"), move |setup| {
        setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{proxy}")).unwrap());
    });
    assert_eq!(secure_client.get(&secure.at("https", "localhost", "/a/x")).reason(), Reason::LocalRefused, "a local name is not asked of the proxy");
    secure_client.finish().unwrap();
    let mut strict = Client::with(&plain.url("/a/"), move |setup| {
        setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{proxy}")).unwrap());
        setup.space = AddressSpace::Public;
    });
    for address in ["http://printer.local/", "http://router/", "http://intranet.corp/"] {
        assert_eq!(strict.get(address).reason(), Reason::LocalRefused, "{address}");
    }
    for address in ["http://10.0.0.1/", "http://127.0.0.1:8000/"] {
        assert_eq!(strict.get(address).reason(), Reason::LocalRefused, "{address}");
    }
    assert!(asked.try_recv().is_err(), "the proxy was asked for none of them");
    strict.finish().unwrap();
    // TLS runs end to end inside the tunnel: the server's own certificate
    // is what is checked, and here refused, being `localhost`'s, which
    // through a proxy can be asked for only by its address.
    let mut tunnelled = Client::with(&secure.at("https", "127.0.0.1", "/a/"), move |setup| {
        setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{proxy}")).unwrap());
    });
    assert_eq!(tunnelled.get(&secure.at("https", "127.0.0.1", "/a/x")).reason(), Reason::Certificate);
    assert_eq!(asked.recv().unwrap(), format!("CONNECT 127.0.0.1:{} HTTP/1.1", secure.port));
    tunnelled.finish().unwrap();
    client.finish().unwrap();
    // A proxy that refuses, or is not there.
    let refusing = raw(Some(b"HTTP/1.1 403 Forbidden\r\n\r\n".to_vec()));
    let mut refused = Client::with(&plain.url("/a/"), move |setup| setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{refusing}")).unwrap()));
    assert_eq!(refused.get(&plain.url("/a/x")).reason(), Reason::Proxy);
    refused.finish().unwrap();
}

/// The browser's requirement 5.14, kept for a script's requests: beside
/// what the script itself set, a request carries a fixed set of headers
/// that says nothing of the viewer. (The browser's own page loads do not
/// come by these messages: DEC-7317 item 7.)
#[test]
fn a_request_says_nothing_of_the_viewer() {
    let server = serve(|_| Reply::ok("PN,10000"));
    let mut client = Client::new(&server.url("/a/"));
    client.get(&server.url("/a/script"));
    let asked = server.the("/a/script");
    assert_eq!(asked.names(), ["accept", "accept-encoding", "host", "referer", "user-agent"]);
    assert_eq!(asked.header("accept").as_deref(), Some("*/*"));
    let family = neoteletext_web::user_agent::os_family().unwrap();
    assert_eq!(asked.header("user-agent"), Some(format!("NeoTeletext/0.1 ({family})")), "the browser's name, version and system family, as today");
    // The page asking is the script host's word, taken only for a page of
    // the process's own channel: another's address is never sent on as a
    // referrer, and gives the request no other origin.
    for claimed in ["http://elsewhere.test/secret/page", &server.url("/other-channel/page"), "", "not an address"] {
        if claimed.contains(' ') {
            continue;
        }
        client.fetch(&server.url("/a/claimed"), |start| start.client = claimed.to_owned());
        let asked = server.asked().pop().unwrap();
        assert_eq!(asked.header("referer"), Some(server.url("/a/")), "{claimed}: the channel's folder stands in");
        assert_eq!(asked.header("origin"), None, "{claimed}");
    }
    client.finish().unwrap();
}

/// DEC-7264 item 3 and TODO-7265's check (b), with finding S-02
/// (TODO-7267): an address of the channel's own origin outside its
/// folder is another origin to a script, however the address is written;
/// the request names the origin, carries no cookie, and its answer is
/// read only by the server's leave.
#[test]
fn another_channel_of_the_origin_is_another_origin() {
    let server = serve(|asked| {
        let reply = Reply::ok(&format!("secret of {}", asked.target)).header("set-cookie", "sid=1; Path=/");
        if asked.target.starts_with("/shared/") { reply.header("access-control-allow-origin", &asked.header("origin").unwrap_or_default()) } else { reply }
    });
    let mut client = Client::new(&server.url("/home/"));
    assert_eq!(client.get(&server.url("/home/page2")).ok().response_type, ResponseType::Basic);
    assert_eq!(client.get(&server.url("/home/again")).ok().status, 200);
    assert_eq!(server.the("/home/again").header("cookie").as_deref(), Some("sid=1"), "its own channel's cookie goes with its own requests");
    for outside in [
        "/demosite/secret",
        "/home/..%2fdemosite/secret",
        "/home/%2e%2e%2fdemosite/secret",
        "/home/..;/demosite/x",
        "/home/%2E%2E%2Fdemosite/secret",
        "/home/..%5cdemosite/secret",
        "/home/%252e%252e%252fdemosite/secret",
        "/home/x%00",
        "/",
        "/homepage/x",
    ] {
        assert_eq!(client.get(&server.url(outside)).reason(), Reason::Cors, "{outside}");
        let asked = server.asked().pop().unwrap();
        assert_eq!(asked.header("origin"), Some(format!("http://site.test:{}", server.port)), "{outside}: asked as another origin is");
        assert_eq!(asked.header("cookie"), None, "{outside}: without the channel's cookies");
        assert_eq!(asked.header("referer"), Some(server.url("/")), "{outside}: told the origin alone");
        assert!(asked.target.eq_ignore_ascii_case(outside) || outside == "/home/x%00", "{outside}: the address judged is the address sent, {}", asked.target);
    }
    // Leave given, it is read as another origin's answer is.
    let shared = client.get(&server.url("/shared/data"));
    assert_eq!((shared.ok().response_type, shared.text().as_str()), (ResponseType::Cors, "secret of /shared/data"));
    assert_eq!(client.fetch(&server.url("/demosite/secret"), |start| start.mode = protocol::web::Mode::SameOrigin).reason(), Reason::SameOriginMode);
    // A redirect out of the folder is checked as one to another origin.
    let moved = serve(|asked| match asked.target.as_str() {
        "/home/out" => Reply::to(302, "/demosite/secret"),
        _ => Reply::ok("secret"),
    });
    let mut other = Client::new(&moved.url("/home/"));
    assert_eq!(other.get(&moved.url("/home/out")).reason(), Reason::Cors);
    other.finish().unwrap();
    client.finish().unwrap();
}

/// The browser's DEC-7268 and DEC-7317 item 4 (TODO-7291's check): a
/// process serves one partition. A start that carries another
/// partition's token ends it, whatever tab it names; and it does not
/// start for what is no channel identity. A request number in use twice
/// is its script host's own fault and costs no one else.
#[test]
fn a_start_with_another_partitions_token_ends_the_process() {
    let server = serve(|asked| if asked.target == "/home/hang" { Reply::new(200).then(Piece::Hang) } else { Reply::ok("x") });
    let mut client = Client::new(&server.url("/home/"));
    assert_eq!(client.get(&server.url("/home/x")).ok().status, 200);
    let mut start = None;
    client.start(&server.url("/home/y"), |made| start = Some(made.clone()));
    assert_eq!(client.outcome(2).ok().status, 200);
    let mut other = start.unwrap();
    other.id = 99;
    client.send_as(TAB, &OTHER_TOKEN, &ToWebClient::Start(Box::new(other)));
    assert_eq!(client.ended(), Err("invalid value: partition token".to_owned()));
    assert_eq!(server.asked().len(), 2, "the start with another's token was never made");
    assert!(Client::new("http://site.test/home").ended().is_err(), "a channel identity is a folder");
    // The same number twice in one tab: the second is not served and the
    // first goes on; the same number in another tab is another request.
    let mut twice = Client::new(&server.url("/home/"));
    let id = twice.start(&server.url("/home/hang"), |_| {});
    assert!(matches!(twice.next_event(), FromWebClient::Head(_)));
    twice.start(&server.url("/home/again"), |start| start.id = id);
    assert_eq!(twice.next_within(Duration::from_millis(300)), None);
    assert_eq!(server.count("GET", "/home/again"), 0);
    twice.start_in(2, &server.url("/home/hang"), |start| start.id = id);
    assert!(matches!(twice.next_event(), FromWebClient::Head(_)));
    assert_eq!(twice.tab, 2);
    twice.send(&ToWebClient::Abort(id));
    assert_eq!(twice.next_event(), FromWebClient::Failed { id, error: ErrorKind::Abort, reason: Reason::Aborted });
    assert_eq!(twice.tab, TAB, "the abort was the first tab's");
    twice.finish().unwrap();
}

/// The web client's limits on a tab: its requests under way, and its
/// starts a second, which a script that starts and aborts in a loop does
/// not escape.
#[test]
fn a_tabs_requests_are_counted_and_its_starts_held_to_a_rate() {
    let server = serve(|asked| if asked.target == "/a/fast" { Reply::ok("fast") } else { Reply::new(200).then(Piece::Hang) });
    let mut client = Client::new(&server.url("/a/"));
    let slow: Vec<u32> = (0..MAX_REQUESTS_PER_TAB).map(|_| client.start(&server.url("/a/slow"), |_| {})).collect();
    for _ in &slow {
        assert!(matches!(client.next_event(), FromWebClient::Head(_)));
    }
    // One more waits its turn; another tab's does not.
    let waiting = client.start(&server.url("/a/fast"), |_| {});
    assert_eq!(client.next_within(Duration::from_millis(300)), None, "the tab's places are taken");
    let other_tab = client.start_in(2, &server.url("/a/fast"), |_| {});
    assert_eq!(client.outcome(other_tab).text(), "fast");
    assert_eq!(client.tab, 2);
    client.send(&ToWebClient::Abort(slow[0]));
    assert_eq!(client.next_event(), FromWebClient::Failed { id: slow[0], error: ErrorKind::Abort, reason: Reason::Aborted });
    assert_eq!(client.outcome(waiting).text(), "fast");
    // A context's closing ends its requests and no other's, saying
    // nothing more of them: one place is free, and the page's requests
    // still hold the rest.
    let channels = client.start(&server.url("/a/slow"), |start| start.context = ContextId::Channel);
    assert!(matches!(client.next_event(), FromWebClient::Head(_)));
    let waiting = client.start(&server.url("/a/fast"), |_| {});
    assert_eq!(client.next_within(Duration::from_millis(300)), None, "the tab's places are taken again");
    client.send(&ToWebClient::ContextCloses(ContextId::Channel));
    assert_eq!(client.outcome(waiting).text(), "fast");
    client.send(&ToWebClient::Abort(channels));
    assert_eq!(client.next_within(Duration::from_millis(200)), None, "a request its context ended is gone");
    // A tab's closing ends its requests, saying nothing more of them.
    client.send(&ToWebClient::TabCloses);
    assert_eq!(client.get(&server.url("/a/fast")).text(), "fast", "the tab's places are free again");
    client.finish().unwrap();

    // Starting and aborting in a loop: past the burst, a start is refused
    // as busy until time has passed.
    let mut hasty = Client::new(&server.url("/a/"));
    let mut refused = 0;
    for _ in 0..START_BURST * 4 {
        let id = hasty.start(&server.url("/a/slow"), |_| {});
        hasty.send(&ToWebClient::Abort(id));
        loop {
            match hasty.next_event() {
                FromWebClient::Failed { id: of, reason: Reason::Busy, .. } if of == id => {
                    refused += 1;
                    break;
                }
                FromWebClient::Failed { id: of, error: ErrorKind::Abort, .. } if of == id => break,
                FromWebClient::Head(_) => {}
                other => panic!("{other:?}"),
            }
        }
    }
    assert!(refused >= START_BURST * 2, "{refused} of {} starts were refused", START_BURST * 4);
    assert!(
        server.asked().len() <= usize::try_from(START_BURST * 2).unwrap() + MAX_REQUESTS_PER_TAB + 4,
        "the server was asked {} times",
        server.asked().len()
    );
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(hasty.get(&server.url("/a/fast")).text(), "fast", "starts come back with time");
    hasty.finish().unwrap();
}
