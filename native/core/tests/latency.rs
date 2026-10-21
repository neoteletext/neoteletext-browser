//! The timed test of REQ-INT-135: against the reference channel
//! `neoteletext-site terminal`, a session is opened with HELLO and
//! WELCOME as the trusted core opens it, then twenty keys at twenty a
//! second go through the real fetcher to the channel, whose pushed page
//! comes back; each page is then decoded and rendered by the decoder,
//! encoded as the trusted core encodes it, and decoded as the interface
//! decodes it. The numbers are printed; the round trip must be within the
//! design's budget, and in an optimised build the browser's own work too.
#![cfg(feature = "interaction")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::cast_precision_loss, reason = "a test")]

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use interface_core::canvas::PageKey;
use neoteletext_input::control::{Control, FRAGMENT_DATA, from_packet};
use neoteletext_input::event::{Event, Item};
use neoteletext_input::modes::PAGE_CAP;
use neoteletext_input::session::Session;
use protocol::fetcher::{FromFetcher, Origin, ToFetcher, read_from_fetcher, write_to_fetcher};

/// Keys sent, at twenty a second (`interaction.md` section 2).
const KEYS: u16 = 20;
const KEY_INTERVAL: Duration = Duration::from_millis(50);
/// How long anything may take before the test gives up.
const LIMIT: Duration = Duration::from_secs(10);
/// The typing page of the reference channel.
const PAGE: u16 = 0x200;
/// The design's budgets: a pushed packet to the screen in 20 ms, of which
/// the browser's own work here is the decoder's render, the trusted core's
/// encoding and the interface's decoding; and on a computer's own network
/// the channel's answer to a key within the same 20 ms.
const PIPELINE_BUDGET: Duration = Duration::from_millis(20);
const ROUND_TRIP_BUDGET: Duration = Duration::from_millis(20);
/// The fetcher wakes when the channel's answer arrives (REQ-INT-135).
/// When it looked at its sockets every 5 ms, that wait was most of the
/// round trip: a median of 6.3 ms here on 21 October 2026, against 0.3 ms
/// once woken by the bytes themselves, and 0.4 ms in a debug build.
const WAKE_BUDGET: Duration = Duration::from_millis(2);

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The channel and the fetcher, built for this test's profile: a test
/// builds only its own package's programs.
fn programs() -> (PathBuf, PathBuf) {
    let mut build = Command::new(env!("CARGO"));
    build.args(["build", "--quiet", "--package", "neoteletext-site", "--package", "neoteletext-fetcher"]);
    if !cfg!(debug_assertions) {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success(), "the channel and the fetcher did not build");
    // The test runs from target/<profile>/deps; the programs are beside it.
    let deps = std::env::current_exe().unwrap();
    let directory = deps.parent().and_then(|deps| deps.parent()).unwrap().to_path_buf();
    let path = |name: &str| directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    (path("neoteletext-site"), path("neoteletext-fetcher"))
}

/// `item` as the receiver's session encodes it.
fn message(session: &mut Session, item: &Item<'_>) -> Vec<u8> {
    let mut out = [0; neoteletext_input::MAX_MESSAGE];
    let length = session.send(item, Some(PAGE), 0, &mut out).unwrap();
    assert!(length > 0, "{item:?} went");
    out[..length].to_vec()
}

/// HELLO, and the channel's WELCOME in pushed packets (INPUT.md section
/// 6.1): the session the keys then go in.
fn open_session(input: &mut BufWriter<std::process::ChildStdin>, received: &mpsc::Receiver<(Instant, FromFetcher)>, deadline: Instant) -> Session {
    let mut session = Session::new(PAGE_CAP);
    let mut hello = [0; neoteletext_input::MAX_MESSAGE];
    let length = neoteletext_input::event::write(&Item::Event(session.open(0, None)), &mut hello).unwrap();
    write_to_fetcher(input, &ToFetcher::Send(hello[..length].to_vec())).unwrap();
    input.flush().unwrap();
    loop {
        match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((_, FromFetcher::Page { origin: Origin::PushedPackets, bytes })) => {
                let mut buffer = [0; FRAGMENT_DATA];
                let welcome =
                    bytes.as_chunks::<42>().0.iter().find_map(|bytes| match from_packet(&neoteletext_core::packet::Packet { bytes: *bytes }, &mut buffer) {
                        Some(Ok(Control::Welcome { version, caps })) => Some((version, caps)),
                        _ => None,
                    });
                if let Some((version, caps)) = welcome {
                    session.welcome(version, caps).unwrap();
                    return session;
                }
            }
            Ok(_) => {}
            Err(error) => panic!("no WELCOME: {error}"),
        }
    }
}

fn median(times: &mut [Duration]) -> Duration {
    times.sort();
    times[times.len() / 2]
}

#[test]
fn keys_round_trip_and_pages_reach_the_screen_within_budget() {
    let (site_program, fetcher_program) = programs();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut site = Command::new(site_program).args(["terminal", &port.to_string()]).stderr(Stdio::piped()).spawn().unwrap();
    let mut serving = BufReader::new(site.stderr.take().unwrap());
    let mut line = String::new();
    serving.read_line(&mut line).unwrap();
    assert!(line.contains("serving"), "{line}");
    let _site = Killed(site);
    let channel = format!("http://127.0.0.1:{port}/terminal/");
    let mut fetcher =
        Command::new(fetcher_program).args([channel.as_str(), "--allow-local", "127.0.0.1"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut input = BufWriter::new(fetcher.stdin.take().unwrap());
    let output = fetcher.stdout.take().unwrap();
    let _fetcher = Killed(fetcher);
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = BufReader::new(output);
        while let Ok(Some(message)) = read_from_fetcher(&mut output) {
            if sender.send((Instant::now(), message)).is_err() {
                return;
            }
        }
    });
    let next_push = |deadline: Instant| loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(wait) {
            Ok((at, FromFetcher::Page { origin: Origin::PushedPage, bytes })) => return (at, bytes),
            Ok(_) => {}
            Err(error) => panic!("no push in time: {error}"),
        }
    };
    let deadline = Instant::now() + LIMIT;
    loop {
        match received.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((_, FromFetcher::Input { open: true, .. })) => break,
            Ok(_) => {}
            Err(error) => panic!("the channel's input never opened: {error}"),
        }
    }
    let mut session = open_session(&mut input, &received, deadline);
    write_to_fetcher(&mut input, &ToFetcher::Send(message(&mut session, &Item::Event(Event::Page(PAGE))))).unwrap();
    input.flush().unwrap();
    next_push(Instant::now() + LIMIT);

    let mut engine = decoder::Engine::new();
    let mut frames = neoteletext::frames::Frames::default();
    let key = PageKey { channel: 1, page: PAGE };
    frames.set_visible(vec![key]);
    let (mut round_trips, mut renders, mut encodes, mut decodes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for index in 0..KEYS {
        let character = char::from(b'a' + u8::try_from(index % 26).unwrap());
        let typed = message(&mut session, &Item::Text(character.encode_utf8(&mut [0; 4])));
        let sent = Instant::now();
        write_to_fetcher(&mut input, &ToFetcher::Send(typed)).unwrap();
        input.flush().unwrap();
        let (arrived, bytes) = next_push(sent + LIMIT);
        round_trips.push(arrived - sent);
        let started = Instant::now();
        engine.feed_whole(false, true, &bytes);
        let rendered = engine.render(PAGE).expect("the page renders");
        renders.push(started.elapsed());
        let started = Instant::now();
        let (_, png) = frames.accept(key, rendered.palette, rendered.indices).expect("a new image");
        encodes.push(started.elapsed());
        let started = Instant::now();
        neoteletext_interface::image::decode(&png).expect("the interface reads it");
        decodes.push(started.elapsed());
        std::thread::sleep(KEY_INTERVAL.saturating_sub(sent.elapsed()));
    }
    let pipelines: Vec<Duration> = (0..renders.len()).map(|index| renders[index] + encodes[index] + decodes[index]).collect();
    let worst = |times: &[Duration]| times.iter().max().copied().unwrap_or_default();
    eprintln!(
        "latency over {KEYS} keys: key to pushed page median {:?} worst {:?}; render median {:?}, encode median {:?}, interface decode median {:?}; pipeline worst {:?}",
        median(&mut round_trips.clone()),
        worst(&round_trips),
        median(&mut renders),
        median(&mut encodes),
        median(&mut decodes),
        worst(&pipelines)
    );
    assert!(median(&mut round_trips.clone()) <= ROUND_TRIP_BUDGET, "{round_trips:?}");
    assert!(median(&mut round_trips.clone()) <= WAKE_BUDGET, "the fetcher waited for its answer: {round_trips:?}");
    // The budget is the shipped build's: an unoptimised build's render and
    // encoding take about three times as long, and are only reported.
    if !cfg!(debug_assertions) {
        assert!(median(&mut pipelines.clone()) <= PIPELINE_BUDGET, "{pipelines:?}");
    }
}
