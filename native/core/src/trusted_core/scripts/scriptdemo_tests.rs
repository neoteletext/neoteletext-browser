//! The example channel of script pages, end to end (rdm TODO-7310, work
//! package X1 of `neoteletext-common/docs/script-api.md`): the channel of
//! the repository neoteletext-server-channel-scriptdemo, a folder of
//! static files, served over HTTP on this computer by the test, and
//! opened page by page through the trusted core as built, the real
//! fetcher, the real decoder and the real script host, each confined as
//! the broker confines it. No interface and no display take part: the
//! test gives the core the intents an interface would. The harness is
//! that of `loading_tests`.
//!
//! What the test can see is what the trusted core is told. For each page
//! it waits until the channel's scripts claim exactly what that page's
//! manifest and the channel script's claim together (section 4.6), which
//! the decoder reports only when it has read the manifests and opened the
//! contexts; lets the scripts run on for [`SETTLE`], so that a fault in a
//! tick, a timer or a request's answer has time to show; and then asks
//! that the scripts still run and that the channel's developer log holds
//! no line of a script that was refused, threw or was stopped.
//!
//! The server is as plain as a static host: a file of the folder by its
//! name, with the media type of its extension, and 404 for anything else,
//! which is what the event stream and the WebSocket pages are answered.

#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::indexing_slicing, clippy::panic, reason = "a test")]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use confine::broker::stand_in::{StandIn, Started};
use interface_core::canvas::PageKey;
use neoteletext::bookmarks::Bookmarks;
use neoteletext::outbox::Outbox;
use neoteletext::settings::Settings;
use protocol::ToCore;
use protocol::console::ConsoleTab;
use protocol::decoder::script::PointerClaim;

use super::super::run::{handle, shut_down};
use super::super::test_support::Sink;
use super::*;
use crate::session::Session;

/// The address the fetcher may reach on this computer, as the browser's
/// command line gives it (`--allow-local`).
const LOCAL: &str = "127.0.0.1";
/// How long a page may take to have its scripts running before the test
/// gives up: a loaded machine starts four confined programs here.
const LIMIT: Duration = Duration::from_secs(60);
/// How long a page's scripts are left to run once they have started.
const SETTLE: Duration = Duration::from_secs(3);
/// The environment variable that names the channel repository's
/// checkout, and where it is looked for without it, under the home
/// folder.
const CHECKOUT_VARIABLE: &str = "NEOTELETEXT_SCRIPTDEMO";
const CHECKOUT_DEFAULT: &str = "devel/neoteletext-server-channel-scriptdemo-worktrees/reqresp";
/// The folder of the checkout that is served.
const SERVED_FOLDER: &str = "channel";

/// The media type a static server gives a file of the channel, by its
/// extension (the channel's README lists the same).
const MEDIA_TYPES: [(&str, &str); 6] = [
    ("ttx", "application/vnd.neoteletext+json"),
    ("tti", "text/x-teletext-tti"),
    ("t42", "application/x-teletext-t42"),
    ("js", "text/javascript"),
    ("json", "application/json"),
    ("txt", "text/plain; charset=utf-8"),
];

/// What a page's scripts claim, which is how the test knows them to run.
struct Page {
    number: u16,
    /// The page-level script, which must have been asked of the server.
    script: &'static str,
    keys: &'static [&'static str],
    text: bool,
    pointer: PointerClaim,
    network: bool,
}

const fn page(number: u16, script: &'static str, keys: &'static [&'static str]) -> Page {
    Page { number, script, keys, text: false, pointer: PointerClaim::None, network: false }
}

/// The keys the pages of components claim: every key a kind of component
/// uses (section 8).
const COMPONENT_KEYS: &[&str] =
    &["Tab", "Enter", " ", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End", "PageUp", "PageDown", "Escape", "Backspace"];

/// Every page of the channel, with what its manifest claims. The
/// channel-level script claims nothing, so the claims are the page's own.
const PAGES: [Page; 14] = [
    Page { pointer: PointerClaim::Press, ..page(0x100, "index.js", &[]) },
    Page { pointer: PointerClaim::Press, ..page(0x101, "teletext-page.js", &["1", "2"]) },
    page(0x110, "teletext-game.js", &["ArrowLeft", "ArrowRight", " "]),
    page(0x120, "bitmap.js", &[]),
    Page { text: true, pointer: PointerClaim::Press, ..page(0x130, "components-teletext.js", COMPONENT_KEYS) },
    Page { network: true, ..page(0x140, "fetch-json.js", &[]) },
    Page { network: true, ..page(0x141, "event-stream.js", &[]) },
    Page { network: true, ..page(0x142, "websocket.js", &["Enter"]) },
    page(0x150, "typescript-page.js", &["Enter", " "]),
    Page { pointer: PointerClaim::Press, ..page(0x200, "regis-page.js", &["Enter"]) },
    page(0x210, "regis-game.js", &["ArrowLeft", "ArrowRight"]),
    page(0x300, "naplps-page.js", &["l", "b"]),
    page(0x310, "naplps-game.js", &["ArrowUp", "ArrowDown"]),
    Page { text: true, pointer: PointerClaim::Press, ..page(0x330, "components-naplps.js", COMPONENT_KEYS) },
];

/// What a line of the developer log holds when a script was refused,
/// threw, or was stopped.
const FAULT_WORDS: [&str; 4] = ["was not run", "uncaught", "stopped", "Error"];

/// The channel's folder.
fn channel_folder() -> PathBuf {
    let checkout = std::env::var_os(CHECKOUT_VARIABLE).map_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(CHECKOUT_DEFAULT), PathBuf::from);
    let folder = checkout.join(SERVED_FOLDER);
    assert!(folder.join("channel.ttx").is_file(), "no channel at {}: set {CHECKOUT_VARIABLE} to the channel repository's checkout", folder.display());
    folder
}

/// A file of the folder by the path asked for, with its media type: a
/// plain file name under `/chan/`, of an extension the channel has.
fn file(folder: &Path, path: &str) -> Option<(&'static str, Vec<u8>)> {
    let name = path.strip_prefix("/chan/")?;
    let plain = !name.is_empty() && !name.starts_with('.') && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
    let extension = Path::new(name).extension()?.to_str()?;
    let media = MEDIA_TYPES.iter().find(|(known, _)| *known == extension)?.1;
    plain.then(|| std::fs::read(folder.join(name)).ok()).flatten().map(|bytes| (media, bytes))
}

/// Answers one connection's requests for the channel's files until it
/// closes, noting each path asked for and the status it was given.
fn serve(stream: TcpStream, folder: &Path, asked: &Mutex<Vec<(String, u16)>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut stream = stream;
    loop {
        let mut request = String::new();
        if reader.read_line(&mut request).unwrap_or(0) == 0 {
            return;
        }
        // The rest of the head, to its empty line.
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 2 {
            line.clear();
        }
        let path = request.split(' ').nth(1).unwrap_or_default().to_owned();
        let found = file(folder, &path);
        asked.lock().unwrap().push((path, if found.is_some() { 200 } else { 404 }));
        let answer = match found {
            Some((media, body)) => {
                let mut answer = format!("HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
                answer.extend(body);
                answer
            }
            None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
        };
        if stream.write_all(&answer).is_err() {
            return;
        }
    }
}

/// The folder of the fetcher, the decoder and the script host, built
/// beside this test in its own profile.
fn programs() -> PathBuf {
    let mut build = std::process::Command::new(env!("CARGO"));
    for (name, _) in std::env::vars_os() {
        let runners = name.to_str().is_some_and(|name| name == "OUT_DIR" || (name.starts_with("CARGO_") && !matches!(name, "CARGO_HOME" | "CARGO_TARGET_DIR")));
        if runners {
            build.env_remove(name);
        }
    }
    build.args(["build", "--quiet"]);
    for package in ["neoteletext-script-host", "neoteletext-fetcher", "neoteletext-decoder"] {
        build.args(["--package", package]);
    }
    if !cfg!(debug_assertions) {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success(), "the helpers did not build");
    // The test runs from target/<profile>/deps; the programs are beside it.
    let deps = std::env::current_exe().unwrap();
    deps.parent().and_then(|deps| deps.parent()).unwrap().to_path_buf()
}

/// A trusted core with the real helpers, on the served channel.
struct Browser {
    core: Core,
    incoming: mpsc::Receiver<Event>,
    _stand_in: StandIn,
    channel: u32,
    asked: Arc<Mutex<Vec<(String, u16)>>>,
}

impl Browser {
    fn start(folder: PathBuf) -> Self {
        let listener = TcpListener::bind((LOCAL, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&asked);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (heard, folder) = (Arc::clone(&heard), folder.clone());
                std::thread::spawn(move || serve(stream, &folder, &heard));
            }
        });
        let (events, incoming) = mpsc::channel();
        let (broker, stand_in) = StandIn::start_real(
            Box::new(|path| Err(format!("this test reads no file, and {} was asked for", path.display()))),
            Box::new(|_: &Started| Some(Vec::new())),
            programs(),
        )
        .unwrap();
        let settings = Settings { tab_pictures: false, ..Settings::default() };
        let mut core = Core::new(
            Session::new(Bookmarks::default()),
            vec![String::from(LOCAL)],
            (settings, None),
            events,
            Outbox::new(Sink(mpsc::channel().0)),
            Some(broker),
        );
        core.session.open(&format!("http://{LOCAL}:{port}/chan/")).unwrap();
        core.session.active = 0;
        core.send_window_whole().unwrap();
        core.flush().unwrap();
        let channel = core.session.tabs[0].channel.id;
        Self { core, incoming, _stand_in: stand_in, channel, asked }
    }

    /// The interface says the viewer opened `page` and that it is on
    /// screen.
    fn open(&mut self, page: u16) {
        let key = PageKey { channel: self.channel, page };
        let window = self.core.session.window;
        assert_eq!(self.core.intent(ToCore::From(window)), Ok(true));
        assert_eq!(self.core.intent(ToCore::Visible(vec![key])), Ok(true));
        assert_eq!(self.core.intent(ToCore::Opened(Some(key))), Ok(true));
    }

    /// One turn of the event loop.
    fn turn(&mut self) {
        if let Ok(event) = self.incoming.recv_timeout(Duration::from_millis(20)) {
            assert_eq!(handle(&mut self.core, event), Ok(true), "an event of the helpers");
        }
        self.core.tick().and_then(|()| self.core.flush()).unwrap();
    }

    /// Whether the channel's scripts claim exactly what `page` and the
    /// channel script claim, for that page.
    fn claims(&self, page: &Page) -> bool {
        self.core.script_claims(self.channel).is_some_and(|claims| {
            claims.running
                && claims.page.page == page.number
                && claims.keys == page.keys
                && claims.text == page.text
                && claims.pointer == page.pointer
                && claims.network == page.network
                && !claims.wheel
        })
    }

    /// Runs the event loop until `page`'s scripts are claimed for, or
    /// [`LIMIT`]; then on for [`SETTLE`].
    fn run(&mut self, page: &Page) -> bool {
        let deadline = Instant::now() + LIMIT;
        while !self.claims(page) {
            // Scripts that were stopped are not started again: there is
            // nothing to wait for.
            if Instant::now() >= deadline || matches!(self.core.script_state(self.channel), ScriptState::Stopped(_)) {
                return false;
            }
            self.turn();
        }
        let settled = Instant::now() + SETTLE;
        while Instant::now() < settled {
            self.turn();
        }
        true
    }

    /// The lines of the channel's developer log since the last asking.
    fn log(&mut self) -> Vec<String> {
        self.core.console.set_open(true);
        let lines = self.core.console.updates(Some(self.channel)).into_iter().flat_map(|update| update.entries).filter(|entry| entry.tab == ConsoleTab::Log);
        lines.map(|entry| format!("{:?}: {}", entry.level, entry.text)).collect()
    }

    fn asked(&self) -> Vec<(String, u16)> {
        self.asked.lock().unwrap().clone()
    }
}

/// Opens `page` and says what is wrong with how its scripts ran: nothing,
/// when they claim what its manifest does, still run after [`SETTLE`],
/// its script was fetched once, and the developer log holds no line of a
/// script refused, thrown or stopped.
fn faults(browser: &mut Browser, page: &Page) -> Vec<String> {
    let mut faults = Vec::new();
    browser.open(page.number);
    let ran = browser.run(page);
    let state = browser.core.script_state(browser.channel);
    let log = browser.log();
    println!("page {:03X}: ran {ran}, {state:?}", page.number);
    for line in &log {
        println!("    {line}");
    }
    if !ran {
        faults.push(format!("page {:03X}: its scripts never claimed what its manifest does: {:?}", page.number, browser.core.script_claims(browser.channel)));
    }
    if state != ScriptState::Running {
        faults.push(format!("page {:03X}: the scripts are {state:?}", page.number));
    }
    let script = format!("/chan/{}", page.script);
    if browser.asked().iter().filter(|(path, status)| *path == script && *status == 200).count() != 1 {
        faults.push(format!("page {:03X}: {script} was not fetched exactly once", page.number));
    }
    for line in log.iter().filter(|line| line.contains("script") && FAULT_WORDS.iter().any(|word| line.contains(word))) {
        faults.push(format!("page {:03X}: the log says: {line}", page.number));
    }
    faults
}

/// TODO-7310's check, a page at a time: each page of the example channel,
/// opened as the first page of a browser of its own, has its script and
/// the channel's run in the real fetcher, core, decoder and confined
/// script host, from a static server; none is refused, throws or is
/// stopped. The pages of the event stream and the WebSocket are answered
/// 404, as a static host answers them, and run on.
#[test]
#[ignore = "needs the checkout of neoteletext-server-channel-scriptdemo's branch reqresp, at ~/devel/neoteletext-server-channel-scriptdemo-worktrees/reqresp or where NEOTELETEXT_SCRIPTDEMO names it"]
fn each_page_of_the_example_channel_opens_and_its_scripts_run() {
    confine::raise_descriptor_limit().unwrap();
    let folder = channel_folder();
    let mut failures = Vec::new();
    let mut asked = Vec::new();
    for page in &PAGES {
        let mut browser = Browser::start(folder.clone());
        failures.extend(faults(&mut browser, page));
        asked.extend(browser.asked());
        shut_down(browser.core);
    }
    println!("asked of the servers: {asked:?}");
    // What a static host has no file for, and is fine without (the
    // channel's README): the two pages' stream and socket.
    assert!(asked.contains(&("/chan/events".to_owned(), 404)), "the event stream page asked for its stream: {asked:?}");
    assert!(asked.contains(&("/chan/socket".to_owned(), 404)), "the WebSocket page asked for its socket: {asked:?}");
    assert!(asked.contains(&("/chan/tides.json".to_owned(), 200)), "the fetch page fetched its file: {asked:?}");
    assert!(failures.is_empty(), "{failures:#?}");
}

/// TODO-7310's check, the channel as a viewer walks it: one browser opens
/// every page in turn, the channel-level script living on from page to
/// page (section 3.1), and each page's scripts run as in the test above;
/// a page's script is fetched only when its page is opened.
#[test]
#[ignore = "needs the checkout of neoteletext-server-channel-scriptdemo's branch reqresp, at ~/devel/neoteletext-server-channel-scriptdemo-worktrees/reqresp or where NEOTELETEXT_SCRIPTDEMO names it"]
fn every_page_of_the_example_channel_opens_in_turn_in_one_browser() {
    confine::raise_descriptor_limit().unwrap();
    let mut browser = Browser::start(channel_folder());
    let mut failures = Vec::new();
    for page in &PAGES {
        let before = browser.asked();
        for later in PAGES.iter().filter(|later| later.number > page.number) {
            let script = format!("/chan/{}", later.script);
            if before.iter().any(|(path, _)| *path == script) {
                failures.push(format!("page {:03X}: {script} was fetched before its page was opened", later.number));
            }
        }
        failures.extend(faults(&mut browser, page));
    }
    println!("asked of the server: {:?}", browser.asked());
    assert!(failures.is_empty(), "{failures:#?}");
    shut_down(browser.core);
}
