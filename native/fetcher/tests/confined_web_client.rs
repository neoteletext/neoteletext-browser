//! The general web client, confined as the broker confines a fetcher
//! (neoteletext-common's DEC-7260 item 6, the browser's DEC-7268): its
//! runtime, its sockets and its threads work inside the fetcher's
//! confinement, and it speaks only `protocol::web`.

use std::ffi::OsString;
use std::io::{BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;

use confine::{Role, Spawn};
use protocol::fetcher::PROCESS_TAB;
use protocol::script::ContextId;
use protocol::web::{
    BodySource, CacheMode, Credentials, FromWebClient, Mode, PartitionToken, RedirectMode, Referrer, ReferrerPolicy, RequestKind, ResponseType, Start,
    ToWebClient, read_from_client, write_to_client,
};

/// A server on this computer that answers every request with `BODY`, and
/// what each connection asked.
fn serve() -> (u16, std::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (asked, told) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            let mut connection = connection.unwrap();
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") && connection.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let _ = asked.send(String::from_utf8_lossy(&head).into_owned());
            let _ = write!(connection, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nSet-Cookie: sid=1\r\nConnection: close\r\n\r\n{BODY}", BODY.len());
        }
    });
    (port, told)
}

const BODY: &str = "PN,10000";
/// The partition's token, as the trusted core would make and give it, and
/// as the process's argument writes it.
const TOKEN: PartitionToken = PartitionToken::new([0xA5; 16]);
const TOKEN_ARGUMENT: &str = "a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5";
/// The tab the test's request is of.
const TAB: u32 = 1;

/// A confined web client fetches a file of its channel from a server on
/// this computer, as a channel loaded from this computer may, and says what the pipe's
/// messages say: that it is confined, the answer's head without its
/// cookie, the body, the jar written out, and the end.
#[test]
#[cfg_attr(
    windows,
    ignore = "a confined fetcher reaches this computer only once an administrator has run CheckNetIsolation.exe LoopbackExempt -a -p=<the fetcher container's SID> (decision 78); run with --ignored after it"
)]
fn a_confined_web_client_fetches_through_its_pipes() {
    let (port, asked) = serve();
    let channel = format!("http://127.0.0.1:{port}/a/");
    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-fetcher"));
    // Its channel was loaded from this computer, so it may ask it.
    let arguments = [channel.as_str(), "--web-client", TOKEN_ARGUMENT, "--page-space", "loopback"].map(OsString::from);
    let environment = confine::environment(Role::Network);
    let mut process = confine::spawn(&Spawn {
        program,
        arguments: &arguments,
        role: Role::Network,
        environment: &environment,
        local_network: true,
        udp_port: None,
        writable: None,
    })
    .unwrap();
    let mut input = process.stdin.take().unwrap();
    let mut output = BufReader::new(process.stdout.take().unwrap());
    let mut next = || read_from_client(&mut output).map_err(|error| error.to_string());
    assert_eq!(next(), Ok(Some((PROCESS_TAB, FromWebClient::Confined))));
    let start = Start {
        id: 1,
        context: ContextId::Page,
        kind: RequestKind::Fetch,
        method: "GET".into(),
        url: format!("{channel}100.tti"),
        client: format!("{channel}page"),
        headers: Vec::new(),
        mode: Mode::Cors,
        credentials: Credentials::SameOrigin,
        cache: CacheMode::Default,
        redirect: RedirectMode::Follow,
        referrer: Referrer::Client,
        referrer_policy: ReferrerPolicy::Unset,
        integrity: String::new(),
        keepalive: false,
        body: BodySource::None,
        protocols: Vec::new(),
        last_event_id: String::new(),
    };
    write_to_client(&mut input, TAB, &TOKEN, &ToWebClient::Start(Box::new(start))).unwrap();
    input.flush().unwrap();
    let mut jar = 0;
    let mut said = Vec::new();
    loop {
        match next() {
            Ok(Some((PROCESS_TAB, FromWebClient::Jar(_)))) => jar += 1,
            Ok(Some((TAB, FromWebClient::End(1)))) => break,
            Ok(Some((TAB, other))) => said.push(other),
            other => panic!("the web client ended early: {other:?}, having said {said:?}"),
        }
    }
    let [FromWebClient::Head(head), FromWebClient::Body { id: 1, bytes }] = said.as_slice() else { panic!("{said:?}") };
    assert_eq!((head.status, head.response_type, head.url.as_str()), (200, ResponseType::Basic, format!("{channel}100.tti").as_str()));
    assert!(head.headers.iter().all(|(name, _)| name != "set-cookie"));
    assert_eq!(bytes, BODY.as_bytes());
    assert_eq!(jar, 2, "the jar written out, its lasting part and its session's");
    assert!(asked.recv().unwrap().starts_with("GET /a/100.tti HTTP/1.1\r\n"));
    // Its pipe closed, it ends by itself, having failed at nothing.
    drop(input);
    assert!(process.wait().unwrap().success());
}
