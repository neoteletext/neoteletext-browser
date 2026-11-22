//! The protections of the channel fetcher's client, rebuilt over hyper:
//! the local network asked only as DEC-7315 allows, no downgrade, the one deadline over a
//! connection's opening, the proxy's tunnel, strict heads, and a page
//! load that says nothing of the viewer; and the web client's own: one
//! partition a process, the channel as the origin, and its limits.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::rc::Rc;
use std::time::{Duration, Instant};

use neoteletext_web::proxy::Proxy;
use protocol::web::{AddressSpace, ErrorKind, FromWebClient, MAX_HEADER_VALUE, MAX_RESPONSE_HEADERS, Reason, ResponseType, ToWebClient};

use super::support::{Client, HERE, Piece, Reply, as_page, header, raw, serve, serve_tls};
use crate::web::limits::{MAX_REQUESTS_PER_TAB, START_BURST};

/// neoteletext-common's DEC-7315 and finding F-62: this computer and the
/// local network are reached as that decision allows. A page loaded from this
/// computer asks it freely; a page from the internet, or from the local
/// network asking this computer, needs the viewer's permission, whatever
/// name resolved to the address, and is refused until it can be asked
/// for.
#[test]
fn a_more_private_address_is_asked_only_with_permission() {
    let server = serve(|_| Reply::ok("local"));
    let channel = server.url("/a/");
    let public = |start: &mut protocol::web::Start| start.client_space = AddressSpace::Public;
    let mut client = Client::new(&channel);
    // A page loaded from this computer: reached, by name and by address.
    assert_eq!(client.get(&server.url("/a/x")).text(), "local");
    assert_eq!(client.get(&server.at("http", "127.0.0.1", "/a/x")).reason(), Reason::Cors, "reached, and another origin");
    let before = server.asked().len();
    // A public page: it needs permission, on a new connection and on the
    // one kept, for the name that resolves here and for the address in
    // each of its spellings.
    for address in [
        server.url("/a/x"),
        server.at("http", "127.0.0.1", "/"),
        server.at("http", "localhost", "/"),
        server.at("http", "[::1]", "/"),
        server.at("http", "[::ffff:127.0.0.1]", "/"),
        server.at("http", "0x7f.1", "/"),
    ] {
        assert_eq!(client.fetch(&address, public).reason(), Reason::LocalPermission, "{address}");
    }
    for address in ["http://10.0.0.1/", "http://192.168.1.1/", "http://169.254.169.254/latest/meta-data/", "http://[fe80::1]/", "http://[fd00::1]/"] {
        assert_eq!(client.fetch(address, public).reason(), Reason::LocalPermission, "{address}");
    }
    // A page on the local network asks this computer only with permission.
    assert_eq!(client.fetch(&server.url("/a/x"), |start| start.client_space = AddressSpace::Local).reason(), Reason::LocalPermission);
    assert_eq!(server.asked().len(), before, "nothing was connected to");
    client.finish().unwrap();
    // What the viewer already allowed is open to a public page too.
    let mut permitted = Client::with(&channel, 0, |setup| setup.settings.allowed = vec![HERE]);
    assert_eq!(permitted.fetch(&server.url("/a/x"), public).text(), "local");
    assert_eq!(permitted.fetch("http://192.168.1.1/", public).reason(), Reason::LocalPermission, "and nothing else");
    permitted.finish().unwrap();
    // A page on the local network asks the local network freely: no
    // permission is wanted, whatever then becomes of the connection.
    let mut local = Client::with(&channel, 0, |setup| setup.settings.open_deadline = Duration::from_millis(300));
    let outcome = local.fetch("http://10.255.255.1/", |start| start.client_space = AddressSpace::Local);
    assert!(matches!(outcome.reason(), Reason::Timeout | Reason::Connection), "{outcome:?}");
    local.finish().unwrap();
    // A name with an address here, asked by a public page: the address is
    // what is judged (a name pointed here after it was first resolved
    // gains nothing).
    let port = server.port;
    let mut rebound = Client::with(&channel, 0, move |setup| setup.settings.names = Some(Rc::new(move |_: &str, _: u16| vec![SocketAddr::new(HERE, port)])));
    assert_eq!(rebound.fetch("http://public.test/", public).reason(), Reason::LocalPermission);
    rebound.finish().unwrap();
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
    assert_eq!(client.load(&secure.at("https", "localhost", "/a/down")).reason(), Reason::Downgrade, "nor a page load");
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
    let mut untrusting = Client::with(&secure.at("https", "localhost", "/a/"), 0, |setup| {
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
    let mut client = Client::with(&format!("http://site.test:{silent}/a/"), 0, move |setup| {
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
    let long = format!("X-Long: {}\r\n", "v".repeat(MAX_HEADER_VALUE + 1));
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
    let mut client = Client::with(&plain.url("/a/"), 0, move |setup| {
        setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{proxy}")).unwrap());
        // The proxy resolves names: nothing resolves here.
        setup.settings.names = Some(Rc::new(|name: &str, _: u16| panic!("{name} was resolved beside the proxy")));
    });
    assert_eq!(client.get(&plain.url("/a/x")).text(), "through the tunnel");
    assert_eq!(asked.recv().unwrap(), format!("CONNECT site.test:{} HTTP/1.1", plain.port));
    let mut secure_client = Client::with(&secure.at("https", "localhost", "/a/"), 0, move |setup| {
        setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{proxy}")).unwrap());
    });
    assert_eq!(secure_client.get(&secure.at("https", "localhost", "/a/x")).reason(), Reason::LocalPermission, "a local name is not asked of the proxy");
    secure_client.finish().unwrap();
    let mut strict = Client::with(&plain.url("/a/"), 0, move |setup| {
        setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{proxy}")).unwrap());
    });
    for address in ["http://printer.local/", "http://router/", "http://intranet.corp/"] {
        assert_eq!(strict.get(address).reason(), Reason::LocalPermission, "{address}");
    }
    for address in ["http://10.0.0.1/", "http://127.0.0.1:8000/"] {
        assert_eq!(strict.fetch(address, |start| start.client_space = AddressSpace::Public).reason(), Reason::LocalPermission, "{address}");
    }
    assert!(asked.try_recv().is_err(), "the proxy was asked for none of them");
    strict.finish().unwrap();
    // TLS runs end to end inside the tunnel: the server's own certificate
    // is what is checked, and here refused, being `localhost`'s, which
    // through a proxy can be asked for only by its address.
    let mut tunnelled = Client::with(&secure.at("https", "127.0.0.1", "/a/"), 0, move |setup| {
        setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{proxy}")).unwrap());
    });
    assert_eq!(tunnelled.get(&secure.at("https", "127.0.0.1", "/a/x")).reason(), Reason::Certificate);
    assert_eq!(asked.recv().unwrap(), format!("CONNECT 127.0.0.1:{} HTTP/1.1", secure.port));
    tunnelled.finish().unwrap();
    client.finish().unwrap();
    // A proxy that refuses, or is not there.
    let refusing = raw(Some(b"HTTP/1.1 403 Forbidden\r\n\r\n".to_vec()));
    let mut refused =
        Client::with(&plain.url("/a/"), 0, move |setup| setup.settings.proxy = Some(Proxy::parse(&format!("http://127.0.0.1:{refusing}")).unwrap()));
    assert_eq!(refused.get(&plain.url("/a/x")).reason(), Reason::Proxy);
    refused.finish().unwrap();
}

/// The browser's requirement 5.14 and REQ-702, kept: its own load of a
/// page carries a fixed set of headers that says nothing of the viewer,
/// no `Referer`, and no header of a caller's but its validators.
#[test]
fn a_page_load_says_nothing_of_the_viewer() {
    let server = serve(|asked| if asked.target == "/a/moved" { Reply::to(302, "/a/100.tti") } else { Reply::ok("PN,10000") });
    let mut client = Client::new(&server.url("/a/"));
    let loaded = client.fetch(&server.url("/a/100.tti"), |start| {
        as_page(start);
        start.headers = vec![header("X-Viewer", "me"), header("Accept-Language", "en-AU"), header("If-None-Match", "\"v1\""), header("Authorization", "x")];
    });
    assert_eq!((loaded.ok().response_type, loaded.text().as_str()), (ResponseType::Basic, "PN,10000"));
    let asked = server.the("/a/100.tti");
    assert_eq!(asked.names(), ["accept", "accept-encoding", "host", "if-none-match", "user-agent"]);
    assert_eq!(asked.header("accept").as_deref(), Some("*/*"));
    let family = neoteletext_web::user_agent::os_family().unwrap();
    assert_eq!(asked.header("user-agent"), Some(format!("NeoTeletext/0.1 ({family})")), "the browser's name, version and system family, as today");
    assert!(asked.header("cache-control").is_none(), "a page's validators are its own cache's");
    // A link followed from another page of the channel still tells the
    // server nothing of where the viewer was.
    client.fetch(&server.url("/a/200.tti"), |start| {
        as_page(start);
        start.client = server.url("/a/100.tti");
    });
    assert_eq!(server.the("/a/200.tti").names(), ["accept", "accept-encoding", "host", "user-agent"]);
    // The browser's loader may follow a redirect itself: it is given the
    // redirect as it came.
    let manual = client.fetch(&server.url("/a/moved"), |start| {
        as_page(start);
        start.redirect = protocol::web::RedirectMode::Manual;
    });
    assert_eq!((manual.ok().status, manual.header("location").as_deref()), (302, Some("/a/100.tti")));
    // A script's own request carries the same fixed words and a referrer.
    client.get(&server.url("/a/script"));
    assert_eq!(server.the("/a/script").names(), ["accept", "accept-encoding", "host", "referer", "user-agent"]);
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

/// The browser's DEC-7268: a process serves one partition. A request that
/// names another channel, or another jar of windows, ends it; and it does
/// not start for what is no channel identity.
#[test]
fn a_request_for_another_partition_ends_the_process() {
    let server = serve(|_| Reply::ok("x"));
    let mut client = Client::new(&server.url("/home/"));
    assert_eq!(client.get(&server.url("/home/x")).ok().status, 200);
    client.start(&server.url("/demosite/x"), |start| start.channel = server.url("/demosite/"));
    assert_eq!(client.ended(), Err("a request for another partition".to_owned()));
    let mut private = Client::with(&server.url("/home/"), 3, |_| {});
    private.start(&server.url("/home/x"), |start| start.jar = 0);
    assert!(private.ended().is_err());
    assert_eq!(server.asked().len(), 1, "neither request was made");
    assert!(Client::with("http://site.test/home", 0, |_| {}).ended().is_err(), "a channel identity is a folder");
    // A request number in use twice is the trusted core's fault, and ends
    // the process too.
    let hanging = serve(|_| Reply::new(200).then(Piece::Hang));
    let mut twice = Client::new(&hanging.url("/a/"));
    let id = twice.start(&hanging.url("/a/x"), |_| {});
    twice.start(&hanging.url("/a/y"), |start| start.id = id);
    assert!(twice.ended().is_err());
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
    let other_tab = client.start(&server.url("/a/fast"), |start| start.tab = 2);
    assert_eq!(client.outcome(other_tab).text(), "fast");
    client.send(&ToWebClient::Abort(slow[0]));
    assert_eq!(client.next_event(), FromWebClient::Failed { id: slow[0], error: ErrorKind::Abort, reason: Reason::Aborted });
    assert_eq!(client.outcome(waiting).text(), "fast");
    // A tab's closing ends its requests, saying nothing more of them.
    client.send(&ToWebClient::TabCloses(1));
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
