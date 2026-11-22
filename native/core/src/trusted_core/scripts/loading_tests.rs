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
const FILES: [(&str, &str, &str); 12] = [
    (
        "channel.ttx",
        "application/vnd.neoteletext+json",
        r#"{"name":"Harbour","initial_page":"100","scripts":{"channel":"channel.js","pages":{"100":"p100.js","101":"other.js","201":"js201.js","202":"ticks.js","300":"js300.js","400":"js400.js"}}}"#,
    ),
    ("100.tti", "text/x-teletext-tti", "PN,10000\nSC,0000\nPS,8000\nOL,1,PAGE ONE HUNDRED\n"),
    ("101.tti", "text/x-teletext-tti", "PN,10100\nSC,0000\nPS,8000\nJS,own.js\nOL,1,PAGE ONE OH ONE\n"),
    (
        "channel.js",
        "text/javascript",
        "/*! nt {\"api\":1,\"keys\":[\"c\"],\"tick\":6} */ nt.log('channel'); nt.addEventListener('tick', () => console.log('channel ticks'));",
    ),
    // A JavaScript page whose script ticks and has a timer (TODO-7423).
    (
        "ticks.js",
        "text/javascript",
        "/*! nt {\"api\":1,\"keys\":[\"k\"],\"tick\":6} */ nt.addEventListener('tick', () => console.log('page ticks')); setInterval(() => console.log('page timer'), 100);",
    ),
    ("p100.js", "text/javascript", "/*! nt {\"api\":1,\"keys\":[\"1\"]} */ nt.teletext.page.put(5, 0, 'DRAWN BY P100');"),
    ("other.js", "text/javascript", "/*! nt {\"api\":1,\"keys\":[\"x\"]} */ nt.teletext.page.put(5, 0, 'OTHER');"),
    ("own.js", "text/javascript", "/*! nt {\"api\":1,\"keys\":[\"o\"]} */ nt.teletext.page.put(5, 0, 'DRAWN BY OWN');"),
    // Three JavaScript pages (section 12.3): a script and no page file.
    ("js201.js", "text/javascript", "/*! nt {\"api\":1,\"keys\":[\"j\"]} */ nt.teletext.page.put(5, 0, 'A JAVASCRIPT PAGE');"),
    (
        "js300.js",
        "text/javascript",
        "/*! nt {\"api\":1,\"renderer\":\"regis\",\"keys\":[\"r\"]} */ nt.regis.drawing({ x: 100, y: 100 }).draw((p) => p.ink(2).rect(0, 0, 50, 50));",
    ),
    ("js400.js", "text/javascript", include_str!("../../../../script/tests/naplps_page/harbour-hatched.js")),
    ("record.js", "text/javascript", "/*! nt {\"api\":1,\"keys\":[\"t\"]} */ nt.teletext.page.put(5, 0, 'DRAWN BY RECORD');"),
];

/// Page 102 as a file of packets, `102.t42`: its header, a row, and a
/// `SCRIPT_REF` record that names its script (section 12.4).
fn packet_page() -> Vec<u8> {
    use neoteletext_core::ecc::with_odd_parity;
    use neoteletext_core::packet::encode;
    use neoteletext_core::types::{ControlBits, PageAddress, ScriptName, SubCode};
    let address = PageAddress::from_mpp(0x102).unwrap();
    let mut row = [with_odd_parity(b' '); neoteletext_core::consts::DATA_BYTES];
    for (slot, code) in row.iter_mut().zip(b"PAGE ONE OH TWO") {
        *slot = with_odd_parity(*code);
    }
    let header = encode::header(address.magazine, address.page, SubCode::new(0), ControlBits(0), &[with_odd_parity(b' '); 32]).unwrap();
    let mut bytes: Vec<u8> = [header, encode::row(address.magazine, 1, &row)].iter().flat_map(|packet| packet.bytes).collect();
    let record =
        neoteletext_web::records::script_ref::ScriptRefRecord { page: address, subcode: SubCode::new(0), name: ScriptName::parse(b"record.js").unwrap() };
    neoteletext_web::records::script_ref::encode(record, 0, 0, &mut |packet| bytes.extend_from_slice(&packet.bytes)).unwrap();
    bytes
}

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
        let file = FILES.iter().find(|(name, _, _)| path == format!("/chan/{name}")).map(|(_, media, body)| (*media, body.as_bytes().to_vec()));
        let file = file.or_else(|| (path == "/chan/102.t42").then(|| ("application/x-teletext-t42", packet_page())));
        let answer = match file {
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

/// The decoder's and the fetcher's features for this build's shape, as
/// the gates build every package of a shape (`scripts/check.sh`): each
/// side refuses the other's messages of an area compiled out of it, so a
/// decoder built with hyperlinks is stopped at once by a core built
/// without (TODO-7440). `None`, their defaults, when hyperlinks and
/// typing are both in.
fn helper_features() -> Option<String> {
    let areas = [("links", cfg!(feature = "links")), ("interaction", cfg!(feature = "interaction"))];
    if areas.iter().all(|(_, on)| *on) {
        return None;
    }
    let features: Vec<String> = areas
        .iter()
        .filter(|(_, on)| *on)
        .flat_map(|(area, _)| ["neoteletext-decoder", "neoteletext-fetcher"].map(|package| format!("{package}/{area}")))
        .collect();
    Some(features.join(","))
}

/// Builds `packages`' programs beside this test, in its profile: with
/// their default features, or with `features` alone when given.
fn build(packages: &[&str], features: Option<&str>) {
    let mut build = std::process::Command::new(env!("CARGO"));
    for (name, _) in std::env::vars_os() {
        let runners = name.to_str().is_some_and(|name| name == "OUT_DIR" || (name.starts_with("CARGO_") && !matches!(name, "CARGO_HOME" | "CARGO_TARGET_DIR")));
        if runners {
            build.env_remove(name);
        }
    }
    build.args(["build", "--quiet"]);
    for package in packages {
        build.args(["--package", package]);
    }
    if let Some(features) = features {
        build.arg("--no-default-features");
        if !features.is_empty() {
            build.args(["--features", features]);
        }
    }
    if !cfg!(debug_assertions) {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success(), "{packages:?} did not build");
}

/// The folder of the fetcher, the decoder and the script host, built
/// beside this test in its own profile and its own feature shape: the
/// script host has no optional area, and the decoder and the fetcher are
/// built with the areas this test binary has.
pub(super) fn programs() -> std::path::PathBuf {
    build(&["neoteletext-script-host"], None);
    build(&["neoteletext-fetcher", "neoteletext-decoder"], helper_features().as_deref());
    // The test runs from target/<profile>/deps; the programs are beside it.
    let deps = std::env::current_exe().unwrap();
    deps.parent().and_then(|deps| deps.parent()).unwrap().to_path_buf()
}

/// A trusted core with the real helpers, on the test's channel.
struct Browser {
    core: Core,
    incoming: mpsc::Receiver<Event>,
    stand_in: StandIn,
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
        Self { core, incoming, stand_in, channel, asked }
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

    // A decoder that replaces another is given the scripts its predecessor
    // held (TODO-7391 step 4): the decoder is ended, the core starts
    // another for the channel, and the page's own script runs again,
    // with nothing fetched for it.
    let before = browser.asked().len();
    let old = browser.core.decoders.remove(&browser.channel).unwrap();
    let old_generation = old.generation();
    super::super::processes::stop_decoder(old);
    browser.core.scripts.claims.clear();
    let ran = browser.until_claimed(&["c", "o"]);
    assert!(
        ran,
        "the new decoder's scripts never ran: claims {:?}, state {:?}",
        browser.core.script_claims(browser.channel),
        browser.core.script_state(browser.channel)
    );
    assert_ne!(browser.core.decoders.get(&browser.channel).map(Decoder::generation), Some(old_generation), "another decoder");
    assert!(
        !browser.asked().iter().skip(before).any(|path| std::path::Path::new(path).extension().is_some_and(|extension| extension == "js")),
        "no script was fetched again: {:?}",
        browser.asked()
    );

    // The T42 route (section 12.4): a file of packets names its script by
    // a SCRIPT_REF record, which is asked for by name and runs.
    browser.open(0x102);
    let ran = browser.until_claimed(&["c", "t"]);
    assert!(ran, "page 102's script, named by its record, never ran: claims {:?}, asked {:?}", browser.core.script_claims(browser.channel), browser.asked());
    assert_eq!(browser.asked().iter().filter(|path| *path == "/chan/record.js").count(), 1);
    shut_down(browser.core);
}

/// Section 12.7 and DEC-7315 version 2 (TODO-7304 step 6): a page file
/// opened from disk that names a script by a `JS` line has that script
/// read by the broker, by its name, from the file's folder, and it runs
/// at once, with no prompt, in the confined script host; the core asks
/// the broker for nothing else of the folder.
#[test]
fn a_file_opened_from_disk_runs_the_script_its_page_names() {
    confine::raise_descriptor_limit().unwrap();
    let folder = std::env::temp_dir().join(format!("neoteletext-file-script-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let page = folder.join("101.tti");
    std::fs::write(&page, "PN,10100\nSC,0000\nPS,8000\nJS,own.js\nOL,1,A PAGE FROM DISK\n").unwrap();
    std::fs::write(folder.join("own.js"), "/*! nt {\"api\":1,\"keys\":[\"o\"]} */ nt.teletext.page.put(5, 0, 'DRAWN FROM DISK');").unwrap();
    let (events, incoming) = mpsc::channel();
    let (broker, stand_in) =
        StandIn::start_real(Box::new(|path| std::fs::read(path).map_err(|error| error.to_string())), Box::new(|_: &Started| Some(Vec::new())), programs())
            .unwrap();
    let reader = Arc::clone(&broker);
    let session =
        Session::with_reader(Bookmarks::default(), Box::new(move |path: &std::path::Path| reader.read_source(path).map(crate::source::Source::from_bytes)));
    let settings = Settings { tab_pictures: false, ..Settings::default() };
    let mut core = Core::new(session, Vec::new(), (settings, None), events, Outbox::new(Sink(mpsc::channel().0)), Some(broker));
    core.session.open(page.to_str().unwrap()).unwrap();
    core.session.active = 0;
    core.send_window_whole().unwrap();
    core.flush().unwrap();
    let channel = core.session.tabs[0].channel.id;
    let mut browser = Browser { core, incoming, stand_in, channel, asked: Arc::new(Mutex::new(Vec::new())) };
    browser.open(0x101);
    let ran = browser.until_claimed(&["o"]);
    assert!(ran, "the file's script never ran: claims {:?}, state {:?}", browser.core.script_claims(channel), browser.core.script_state(channel));
    let requests = browser.stand_in.take_requests();
    let scripts: Vec<&confine::broker::Request> = requests.iter().filter(|request| matches!(request, confine::broker::Request::ReadScript { .. })).collect();
    assert!(
        matches!(scripts.as_slice(), [confine::broker::Request::ReadScript { file, name }] if *file == page.to_string_lossy() && name == "own.js"),
        "the script was asked of the broker once, by its name: {scripts:?}"
    );
    shut_down(browser.core);
    let _ = std::fs::remove_dir_all(&folder);
}

/// Section 12.3 (DEC-7246 item 2; TODO-7406): a page the channel file
/// gives a script and that has no page file is a JavaScript page. It
/// opens blank in the renderer its manifest names and its script runs,
/// for teletext, ReGIS and NAPLPS; the 404 of its absent page file is not
/// shown as a page that does not exist. A page with no page file and no
/// script is still refused.
#[test]
fn a_javascript_page_opens_blank_in_its_renderer_and_its_script_runs() {
    confine::raise_descriptor_limit().unwrap();
    let mut browser = Browser::start();
    for (page, keys) in [(0x201, ["c", "j"].as_slice()), (0x300, &["c", "r"]), (0x400, &["c", "l", "b"])] {
        browser.open(page);
        let ran = browser.until_claimed(keys);
        let channel = &browser.core.session.tabs[0].channel;
        assert!(
            ran,
            "page {page:03X}'s script never ran: claims {:?}, failed {:?}, asked {:?}",
            browser.core.script_claims(browser.channel),
            channel.failed,
            browser.asked()
        );
        assert_eq!(browser.core.session.tabs[0].channel.failed, None, "page {page:03X} is shown as its script draws it, not as refused");
        assert_eq!(browser.core.script_state(browser.channel), ScriptState::Running);
    }
    // A page with neither a file nor a script does not exist.
    browser.open(0x555);
    let deadline = Instant::now() + LIMIT;
    while browser.core.session.tabs[0].channel.failed.is_none() && Instant::now() < deadline {
        if let Ok(event) = browser.incoming.recv_timeout(Duration::from_millis(20)) {
            assert_eq!(handle(&mut browser.core, event), Ok(true));
        }
        browser.core.tick().and_then(|()| browser.core.flush()).unwrap();
    }
    assert!(matches!(browser.core.session.tabs[0].channel.failed, Some((_, Some(0x555)))), "{:?}", browser.core.session.tabs[0].channel.failed);
    shut_down(browser.core);
}

/// Runs the event loop for `time`.
fn run(browser: &mut Browser, time: Duration) {
    let until = Instant::now() + time;
    while Instant::now() < until {
        if let Ok(event) = browser.incoming.recv_timeout(Duration::from_millis(20)) {
            assert_eq!(handle(&mut browser.core, event), Ok(true), "an event of the helpers");
        }
        browser.core.tick().and_then(|()| browser.core.flush()).unwrap();
    }
}

/// Section 3.1's table and TODO-7423, through the real fetcher, decoder
/// and confined script host, with the intents an interface gives: a tab
/// hidden behind another pauses both of its contexts (no tick, no
/// timer), and shown again both go on; and the tab already in front
/// chosen again changes nothing: its scripts tick on and its page keeps
/// its keys, though the interface says nothing new of the page.
#[test]
fn a_hidden_tab_pauses_its_scripts_and_the_tab_in_front_chosen_again_changes_nothing() {
    confine::raise_descriptor_limit().unwrap();
    let mut browser = Browser::start();
    // A new tab opens the same channel on this computer, never the
    // default home page.
    browser.core.settings.home = browser.core.session.tabs[0].address().unwrap().text.clone();
    browser.open(0x202);
    assert!(browser.until_claimed(&["c", "k"]), "page 202's script never ran: {:?}", browser.core.script_claims(browser.channel));
    let channel = browser.channel;
    let lines = ["channel ticks", "page ticks", "page timer"];
    // How many of each line the channel's log has gained since last
    // asked: the console opened afresh gives the whole log.
    let mut seen = [0_usize; 3];
    let mut gained = |browser: &mut Browser| -> [usize; 3] {
        browser.core.console.set_open(false);
        browser.core.console.set_open(true);
        let said = format!("{:?}", browser.core.console.updates(Some(channel)));
        browser.core.console.set_open(false);
        let now = lines.map(|line| said.matches(line).count());
        let more = [now[0].saturating_sub(seen[0]), now[1].saturating_sub(seen[1]), now[2].saturating_sub(seen[2])];
        seen = now;
        more
    };
    // The scripts' start takes its time on a loaded machine: wait for the
    // first tick of each context and the timer's first firing.
    let mut first = [0_usize; 3];
    let deadline = Instant::now() + LIMIT;
    while first.contains(&0) && Instant::now() < deadline {
        run(&mut browser, Duration::from_millis(200));
        for (total, more) in first.iter_mut().zip(gained(&mut browser)) {
            *total += more;
        }
    }
    assert!(first.iter().all(|count| *count > 0), "both contexts tick and the page's timer runs: {first:?}");
    // The tab already in front is chosen again (its own key): nothing
    // changes, and the interface says nothing new.
    assert_eq!(browser.core.intent(ToCore::SelectTab(0)), Ok(true));
    run(&mut browser, Duration::from_secs(1));
    let again = gained(&mut browser);
    assert!(again.iter().all(|count| *count > 0), "chosen again, everything goes on: {again:?}");
    let key = PageKey { channel, page: 0x202 };
    assert_eq!(browser.core.typing.opened_page(), Some(key), "and the page is still the open one, with its keys");
    assert_eq!(browser.core.typing.rules_open(channel), browser.core.script_claims(channel).map(|claims| claims.page));
    // Another tab in front: both contexts pause.
    assert_eq!(browser.core.intent(ToCore::NewTab), Ok(true));
    run(&mut browser, Duration::from_millis(500));
    gained(&mut browser);
    run(&mut browser, Duration::from_secs(1));
    let hidden = gained(&mut browser);
    assert_eq!(hidden, [0, 0, 0], "hidden: no tick and no timer of either context");
    assert_eq!(browser.core.script_state(channel), ScriptState::Running);
    // The first tab comes back, and the interface says its page is open.
    assert_eq!(browser.core.intent(ToCore::SelectTab(0)), Ok(true));
    browser.open(0x202);
    run(&mut browser, Duration::from_secs(1));
    let back = gained(&mut browser);
    assert!(back.iter().all(|count| *count > 0), "shown again, both contexts go on: {back:?}");
    assert_eq!(browser.core.script_state(channel), ScriptState::Running);
    shut_down(browser.core);
}
