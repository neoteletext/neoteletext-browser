//! Cookies end to end: kept from answers and sent with requests by the
//! credentials mode, never given to a script, kept apart by partition,
//! and written out for the trusted core to keep.

use protocol::web::{Credentials, JarPiece, MAX_CHUNK_BYTES, ToWebClient};

use super::support::{Client, Reply, Server, serve};

/// A server that sets a cookie where asked to and says which it was sent.
fn shop() -> Server {
    serve(|asked| {
        let sent = asked.header("cookie").unwrap_or_default();
        let reply = Reply::ok(&sent)
            .header("access-control-allow-origin", &asked.header("origin").unwrap_or_default())
            .header("access-control-allow-credentials", "true");
        match asked.target.rsplit('/').next().unwrap_or_default() {
            "login" => reply.header("set-cookie", "sid=secret; Path=/; HttpOnly").header("set-cookie", "theme=dark; Path=/; Max-Age=3600"),
            "strict" => reply.header("set-cookie", "strict=1; Path=/; SameSite=Strict").header("set-cookie", "lax=1; Path=/; SameSite=Lax"),
            "logout" => reply.header("set-cookie", "sid=; Path=/; Max-Age=0"),
            _ => reply,
        }
    })
}

/// DEC-7250: a server's cookies are kept and returned as the standard
/// says, by the Fetch standard's credentials modes; and no script reads
/// one, `HttpOnly` or not, from an answer's headers.
#[test]
fn cookies_are_kept_and_returned_by_the_credentials_mode() {
    let server = shop();
    let mut client = Client::new(&server.url("/a/"));
    let login = client.get(&server.url("/a/login"));
    assert!(login.ok().headers.iter().all(|(name, _)| name != "set-cookie"), "no Set-Cookie is a script's to read");
    assert_eq!(client.get(&server.url("/a/x")).text(), "sid=secret; theme=dark");
    assert_eq!(client.fetch(&server.url("/a/x"), |start| start.credentials = Credentials::Omit).text(), "");
    // `omit` keeps none either.
    client.fetch(&server.url("/a/strict"), |start| start.credentials = Credentials::Omit);
    assert_eq!(client.get(&server.url("/a/x")).text(), "sid=secret; theme=dark");
    // Another origin is sent cookies only under `include`, and then only
    // those it set itself that SameSite lets cross sites: none here.
    let there = server.at("http", "other.test", "/login");
    assert_eq!(client.get(&there).text(), "", "same-origin credentials do not cross");
    assert_eq!(client.fetch(&there, |start| start.credentials = Credentials::Include).text(), "");
    assert_eq!(
        client.fetch(&server.at("http", "other.test", "/x"), |start| start.credentials = Credentials::Include).text(),
        "",
        "a Lax cookie set across sites is not kept"
    );
    // A subdomain is the same site: its Strict and Lax cookies are kept
    // and sent.
    let sub = server.at("http", "api.site.test", "/strict");
    client.fetch(&sub, |start| start.credentials = Credentials::Include);
    assert_eq!(client.fetch(&server.at("http", "api.site.test", "/x"), |start| start.credentials = Credentials::Include).text(), "strict=1; lax=1");
    client.get(&server.url("/a/logout"));
    assert_eq!(client.get(&server.url("/a/x")).text(), "theme=dark");
    client.finish().unwrap();
}

/// DEC-7264 item 2 and TODO-7265's checks (a), (c), (d) and (e), under the
/// browser's DEC-7268: each partition's process has its own jar, so one
/// channel's session never goes with another channel's request to the
/// same address, nor a private window's with an ordinary one's.
#[test]
fn each_partitions_process_keeps_its_own_cookies() {
    let server = shop();
    let mut home = Client::new(&server.url("/home/"));
    let mut demo = Client::new(&server.url("/demosite/"));
    let mut inner = Client::new(&server.url("/home/inner/"));
    // A private window's partition of the same channel is another process.
    let mut private = Client::new(&server.url("/home/"));
    // The viewer logs in to /home/.
    home.get(&server.url("/home/login"));
    assert_eq!(home.get(&server.url("/home/x")).text(), "sid=secret; theme=dark");
    // (a), (d), (e): no other partition's request to /home/ carries it,
    // even with credentials asked for and leave given.
    for (other, what) in [(&mut demo, "another channel"), (&mut inner, "a channel inside its folder"), (&mut private, "a private window")] {
        let outcome = other.fetch(&server.url("/home/x"), |start| start.credentials = Credentials::Include);
        assert_eq!(outcome.text(), "", "{what}");
    }
    // (c): what /demosite/'s request to /home/ is set stays in
    // /demosite/'s jar: /home/ keeps its own session.
    demo.fetch(&server.url("/home/logout"), |start| start.credentials = Credentials::Include);
    demo.fetch(&server.url("/home/strict"), |start| start.credentials = Credentials::Include);
    assert_eq!(home.get(&server.url("/home/x")).text(), "sid=secret; theme=dark", "not logged out, and set nothing, by another channel");
    assert_eq!(demo.fetch(&server.url("/home/x"), |start| start.credentials = Credentials::Include).text(), "strict=1; lax=1", "its own, partitioned");
    for client in [home, demo, inner, private] {
        client.finish().unwrap();
    }
}

/// The browser's DEC-7268 item 3: the process writes no file. After a
/// change it writes its jar out for the trusted core, the session's
/// cookies apart from the lasting ones, and a process started later for
/// the partition is given it back.
#[test]
fn the_jar_is_written_out_and_given_back() {
    let server = shop();
    let mut first = Client::new(&server.url("/a/"));
    first.get(&server.url("/a/login"));
    assert_eq!(first.get(&server.url("/a/x")).text(), "sid=secret; theme=dark");
    let [lasting, session] = first.jar.clone();
    assert!(!lasting.is_empty() && !session.is_empty());
    assert!(lasting.windows(5).any(|window| window == b"theme") && !lasting.windows(3).any(|window| window == b"sid"), "what outlives the session, alone");
    assert!(session.windows(3).any(|window| window == b"sid") && !session.windows(5).any(|window| window == b"theme"));
    first.finish().unwrap();

    // The browser was closed: only the lasting part was kept.
    let mut later = Client::new(&server.url("/a/"));
    for piece in lasting.chunks(7) {
        later.send(&ToWebClient::JarKept(JarPiece { session: false, last: false, bytes: piece.to_vec() }));
    }
    later.send(&ToWebClient::JarKept(JarPiece { session: false, last: true, bytes: Vec::new() }));
    assert_eq!(later.get(&server.url("/a/x")).text(), "theme=dark");
    assert_eq!(later.jar, [Vec::new(), Vec::new()], "reading what was kept is no change to write out");
    later.finish().unwrap();

    // The process was ended to make room and started again in the same
    // session: both parts come back.
    let mut again = Client::new(&server.url("/a/"));
    again.send(&ToWebClient::JarKept(JarPiece { session: false, last: true, bytes: lasting }));
    again.send(&ToWebClient::JarKept(JarPiece { session: true, last: true, bytes: session }));
    assert_eq!(again.get(&server.url("/a/x")).text(), "sid=secret; theme=dark");
    again.get(&server.url("/a/logout"));
    assert!(!again.jar[0].is_empty() && again.jar[1].is_empty(), "the session's part, written out empty");
    again.finish().unwrap();

    // The trusted core has the process forget its jar: every cookie goes,
    // and the jar is written out empty.
    let mut forgetful = Client::new(&server.url("/a/"));
    forgetful.get(&server.url("/a/login"));
    assert!(!forgetful.jar[0].is_empty() && !forgetful.jar[1].is_empty());
    forgetful.send(&ToWebClient::ForgetJar);
    assert_eq!(forgetful.get(&server.url("/a/x")).text(), "");
    assert_eq!(forgetful.jar, [Vec::new(), Vec::new()]);
    forgetful.finish().unwrap();

    // A kept jar past its bound is not the trusted core's doing, and ends
    // the process.
    let mut flooded = Client::new(&server.url("/a/"));
    for _ in 0..=protocol::web::MAX_JAR_BYTES / MAX_CHUNK_BYTES {
        flooded.send(&ToWebClient::JarKept(JarPiece { session: false, last: false, bytes: vec![0; MAX_CHUNK_BYTES] }));
    }
    assert!(flooded.ended().is_err());
}
