//! The addresses that break the `url` crate's slicing by position (rdm
//! TODO-7390), given to the web client by every way an address from
//! outside reaches it: a server's `Location`, a script's request, its
//! referrer, an event stream's and a WebSocket's address.
//!
//! The crate (2.5.9) indexes past the end of an address that ends at an
//! empty authority (`foo://`) when it is sliced at
//! `Position::BeforePassword` or `Position::AfterPassword`, and in a
//! debug build fails an assertion at the same two positions for an
//! address with a user name and no password. The web client never
//! slices by position: it reads an address by the crate's accessors and
//! changes it by its setters, none of which goes through that code.
//! These tests hold that for the paths themselves, with those addresses
//! and their neighbours, and are run in a debug build, where the
//! crate's assertions are on, as well as in release.

use protocol::web::{FromWebClient, Referrer, ReferrerPolicy, RequestKind};
use url::Url;

use super::support::{Client, Reply, serve};
use crate::web::policy::ChannelPolicy;
use crate::web::referrer::referer;

/// Addresses at and around the crate's two faults: ending at an empty
/// authority; a user name with no password; a password with no user
/// name; no path; only a query or a fragment after the authority.
const HARD: [&str; 22] = [
    "foo://",
    "web://",
    "foo://x",
    "foo://?q",
    "foo://#f",
    "foo:///",
    "foo:",
    "foo:/",
    "x://@",
    "x://:@",
    "x://u@",
    "ssh://me@example.net",
    "http://me@localhost/",
    "http://me@localhost",
    "http://:pw@localhost/",
    "http://me:@localhost/",
    "http://me:pw@localhost/",
    "http://@localhost/",
    "//me@localhost/a/fast",
    "http://",
    "//",
    "",
];

/// Every reading and changing of an address the web client's code makes
/// (`fetch.rs`, `pool.rs`, `socket.rs`, `referrer.rs`, `cors.rs`,
/// `cookies.rs`, `policy.rs`), on `url`.
fn read_as_the_web_client_reads(url: &Url) {
    let _ = (url.scheme(), url.host(), url.host_str(), url.port(), url.port_or_known_default(), url.path(), url.query(), url.fragment());
    let _ = (url.username(), url.password(), url.as_str(), url.origin().ascii_serialization(), url.origin().is_tuple());
    let mut changed = url.clone();
    changed.set_fragment(None);
    let _ = changed.set_username("");
    let _ = changed.set_password(None);
    let _ = changed.set_scheme("https");
    changed.set_path("/");
    changed.set_query(None);
    let mut other = url.clone();
    other.set_fragment(url.fragment());
    let _ = (changed.as_str(), other.as_str());
}

/// TODO-7390 step 3, the web client's own functions: each hard address,
/// alone and joined to a base as a redirect's `Location` is joined, is
/// read and changed as the web client does, given to the referrer's
/// cutting under every policy both as the page and as the target, and
/// judged by the channel's policy. Nothing panics, in debug or release.
#[test]
fn the_web_clients_readings_take_the_addresses_that_break_position_slicing() {
    let bases: Vec<Url> = ["http://site.test/a/page", "https://user@site.test/a/", "http://site.test"].iter().map(|base| Url::parse(base).unwrap()).collect();
    let policy = ChannelPolicy::new("http://site.test/a/").unwrap();
    let policies = [
        ReferrerPolicy::Unset,
        ReferrerPolicy::NoReferrer,
        ReferrerPolicy::NoReferrerWhenDowngrade,
        ReferrerPolicy::SameOrigin,
        ReferrerPolicy::Origin,
        ReferrerPolicy::StrictOrigin,
        ReferrerPolicy::OriginWhenCrossOrigin,
        ReferrerPolicy::StrictOriginWhenCrossOrigin,
        ReferrerPolicy::UnsafeUrl,
    ];
    let mut parsed = 0;
    for hard in HARD {
        let mut urls: Vec<Url> = Url::parse(hard).into_iter().collect();
        urls.extend(bases.iter().filter_map(|base| base.join(hard).ok()));
        for url in &urls {
            parsed += 1;
            read_as_the_web_client_reads(url);
            let _ = (policy.relation(url), policy.is_same_origin(url));
            for referrer_policy in policies {
                for base in &bases {
                    let _ = referer(referrer_policy, url, base, true);
                    let _ = referer(referrer_policy, base, url, false);
                }
            }
        }
    }
    // The addresses did reach the code: most of them parse or join.
    assert!(parsed >= 40, "{parsed} addresses were read");
    // A referrer never carries a user name, whatever the page's address held.
    let told = referer(ReferrerPolicy::UnsafeUrl, &Url::parse("http://me@localhost/p?q#f").unwrap(), &bases[0], true);
    assert_eq!(told.as_deref(), Some("http://localhost/p?q"));
}

/// TODO-7390 step 3, the web client whole: a server whose redirect names
/// each hard address, and a script whose request, referrer, event stream
/// and socket name them, are each answered or failed, and the web client
/// goes on serving afterwards. A panic in the crate would end its process.
#[test]
fn a_location_or_a_request_with_such_an_address_is_answered_or_failed() {
    let server = serve(|asked| {
        if asked.target == "/a/fast" {
            return Reply::ok("fast");
        }
        let index: usize = asked.target.rsplit('/').next().and_then(|digits| digits.parse().ok()).unwrap_or(0);
        Reply::to(302, HARD[index % HARD.len()])
    });
    let mut client = Client::new(&server.url("/a/"));
    // Each request ends, one way or the other.
    let ended = |client: &mut Client, id: u32| loop {
        match client.next_event() {
            FromWebClient::End(of) | FromWebClient::Failed { id: of, .. } | FromWebClient::SocketClosed { id: of, .. } if of == id => break,
            _ => {}
        }
    };
    for index in 0..HARD.len() {
        let id = client.start(&server.url(&format!("/a/to/{index}")), |_| {});
        ended(&mut client, id);
    }
    // What a script may name: the pipe's own check lets through only
    // http, https, ws and wss, so the user-name forms are what can arrive.
    let port = server.url("/").rsplit(':').next().unwrap().trim_end_matches('/').to_owned();
    for address in [format!("http://me@127.0.0.1:{port}/a/fast"), format!("http://:pw@127.0.0.1:{port}/a/fast"), format!("http://me:@127.0.0.1:{port}/a/fast")]
    {
        for kind in [RequestKind::Fetch, RequestKind::EventSource] {
            let referrer = address.clone();
            let id = client.start(&address, |start| {
                start.kind = kind;
                start.referrer = Referrer::Url(referrer);
                start.referrer_policy = ReferrerPolicy::UnsafeUrl;
            });
            ended(&mut client, id);
        }
        let socket = address.replacen("http", "ws", 1);
        let id = client.start(&socket, |start| start.kind = RequestKind::WebSocket);
        ended(&mut client, id);
    }
    assert_eq!(client.get(&server.url("/a/fast")).text(), "fast", "the web client still serves");
    client.finish().unwrap();
}
