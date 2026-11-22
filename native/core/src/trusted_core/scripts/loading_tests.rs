//! Script loading end to end (`neoteletext-common/docs/script-api.md`
//! sections 12.1 to 12.6; work package C2, TODO-7304): the trusted core
//! as built, against a stand-in broker that starts the real fetcher, the
//! real decoder and the real script host, each confined as the broker
//! confines it, with a static channel served on this computer by the
//! test. No interface and no display take part: the test gives the core
//! the intents an interface would.
//!
//! What the test can see is what the trusted core is told: the claims of
//! the scripts that run, which are their manifests', so a script is known
//! to run by the key only it claims; and the files the channel was asked
//! for. That a running script's batch draws in the decoder is the
//! script-host program's test (`native/script-host/tests/decoder.rs`).

#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::indexing_slicing, clippy::panic, reason = "a test")]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use confine::broker::stand_in::{StandIn, Started};
use interface_core::canvas::PageKey;
use neoteletext::bookmarks::Bookmarks;
use neoteletext::outbox::Outbox;
use neoteletext::settings::Settings;
use protocol::ToCore;

use super::super::run::{handle, shut_down};
use super::super::test_support::Sink;
use super::*;
use crate::session::Session;

/// The address the fetcher may reach on this computer, as the browser's
/// command line gives it (`--allow-local`).
const LOCAL: &str = "127.0.0.1";
/// How long anything may take before the test gives up: a loaded machine
/// builds nothing here, but starts four confined programs.
const LIMIT: Duration = Duration::from_secs(60);

/// The channel's files, by the name each has in its folder, with the
/// media type each is served with. The channel file names a script for
/// the channel, one for page 100 and one for page 101; page 101's own
/// file names another, which wins (section 12.6).
const FILES: [(&str, &str, &str); 7] = [
    (
        "channel.ttx",
        "application/vnd.neoteletext+json",
        r#"{"name":"Harbour","initial_page":"100","scripts":{"channel":"channel.js","pages":{"100":"p100.js","101":"other.js"}}}"#,
    ),
    ("100.tti", "text/x-teletext-tti", "PN,10000\nSC,0000\nPS,8000\nOL,1,PAGE ONE HUNDRED\n"),
    ("101.tti", "text/x-teletext-tti", "PN,10100\nSC,0000\nPS,8000\nJS,own.js\nOL,1,PAGE ONE OH ONE\n"),
    ("channel.js", "text/javascript", "/*! nt {\"api\":1,\"keys\":[\"c\"]} */ nt.log('channel');"),
    ("p100.js", "text/javascript", "/*! nt {\"api\":1,\"keys\":[\"1\"]} */ nt.teletext.page.put(5, 0, 'DRAWN BY P100');"),
    ("other.js", "text/javascript", "/*! nt {\"api\":1,\"keys\":[\"x\"]} */ nt.teletext.page.put(5, 0, 'OTHER');"),
    ("own.js", "text/javascript", "/*! nt {\"api\":1,\"keys\":[\"o\"]} */ nt.teletext.page.put(5, 0, 'DRAWN BY OWN');"),
];

/// Answers one connection's requests for the channel's files until it
/// closes, noting each path asked for.
fn serve(stream: TcpStream, asked: &Mutex<Vec<String>>) {
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
        asked.lock().unwrap().push(path.clone());
        let file = FILES.iter().find(|(name, _, _)| path == format!("/chan/{name}"));
        let answer = match file {
            Some((_, media, body)) => format!("HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
            None => String::from("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"),
        };
        if stream.write_all(answer.as_bytes()).is_err() {
            return;
        }
    }
}

/// The folder of the fetcher, the decoder and the script host, built
/// beside this test in its own profile.
fn programs() -> std::path::PathBuf {
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

/// A trusted core with the real helpers, on the test's channel.
struct Browser {
    core: Core,
    incoming: mpsc::Receiver<Event>,
    _stand_in: StandIn,
    channel: u32,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Browser {
    fn start() -> Self {
        let listener = TcpListener::bind((LOCAL, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let heard = Arc::clone(&asked);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let heard = Arc::clone(&heard);
                std::thread::spawn(move || serve(stream, &heard));
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

    /// Runs the event loop until the channel's scripts claim exactly
    /// `keys`, in order, or [`LIMIT`].
    fn until_claimed(&mut self, keys: &[&str]) -> bool {
        let deadline = Instant::now() + LIMIT;
        while Instant::now() < deadline {
            if let Ok(event) = self.incoming.recv_timeout(Duration::from_millis(20)) {
                assert_eq!(handle(&mut self.core, event), Ok(true), "an event of the helpers");
            }
            self.core.tick().and_then(|()| self.core.flush()).unwrap();
            if self.core.script_claims(self.channel).is_some_and(|claims| claims.keys == keys) {
                return true;
            }
        }
        false
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

/// TODO-7304's "done" and its checks, through the real fetcher, core,
/// decoder and confined script host: a static channel's channel-level
/// script and the page script its channel file names both run when the
/// page opens (section 12.5 step 1); a page whose own file names a script
/// by a `JS` line has that script asked for by name, fetched from the
/// channel's folder and run, and the channel file's script for that page
/// is not run (sections 12.4 to 12.6).
#[test]
fn a_static_channels_scripts_run_by_the_channel_file_and_by_a_page_files_own_line() {
    confine::raise_descriptor_limit().unwrap();
    let mut browser = Browser::start();
    browser.open(0x100);
    let ran = browser.until_claimed(&["c", "1"]);
    assert!(
        ran,
        "the channel's and page 100's scripts never ran: claims {:?}, state {:?}, asked {:?}",
        browser.core.script_claims(browser.channel),
        browser.core.script_state(browser.channel),
        browser.asked()
    );
    assert_eq!(browser.core.script_state(browser.channel), ScriptState::Running);
    assert!(!browser.asked().iter().any(|path| path == "/chan/own.js"), "page 101's script was fetched before its page: {:?}", browser.asked());

    browser.open(0x101);
    let ran = browser.until_claimed(&["c", "o"]);
    assert!(
        ran,
        "page 101's own script never ran: claims {:?}, state {:?}, asked {:?}",
        browser.core.script_claims(browser.channel),
        browser.core.script_state(browser.channel),
        browser.asked()
    );
    let asked = browser.asked();
    assert_eq!(asked.iter().filter(|path| *path == "/chan/own.js").count(), 1, "the page file's script, asked for once by its name: {asked:?}");
    assert_eq!(browser.core.script_state(browser.channel), ScriptState::Running);
    for (channel, decoder) in &browser.core.decoders {
        assert!(decoder.heard(), "the decoder of channel {channel} never said it was confined");
    }
    shut_down(browser.core);
}
