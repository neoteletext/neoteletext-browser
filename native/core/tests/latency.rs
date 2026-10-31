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

use std::time::{Duration, Instant};

use interface_core::canvas::PageKey;
use neoteletext_input::event::{Event, Item};
use protocol::fetcher::{FromFetcher, Origin};

mod reference_channel;
use reference_channel::{Site, Tab, alone, has_header, message, programs, send};

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

fn median(times: &mut [Duration]) -> Duration {
    times.sort();
    times[times.len() / 2]
}

#[test]
fn keys_round_trip_and_pages_reach_the_screen_within_budget() {
    let (site_program, fetcher_program) = programs();
    let (site, _) = Site::start(&site_program);
    let tab = Tab::open(&fetcher_program, site.port);
    let (input, received) = (tab.input(), &tab.received);
    let next_push = |deadline: Instant| loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(wait) {
            // The page, not the sounds pushed before it: whole T42 packets
            // with a page header (neoteletext-common's DEC-1459).
            Ok((at, FromFetcher::Page { origin: Origin::PushedPackets, bytes })) if has_header(&bytes) => return (at, bytes),
            Ok(_) => {}
            Err(error) => panic!("no push in time: {error}"),
        }
    };
    let mut session = tab.session(Instant::now() + LIMIT);
    let select = neoteletext_input::event::Select::new(PAGE, neoteletext_core::limits::SUBCODE_ANY).unwrap();
    send(&input, &alone(message(&mut session, &Item::Event(Event::Select(select, "")))));
    next_push(Instant::now() + LIMIT);

    let mut engine = decoder::Engine::new();
    let mut frames = neoteletext::frames::Frames::default();
    let key = PageKey { channel: 1, page: PAGE };
    frames.set_visible(vec![key]);
    let (mut round_trips, mut renders, mut encodes, mut decodes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    // The frame the interface holds, which the next is read against.
    let mut held: Option<(u32, Vec<u8>)> = None;
    for index in 0..KEYS {
        let character = char::from(b'a' + u8::try_from(index % 26).unwrap());
        let typed = message(&mut session, &Item::Text(character.encode_utf8(&mut [0; 4])));
        let sent = Instant::now();
        send(&input, &alone(typed));
        let (arrived, bytes) = next_push(sent + LIMIT);
        round_trips.push(arrived - sent);
        let started = Instant::now();
        engine.feed_whole(true, true, &bytes);
        let rendered = engine.render(PAGE).expect("the page renders");
        renders.push(started.elapsed());
        let started = Instant::now();
        let (version, image) = frames.accept(key, rendered.palette, rendered.screen, rendered.indices, Instant::now()).expect("a new image");
        encodes.push(started.elapsed());
        let started = Instant::now();
        let read = protocol::frame::decode(&image, held.as_ref().map(|(version, indices): &(u32, Vec<u8>)| (*version, indices.as_slice())))
            .expect("the interface reads it");
        decodes.push(started.elapsed());
        held = Some((version, read.indices));
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
