//! Requirement 3.9's harness with the real helpers (TODO-1254): the trusted
//! core, as built, against a stand-in broker that starts the real decoder
//! and fetcher confined as the broker starts them, with two web channels
//! served on this computer by the reference channel, `neoteletext-site
//! terminal`, each of which logs every request it receives and each
//! connection's SELECTs. The same session is run twice, each run with a
//! new pair of channels: once quiet, and once with the hostile rounds
//! interleaved while the first channel's tab is in front. The harness then
//! checks:
//!
//! - that the channel behind, which nothing the interface does may reach
//!   (requirement 4.7, REQ-687; requirement 7.3), received exactly the
//!   requests of the quiet run, in the same order;
//! - that the channel in front received the quiet run's requests, in order,
//!   and besides them only SELECTs of its own pages with the parameters its
//!   address holds, which are none: REQ-687's fourth exception, the page the
//!   interface says is open;
//! - that no request reached any other address: a listener on this computer
//!   that no channel names is never connected to, every fetcher started is
//!   for one of the two channels and may reach only their address, and the
//!   helpers started are those of the quiet run;
//! - that nothing was read or handed on through the broker (no file, no web
//!   address), and that every real helper said it was confined, which on
//!   Linux a helper says only once its every layer is in force (decision
//!   58), so that what it can reach is its sandbox's, which `confine`'s own
//!   tests probe (`confine/tests/probe.rs`).
//!
//! The rounds here send every intent but strip input, the intents that
//! open, close, choose, move or navigate tabs and windows, change settings
//! or answer the strip: REQ-675 excepts strip input, which the user gives,
//! and those intents change what is fetched by design. The first harness
//! sends them too, against the stand-in's pipes. Besides the first
//! harness's samples, which name the channel in front, the rounds replay
//! and mutate messages naming a page of the channel behind.
//!
//! Faults planted on 4 November 2026 were each caught: a core that took a
//! page opened of another tab's channel and selected it on that channel's
//! own connection (the channel behind heard SELECTs of the pages named), and
//! one that started a fetcher for an address the interface typed for
//! suggestions (a fetcher for no channel's address). A fault the quiet run
//! has as well, such as one channel's input given to another's fetcher on
//! every SELECT, is no hostile interface's doing and is not this test's to
//! catch: `a_tabs_reconnection_leaves_the_other_tab_on_its_channel_untouched`
//! (`fetcher_messages.rs`) catches it.

#![allow(clippy::arithmetic_side_effects, reason = "a test's counts and deadlines")]

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use confine::broker::stand_in::{StandIn, Started};
use confine::broker::{Program, Request};
use interface_core::canvas::PageKey;
use protocol::{ToCore, read_from_interface};

use super::super::Core;
use super::super::run::{handle, shut_down};
use super::reference_channel::{Site, build};
use super::{OPEN_WINDOW_EVERY, Random, Sink, encoded, frame, samples, shapes};
use crate::session::Session;
use neoteletext::bookmarks::Bookmarks;
use neoteletext::outbox::Outbox;
use neoteletext::settings::Settings;

/// The address the fetchers may reach on this computer, as the browser's
/// command line gives it (`--allow-local`).
const LOCAL: &str = "127.0.0.1";
/// What the reference channel puts before each line it says.
const SITE_PREFIX: &str = "neoteletext-site: ";
/// The line with which a channel opens a viewer's connection.
const PUSH: &str = "GET /terminal/push";
/// The line with which it ends one.
const CLOSED: &str = ": closed";
/// How long a channel must say nothing new before the session goes on: far
/// more than a fetcher's answer to the core takes on this computer, so the
/// requests of one step are all heard before the next step.
const SETTLED: Duration = Duration::from_millis(600);
/// How long anything may take before the test gives up.
const LIMIT: Duration = Duration::from_secs(30);
/// The hostile rounds before each step of the session.
const ROUNDS_PER_STEP: usize = 2_500;
/// The pages the session opens on the channel in front, in turn, as a
/// viewer stepping through them would.
const STEPS: [u16; 4] = [0x100, 0x200, 0x300, 0x100];
/// The rounds the reader takes that must be sent rather than left as strip
/// input: more than one in this many. Measured on 4 November 2026 on Linux,
/// with every optional feature: 4,858 taken, 2,333 sent, 2,141 of them acted
/// on.
const SENT_FLOOR_DIVISOR: usize = 4;
/// The rounds sent that the core must act on, rather than ignore as for a
/// window not open: more than one in this many, as the first harness asks
/// (its `REACHED_FLOOR_DIVISOR`).
const REACHED_FLOOR_DIVISOR: usize = 2;

/// The decoder's and fetcher's features for this build's shape, as the gates
/// build every package of a shape (`scripts/check.sh`, `feature_list`): each
/// side refuses the other's messages of an area compiled out of it, so a
/// helper built with typing ends at once under a core built without. `None`,
/// their defaults, when hyperlinks and typing are both in.
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

/// Strip input: what the user gives in the trusted strip, which REQ-675
/// excepts and which changes what is fetched by design. Every kind is
/// named, so a new kind must be placed.
fn is_strip_input(intent: &ToCore) -> bool {
    match intent {
        ToCore::SelectTab(_)
        | ToCore::CloseTab(_)
        | ToCore::MoveTab { .. }
        | ToCore::NewTab
        | ToCore::OpenFile { .. }
        | ToCore::Dropped { .. }
        | ToCore::FromSystem(_)
        | ToCore::Navigate(_)
        | ToCore::Back
        | ToCore::Forward
        | ToCore::Bookmark
        | ToCore::Home
        | ToCore::Reload
        | ToCore::ChangeSetting(_)
        | ToCore::InstallUpdate
        | ToCore::History(_)
        | ToCore::Bookmarks(_)
        | ToCore::Star(_)
        | ToCore::Bar(_)
        | ToCore::Suggestion { .. }
        | ToCore::GoTo { .. }
        | ToCore::NewPrivateTab
        | ToCore::Known(_)
        | ToCore::PageControl(_)
        | ToCore::NewWindow { .. }
        | ToCore::TabToWindow { .. }
        | ToCore::CloseWindow
        // The Tools menu's picture panel (DEC-2390) is the browser's own
        // interface, as the strip is: the user's choice of a picture and its
        // options.
        | ToCore::Picture(_) => true,
        #[cfg(feature = "links")]
        ToCore::Confirm(_) | ToCore::Cancel(_) => true,
        #[cfg(feature = "phone-keyboard")]
        ToCore::PhonePanel(_) | ToCore::PhoneFinish | ToCore::PhonePin => true,
        ToCore::Visible(_)
        | ToCore::Cell { .. }
        | ToCore::Opened(_)
        | ToCore::MuteTab(_)
        | ToCore::MuteAll(_)
        | ToCore::Drawn
        | ToCore::FramePhase(_)
        | ToCore::ReducedMotion(_)
        | ToCore::Console(_)
        | ToCore::Recording(_)
        | ToCore::Suggest(_)
        | ToCore::TabList(_)
        | ToCore::Window(_)
        | ToCore::SubPageRows { .. }
        | ToCore::RegisPointer(_)
        | ToCore::PictureFocus { .. }
        | ToCore::PictureFollow { .. }
        | ToCore::Thumbnails(_)
        | ToCore::From(_) => false,
        #[cfg(feature = "links")]
        ToCore::Point(_) | ToCore::Lit(_) | ToCore::Typed { .. } => false,
        #[cfg(any(feature = "links", feature = "interaction"))]
        ToCore::Focused(_) => false,
        #[cfg(feature = "interaction")]
        ToCore::Editing(_) | ToCore::WallWithoutPage | ToCore::Input(_) => false,
    }
}

/// What a channel said, a line at a time, as it says it.
struct Heard {
    lines: mpsc::Receiver<String>,
    said: Vec<String>,
    last: Instant,
}

impl Heard {
    fn new(lines: mpsc::Receiver<String>) -> Self {
        Self { lines, said: Vec::new(), last: Instant::now() }
    }

    /// Takes what has been said so far.
    fn drain(&mut self) {
        while let Ok(line) = self.lines.try_recv() {
            self.said.push(line.strip_prefix(SITE_PREFIX).unwrap_or(&line).to_owned());
            self.last = Instant::now();
        }
    }

    fn count(&self, words: &str) -> usize {
        self.said.iter().filter(|line| line.contains(words)).count()
    }
}

/// One run of the session: the core, its stand-in broker with the real
/// helpers, and the two channels it reaches.
struct Browser {
    core: Core,
    incoming: mpsc::Receiver<super::super::Event>,
    stand_in: StandIn,
    channels: [(Site, Heard); 2],
    /// A listener on this computer that no channel names: nothing may
    /// connect to it.
    decoy: TcpListener,
    folder: PathBuf,
}

impl Browser {
    fn start(programs: &std::path::Path, name: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("neoteletext-real-helpers-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let site = programs.join(format!("neoteletext-site{}", std::env::consts::EXE_SUFFIX));
        let channels = [(), ()].map(|()| {
            let (site, lines) = Site::start(&site);
            (site, Heard::new(lines))
        });
        let decoy = TcpListener::bind((LOCAL, 0)).unwrap();
        decoy.set_nonblocking(true).unwrap();
        let (events, incoming) = mpsc::channel();
        let (sender, _written) = mpsc::channel();
        let (broker, stand_in) = StandIn::start_real(
            Box::new(|path| Err(format!("this test reads no file, and {} was asked for", path.display()))),
            Box::new(|_: &Started| Some(Vec::new())),
            programs.to_path_buf(),
        )
        .unwrap();
        let mut core = Core::new(
            Session::new(Bookmarks::default()),
            vec![String::from(LOCAL)],
            // Tab pictures off: the filmstrip's anonymous snapshots are
            // REQ-687's fifth exception, which the stand-in harness checks
            // (`watch.rs`); this run compares every request each channel
            // heard with a quiet run's, which a snapshot would add to.
            (Settings { tab_pictures: false, ..Settings::default() }, Some(folder.clone())),
            events,
            Outbox::new(Sink(sender)),
            Some(broker),
        );
        for (site, _) in &channels {
            core.session.open(&Self::address(site)).unwrap();
        }
        core.session.active = 0;
        core.send_window_whole().unwrap();
        core.flush().unwrap();
        Self { core, incoming, stand_in, channels, decoy, folder }
    }

    fn address(site: &Site) -> String {
        format!("http://{LOCAL}:{}/terminal/", site.port)
    }

    /// The channel in front's page `page`.
    fn front_page(&self, page: u16) -> PageKey {
        PageKey { channel: self.core.session.tabs[0].channel.id, page }
    }

    /// The channel behind's page `page`.
    fn behind_page(&self, page: u16) -> PageKey {
        PageKey { channel: self.core.session.tabs[1].channel.id, page }
    }

    /// Handles every event that has arrived, then the timed work and the
    /// flush, as the event loop does.
    fn handle_arrived(&mut self) {
        while let Ok(event) = self.incoming.try_recv() {
            assert_eq!(handle(&mut self.core, event), Ok(true), "an event of the helpers");
        }
        self.core.tick().and_then(|()| self.core.flush()).unwrap();
        for (_, heard) in &mut self.channels {
            heard.drain();
        }
    }

    /// Runs the event loop until both channels have opened their viewer's
    /// connection and neither has said anything new for [`SETTLED`].
    fn settle(&mut self) {
        let deadline = Instant::now() + LIMIT;
        loop {
            match self.incoming.recv_timeout(Duration::from_millis(20)) {
                Ok(event) => assert_eq!(handle(&mut self.core, event), Ok(true), "an event of the helpers"),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!("the core's events ended"),
            }
            self.handle_arrived();
            let quiet = self.channels.iter().all(|(_, heard)| heard.count(PUSH) > 0 && heard.last.elapsed() >= SETTLED);
            if quiet {
                return;
            }
            assert!(Instant::now() < deadline, "the channels never settled: {:?}", self.said());
        }
    }

    fn said(&self) -> [Vec<String>; 2] {
        [self.channels[0].1.said.clone(), self.channels[1].1.said.clone()]
    }

    /// Ends the browser as `run` does, and waits until each channel has
    /// closed every viewer's connection it opened; what each said, and every
    /// request the core made of the broker.
    fn finish(mut self) -> ([Vec<String>; 2], Vec<Request>) {
        // Every helper the core holds said it was confined (decision 58).
        for (channel, decoder) in &self.core.decoders {
            assert!(decoder.heard(), "the decoder of channel {channel} never said it was confined");
        }
        for (channel, fetcher) in &self.core.fetchers {
            assert!(fetcher.heard(), "the fetcher of channel {channel} never said it was confined");
        }
        assert!(self.core.fetchers.len() == 2, "a fetcher for each channel: {:?}", self.core.fetchers.keys().collect::<Vec<_>>());
        shut_down(self.core);
        let deadline = Instant::now() + LIMIT;
        for (_, heard) in &mut self.channels {
            while heard.count(CLOSED) < heard.count(PUSH) {
                match heard.lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(line) => heard.said.push(line.strip_prefix(SITE_PREFIX).unwrap_or(&line).to_owned()),
                    Err(error) => panic!("a channel's connections never closed ({error}): {:?}", heard.said),
                }
            }
        }
        assert!(
            matches!(self.decoy.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "something connected to an address no channel names"
        );
        let requests = self.stand_in.take_requests();
        let _ = std::fs::remove_dir_all(&self.folder);
        ([std::mem::take(&mut self.channels[0].1.said), std::mem::take(&mut self.channels[1].1.said)], requests)
    }
}

/// The hostile rounds before a step: frames made as the first harness makes
/// them, each read by the protocol's own reader; those it takes that are not
/// strip input are acted on, with the core's timed work, its flush and the
/// helpers' events after each. One round in [`OPEN_WINDOW_EVERY`] names the
/// open window, as the first harness's do, so that a `From` naming none does
/// not leave the core ignoring the rest. Adds to `counts` how many the reader
/// took, how many were sent, and how many of those the core acted on.
fn hostile_rounds(browser: &mut Browser, random: &mut Random, shapes: &[(u8, usize)], samples: &[Vec<u8>], counts: &mut Counts) {
    for _ in 0..ROUNDS_PER_STEP {
        let bytes = if random.below(OPEN_WINDOW_EVERY) == 0 { encoded(&ToCore::From(browser.core.session.window)) } else { frame(random, shapes, samples) };
        let Ok(Some(intent)) = read_from_interface(&mut bytes.as_slice()) else { continue };
        counts.taken += 1;
        if is_strip_input(&intent) {
            continue;
        }
        counts.sent += 1;
        if !browser.core.from_closed || matches!(intent, ToCore::From(_)) {
            counts.reached += 1;
        }
        let described = format!("{intent:?}");
        let before: Vec<_> = browser.core.session.all_tabs().map(|tab| (tab.channel.id, tab.channel.web.clone())).collect();
        let active = browser.core.session.active;
        assert_eq!(browser.core.intent(intent), Ok(true), "{described}");
        browser.handle_arrived();
        let after: Vec<_> = browser.core.session.all_tabs().map(|tab| (tab.channel.id, tab.channel.web.clone())).collect();
        assert_eq!((after, browser.core.session.active), (before, active), "an intent that is not strip input moved a tab: {described}");
    }
}

/// The hostile rounds' counts.
#[derive(Debug, Default)]
struct Counts {
    /// Taken by the protocol's reader.
    taken: usize,
    /// Of those, sent: not strip input.
    sent: usize,
    /// Of those, acted on by the core.
    reached: usize,
}

/// Messages a hostile interface sends naming a page of the channel behind,
/// `behind`, which the core must not let reach that channel: replayed and
/// mutated with the first harness's samples, which name only the channel in
/// front, since a channel's number made at random is almost never a tab's.
fn naming(behind: PageKey) -> Vec<Vec<u8>> {
    #[cfg_attr(not(any(feature = "links", feature = "interaction")), expect(unused_mut, reason = "only the optional features' messages are added to it"))]
    let mut messages: Vec<ToCore> = [
        ToCore::Opened(Some(behind)),
        ToCore::Visible(vec![behind]),
        ToCore::Cell { key: behind, version: 1, column: 1, row: 1 },
        ToCore::SubPageRows { channel: behind.channel, page: behind.page, range: protocol::subpages::SubPageRange { first: 0, count: 1 }, request: 0 },
    ]
    .into();
    #[cfg(feature = "links")]
    messages.extend([
        ToCore::Typed { number: 0x300, opened: Some((behind, 1)) },
        ToCore::Point(Some(protocol::links::Spot { key: behind, version: 1, column: 1, row: 1 })),
    ]);
    #[cfg(feature = "interaction")]
    {
        use neoteletext_input::event::{Action, KeyCode};
        use protocol::interaction::{InputIntent, Intent};
        let key = Intent::Key { action: Action::Press, key: KeyCode::Character('7'), mods: 0, text: Some(String::from("7")), base: None };
        messages.extend([
            ToCore::Input(InputIntent { key: behind, intent: key }),
            ToCore::Input(InputIntent { key: behind, intent: Intent::Text(String::from("hostile")) }),
        ]);
    }
    messages.iter().map(encoded).collect()
}

/// What one run of the session gave.
struct Run {
    /// What each channel said, the one in front first.
    said: [Vec<String>; 2],
    /// Every request the core made of the broker.
    requests: Vec<Request>,
    /// The channels' addresses, in the same order.
    addresses: [String; 2],
    /// The hostile rounds'.
    counts: Counts,
}

/// The session, quiet or hostile: both channels opened, then the pages of
/// [`STEPS`] opened on the first in turn, the channels settled after each.
fn session(programs: &std::path::Path, hostile: bool) -> Run {
    let mut browser = Browser::start(programs, if hostile { "hostile" } else { "quiet" });
    let addresses = [Browser::address(&browser.channels[0].0), Browser::address(&browser.channels[1].0)];
    browser.settle();
    let mut random = Random(0x5EED_1254_2026);
    let shapes = shapes(&mut random);
    let mut samples = samples(browser.front_page(0x201), browser.core.session.window);
    samples.extend(naming(browser.behind_page(0x201)));
    let mut counts = Counts::default();
    for page in STEPS {
        if hostile {
            hostile_rounds(&mut browser, &mut random, &shapes, &samples, &mut counts);
        }
        // The viewer's step, from the open window, as an interface names it
        // before a message for a window other than the last one named.
        let key = browser.front_page(page);
        let window = browser.core.session.window;
        assert_eq!(browser.core.intent(ToCore::From(window)), Ok(true));
        assert_eq!(browser.core.intent(ToCore::Opened(Some(key))), Ok(true));
        browser.handle_arrived();
        browser.settle();
    }
    let (said, requests) = browser.finish();
    Run { said, requests, addresses, counts }
}

/// The lines of `hostile` that `quiet` lacks, once `quiet` is found in it in
/// order; panics if it is not.
fn beyond(quiet: &[String], hostile: &[String]) -> Vec<String> {
    let mut extra = Vec::new();
    let mut expected = quiet.iter().peekable();
    for line in hostile {
        if expected.peek() == Some(&line) {
            expected.next();
        } else {
            extra.push(line.clone());
        }
    }
    assert!(expected.peek().is_none(), "the hostile run lacks the quiet run's {:?}: quiet {quiet:?}, hostile {hostile:?}", expected.collect::<Vec<_>>());
    extra
}

/// Whether `line` is a SELECT of a page of the channel with no parameters:
/// its address has no query, so a SELECT with any came from elsewhere.
fn is_bare_select(line: &str) -> bool {
    let Some((_, select)) = line.split_once(": SELECT ") else { return false };
    let Some((page, parameters)) = select.split_once(' ') else { return false };
    parameters.is_empty()
        && page.len() == "100/3F7F".len()
        && page.split_once('/').is_some_and(|(page, sub)| [page, sub].iter().all(|part| part.chars().all(|c| c.is_ascii_hexdigit())))
}

/// Every helper started, as (program, arguments), with each fetcher's
/// channel replaced by its place among `addresses`, so that two runs on
/// different ports compare; panics on anything but a decoder or a fetcher of
/// one of the channels that may reach only this computer's address.
fn helpers_started(requests: &[Request], addresses: &[String; 2]) -> Vec<(Program, Vec<String>)> {
    let mut started = Vec::new();
    for request in requests {
        match request {
            Request::Spawn { program: Program::Decoder, local_network: false, udp_port: None, arguments } => {
                started.push((Program::Decoder, arguments.clone()));
            }
            Request::Spawn { program: Program::Fetcher, local_network: true, udp_port: None, arguments } => {
                let place = addresses.iter().position(|address| arguments.first() == Some(address));
                let place = place.unwrap_or_else(|| panic!("a fetcher for {arguments:?}, no channel's address"));
                assert_eq!(arguments[1..], ["--allow-local", LOCAL], "a fetcher's reach");
                started.push((Program::Fetcher, vec![place.to_string()]));
            }
            Request::Spawn { .. }
            | Request::Read(_)
            | Request::OpenWeb(_)
            | Request::Openings
            | Request::Choose { .. }
            | Request::ReadPicture(_)
            | Request::ReadFont(_)
            | Request::Save { .. } => {
                panic!("the core asked the broker for {request:?}")
            }
            Request::Kill(_)
            | Request::Wait(_)
            | Request::ProcessorTime(_)
            | Request::LocalTime
            | Request::ViewerKeysKey
            | Request::Healthy
            | Request::UpdateState => {}
        }
    }
    started
}

/// REQ-675 (requirement 3.9), REQ-687 (requirement 4.7) and requirement 7.3,
/// with the real helpers (TODO-1254): what a hostile interface does reaches
/// no channel behind, the channel in front only as REQ-687's exceptions let
/// it, and nothing outside the channels.
#[test]
fn real_helpers_reach_nothing_more_under_a_hostile_interface() {
    confine::raise_descriptor_limit().unwrap();
    build(&["neoteletext-site"], None);
    let programs = build(&["neoteletext-fetcher", "neoteletext-decoder"], helper_features().as_deref());
    let quiet = session(&programs, false);
    let hostile = session(&programs, true);
    let Counts { taken, sent, reached } = hostile.counts;
    eprintln!("quiet: {:?}\nhostile: {:?}\nrounds: {:?}", quiet.said, hostile.said, hostile.counts);
    // The rounds reached the core, not only its reader.
    assert!(sent > taken / SENT_FLOOR_DIVISOR && reached > sent / REACHED_FLOOR_DIVISOR, "rounds: {:?}", hostile.counts);
    // The channel behind heard exactly what it heard in the quiet run.
    assert_eq!(hostile.said[1], quiet.said[1], "the channel behind heard more or less than in the quiet run");
    // The channel in front heard the quiet run's requests, and besides them
    // only SELECTs of its pages, with no parameters.
    let extra = beyond(&quiet.said[0], &hostile.said[0]);
    assert!(extra.iter().all(|line| is_bare_select(line)), "the channel in front heard {extra:?} beyond the quiet run");
    // The same helpers, for the same channels, and nothing else.
    assert_eq!(helpers_started(&hostile.requests, &hostile.addresses), helpers_started(&quiet.requests, &quiet.addresses), "the helpers started");
}
