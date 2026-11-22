//! The security review's request floods (rdm TODO-7267, TODO-7271 and
//! TODO-7336 item 2; script-api.md section 10.1 rule 6 and section 10.5):
//! `fetchabort.js`, `fetchabort2.js`, `sse.js` and `sse2.js`, which made
//! 1,240 requests in one step of the spike by starting each and giving
//! it up at once. Here a request is counted when it is started and the
//! count is never given back, so the scripts start no more than
//! `MAX_REQUESTS_PER_STEP` a step and `MAX_REQUESTS_PER_SECOND` a
//! second, and open no more than `MAX_EVENT_STREAM_OPENS_PER_STEP` event
//! streams a step, however many they ask for.
mod common;

use common::{Opens as _, container, limited, lines, opening, tick};
use neoteletext_script::Container;
use neoteletext_script::limits::MAX_HOST_STATE_BYTES;
use neoteletext_script::web::limits::{MAX_EVENT_STREAM_OPENS_PER_STEP, MAX_REQUESTS_PER_SECOND, MAX_REQUESTS_PER_STEP};
use neoteletext_script::web::mock::{Answer, MockWebClient};
use protocol::script::ContextId;
use protocol::web::{RequestKind, ToWebClient};

macro_rules! hostile {
    ($name:literal) => {
        include_str!(concat!("hostile/", $name, ".js"))
    };
}

/// The ticks each script is run for: past a second, so that the bound on
/// a second is met as well as the bound on a step.
const STEPS: u64 = 64;
/// The presentation clock's ticks a second.
const SECOND: usize = 60;

/// What a run of a hostile script did.
struct Ran {
    /// The requests of every kind started in each step.
    starts: Vec<usize>,
    /// The event streams among them.
    streams: Vec<usize>,
    /// The requests given up.
    aborts: usize,
    log: Vec<String>,
    /// The most the host held for the container.
    host_bytes: usize,
}

/// Runs `source` as a page's script with the network, in `container`,
/// against a web client that answers nothing a script could wait for.
fn ran(mut container: Container, source: &str) -> Ran {
    let mut open = opening(ContextId::Page);
    open.flags.network = true;
    container.open_with(open, source.as_bytes()).expect("the script opens");
    let mut mock =
        MockWebClient::new(|requested| if requested.start.kind == RequestKind::EventSource { Answer::EventStream(Vec::new()) } else { Answer::Hold });
    let mut ran = Ran { starts: Vec::new(), streams: Vec::new(), aborts: 0, log: Vec::new(), host_bytes: 0 };
    let mut seen = 0;
    for at in 0..STEPS {
        mock.exchange_with(&mut container);
        let new = &mock.seen()[seen..];
        seen = mock.seen().len();
        let started = |kind: Option<RequestKind>| {
            new.iter().filter(|message| matches!(message, ToWebClient::Start(start) if kind.is_none_or(|kind| start.kind == kind))).count()
        };
        ran.starts.push(started(None));
        ran.streams.push(started(Some(RequestKind::EventSource)));
        ran.aborts += new.iter().filter(|message| matches!(message, ToWebClient::Abort(_))).count();
        ran.host_bytes = ran.host_bytes.max(container.host_bytes());
        // A container stopped for its time answers no further step; what it
        // started before then is what is held to the bounds.
        if let Ok(batch) = container.step(&tick(u32::try_from(at).unwrap(), at)) {
            ran.log.extend(lines(&batch));
        }
        ran.host_bytes = ran.host_bytes.max(container.host_bytes());
    }
    ran
}

/// Asserts the bounds of section 10.5 on a run.
#[track_caller]
fn assert_held(name: &str, ran: &Ran) {
    let most_in_a_step = ran.starts.iter().copied().max().unwrap();
    let most_in_a_second = ran.starts.windows(SECOND).map(|window| window.iter().sum::<usize>()).max().unwrap();
    assert!(most_in_a_step <= MAX_REQUESTS_PER_STEP as usize, "{name} started {most_in_a_step} requests in one step");
    assert!(most_in_a_second <= MAX_REQUESTS_PER_SECOND as usize, "{name} started {most_in_a_second} requests in one second");
    let most_streams = ran.streams.iter().copied().max().unwrap();
    assert!(most_streams <= MAX_EVENT_STREAM_OPENS_PER_STEP as usize, "{name} opened {most_streams} event streams in one step");
    assert!(ran.host_bytes <= MAX_HOST_STATE_BYTES, "{name}: the host held {} bytes", ran.host_bytes);
}

/// `fetchabort.js`: 300 fetches, each aborted at once, in one callback.
/// The spike made 300 requests in that step.
#[test]
fn three_hundred_fetches_aborted_at_once_start_eight() {
    let ran = ran(container(), hostile!("fetchabort"));
    assert!(ran.log.iter().any(|line| line == "issued 300"), "{:?}", ran.log);
    assert_held("fetchabort.js", &ran);
    // Eight were started, and counted; the rest waited their turn and were given up before it came.
    assert_eq!(ran.starts.iter().sum::<usize>(), MAX_REQUESTS_PER_STEP as usize);
    assert_eq!(ran.aborts, MAX_REQUESTS_PER_STEP as usize, "only a request that started is given up at the web client");
}

/// `fetchabort2.js`: 31 timers of 40 fetches each, every tick: the 1,240
/// requests a step of the review's measurement.
#[test]
fn twelve_hundred_fetches_a_step_start_eight_a_step_and_thirty_two_a_second() {
    let ran = ran(container(), hostile!("fetchabort2"));
    assert_held("fetchabort2.js", &ran);
    assert_eq!(ran.starts.iter().copied().max(), Some(MAX_REQUESTS_PER_STEP as usize));
    assert_eq!(ran.starts.windows(SECOND).map(|window| window.iter().sum::<usize>()).max(), Some(MAX_REQUESTS_PER_SECOND as usize));
}

/// `sse.js`: 300 event streams, each closed at once, in one callback.
#[test]
fn three_hundred_event_streams_closed_at_once_open_two() {
    let ran = ran(container(), hostile!("sse"));
    assert!(ran.log.iter().any(|line| line == "opened 300"), "{:?}", ran.log);
    assert_held("sse.js", &ran);
    assert_eq!(ran.streams.iter().sum::<usize>(), MAX_EVENT_STREAM_OPENS_PER_STEP as usize);
}

/// `sse2.js`: 31 timers of 60 event streams each, every tick: 1,860 a
/// step asked for.
#[test]
fn eighteen_hundred_event_streams_a_step_open_two_a_step() {
    let ran = ran(container(), hostile!("sse2"));
    assert_held("sse2.js", &ran);
    assert_eq!(ran.streams.iter().copied().max(), Some(MAX_EVENT_STREAM_OPENS_PER_STEP as usize));
    assert!(ran.starts.windows(SECOND).map(|window| window.iter().sum::<usize>()).max().unwrap() <= MAX_REQUESTS_PER_SECOND as usize);
}

/// The same four under the limits the browser's script host runs with:
/// whatever the time budgets let each script do, or stop it for, the
/// bounds on what it starts hold.
#[test]
fn the_floods_are_held_under_the_browsers_time_limits_too() {
    common::unless_starved(|| {
        for (name, source) in
            [("fetchabort.js", hostile!("fetchabort")), ("fetchabort2.js", hostile!("fetchabort2")), ("sse.js", hostile!("sse")), ("sse2.js", hostile!("sse2"))]
        {
            assert_held(name, &ran(limited(), source));
        }
    });
}
