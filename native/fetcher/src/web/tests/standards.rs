//! The fixed lists and rules held against the standards' own text, read on
//! 22 November 2026: the Fetch standard (fetch.spec.whatwg.org, "Last
//! Updated 20 November 2026", SHA-256 996127d0…c7ed of the page fetched),
//! the W3C Referrer Policy editor's draft (SHA-256 339bdd20…43a6), and
//! the address spaces of the behaviour neoteletext-common's DEC-7315
//! follows, as the browser's TODO-7316 item 5 lists them verified. Each
//! test names the clause it follows. A test that
//! found a fault is ignored with the to-do that owns it, so that it runs
//! with `--ignored` and fails until the fault is mended.

use std::net::IpAddr;

use protocol::web::{AddressSpace, CacheMode, Mode, ReferrerPolicy};

use super::support::{Client, Reply, header, serve};
use crate::web::cors::{exposed, is_safelisted_request_header, unsafe_header_names};
use crate::web::local::space_of;
use crate::web::referrer::policy_of;
use crate::web::rules::{is_bad_port, is_forbidden_method, is_forbidden_request_header, is_forbidden_response_header};

/// The Fetch standard section 2.9, "bad port": the first column of its
/// table, copied from the standard's text, in its order.
const STANDARD_BAD_PORTS: [u16; 83] = [
    0, 1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101, 102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135,
    137, 139, 143, 161, 179, 389, 427, 465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990, 993, 995, 1719, 1720,
    1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669, 6679, 6697, 10080,
];

/// The Fetch standard section 2.9: exactly the standard's ports are
/// blocked, every other of the 65,536 is not.
#[test]
fn the_bad_ports_are_exactly_the_standards() {
    let blocked: Vec<u16> = (0..=u16::MAX).filter(|&port| is_bad_port(port)).collect();
    assert_eq!(blocked, STANDARD_BAD_PORTS);
}

/// The Fetch standard section 2.2.1, "forbidden method": `CONNECT`,
/// `TRACE` and `TRACK`, byte-case-insensitively, and nothing else.
#[test]
fn the_forbidden_methods_are_exactly_the_standards() {
    for method in ["CONNECT", "connect", "TRACE", "Trace", "TRACK", "tRaCk"] {
        assert!(is_forbidden_method(method), "{method}");
    }
    for method in ["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "HEAD", "CHICKEN", "CONNECTS", "TRAC", ""] {
        assert!(!is_forbidden_method(method), "{method}");
    }
}

/// The Fetch standard section 2.2.2, "forbidden request-header": the
/// standard's 21 names, the `proxy-` and `sec-` prefixes, and the three
/// method-override names when a value names a forbidden method. The
/// names reach the rule in lower case (`fetch::Request::new`).
#[test]
fn the_forbidden_request_headers_are_exactly_the_standards() {
    let standard = [
        "accept-charset",
        "accept-encoding",
        "access-control-request-headers",
        "access-control-request-method",
        "connection",
        "content-length",
        "cookie",
        "cookie2",
        "date",
        "dnt",
        "expect",
        "host",
        "keep-alive",
        "origin",
        "referer",
        "set-cookie",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "via",
    ];
    for name in standard {
        assert!(is_forbidden_request_header(name, b"x"), "{name}");
    }
    for name in ["proxy-authorization", "proxy-", "sec-fetch-mode", "sec-", "sec-websocket-key"] {
        assert!(is_forbidden_request_header(name, b"x"), "{name}");
    }
    // Names near the list that the standard leaves to a script.
    for name in [
        "accept",
        "accept-language",
        "authorization",
        "content-type",
        "cookies",
        "dates",
        "hosts",
        "origin-agent-cluster",
        "referrer",
        "secure",
        "proxy",
        "user-agent",
        "x-forwarded-for",
        "te-x",
    ] {
        assert!(!is_forbidden_request_header(name, b"x"), "{name}");
    }
    for name in ["x-http-method", "x-http-method-override", "x-method-override"] {
        assert!(is_forbidden_request_header(name, b"TRACE"), "{name}");
        assert!(is_forbidden_request_header(name, b"get, connect"), "{name}: any of its values");
        assert!(is_forbidden_request_header(name, b"PUT,\tTrack "), "{name}: trimmed of tabs and spaces");
        assert!(!is_forbidden_request_header(name, b"PUT, PATCH"), "{name}");
    }
    assert!(is_forbidden_response_header("set-cookie") && is_forbidden_response_header("set-cookie2"));
    assert!(!is_forbidden_response_header("cookie") && !is_forbidden_response_header("set-cookies"));
}

/// The Fetch standard section 2.2.2, "get, decode, and split": a
/// method-override value is split as the standard splits a header value,
/// so a comma inside double quotes splits nothing, and a quoted string is
/// a value with its quotes.
#[test]
fn a_quoted_method_override_is_read_as_the_standard_splits_it() {
    assert!(!is_forbidden_request_header("x-http-method", b"\"PUT,TRACE\""), "one quoted value that is no method");
    assert!(is_forbidden_request_header("x-method-override", b"\"a,b\", connect"), "a comma inside quotes splits nothing, and the value after it is read");
    assert!(is_forbidden_request_header("x-http-method-override", b"PUT,\tTRACE "), "tabs and spaces at a value's ends are not part of it");
    // The standard keeps a quoted string's quotes (its example: `"1,2", 3`
    // is `"1,2"` and `3`), so a method in quotes is not that method, and a
    // server that reads the header reads no method in it either.
    assert!(!is_forbidden_request_header("x-http-method-override", b"\"TRACE\""));
    assert!(!is_forbidden_request_header("x-method-override", b"PUT, \"connect\""));
}

/// The Fetch standard section 2.2.2, "CORS-safelisted request-header",
/// and its `Range` rule by "parse a single range header value" with
/// whitespace not allowed.
#[test]
fn the_cors_safelisted_request_headers_are_the_standards() {
    for (name, value) in [
        ("accept", "*/*"),
        ("accept", "text/html,\tapplication/xhtml+xml"),
        ("accept-language", "en-AU,en;q=0.9"),
        ("content-language", "de-DE"),
        ("content-type", "application/x-www-form-urlencoded"),
        ("content-type", "TEXT/PLAIN; charset=utf-8"),
        ("content-type", "multipart/form-data; boundary=----x"),
        ("range", "bytes=0-"),
        ("range", "bytes=0-0"),
        ("range", "bytes=10-20"),
    ] {
        assert!(is_safelisted_request_header(name, value.as_bytes()), "{name}: {value}");
    }
    for (name, value) in [
        // Each CORS-unsafe request-header byte in a safelisted name.
        ("accept", "a\"b"),
        ("accept", "a{b}"),
        ("accept", "a\u{7f}"),
        ("accept", "a\u{1}"),
        ("content-type", "text/plain; a=\"b\""),
        ("content-type", "text/plain; a=[b]"),
        ("content-type", "application/json"),
        ("content-type", "text/html"),
        ("accept-language", "en_AU"),
        ("content-language", "de/DE"),
        // Ranges the standard does not safelist.
        ("range", "bytes=-500"),
        ("range", "bytes= 0-1"),
        ("range", "bytes=0-1,3-4"),
        ("range", "items=0-1"),
        ("range", "bytes=a-1"),
        ("range", "bytes="),
        ("range", "bytes=-"),
        // Any other name.
        ("x-requested-with", "XMLHttpRequest"),
        ("cache-control", "no-cache"),
    ] {
        assert!(!is_safelisted_request_header(name, value.as_bytes()), "{name}: {value}");
    }
    // A value of 128 bytes may be safelisted; one of 129 never is.
    assert!(is_safelisted_request_header("accept", "a".repeat(128).as_bytes()));
    assert!(!is_safelisted_request_header("accept", "a".repeat(129).as_bytes()));
    // "CORS-unsafe request-header names": 1,024 bytes of safelisted
    // values together are still safe, 1,025 are not.
    let at = vec![header("accept", &"a".repeat(128)); 8];
    assert_eq!(unsafe_header_names(&at), Vec::<String>::new());
    let mut past = at;
    past.push(header("content-language", "d"));
    assert_eq!(unsafe_header_names(&past), ["accept", "content-language"]);
}

/// The Fetch standard's "parse a single range header value", step "If
/// rangeStartValue and rangeEndValue are numbers, and rangeStartValue is
/// greater than rangeEndValue, then return failure": such a `Range` is not
/// safelisted, so a request across origins with it asks leave first.
#[test]
fn a_range_that_ends_before_it_starts_needs_a_preflight() {
    assert!(!is_safelisted_request_header("range", b"bytes=5-1"));
    assert!(is_safelisted_request_header("range", b"bytes=5-5"));
    assert!(!is_safelisted_request_header("range", b"bytes=10-9") && is_safelisted_request_header("range", b"bytes=9-10"), "as numbers, not as text");
    assert!(
        is_safelisted_request_header("range", b"bytes=007-7") && !is_safelisted_request_header("range", b"bytes=8-007"),
        "leading zeros are no part of the number"
    );
    // Numbers past any machine's are still compared as numbers.
    assert!(is_safelisted_request_header("range", b"bytes=99999999999999999999999998-99999999999999999999999999"));
    assert!(!is_safelisted_request_header("range", b"bytes=99999999999999999999999999-99999999999999999999999998"));
}

/// The Fetch standard section 2.2.2, "CORS-safelisted response-header
/// name": an answer from another origin shows exactly the seven names,
/// those exposed, and never a forbidden response-header name.
#[test]
fn the_cors_safelisted_response_headers_are_exactly_the_standards() {
    let names = [
        "cache-control",
        "content-language",
        "content-length",
        "content-type",
        "expires",
        "last-modified",
        "pragma",
        "date",
        "etag",
        "server",
        "set-cookie",
        "set-cookie2",
        "x-other",
        "access-control-allow-origin",
    ];
    let answer: Vec<(String, Vec<u8>)> = names.iter().map(|name| header(name, "v")).collect();
    let shown: Vec<String> = exposed(&answer, false).into_iter().map(|(name, _)| name).collect();
    assert_eq!(shown, ["cache-control", "content-language", "content-length", "content-type", "expires", "last-modified", "pragma"]);
    let mut asking = answer.clone();
    asking.push(header("access-control-expose-headers", "ETag, Set-Cookie, set-cookie2"));
    let shown: Vec<String> = exposed(&asking, true).into_iter().map(|(name, _)| name).collect();
    assert_eq!(shown, ["cache-control", "content-language", "content-length", "content-type", "expires", "last-modified", "pragma", "etag"]);
}

/// The Referrer Policy draft section 8.1: a token sets the policy when it
/// "is a referrer policy", one of the enumeration's strings, which are
/// written in lower case; a token in other case is no policy and is
/// passed over.
#[test]
#[ignore = "fault TODO-7345 item 3: Referrer-Policy tokens are compared without case (\"No-Referrer\" is taken)"]
fn a_referrer_policy_token_is_compared_exactly() {
    let headers = |value: &str| vec![header("referrer-policy", value)];
    assert_eq!(policy_of(&headers("No-Referrer")), None);
    assert_eq!(policy_of(&headers("origin, UNSAFE-URL")), Some(ReferrerPolicy::Origin));
}

/// The address spaces DEC-7315 follows, as TODO-7316 item 5 lists them at each range's edges: its first and last
/// addresses are of its space, and the address on each side outside it
/// is public unless the engine's wider list (its REQ-SEC-006, kept by
/// DEC-7318 departure F4) holds it, in which case it is local.
#[test]
fn each_range_holds_its_edges_and_nothing_past_them() {
    use AddressSpace::{Local, Loopback, Public};
    let ip = |text: &str| text.parse::<IpAddr>().unwrap();
    for (address, space) in [
        ("127.0.0.0", Loopback),
        ("127.255.255.255", Loopback),
        ("126.255.255.255", Public),
        ("128.0.0.0", Public),
        ("0.0.0.0", Loopback),
        ("0.0.0.1", Local),
        ("0.255.255.255", Local),
        ("1.0.0.0", Public),
        ("::1", Loopback),
        ("::", Loopback),
        ("10.0.0.0", Local),
        ("10.255.255.255", Local),
        ("9.255.255.255", Public),
        ("11.0.0.0", Public),
        ("172.16.0.0", Local),
        ("172.31.255.255", Local),
        ("172.15.255.255", Public),
        ("172.32.0.0", Public),
        ("192.168.0.0", Local),
        ("192.168.255.255", Local),
        ("192.167.255.255", Public),
        ("192.169.0.0", Public),
        ("169.254.0.0", Local),
        ("169.254.255.255", Local),
        ("169.253.255.255", Public),
        ("169.255.0.0", Public),
        ("100.64.0.0", Local),
        ("100.127.255.255", Local),
        ("100.63.255.255", Public),
        ("100.128.0.0", Public),
        ("fc00::", Local),
        ("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Local),
        ("fbff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Public),
        ("fe00::", Public),
        ("fe80::", Local),
        ("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Local),
        ("fe7f:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Public),
        ("fec0::", Local),
        ("feff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Local),
        ("2001:db8::", Local),
        ("2001:db8:ffff:ffff:ffff:ffff:ffff:ffff", Local),
        ("2001:db7:ffff:ffff:ffff:ffff:ffff:ffff", Public),
        ("2001:db9::", Public),
        ("3fff::", Local),
        ("3fff:fff:ffff:ffff:ffff:ffff:ffff:ffff", Local),
        ("3ffe:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Public),
        ("3fff:1000::", Public),
        ("::ffff:10.0.0.1", Local),
        ("::ffff:127.0.0.1", Loopback),
        ("::ffff:1.1.1.1", Public),
    ] {
        assert_eq!(space_of(ip(address)), space, "{address}");
    }
}

/// The Fetch standard section 3.1, "append a request `Origin` header":
/// the referrer policy withholds the origin of a request that is not a
/// `GET` or `HEAD` only "if request's mode is not "cors"". A script's
/// `fetch` is in the mode `cors`, so a `POST` to its own channel under
/// `no-referrer` still names its origin; in the mode `same-origin` it is
/// `null`.
#[test]
#[ignore = "fault TODO-7345 item 4: the referrer policy withholds Origin in the mode cors too (and fetching.rs asserts that)"]
fn the_referrer_policy_withholds_origin_only_outside_the_mode_cors() {
    let server = serve(|_| Reply::ok("x"));
    let mut client = Client::new(&server.url("/a/"));
    let origin = format!("http://site.test:{}", server.port);
    let post = |mode: Mode| {
        move |start: &mut protocol::web::Start| (start.method, start.referrer_policy, start.mode) = ("POST".into(), ReferrerPolicy::NoReferrer, mode)
    };
    client.fetch(&server.url("/a/cors"), post(Mode::Cors));
    client.fetch(&server.url("/a/same"), post(Mode::SameOrigin));
    assert_eq!(server.the("/a/same").header("origin").as_deref(), Some("null"), "outside the mode cors the policy holds");
    assert_eq!(server.the("/a/cors").header("origin"), Some(origin), "in the mode cors it does not");
    client.finish().unwrap();
}

/// The Fetch standard section 4.6, HTTP-network-or-cache fetch: a
/// conditional header turns only the cache mode `default` into
/// `no-store`; under `force-cache` the request carries no `Pragma` or
/// `Cache-Control` of the browser's.
#[test]
#[ignore = "fault TODO-7345 item 5: force-cache with a conditional header is sent as no-store (Pragma and Cache-Control added)"]
fn a_conditional_request_under_force_cache_is_not_made_no_store() {
    let server = serve(|_| Reply::ok("x"));
    let mut client = Client::new(&server.url("/a/"));
    client.fetch(&server.url("/a/force"), |start| (start.cache, start.headers) = (CacheMode::ForceCache, vec![header("If-None-Match", "\"v1\"")]));
    client.fetch(&server.url("/a/default"), |start| start.headers = vec![header("If-None-Match", "\"v1\"")]);
    let (force, default) = (server.the("/a/force"), server.the("/a/default"));
    assert_eq!((default.header("pragma").as_deref(), default.header("cache-control").as_deref()), (Some("no-cache"), Some("no-cache")));
    assert_eq!((force.header("pragma"), force.header("cache-control")), (None, None));
    client.finish().unwrap();
}

/// The Fetch standard's main fetch step "set request's referrer to the
/// result of invoking determine request's referrer", run again for each
/// redirect: once a hop has cut the referrer to its origin, a later hop
/// under a looser policy is told no more than that origin. Here the page
/// asks its own channel, the first answer sets `Referrer-Policy: origin`
/// and redirects, the second sets `unsafe-url` and redirects again.
#[test]
#[ignore = "fault TODO-7345 item 6: each hop's Referer is cut from the page's whole address, so a later looser policy gives back the path"]
fn a_referrer_cut_on_one_hop_stays_cut_on_the_next() {
    let server = serve(|asked| match asked.target.as_str() {
        "/a/one" => Reply::to(302, "/a/two").header("referrer-policy", "origin"),
        "/a/two" => Reply::to(302, "/a/three").header("referrer-policy", "unsafe-url"),
        _ => Reply::ok("x"),
    });
    let mut client = Client::new(&server.url("/a/"));
    assert_eq!(client.get(&server.url("/a/one")).ok().status, 200);
    assert_eq!(server.the("/a/one").header("referer"), Some(server.url("/a/page")));
    assert_eq!(server.the("/a/two").header("referer"), Some(server.url("/")));
    assert_eq!(server.the("/a/three").header("referer"), Some(server.url("/")), "the cut referrer is the one carried on");
    client.finish().unwrap();
}
