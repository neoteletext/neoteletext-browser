//! The timed test of `interaction.md` section 10 and requirement 18.5:
//! against the reference channel `t42-site terminal`, twenty keys at twenty
//! a second go through the real fetcher to the channel, whose pushed page
//! comes back; each page is then decoded and rendered by the decoder,
//! encoded as the trusted core encodes it, and decoded as the interface
//! decodes it. The numbers are printed, and each stage must be within the
//! design's budget.
#![cfg(feature = "interaction")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::cast_precision_loss, reason = "a test")]

use std::io::{BufRead, BufReader, BufWriter};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use interface_core::canvas::PageKey;
use neoteletext_input::key::{Event, MAX_RECORD, Press, Record};
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

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn program(name: &str) -> PathBuf {
    // The test runs from target/<profile>/deps; the programs are beside it.
    let deps = std::env::current_exe().unwrap();
    let directory = deps.parent().and_then(|deps| deps.parent()).unwrap().to_path_buf();
    let path = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(path.is_file(), "{} is not built: build the workspace first", path.display());
    path
}

fn record(sequence: u16, event: Event<'_>) -> Vec<u8> {
    let mut out = [0; MAX_RECORD];
    let length = Record { sequence, clock_ms: 0, event }.write(&mut out).unwrap();
    out[..length].to_vec()
}

fn median(times: &mut [Duration]) -> Duration {
    times.sort();
    times[times.len() / 2]
}

#[test]
fn keys_round_trip_and_pages_reach_the_screen_within_budget() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut site = Command::new(program("t42-site")).args(["terminal", &port.to_string()]).stderr(Stdio::piped()).spawn().unwrap();
    let mut serving = BufReader::new(site.stderr.take().unwrap());
    let mut line = String::new();
    serving.read_line(&mut line).unwrap();
    assert!(line.contains("serving"), "{line}");
    let _site = Killed(site);
    let channel = format!("http://127.0.0.1:{port}/terminal/");
    let mut fetcher = Command::new(program("t42-fetcher"))
        .args([channel.as_str(), "--allow-local", "127.0.0.1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
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
    write_to_fetcher(&mut input, &ToFetcher::Send(record(1, Event::Page(Some(PAGE))))).unwrap();
    next_push(Instant::now() + LIMIT);

    let mut engine = decoder::Engine::new();
    let mut frames = t42_browser::frames::Frames::default();
    let key = PageKey { channel: 1, page: PAGE };
    frames.set_visible(vec![key]);
    let (mut round_trips, mut renders, mut encodes, mut decodes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for index in 0..KEYS {
        let character = b'a' + u8::try_from(index % 26).unwrap();
        let press = Press { key: u16::from(character), modifiers: 0, physical: 0 };
        let sent = Instant::now();
        write_to_fetcher(&mut input, &ToFetcher::Send(record(index + 2, Event::Down(press)))).unwrap();
        let (arrived, bytes) = next_push(sent + LIMIT);
        round_trips.push(arrived - sent);
        let started = Instant::now();
        engine.feed_whole(false, true, &bytes);
        let rendered = engine.render(PAGE).expect("the page renders");
        renders.push(started.elapsed());
        let started = Instant::now();
        let (_, png) = frames.accept(key, &rendered.palette, &rendered.indices).expect("a new image");
        encodes.push(started.elapsed());
        let started = Instant::now();
        t42_interface::image::decode(&png).expect("the interface reads it");
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
    assert!(median(&mut pipelines.clone()) <= PIPELINE_BUDGET, "{pipelines:?}");
}
