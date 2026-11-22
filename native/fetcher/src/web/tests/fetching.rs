//! The Fetch standard's network half, end to end: methods, headers and
//! bodies, redirects, CORS and its preflight, the modes, integrity and
//! content codings.

use std::sync::atomic::{AtomicUsize, Ordering};

use protocol::web::{BodySource, CacheMode, Credentials, Mode, Reason, RedirectMode, Referrer, ReferrerPolicy, ResponseType, ToWebClient};

use super::support::{Client, Reply, Server, header, serve};
use crate::web::coding::tests::{BROTLI, BROTLI_TEXT, gzip};

/// A server that says what it was asked, and the channel `/a/` on it.
fn echo() -> (Server, Client) {
    let server = serve(|asked| Reply::ok(&format!("{} {}", asked.method, asked.target)).header("x-seen", "yes").header("set-cookie", "s=1"));
    let client = Client::new(&server.url("/a/"));
    (server, client)
}

/// DEC-7260 item 7: every method, the script's headers and its body go to
/// the server, and the answer's status, status text, headers and body
/// come back.
#[test]
fn every_method_carries_its_headers_and_body() {
    let (server, mut client) = echo();
    for method in ["GET", "HEAD", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "QUERY"] {
        let path = format!("/a/{method}");
        let with_body = !matches!(method, "GET" | "HEAD");
        let id = client.start(&server.url(&path), |start| {
            start.method = method.to_owned();
            start.headers = vec![header("X-Custom", "v"), header("content-type", "text/plain")];
            if with_body {
                start.body = BodySource::Bytes(11);
            }
        });
        if with_body {
            client.send(&ToWebClient::RequestBody { id, bytes: b"hello ".to_vec() });
            client.send(&ToWebClient::RequestBody { id, bytes: b"there".to_vec() });
            client.send(&ToWebClient::RequestBodyEnd(id));
        }
        let outcome = client.outcome(id);
        let head = outcome.ok();
        assert_eq!((head.status, head.status_text.as_str(), head.response_type, head.redirected), (200, "OK", ResponseType::Basic, false), "{method}");
        assert_eq!(head.url, server.url(&path));
        assert_eq!(outcome.header("x-seen").as_deref(), Some("yes"));
        assert_eq!(outcome.text(), if method == "HEAD" { String::new() } else { format!("{method} {path}") });
        assert_eq!(outcome.sent, if with_body { 11 } else { 0 }, "{method}: the body's bytes, said as written");
        let asked = server.the(&path);
        assert_eq!(asked.method, method);
        assert_eq!(asked.header("x-custom").as_deref(), Some("v"));
        assert_eq!(asked.body, if with_body { b"hello there".to_vec() } else { Vec::new() });
        assert_eq!(asked.header("content-length"), with_body.then(|| "11".to_owned()), "{method}");
    }
    // A `POST` with no body says so (the Fetch standard's
    // HTTP-network-or-cache fetch, step 8.5).
    client.fetch(&server.url("/a/empty"), |start| start.method = "POST".into());
    assert_eq!(server.the("/a/empty").header("content-length").as_deref(), Some("0"));
    client.finish().unwrap();
}

/// The Fetch standard's forbidden methods and headers, its bad ports and
/// the schemes that are not the web's: refused or dropped by the web
/// client itself, whatever the script side let through.
#[test]
fn what_the_standard_forbids_never_reaches_the_network() {
    let (server, mut client) = echo();
    // A forbidden method, and an address that is not the web's, are no
    // message at all: the pipe's one definition refuses to write or read
    // them (DEC-7317 item 11), and the web client holds the same rules
    // behind it.
    for method in ["TRACE", "CONNECT", "track"] {
        assert!(client.unwritable(&server.url("/a/x"), |start| start.method = method.into()), "{method}");
    }
    assert_eq!(client.get("http://site.test:25/a/").reason(), Reason::BadPort);
    assert_eq!(client.get("http://site.test:6667/a/").reason(), Reason::BadPort);
    for address in ["ftp://site.test/a/", "file:///etc/passwd", "data:text/plain,x", "about:blank", "javascript:1", "ws://site.test/a/", "not an address"] {
        assert!(client.unwritable(address, |_| {}), "{address}");
    }
    assert_eq!(client.get("http://").reason(), Reason::Address);
    assert_eq!(client.get("http://[::1/").reason(), Reason::Address);
    assert!(server.asked().is_empty());
    client.fetch(&server.url("/a/headers"), |start| {
        start.headers = vec![
            header("Cookie", "stolen=1"),
            header("Host", "evil.test"),
            header("Origin", "http://evil.test"),
            header("Referer", "http://evil.test/"),
            header("Sec-Fetch-Site", "same-origin"),
            header("Proxy-Authorization", "x"),
            header("Accept-Encoding", "zstd"),
            header("X-HTTP-Method-Override", "TRACE"),
            header("X-Kept", "1"),
        ];
    });
    let asked = server.the("/a/headers");
    assert_eq!(asked.names(), ["accept", "accept-encoding", "host", "referer", "user-agent", "x-kept"]);
    assert_eq!(asked.header("host").as_deref(), Some(format!("site.test:{}", server.port).as_str()));
    assert_eq!(asked.header("referer"), Some(server.url("/a/page")));
    assert_eq!(asked.header("accept-encoding").as_deref(), Some("gzip, deflate, br"));
    client.finish().unwrap();
}

/// The Fetch standard's HTTP-redirect fetch: the method and body rules of
/// each status, the address the answer finally came from, and the limit
/// of 20.
#[test]
fn redirects_are_followed_by_the_standards_rules() {
    let server = serve(|asked| match asked.target.as_str() {
        "/a/301" => Reply::to(301, "/a/end"),
        "/a/303" => Reply::to(303, "end?from=303#frag"),
        "/a/307" => Reply::to(307, "/a/end"),
        "/a/308" => Reply::to(308, "/a/end"),
        "/a/none" => Reply::new(302).body(b"no location"),
        "/a/bad" => Reply::to(302, "ftp://site.test/x"),
        "/a/loop" => Reply::to(302, "/a/loop"),
        target if target.starts_with("/a/chain/") => {
            let left: u32 = target.rsplit('/').next().unwrap().parse().unwrap();
            if left == 0 { Reply::ok("end of chain") } else { Reply::to(302, &format!("/a/chain/{}", left - 1)) }
        }
        _ => Reply::ok(&format!("{} {} {}", asked.method, asked.target, String::from_utf8_lossy(&asked.body)))
            .header("x-type", &asked.header("content-type").unwrap_or_default()),
    });
    let mut client = Client::new(&server.url("/a/"));
    let mut post = |path: &str, source: BodySource| {
        let id = client.start(&server.url(path), |start| {
            start.method = "POST".into();
            start.body = source;
            start.headers = vec![header("content-type", "text/plain")];
        });
        client.send(&ToWebClient::RequestBody { id, bytes: b"body".to_vec() });
        client.send(&ToWebClient::RequestBodyEnd(id));
        client.outcome(id)
    };
    // 301 and 303 make a POST a GET with no body or body headers; 307 and
    // 308 send it again as it was.
    let after_301 = post("/a/301", BodySource::Bytes(4));
    assert_eq!((after_301.text().as_str(), after_301.header("x-type").as_deref()), ("GET /a/end ", Some("")));
    assert!(after_301.ok().redirected);
    let after_303 = post("/a/303", BodySource::Bytes(4));
    assert_eq!(after_303.text(), "GET /a/end?from=303 ");
    assert_eq!(after_303.ok().url, server.url("/a/end?from=303"), "the answer's address, without its fragment");
    for path in ["/a/307", "/a/308"] {
        let after = post(path, BodySource::Bytes(4));
        assert_eq!((after.text().as_str(), after.header("x-type").as_deref()), ("POST /a/end body", Some("text/plain")), "{path}");
        // A stream cannot be sent twice.
        assert_eq!(post(path, BodySource::Stream).reason(), Reason::Redirect, "{path}");
    }
    assert_eq!(post("/a/303", BodySource::Stream).text(), "GET /a/end?from=303 ", "a 303 needs no second sending");
    // A redirect with nowhere to go is the answer; one to another scheme
    // fails.
    let none = client.get(&server.url("/a/none"));
    assert_eq!((none.ok().status, none.text().as_str(), none.ok().redirected), (302, "no location", false));
    assert_eq!(client.get(&server.url("/a/bad")).reason(), Reason::Redirect);
    // Twenty redirects are followed; the twenty-first is not.
    let twenty = client.get(&server.url("/a/chain/20"));
    assert_eq!(twenty.text(), "end of chain");
    assert_eq!(client.get(&server.url("/a/chain/21")).reason(), Reason::RedirectLimit);
    assert_eq!(client.get(&server.url("/a/loop")).reason(), Reason::RedirectLimit);
    assert_eq!(server.count("GET", "/a/loop"), 21);
    // The redirect modes.
    assert_eq!(client.fetch(&server.url("/a/307"), |start| start.redirect = RedirectMode::Error).reason(), Reason::Redirect);
    let manual = client.fetch(&server.url("/a/307"), |start| start.redirect = RedirectMode::Manual);
    let head = manual.ok();
    assert_eq!(
        (head.response_type, head.status, head.url.as_str(), head.headers.len()),
        (ResponseType::OpaqueRedirect, 0, server.url("/a/307").as_str(), 0),
        "its address is the one asked, which the standard keeps (TODO-7371 step 6)"
    );
    assert_eq!(manual.body, b"");
    client.finish().unwrap();
}

/// The Fetch standard's CORS protocol: another origin's answer is read
/// only if it allows the page's origin, and then only the headers it
/// exposes.
#[test]
fn another_origin_is_read_only_by_its_leave() {
    let server = serve(|asked| {
        let origin = asked.header("origin").unwrap_or_default();
        let reply = Reply::ok("data").header("x-secret", "1").header("x-shown", "2").header("set-cookie", "c=1");
        match asked.target.as_str() {
            "/open" => reply.header("access-control-allow-origin", "*").header("access-control-expose-headers", "x-shown"),
            "/mine" => reply.header("access-control-allow-origin", &origin),
            "/credentialed" => reply.header("access-control-allow-origin", &origin).header("access-control-allow-credentials", "true"),
            "/other" => reply.header("access-control-allow-origin", "http://elsewhere.test"),
            "/redirect" => Reply::to(302, "/open").header("access-control-allow-origin", "*"),
            _ => reply,
        }
    });
    let mut client = Client::new(&server.url("/a/"));
    let there = |path: &str| server.at("http", "other.test", path);
    let origin = format!("http://site.test:{}", server.port);
    let open = client.get(&there("/open"));
    assert_eq!((open.ok().response_type, open.text().as_str()), (ResponseType::Cors, "data"));
    assert_eq!(open.header("x-shown").as_deref(), Some("2"));
    assert_eq!(open.header("content-type"), None);
    assert!(open.header("x-secret").is_none() && open.header("set-cookie").is_none(), "only what is exposed");
    assert_eq!(server.the("/open").header("origin"), Some(origin.clone()), "a request to another origin names the page's");
    assert_eq!(server.the("/open").header("referer"), Some(format!("{origin}/")), "and is told the origin alone as its referrer");
    assert_eq!(client.get(&there("/mine")).ok().status, 200);
    assert_eq!(client.get(&there("/closed")).reason(), Reason::Cors);
    assert_eq!(client.get(&there("/other")).reason(), Reason::Cors);
    // With credentials a wildcard is not leave, and the origin alone is not.
    let include = |start: &mut protocol::web::Start| start.credentials = Credentials::Include;
    assert_eq!(client.fetch(&there("/open"), include).reason(), Reason::Cors);
    assert_eq!(client.fetch(&there("/mine"), include).reason(), Reason::Cors);
    assert_eq!(client.fetch(&there("/credentialed"), include).ok().status, 200);
    // Each hop of a redirect is checked, and its answer is the last's.
    let redirected = client.get(&there("/redirect"));
    assert_eq!((redirected.ok().response_type, redirected.ok().redirected, redirected.ok().url.as_str()), (ResponseType::Cors, true, there("/open").as_str()));
    // The mode `same-origin` asks no other origin. (The mode `no-cors` has
    // no code on the pipe: `docs/script-api.md` departure D5.)
    assert_eq!(client.fetch(&there("/open"), |start| start.mode = Mode::SameOrigin).reason(), Reason::SameOriginMode);
    client.finish().unwrap();
}

/// The Fetch standard's CORS-preflight fetch and its cache: a request
/// that is not simple asks leave first, with `OPTIONS` and no
/// credentials, and a remembered answer is not asked for again.
#[test]
fn a_request_that_is_not_simple_asks_leave_first() {
    let server = serve(|asked| {
        let origin = asked.header("origin").unwrap_or_default();
        let allow = Reply::new(if asked.method == "OPTIONS" { 204 } else { 200 }).header("access-control-allow-origin", &origin);
        match (asked.method.as_str(), asked.target.as_str()) {
            ("OPTIONS", "/api") => allow
                .header("access-control-allow-methods", "PUT")
                .header("access-control-allow-headers", "X-Token, Content-Type")
                .header("access-control-max-age", "60"),
            ("OPTIONS", "/brief") => allow.header("access-control-allow-methods", "PUT"),
            ("OPTIONS", "/refuses") => Reply::new(403),
            ("OPTIONS", "/redirects") => Reply::to(302, "/api").header("access-control-allow-origin", &origin),
            ("OPTIONS", "/get-only") => allow,
            _ => allow.body(b"done"),
        }
    });
    let mut client = Client::new(&server.url("/a/"));
    let there = |path: &str| server.at("http", "other.test", path);
    let put = |start: &mut protocol::web::Start| {
        start.method = "PUT".into();
        start.headers = vec![header("X-Token", "t"), header("Content-Type", "application/json"), header("Accept", "text/plain")];
    };
    assert_eq!(client.fetch(&there("/api"), put).text(), "done");
    let preflight = server.asked().into_iter().find(|asked| asked.method == "OPTIONS").unwrap();
    assert_eq!(preflight.header("access-control-request-method").as_deref(), Some("PUT"));
    assert_eq!(preflight.header("access-control-request-headers").as_deref(), Some("content-type,x-token"), "sorted, in lower case, the unsafe ones alone");
    assert_eq!(preflight.header("origin"), Some(format!("http://site.test:{}", server.port)));
    assert!(preflight.header("x-token").is_none() && preflight.header("cookie").is_none() && preflight.body.is_empty());
    // Remembered for its max-age: asked once for two requests.
    assert_eq!(client.fetch(&there("/api"), put).text(), "done");
    assert_eq!((server.count("OPTIONS", "/api"), server.count("PUT", "/api")), (1, 2));
    // What the answer did not allow is asked again, and refused.
    assert_eq!(client.fetch(&there("/api"), |start| start.method = "DELETE".into()).reason(), Reason::Preflight);
    assert_eq!(server.count("DELETE", "/api"), 0, "the request itself was never made");
    assert_eq!(client.fetch(&there("/get-only"), put).reason(), Reason::Preflight);
    assert_eq!(client.fetch(&there("/refuses"), put).reason(), Reason::Preflight);
    assert_eq!(client.fetch(&there("/redirects"), put).reason(), Reason::Preflight, "a preflight is not redirected");
    // A simple request asks no leave.
    client.fetch(&there("/simple"), |start| {
        start.method = "POST".into();
        start.headers = vec![header("Content-Type", "text/plain")];
    });
    assert_eq!(server.count("OPTIONS", "/simple"), 0);
    client.finish().unwrap();
}

/// The Fetch standard's cache modes with no cache behind them, the
/// referrer and its policy, and the `Origin` header of a request that is
/// not a `GET`.
#[test]
fn cache_modes_and_the_referrer_set_the_headers_the_standard_gives() {
    let (server, mut client) = echo();
    let mut asked_with = |path: &str, change: &dyn Fn(&mut protocol::web::Start)| {
        client.fetch(&server.url(path), change);
        server.the(path)
    };
    let plain = asked_with("/a/default", &|_| {});
    assert!(plain.header("cache-control").is_none() && plain.header("pragma").is_none() && plain.header("origin").is_none());
    assert_eq!(asked_with("/a/no-cache", &|start| start.cache = CacheMode::NoCache).header("cache-control").as_deref(), Some("max-age=0"));
    for (path, mode) in [("/a/no-store", CacheMode::NoStore), ("/a/reload", CacheMode::Reload)] {
        let asked = asked_with(path, &|start| start.cache = mode);
        assert_eq!((asked.header("cache-control").as_deref(), asked.header("pragma").as_deref()), (Some("no-cache"), Some("no-cache")), "{path}");
    }
    assert_eq!(
        asked_with("/a/conditional", &|start| start.headers = vec![header("If-None-Match", "\"v1\"")]).header("cache-control").as_deref(),
        Some("no-cache")
    );
    assert!(asked_with("/a/force", &|start| start.cache = CacheMode::ForceCache).header("cache-control").is_none());
    assert_eq!(
        client.fetch(&server.url("/a/only"), |start| (start.cache, start.mode) = (CacheMode::OnlyIfCached, Mode::SameOrigin)).reason(),
        Reason::NotCached
    );
    // The referrer: the page, an address of the channel the script named,
    // none; and the policy's cut.
    let mut asked_with = |path: &str, change: &dyn Fn(&mut protocol::web::Start)| {
        client.fetch(&server.url(path), change);
        server.the(path)
    };
    assert_eq!(asked_with("/a/r1", &|_| {}).header("referer"), Some(server.url("/a/page")));
    assert_eq!(asked_with("/a/r2", &|start| start.referrer = Referrer::Url(server.url("/a/other?q#f"))).header("referer"), Some(server.url("/a/other?q")));
    assert_eq!(asked_with("/a/r3", &|start| start.referrer = Referrer::Url("http://elsewhere.test/x".into())).header("referer"), Some(server.url("/a/page")));
    assert_eq!(asked_with("/a/r4", &|start| start.referrer = Referrer::None).header("referer"), None);
    assert_eq!(asked_with("/a/r5", &|start| start.referrer_policy = ReferrerPolicy::NoReferrer).header("referer"), None);
    assert_eq!(asked_with("/a/r6", &|start| start.referrer_policy = ReferrerPolicy::Origin).header("referer"), Some(server.url("/")));
    // A request that is not a GET names its origin, or `null` under
    // `no-referrer`.
    let origin = format!("http://site.test:{}", server.port);
    assert_eq!(asked_with("/a/o1", &|start| start.method = "POST".into()).header("origin"), Some(origin.clone()));
    // Under `no-referrer` it still names its origin in the mode `cors`,
    // a script's own, and is `null` only outside it (the standard's
    // "append a request `Origin` header", step 3.1).
    let no_referrer = |mode: Mode| {
        move |start: &mut protocol::web::Start| (start.method, start.referrer_policy, start.mode) = ("POST".into(), ReferrerPolicy::NoReferrer, mode)
    };
    assert_eq!(asked_with("/a/o2", &no_referrer(Mode::Cors)).header("origin"), Some(origin.clone()));
    assert_eq!(asked_with("/a/o3", &no_referrer(Mode::SameOrigin)).header("origin").as_deref(), Some("null"));
    client.finish().unwrap();
}

/// Subresource integrity: a body that matches its metadata is given, one
/// that does not fails before anything of the answer is.
#[test]
fn a_body_is_held_to_its_integrity_metadata() {
    let server = serve(|asked| {
        let reply = Reply::ok("the script").header("access-control-allow-origin", "*");
        if asked.target == "/a/coded" { Reply::new(200).header("content-encoding", "gzip").body(&gzip(b"the script")) } else { reply }
    });
    let mut client = Client::new(&server.url("/a/"));
    let right = crate::web::integrity::metadata_of(b"the script");
    let wrong = "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";
    assert_eq!(client.fetch(&server.url("/a/x"), |start| start.integrity.clone_from(&right)).text(), "the script");
    assert_eq!(client.fetch(&server.url("/a/coded"), |start| start.integrity.clone_from(&right)).text(), "the script", "the decoded body is what is checked");
    assert_eq!(client.fetch(&server.url("/a/x"), |start| start.integrity = wrong.into()).reason(), Reason::Integrity);
    assert_eq!(client.fetch(&server.url("/a/x"), |start| start.integrity = format!("{wrong} {right}")).text(), "the script");
    assert_eq!(client.fetch(&server.at("http", "other.test", "/x"), |start| start.integrity.clone_from(&right)).text(), "the script");
    client.finish().unwrap();
}

/// DEC-7260 item 7's content codings: gzip, deflate and brotli bodies are
/// decoded; an unknown coding is passed on as it came; a body coded
/// twice, cut short or malformed fails.
#[test]
fn coded_bodies_are_decoded() {
    static CUT: AtomicUsize = AtomicUsize::new(0);
    let text: Vec<u8> = (0..5000_u32).flat_map(|n| format!("row {n}\n").into_bytes()).collect();
    let page = text.clone();
    let server = serve(move |asked| {
        let coded = gzip(&page);
        let reply = |coding: &str, body: &[u8]| Reply::new(200).header("content-encoding", coding).body(body);
        match asked.target.as_str() {
            "/a/gzip" => reply("gzip", &coded),
            "/a/br" => reply("br", &BROTLI),
            "/a/identity" => reply("identity", &page),
            "/a/unknown" => reply("zstd", b"\x28\xb5\x2f\xfd"),
            "/a/twice" => reply("gzip, gzip", &gzip(&coded)),
            "/a/cut" => {
                CUT.fetch_add(1, Ordering::SeqCst);
                reply("gzip", &coded[..coded.len() / 2])
            }
            "/a/wrong" => reply("gzip", b"not gzip at all"),
            "/a/head" => reply("gzip", b""),
            _ => Reply::new(404),
        }
    });
    let mut client = Client::new(&server.url("/a/"));
    let gzipped = client.get(&server.url("/a/gzip"));
    assert_eq!(gzipped.body, text);
    assert_eq!(gzipped.header("content-encoding").as_deref(), Some("gzip"), "the header is the script's to read, as in every browser");
    assert_eq!(client.get(&server.url("/a/br")).body, BROTLI_TEXT);
    assert_eq!(client.get(&server.url("/a/identity")).body, text);
    assert_eq!(client.get(&server.url("/a/unknown")).body, b"\x28\xb5\x2f\xfd");
    for path in ["/a/twice", "/a/cut", "/a/wrong"] {
        let outcome = client.get(&server.url(path));
        assert_eq!(outcome.failed.map(|(_, reason)| reason), Some(Reason::Coding), "{path}");
    }
    assert_eq!(client.fetch(&server.url("/a/head"), |start| start.method = "HEAD".into()).ok().status, 200, "a HEAD's answer has no body to decode");
    // A request for a range asks for no coding: a range is of the coded
    // bytes.
    client.fetch(&server.url("/a/range"), |start| start.headers = vec![header("Range", "bytes=0-9")]);
    assert_eq!(server.the("/a/range").header("accept-encoding").as_deref(), Some("identity"));
    client.finish().unwrap();
}
