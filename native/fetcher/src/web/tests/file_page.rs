//! A page opened from disk as a partition (DEC-7315 version 2, the
//! owner's ruling on local files; TODO-7395 step 1; TODO-7316 item 2): its origin is written `null`, every address is cross-origin
//! to it, it sends and keeps no cookie, nothing is mixed content to it,
//! it fetches no file, and it asks this computer and the local network
//! unasked.

use protocol::web::{AddressSpace, Credentials, FILE_PAGE_IDENTITY, FILE_PAGE_SPACE, Mode, Reason, Referrer, ResponseType};

use super::support::{Client, Reply, serve, serve_tls};

fn file_page() -> Client {
    Client::with(FILE_PAGE_IDENTITY, |setup| setup.space = FILE_PAGE_SPACE)
}

fn include(start: &mut protocol::web::Start) {
    start.credentials = Credentials::Include;
}

/// The page's requests are cross-origin requests of the origin `null`:
/// the answer is the script's only where CORS allows that origin, the
/// `Origin` header says `null`, and no referrer goes, whatever the script
/// names.
#[test]
fn a_file_pages_request_is_a_cross_origin_request_of_the_origin_null() {
    let server = serve(|asked| match asked.target.as_str() {
        "/any" => Reply::ok("any").header("access-control-allow-origin", "*"),
        "/null" => Reply::ok("null").header("access-control-allow-origin", "null"),
        "/other" => Reply::ok("other").header("access-control-allow-origin", "http://localhost"),
        "/with" => Reply::ok("with").header("access-control-allow-origin", "null").header("access-control-allow-credentials", "true"),
        _ => Reply::ok("closed"),
    });
    let mut page = file_page();
    // A plain `http` address is not mixed content to the page.
    for (target, body) in [("/any", "any"), ("/null", "null")] {
        let outcome = page.get(&server.url(target));
        assert_eq!((outcome.ok().response_type, outcome.text().as_str()), (ResponseType::Cors, body), "{target}");
    }
    assert_eq!(page.get(&server.url("/closed")).reason(), Reason::Cors, "no CORS header: the answer is not the script's");
    assert_eq!(page.get(&server.url("/other")).reason(), Reason::Cors);
    assert_eq!(page.fetch(&server.url("/any"), |start| start.mode = Mode::SameOrigin).reason(), Reason::SameOriginMode, "no address is the page's own");
    // With credentials the answer must name the origin `null` exactly.
    assert_eq!(page.fetch(&server.url("/any"), include).reason(), Reason::Cors);
    assert_eq!(page.fetch(&server.url("/with"), include).text(), "with");
    let named = page.fetch(&server.url("/null"), |start| start.referrer = Referrer::Url("file:///home/viewer/page.tti".into()));
    assert_eq!(named.text(), "null");
    let asked = server.asked();
    assert_eq!(asked.len(), 7, "all but the one refused for its mode");
    for request in &asked {
        assert_eq!(request.header("origin").as_deref(), Some("null"), "{}", request.target);
        assert_eq!(request.header("referer"), None, "{}", request.target);
        assert_eq!(request.header("cookie"), None, "{}", request.target);
    }
    // A request that needs a preflight asks it as the origin `null` too.
    let put = page.fetch(&server.url("/null"), |start| start.method = "PUT".into());
    assert_eq!(put.reason(), Reason::Preflight, "the preflight's answer allows no PUT");
    let preflight = server.asked().pop().unwrap();
    assert_eq!((preflight.method.as_str(), preflight.header("origin").as_deref()), ("OPTIONS", Some("null")));
    page.finish().unwrap();
}

/// The page has no cookies: a `Set-Cookie` is not kept, none is sent
/// with credentials asked for, nothing is written out for the trusted
/// core to keep.
#[test]
fn a_file_page_sends_and_keeps_no_cookie() {
    let server = serve(|_| {
        Reply::ok("x").header("access-control-allow-origin", "null").header("access-control-allow-credentials", "true").header("set-cookie", "sid=1; Path=/")
    });
    let mut page = file_page();
    assert_eq!(page.fetch(&server.url("/login"), include).text(), "x");
    assert_eq!(page.fetch(&server.url("/again"), include).text(), "x");
    assert_eq!(server.the("/again").header("cookie"), None);
    assert!(page.jar.iter().all(Vec::is_empty), "no jar was written out");
    page.finish().unwrap();
    // The same answers to a channel of this computer are kept and sent
    // back, so it is the page's rule and not the server's cookie.
    let mut channel = Client::new(&server.at("http", "site.test", "/"));
    assert_eq!(channel.get(&server.at("http", "site.test", "/login")).text(), "x");
    assert_eq!(channel.get(&server.at("http", "site.test", "/back")).text(), "x");
    assert_eq!(server.the("/back").header("cookie").as_deref(), Some("sid=1"));
    channel.finish().unwrap();
}

/// The page fetches no file, its own or another, asked for or redirected
/// to: only `http` and `https` are fetched (the behaviour followed refuses a `file:` address to
/// `fetch` from any page).
#[test]
fn a_file_page_fetches_no_file() {
    let mut page = file_page();
    for address in ["file:///etc/passwd", "file:///home/viewer/other.tti", "file://localhost/etc/hosts"] {
        // The pipe itself carries no such address to a web client.
        assert!(page.unwritable(address, |_| {}), "{address}");
    }
    // Nor does a redirect lead to one.
    let server = serve(|_| Reply::to(302, "file:///etc/passwd").header("access-control-allow-origin", "*"));
    assert_eq!(page.get(&server.url("/away")).reason(), Reason::Redirect);
    page.finish().unwrap();
}

/// Two pages from disk are two partitions: each has its own connection
/// to a server both ask, and its own remembered preflights.
#[test]
fn two_file_pages_share_no_connection_and_no_state() {
    let server = serve(|asked| {
        let reply = Reply::ok("x").header("access-control-allow-origin", "null");
        if asked.method == "OPTIONS" { reply.header("access-control-allow-methods", "PUT").header("access-control-max-age", "600") } else { reply }
    });
    let put = |start: &mut protocol::web::Start| start.method = "PUT".into();
    let mut one = file_page();
    let mut other = file_page();
    assert_eq!(one.fetch(&server.url("/x"), put).text(), "x");
    assert_eq!(one.fetch(&server.url("/x"), put).text(), "x");
    let (connections, preflights) = (server.connections(), server.count("OPTIONS", "/x"));
    assert_eq!(preflights, 1, "one page remembers its preflight");
    assert_eq!(one.fetch(&server.url("/x"), put).text(), "x");
    assert_eq!(server.connections(), connections, "and keeps its connections");
    assert_eq!(other.fetch(&server.url("/x"), put).text(), "x");
    assert!(server.connections() > connections, "the other page has no connection of the first's");
    assert_eq!(server.count("OPTIONS", "/x"), 2, "nor its remembered preflight");
    one.finish().unwrap();
    other.finish().unwrap();
}

/// The page is of this computer's address space ([`FILE_PAGE_SPACE`]):
/// it asks this computer and the local network with no permission, where
/// a channel from the internet over `https` is told it needs the
/// viewer's for the same address.
#[test]
fn a_file_page_asks_this_computer_unasked_where_a_public_page_needs_permission() {
    let secure = serve_tls(&["http/1.1"], |_| Reply::ok("here").header("access-control-allow-origin", "*"));
    let address = secure.at("https", "localhost", "/a/x");
    let mut page = file_page();
    assert_eq!(page.get(&address).text(), "here");
    assert_eq!(secure.the("/a/x").header("origin").as_deref(), Some("null"));
    page.finish().unwrap();
    let mut public = Client::with(&secure.at("https", "public.test", "/a/"), |setup| setup.space = AddressSpace::Public);
    assert_eq!(public.get(&address).reason(), Reason::DevicePermission);
    assert_eq!(public.get("https://192.168.1.1/").reason(), Reason::LocalPermission);
    public.finish().unwrap();
    assert_eq!(secure.asked().len(), 1, "the public page connected to nothing");
}
