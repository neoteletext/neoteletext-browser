//! The shared connection end to end, against the NeoTeletext server itself
//! (REQ-6989, REQ-6980, REQ-6982; neoteletext-common's DEC-6979): a fetcher
//! process's tabs, served in this process as `tabs::serve` serves them,
//! with the transport's test certificate trusted in place of Mozilla's
//! roots, and `teletext-server` started with that certificate, so that it
//! serves HTTP/3 over UDP on its port (the server's REQ-6965).
//!
//! These tests need the server's program, which this repository does not
//! build, and `python3` for the channel's program. Each is ignored, saying
//! so; run them with the server's path:
//!
//! ```text
//! NEOTELETEXT_TELETEXT_SERVER=<path to teletext-server> \
//!     cargo test -p neoteletext-fetcher shared_tests -- --ignored --nocapture --test-threads 1
//! ```
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "a test reports a failure by panicking, so it uses `unwrap` on results it expects to be good, `expect` naming what it expected, indexing into data it built, `panic!` on outcomes it rules out and arithmetic on small values it chose"
)]

use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use neoteletext_input::caps::SELECT_CAP;
use neoteletext_input::event::{Event, Item, Select, write as write_item};
use neoteletext_input::state::NoneReason;
use protocol::fetcher::{Origin, StateReply, read_from_fetcher_for};
use rustls::pki_types::CertificateDer;

use super::tests::{LIMIT, PipeWriter, tell};
use super::*;
use crate::process::agent_config;
use crate::transport::{Shared, SharedConnection};

/// The environment variable that names the server's program.
const SERVER: &str = "NEOTELETEXT_TELETEXT_SERVER";
/// Why each test here is ignored unless asked for.
const NEEDS: &str = "needs teletext-server, built from neoteletext-app-server, named by NEOTELETEXT_TELETEXT_SERVER, and python3";
/// The channel's folder on the server.
const FOLDER: &str = "ch";
/// The channel's program (`testdata/channel_program.py`).
const PROGRAM: &str = include_str!("testdata/channel_program.py");
/// The transport's test certificate for `localhost`, valid to 2126, and
/// its key.
const CERTIFICATE: &[u8] = include_bytes!("../transport/testdata/localhost-certificate.der");
const KEY: &[u8] = include_bytes!("../transport/testdata/localhost-key.der");
/// The sub-code that selects a page as a whole (EN 300 706 clause 9.6.1).
const ANY_SUB: u16 = 0x3F7F;
/// How long the server is given to start.
const STARTS_WITHIN: Duration = Duration::from_secs(20);
/// How long a tab may take to learn its push connection is lost when the
/// server goes without a word: the engine's idle timeout of a QUIC
/// connection (30 seconds), or the agent's heartbeat over HTTP/1.1 (a ping
/// after 25 seconds of silence and 10 more for its pong), and a margin.
const LOSS_WITHIN: Duration = Duration::from_secs(60);
/// How often a wait for the server looks again.
const LOOK_EVERY: Duration = Duration::from_millis(20);
/// The characters of a line of PEM (RFC 7468 section 2).
const PEM_LINE: usize = 64;
/// How many tabs a measurement opens one after another.
const MEASURED_TABS: u32 = 12;

/// `der` as PEM under `label` (RFC 7468), which the server's options take.
fn pem(label: &str, der: &[u8]) -> String {
    let mut text = vec![0_u8; der.len().div_ceil(3) * 4];
    let length = neoteletext_web::base64::encode(der, &mut text).unwrap();
    let mut pem = format!("-----BEGIN {label}-----\n");
    for line in text[..length].chunks(PEM_LINE) {
        pem.push_str(std::str::from_utf8(line).unwrap());
        pem.push('\n');
    }
    pem + &format!("-----END {label}-----\n")
}

/// A port of this computer nothing listens on, over TCP or UDP, now.
fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

/// A folder of this test's own, beside the test's program.
fn scratch(name: &str) -> PathBuf {
    let beside = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let folder = beside.join(format!("shared-connection-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// The server, running, with one live channel whose program answers every
/// selection with a page that names it.
struct Server {
    child: Child,
    folder: PathBuf,
    /// The port it listens on, over TCP and, with TLS, over UDP.
    port: u16,
    /// The port viewers reach it at, which its `Alt-Svc` names.
    public: u16,
    admin: u16,
    secure: bool,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::env::var_os("NEOTELETEXT_KEEP_TEST_FOLDERS").is_none() {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }
}

impl Server {
    /// Starts the server in `folder` on `port`, with TLS or without,
    /// telling viewers they reach it at `public`.
    fn start(folder: &Path, port: u16, public: u16, secure: bool) -> Self {
        let program = std::env::var(SERVER).unwrap_or_else(|_| panic!("{NEEDS}"));
        let script = folder.join("channel_program.py");
        std::fs::write(&script, PROGRAM).unwrap();
        let channels = folder.join("channels.json");
        let command = format!(r#"["python3",{:?},{:?}]"#, script.display().to_string(), folder.join("heard").display().to_string());
        std::fs::write(&channels, format!(r#"{{"channels":[{{"folder":"{FOLDER}","name":"Shared","command":{command}}}]}}"#)).unwrap();
        let admin = free_port();
        let scheme = if secure { "https" } else { "http" };
        let mut starting = Command::new(program);
        starting
            .args(["--listen", &format!("127.0.0.1:{port}"), "--origin", &format!("{scheme}://localhost:{public}")])
            .args(["--channels", &channels.display().to_string(), "--admin-listen", &format!("127.0.0.1:{admin}")])
            .env_remove("RUST_LOG")
            .env_remove("STATE_DIRECTORY")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(folder.join(format!("server-{port}.log"))).unwrap());
        if secure {
            std::fs::write(folder.join("certificate.pem"), pem("CERTIFICATE", CERTIFICATE)).unwrap();
            std::fs::write(folder.join("key.pem"), pem("PRIVATE KEY", KEY)).unwrap();
            starting
                .args(["--tls-certificate", &folder.join("certificate.pem").display().to_string()])
                .args(["--tls-key", &folder.join("key.pem").display().to_string()]);
        }
        let server = Self { child: starting.spawn().unwrap(), folder: folder.to_path_buf(), port, public, admin, secure };
        // Serving, and its program started: the program is told `start`
        // first, which it logs.
        let started = Instant::now();
        while server.metric("teletext_connections").is_none() || !folder.join("heard").exists() {
            assert!(started.elapsed() < STARTS_WITHIN, "the server did not start: see {}", folder.display());
            std::thread::sleep(LOOK_EVERY);
        }
        server
    }

    /// The channel's address.
    fn channel(&self) -> String {
        format!("{}://localhost:{}/{FOLDER}/", if self.secure { "https" } else { "http" }, self.public)
    }

    /// A figure of the server's `/metrics`, if it answers.
    fn metric(&self, name: &str) -> Option<u64> {
        let mut stream = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, self.admin)), LIMIT).ok()?;
        stream.write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").ok()?;
        let mut answer = String::new();
        stream.read_to_string(&mut answer).ok()?;
        answer.lines().find_map(|line| line.strip_prefix(name)?.strip_prefix(' ')?.trim().parse().ok())
    }

    /// How many push connections, each a WebSocket, the server holds open now.
    fn websockets(&self) -> u64 {
        self.metric("teletext_connections").expect("the server's metrics")
    }

    /// The TCP connections established to the server's port, as the
    /// operating system counts them.
    fn tcp_connections(&self) -> usize {
        let listing = Command::new("ss").args(["-Htn", "state", "established", &format!("( sport = :{} )", self.port)]).output().expect("ss");
        assert!(listing.status.success(), "ss: {}", String::from_utf8_lossy(&listing.stderr));
        String::from_utf8_lossy(&listing.stdout).lines().filter(|line| !line.trim().is_empty()).count()
    }
}

/// A pipe's reading side that waits for as long as `limit` says, or for
/// as long as nothing is written when it says nothing, as standard input
/// does: the tests' own pipes give up after twenty seconds, which a
/// connection's loss outlasts.
struct Waiting {
    chunks: mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    limit: Option<Duration>,
}

impl Read for Waiting {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            let chunk = match self.limit {
                None => self.chunks.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                Some(limit) => self.chunks.recv_timeout(limit),
            };
            match chunk {
                Ok(chunk) => self.pending = chunk,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(io::ErrorKind::TimedOut.into()),
            }
        }
        let count = buffer.len().min(self.pending.len());
        buffer[..count].copy_from_slice(&self.pending[..count]);
        self.pending.drain(..count);
        Ok(count)
    }
}

/// A fetcher process's tabs, served on a thread as `tabs::serve` serves
/// them, and the trusted core's side of its pipe.
struct Fetcher {
    told: PipeWriter,
    heard: Waiting,
    /// Everything the fetcher has said, in order, with its tab.
    said: Vec<(u32, FromFetcher)>,
    /// The tabs that have said HELLO on the push connection they have now:
    /// a connection takes one, first (the input protocol's DOC-1923).
    greeted: std::collections::BTreeSet<u32>,
    shared: SharedConnection,
    ended: mpsc::Receiver<Result<(), String>>,
}

impl Fetcher {
    /// A fetcher for `channel` that trusts the test certificate, with a
    /// shared connection (as started without `--no-http3`) or without.
    fn start(channel: &str, sharing: bool) -> Self {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(CERTIFICATE.to_vec())).unwrap();
        let config = agent_config(&[IpAddr::V4(Ipv4Addr::LOCALHOST)], false);
        let allowed = config.allowed_local;
        let shared = if sharing { Shared::new(roots.clone(), allowed) } else { None };
        let theirs = shared.clone();
        let transport: MakeTransport = Arc::new(move |wake| Tcp::with_roots(roots.clone(), wake, None, allowed).sharing(theirs.clone()));
        let (written, chunks) = mpsc::channel();
        let (told, mut input) = (PipeWriter(written), Waiting { chunks, pending: Vec::new(), limit: None });
        let (written, chunks) = mpsc::channel();
        let (output, heard) = (PipeWriter(written), Waiting { chunks, pending: Vec::new(), limit: Some(LOSS_WITHIN) });
        let (end, ended) = mpsc::channel();
        let channel = channel.to_owned();
        std::thread::spawn(move || {
            let _ = end.send(serve(&channel, config, &transport, &mut input, Box::new(output)));
        });
        Self { told, heard, said: Vec::new(), greeted: std::collections::BTreeSet::new(), shared, ended }
    }

    /// Reads what the fetcher says until `met` holds of a message, which
    /// is returned; a viewer-state challenge met on the way is answered
    /// "no key", as a trusted core with none answers.
    fn until(&mut self, what: &str, met: impl Fn(u32, &FromFetcher) -> bool) -> (u32, FromFetcher) {
        loop {
            let (tab, message) = read_from_fetcher_for(&mut self.heard)
                .unwrap_or_else(|error| panic!("waiting for {what}: {error}; said: {:?}", self.notices()))
                .unwrap_or_else(|| panic!("the fetcher ended, waiting for {what}"));
            if let FromFetcher::StateChallenged { request, .. } = message {
                tell(&mut self.told, tab, &ToFetcher::StateAnswer { request, reply: StateReply::None(NoneReason::NoKey) });
            }
            if message == FromFetcher::Pushing(false) {
                self.greeted.remove(&tab);
            }
            self.said.push((tab, message.clone()));
            if met(tab, &message) {
                return (tab, message);
            }
        }
    }

    /// As [`Self::until`], counting what was said since the message at
    /// `from` too.
    fn until_since(&mut self, from: usize, what: &str, met: impl Fn(u32, &FromFetcher) -> bool) {
        if !self.said[from..].iter().any(|(tab, message)| met(*tab, message)) {
            self.until(what, met);
        }
    }

    /// Adds `tab` and waits until its push connection is open.
    fn open(&mut self, tab: u32) {
        tell(&mut self.told, tab, &ToFetcher::TabOpens);
        self.until("the push connection", |said, message| said == tab && *message == FromFetcher::Pushing(true));
    }

    /// Selects `page` on `tab`'s push connection, as the trusted core's
    /// input session does, and
    /// waits until `tab` is sent the page the channel's program makes for
    /// it, whose row 1 names it.
    fn select(&mut self, tab: u32, page: u16) {
        let asked = self.ask(tab, page);
        self.sent(asked, tab, page);
    }

    /// Selects `page` on `tab`'s push connection, after HELLO when it is the
    /// connection's first message; where what the fetcher says after it
    /// starts.
    fn ask(&mut self, tab: u32, page: u16) -> usize {
        let mut bytes = Vec::new();
        let hello = Item::Event(Event::Hello { version: neoteletext_input::VERSION, caps: SELECT_CAP, agent: None });
        let select = Item::Event(Event::Select(Select::new(page, ANY_SUB).unwrap(), ""));
        let first = self.greeted.insert(tab);
        for item in first.then_some(hello).into_iter().chain([select]) {
            let mut one = [0_u8; neoteletext_input::MAX_MESSAGE];
            let length = write_item(&item, &mut one).unwrap();
            bytes.extend_from_slice(&one[..length]);
        }
        tell(&mut self.told, tab, &ToFetcher::Send { bytes, move_of: None, more: false });
        self.said.len()
    }

    /// Waits until `tab` has been sent, since `asked`, the page the
    /// channel's program makes for `page`.
    fn sent(&mut self, asked: usize, tab: u32, page: u16) {
        let named = format!("PAGE {page:03X}");
        self.until_since(asked, &named, |said, message| said == tab && names(message, &named));
    }

    /// The pages named to `tab` so far, by the page its row 1 names.
    fn pages_of(&self, tab: u32, named: &str) -> usize {
        self.said.iter().filter(|(said, message)| *said == tab && names(message, named)).count()
    }

    /// What the fetcher has said of its transport, each with its tab.
    fn notices(&self) -> Vec<(u32, String)> {
        self.said.iter().filter_map(|(tab, message)| if let FromFetcher::Notice(text) = message { Some((*tab, text.clone())) } else { None }).collect()
    }

    /// How many times `tab` was told its push connection is a stream of
    /// the shared connection, and how many times that none was available.
    fn streams(&self, tab: u32) -> (usize, usize) {
        let of = |start: &str| self.notices().iter().filter(|(said, text)| *said == tab && text.starts_with(start)).count();
        (of("push: a stream of the channel's shared HTTP/3 connection, answered 200"), of("push: no stream"))
    }
}

/// Whether `message` is a pushed page whose text holds `named`: a row's
/// characters are sent with odd parity (EN 300 706 clause 8.1), so the
/// parity bit is taken off first.
fn names(message: &FromFetcher, named: &str) -> bool {
    let FromFetcher::Page { origin: Origin::PushedPackets, bytes } = message else { return false };
    let text: Vec<u8> = bytes.iter().map(|byte| byte & 0x7F).collect();
    text.windows(named.len()).any(|window| window == named.as_bytes())
}

/// REQ-6989, REQ-6980, DEC-6979 items 1, 2 and 7: two tabs of one channel
/// in one fetcher process open their push connections as two streams of one
/// QUIC connection, each with its own selection, and the server holds no
/// TCP connection for either; where the fetcher has no shared connection
/// (`--no-http3`, REQ-6982) the same two tabs hold a TCP connection each.
/// When the server goes and comes back, both tabs lose their streams
/// together and each opens one again on a new shared connection.
#[test]
#[ignore = "needs teletext-server, built from neoteletext-app-server, named by NEOTELETEXT_TELETEXT_SERVER, and python3"]
fn two_tabs_share_one_http3_connection_and_resume_when_it_is_lost() {
    let folder = scratch("two-tabs");
    let port = free_port();
    let server = Server::start(&folder, port, port, true);
    let idle = server.tcp_connections();

    let mut fetcher = Fetcher::start(&server.channel(), true);
    assert_eq!(fetcher.until("the process's first word", |_, _| true), (PROCESS_TAB, FromFetcher::Confined));
    fetcher.open(1);
    fetcher.open(2);
    assert_eq!((fetcher.streams(1), fetcher.streams(2)), ((1, 0), (1, 0)), "{:?}", fetcher.notices());
    // Each tab's own selection, on its own stream.
    let asked = (fetcher.ask(1, 0x100), fetcher.ask(2, 0x200));
    fetcher.sent(asked.0, 1, 0x100);
    fetcher.sent(asked.1, 2, 0x200);
    assert_eq!((fetcher.pages_of(1, "PAGE 200"), fetcher.pages_of(2, "PAGE 100")), (0, 0), "neither is sent the other's page");
    assert_eq!(server.websockets(), 2, "the server holds two push WebSockets");
    assert_eq!(fetcher.shared.as_ref().unwrap().connections_made(), 1, "on one QUIC connection");
    // The tabs' fetch connections are the only TCP there may be: none
    // carries a WebSocket, which the comparison below shows.
    let shared_tcp = server.tcp_connections() - idle;

    // The same two tabs with no shared connection: a TCP connection each.
    let mut separate = Fetcher::start(&server.channel(), false);
    separate.open(1);
    separate.open(2);
    separate.select(1, 0x100);
    separate.select(2, 0x200);
    assert_eq!((separate.streams(1), separate.streams(2)), ((0, 0), (0, 0)), "no stream was asked for: {:?}", separate.notices());
    assert_eq!(server.websockets(), 4);
    let both_tcp = server.tcp_connections() - idle;
    assert_eq!(both_tcp - shared_tcp, shared_tcp + 2, "two more TCP connections for the two tabs' push WebSockets than their fetches alone");
    drop(separate);

    // The server goes and at once comes back, as when it is restarted:
    // both tabs lose their push connection together, each opens it again
    // for itself, on a new shared connection, and is sent its own selection.
    let before = (fetcher.streams(1).0, fetcher.streams(2).0);
    let gone = fetcher.said.len();
    drop(server);
    let folder = scratch("two-tabs-again");
    let server = Server::start(&folder, port, port, true);
    let restarted = Instant::now();
    for tab in [1, 2] {
        fetcher.until_since(gone, "the push connection's loss", |said, message| said == tab && *message == FromFetcher::Pushing(false));
    }
    println!("both tabs knew their shared connection lost {:?} after the server restarted", restarted.elapsed());
    for tab in [1, 2] {
        let lost = fetcher.said.iter().rposition(|(said, message)| *said == tab && *message == FromFetcher::Pushing(false)).unwrap();
        fetcher.until_since(lost, "the push connection again", |said, message| said == tab && *message == FromFetcher::Pushing(true));
    }
    let asked = (fetcher.ask(1, 0x100), fetcher.ask(2, 0x200));
    fetcher.sent(asked.0, 1, 0x100);
    fetcher.sent(asked.1, 2, 0x200);
    assert_eq!(server.websockets(), 2);
    let after = (fetcher.streams(1).0, fetcher.streams(2).0);
    assert!(after.0 > before.0 && after.1 > before.1, "each tab's push connection is a stream again: {:?}", fetcher.notices());
    assert_eq!(fetcher.shared.as_ref().unwrap().connections_made(), 2, "one new shared connection for both");

    // A tab that leaves closes its own stream, and the other's goes on.
    tell(&mut fetcher.told, 1, &ToFetcher::TabLeaves);
    let started = Instant::now();
    while server.websockets() != 1 {
        assert!(started.elapsed() < LIMIT, "the server still holds {} WebSockets", server.websockets());
        std::thread::sleep(LOOK_EVERY);
    }
    fetcher.select(2, 0x100);
    drop(fetcher.told);
    assert_eq!(fetcher.ended.recv_timeout(LIMIT).unwrap(), Ok(()));
}

/// A relay of TCP connections from a port of its own to `to`, byte for
/// byte: a way to the server that carries no UDP, as a network that drops
/// it does. Its port.
fn tcp_only_way_to(to: u16) -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for near in listener.incoming() {
            let (Ok(near), Ok(far)) = (near, TcpStream::connect((Ipv4Addr::LOCALHOST, to))) else { continue };
            for (mut from, mut onto) in [(near.try_clone().unwrap(), far.try_clone().unwrap()), (far, near)] {
                std::thread::spawn(move || {
                    let _ = io::copy(&mut from, &mut onto);
                    let _ = onto.shutdown(std::net::Shutdown::Write);
                });
            }
        }
    });
    port
}

/// REQ-6989 item 5, DEC-6979 item 3: where the server names HTTP/3 and no
/// QUIC connection can be made, each tab opens its own HTTP/1.1 push
/// connection at once, and the failure is remembered: the next tab asks
/// for no stream, and no second attempt is made.
#[test]
#[ignore = "needs teletext-server, built from neoteletext-app-server, named by NEOTELETEXT_TELETEXT_SERVER, and python3"]
fn where_no_quic_connection_can_be_made_each_tab_falls_back_and_it_is_remembered() {
    let folder = scratch("fallback");
    let port = free_port();
    // The server is told viewers reach it at the relay's port, which it
    // then names in `Alt-Svc`; nothing listens there over UDP.
    let public = tcp_only_way_to(port);
    let server = Server::start(&folder, port, public, true);
    let mut fetcher = Fetcher::start(&server.channel(), true);
    let started = Instant::now();
    fetcher.open(1);
    let first = started.elapsed();
    assert_eq!(fetcher.streams(1), (0, 1), "one attempt, then its own connection: {:?}", fetcher.notices());
    assert!(!fetcher.shared.as_ref().unwrap().usable(), "the failure is remembered");
    assert!(first < neoteletext_transport::quic::HANDSHAKE_TIMEOUT + LIMIT, "the fallback follows the attempt: {first:?}");
    fetcher.select(1, 0x100);
    // The second tab: no attempt at all.
    fetcher.open(2);
    fetcher.select(2, 0x200);
    assert_eq!(fetcher.shared.as_ref().unwrap().connections_made(), 1, "no second QUIC connection was tried");
    assert_eq!(fetcher.streams(2), (0, 0), "its agent was told no stream may be had, and asked for none: {:?}", fetcher.notices());
    assert_eq!(server.websockets(), 2, "both over HTTP/1.1");
    println!("the first tab's push connection opened {first:?} after the tab, the QUIC attempt and its fallback included");
}

/// REQ-6989 item 1: a server that names no HTTP/3 is asked for none. The
/// server started without a certificate serves plain HTTP/1.1, listens on
/// no UDP port and sends no `Alt-Svc`.
#[test]
#[ignore = "needs teletext-server, built from neoteletext-app-server, named by NEOTELETEXT_TELETEXT_SERVER, and python3"]
fn a_server_without_http3_is_served_over_http_1() {
    let folder = scratch("plain");
    let port = free_port();
    let server = Server::start(&folder, port, port, false);
    let mut fetcher = Fetcher::start(&server.channel(), true);
    fetcher.open(1);
    fetcher.open(2);
    fetcher.select(1, 0x100);
    fetcher.select(2, 0x200);
    assert_eq!((fetcher.streams(1), fetcher.streams(2)), ((0, 0), (0, 0)), "{:?}", fetcher.notices());
    assert_eq!(fetcher.shared.as_ref().unwrap().connections_made(), 0, "no QUIC connection was tried");
    assert!(!fetcher.notices().iter().any(|(_, text)| text.contains("AlternativeService")), "it named none");
    assert_eq!(server.websockets(), 2);
}

/// REQ-6980, REQ-881, DEC-6979 item 1: a private window's tab is served by
/// another fetcher process, which has a shared connection of its own: the
/// two processes' tabs are on two QUIC connections.
#[test]
#[ignore = "needs teletext-server, built from neoteletext-app-server, named by NEOTELETEXT_TELETEXT_SERVER, and python3"]
fn another_fetcher_process_shares_no_connection() {
    let folder = scratch("private");
    let port = free_port();
    let server = Server::start(&folder, port, port, true);
    let mut ordinary = Fetcher::start(&server.channel(), true);
    let mut private = Fetcher::start(&server.channel(), true);
    ordinary.open(1);
    private.open(1);
    ordinary.select(1, 0x100);
    private.select(1, 0x200);
    assert_eq!((ordinary.streams(1), private.streams(1)), ((1, 0), (1, 0)));
    let (ours, theirs) = (ordinary.shared.as_ref().unwrap(), private.shared.as_ref().unwrap());
    assert!(!Arc::ptr_eq(ours, theirs));
    assert_eq!((ours.connections_made(), theirs.connections_made()), (1, 1), "a connection each");
    assert_eq!(server.websockets(), 2);
}

/// The time from a tab's opening to its first page, with a tab of the
/// channel already open, over the shared connection and over a connection
/// of the tab's own: the measurement REQ-6989 asks for. Each run opens a
/// tab, selects page 200 the moment its push connection is open, waits for
/// the page and closes the tab. That the shared runs went over the shared
/// connection, and the others did not, is asserted from what each tab said
/// of its transport.
#[test]
#[ignore = "needs teletext-server, built from neoteletext-app-server, named by NEOTELETEXT_TELETEXT_SERVER, and python3"]
fn a_second_tabs_first_page_is_timed_both_ways() {
    let folder = scratch("timed");
    let port = free_port();
    let server = Server::start(&folder, port, port, true);
    let mut results = Vec::new();
    for sharing in [true, false] {
        let mut fetcher = Fetcher::start(&server.channel(), sharing);
        fetcher.open(1);
        fetcher.select(1, 0x100);
        let mut times: Vec<Duration> = (2..2 + MEASURED_TABS)
            .map(|tab| {
                let started = Instant::now();
                fetcher.open(tab);
                fetcher.select(tab, 0x200);
                let took = started.elapsed();
                assert_eq!(fetcher.streams(tab), if sharing { (1, 0) } else { (0, 0) }, "{:?}", fetcher.notices());
                tell(&mut fetcher.told, tab, &ToFetcher::TabLeaves);
                took
            })
            .collect();
        if sharing {
            assert_eq!(fetcher.shared.as_ref().unwrap().connections_made(), 1, "every tab on the one connection");
        }
        times.sort();
        results.push((sharing, times));
    }
    for (sharing, times) in results {
        let way = if sharing { "a stream of the shared HTTP/3 connection" } else { "its own HTTP/1.1 connection" };
        println!(
            "a second tab's opening to its first page over {way}: median {:?}, least {:?}, most {:?}, of {} tabs",
            times[times.len() / 2],
            times[0],
            times[times.len() - 1],
            times.len()
        );
    }
}
