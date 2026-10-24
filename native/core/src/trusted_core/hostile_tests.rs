//! Requirement 3.9's hostile interface harness: the trusted core, as
//! built, against an interface that sends every kind of message with
//! arbitrary values, malformed and replayed, as fast as it can. Messages
//! are made at the byte level and read by the protocol's own reader, as
//! the core reads the interface's pipe, so whatever the reader lets
//! through is what the core must stand against. After each one the harness
//! checks that no intent but strip input moved a tab to another channel,
//! in any window, or changed a setting or the bookmarks (requirement 4.5),
//! and that no page intent asked to start a helper or hand a page to the
//! web browser; at the end, that nothing sent to the interface carried a
//! cookie (requirement 3.6), and that the core never panicked. It runs
//! with bookmarks, the start page, file channels, and a second, private
//! window (finding F-98).
//!
//! On macOS and Linux the core runs against a stand-in for the broker
//! (`confine::broker::stand_in`, a test's feature only), as it does when
//! the browser runs, and each event is followed by the core's timed work
//! and its flush, as in its event loop. The stand-in keeps every request
//! and everything the core writes to each helper, so the harness also
//! checks, after every intent, that a page intent asked the broker for
//! nothing (no helper, no file, no hand-off); that no intent started a
//! helper outside the browser's roles or with arguments the core does not
//! give (an interface or sound helper, a decoder's store outside its
//! bounds, a fetcher for a channel no tab holds, the local network not
//! asked for); that only a confirmation in the strip handed an address to
//! the web browser; and that every decoder and fetcher the core holds is
//! for a tab's channel. At the end it reads what the core sent each
//! helper with the helper's own reader: every message must be one the
//! helper accepts (no page or sub-page outside a decoder's store), and no
//! decoder may be given another channel's bytes, a private tab's among
//! them.
//!
//! The interface's process is not replaced here: the broker starts it, and
//! a way to start another would be a way into a release build. The core is
//! the same code either way; its pipe is the only difference.

use std::io::Write;
use std::sync::mpsc;

#[cfg(unix)]
mod watch;
#[cfg(unix)]
use watch::{Watch, check_helpers};

use interface_core::canvas::PageKey;
use interface_core::controls::{FastextKey, PageControl};
use protocol::fetcher::FromFetcher;
use protocol::{ToCore, read_from_core, read_from_interface, write_to_core};

use super::Core;
use crate::session::Session;
use neoteletext::bookmarks::Bookmarks;
use neoteletext::outbox::Outbox;
use neoteletext::settings::Settings;

/// A cookie's value no message to the interface may ever hold (requirement
/// 3.6).
const SECRET: &str = "hostile-harness-cookie-value-7f3a";
/// Messages sent in a run: enough to reach every kind many times with
/// many values, quick enough for the commit gate.
const ROUNDS: usize = 30_000;
/// The kinds an interface may send are numbered from 16 to 73; a few below
/// and above are sent too, which the reader must refuse.
const KINDS: u8 = 80;
/// The longest made payload: most kinds' bounds, and past several.
const MAX_MADE: usize = 96;
/// Payloads tried for each kind and length while learning which lengths a
/// kind takes.
const TRIES: usize = 12;
/// The mask that makes a byte small: 0 to 3.
const SMALL: u8 = 3;

struct Sink(mpsc::Sender<Vec<u8>>);

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.send(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A small deterministic random source, xorshift64*, so that a run that
/// fails is repeated by its seed.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, limit: usize) -> usize {
        self.next().checked_rem(u64::try_from(limit).unwrap_or(u64::MAX)).and_then(|value| usize::try_from(value).ok()).unwrap_or(0)
    }

    fn byte(&mut self) -> u8 {
        self.next().to_be_bytes()[0]
    }
}

/// A payload of `length` random bytes: half the time small numbers, which
/// reach real tabs, rows, pages and short strings more often than chance.
fn payload(random: &mut Random, length: usize) -> Vec<u8> {
    let small = random.below(2) == 0;
    (0..length).map(|_| if small { random.byte() & SMALL } else { random.byte() }).collect()
}

fn framed(kind: u8, told: usize, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![kind];
    bytes.extend_from_slice(&u32::try_from(told).unwrap_or(u32::MAX).to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn accepted(bytes: &[u8]) -> bool {
    let mut input = bytes;
    matches!(read_from_interface(&mut input), Ok(Some(_)))
}

/// The kinds and lengths the reader accepts, found by trying each: the
/// harness learns the protocol from its reader rather than from a copy of
/// it that could drift.
fn shapes(random: &mut Random) -> Vec<(u8, usize)> {
    let mut found = Vec::new();
    for kind in 0..KINDS {
        for length in 0..MAX_MADE {
            if (0..TRIES).any(|_| accepted(&framed(kind, length, &payload(random, length)))) {
                found.push((kind, length));
            }
        }
    }
    found
}

/// Messages a real interface sends, encoded, to be replayed and mutated;
/// `start_page` is a bookmark page of the start page's tab, and `private`
/// the private window.
fn samples(start_page: PageKey, private: u32) -> Vec<Vec<u8>> {
    #[cfg_attr(not(feature = "links"), expect(unused_mut, reason = "only the links samples are added to it"))]
    let mut messages = vec![
        ToCore::NewTab,
        ToCore::SelectTab(1),
        ToCore::CloseTab(0),
        ToCore::MoveTab { from: 0, to: 1 },
        ToCore::Navigate("https://other.invalid/teletext/#p200".into()),
        ToCore::Back,
        ToCore::Forward,
        ToCore::Reload,
        ToCore::Home,
        ToCore::Bookmark,
        ToCore::Console(true),
        ToCore::Suggest("news".into()),
        ToCore::TabList(false),
        ToCore::GoTo { forward: false, steps: 1 },
        ToCore::NewPrivateTab,
        ToCore::PageControl(PageControl::Fastext(FastextKey::Red)),
        ToCore::PageControl(PageControl::Reveal),
        ToCore::PageControl(PageControl::SubPage(true)),
        ToCore::Opened(Some(start_page)),
        ToCore::TabToWindow { tab: 0, window: private, index: 0 },
        ToCore::From(private),
    ];
    #[cfg(feature = "links")]
    messages.extend([ToCore::Point(None), ToCore::Typed { number: 0x200, opened: None }, ToCore::Confirm(1), ToCore::Cancel(1)]);
    messages
        .iter()
        .map(|message| {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, message).unwrap();
            bytes
        })
        .collect()
}

/// One frame as the interface's pipe carries it: a learnt shape with new
/// contents, a real message with a byte changed or another kind, or bytes
/// at random, sometimes with a length that lies.
fn frame(random: &mut Random, shapes: &[(u8, usize)], samples: &[Vec<u8>]) -> Vec<u8> {
    match random.below(5) {
        // A real message as it is, as a compromised interface would send.
        4 => samples[random.below(samples.len())].clone(),
        0 | 1 => {
            let (kind, length) = shapes[random.below(shapes.len())];
            framed(kind, length, &payload(random, length))
        }
        2 => {
            let mut bytes = samples[random.below(samples.len())].clone();
            let at = random.below(bytes.len());
            match random.below(3) {
                0 => bytes[0] = u8::try_from(random.below(usize::from(KINDS))).unwrap_or(0),
                1 => bytes[at] = random.byte(),
                _ => bytes[at] ^= 1 << random.below(8),
            }
            bytes
        }
        _ => {
            let kind = u8::try_from(random.below(usize::from(KINDS))).unwrap_or(0);
            let length = random.below(MAX_MADE);
            let told = if random.below(4) == 0 { length.saturating_add(random.below(8)) } else { length };
            framed(kind, told, &payload(random, length))
        }
    }
}

/// The intents a page, not the user in the strip, can make: none of them
/// may take a tab to another channel (requirement 4.5).
fn is_page_intent(intent: &ToCore) -> bool {
    match intent {
        ToCore::Visible(_) | ToCore::Cell { .. } | ToCore::Opened(_) | ToCore::Window(_) => true,
        #[cfg(feature = "links")]
        ToCore::Point(_) | ToCore::Typed { .. } | ToCore::Focused(_) => true,
        #[cfg(feature = "interaction")]
        ToCore::Input(_) | ToCore::Editing(_) => true,
        _ => false,
    }
}

/// A confirmation in the strip: the one intent that may hand an address to
/// the web browser (requirement 8.6).
fn is_confirmation(intent: &ToCore) -> bool {
    #[cfg(feature = "links")]
    return matches!(intent, ToCore::Confirm(_));
    #[cfg(not(feature = "links"))]
    {
        let _ = intent;
        false
    }
}

/// Each tab's channel in every window, as the strip names it, with
/// whether it is private.
fn channels(session: &Session) -> Vec<(String, bool)> {
    session.all_tabs().map(|tab| (tab.address().map(|address| address.channel_text().to_owned()).unwrap_or_default(), tab.private)).collect()
}

/// On Windows the core has no stand-in broker (it passes pipes the
/// Windows way, which `confine` has no stand-in of): nothing is watched,
/// and a page intent's asking is caught by [`NO_BROKER`] alone.
#[cfg(not(unix))]
struct Watch;

#[cfg(not(unix))]
#[allow(clippy::unused_self, reason = "the same calls as the stand-in's watch, which does something with each")]
impl Watch {
    fn start() -> (Option<std::sync::Arc<confine::broker::Broker>>, Self) {
        (None, Self)
    }
    fn take(&mut self) {}
    fn check_intent(&mut self, _: &Core, _: bool, _: bool, _: &str) {}
    fn check_timed(&mut self, _: &Core, _: &str) {}
    fn note_channels(&mut self, _: &Core, _: u64) {}
}

#[cfg(not(unix))]
fn check_helpers(_: &Core, _: &str) {}

/// What this core without a broker answers when asked to start a helper
/// or hand a page to the web browser: a page intent must never ask.
const NO_BROKER: &str = "no broker: this trusted core starts nothing";

/// The [`ROUNDS`] of messages made at random, each read as the core reads
/// the interface's pipe and acted on, checked after each; how many the
/// reader took and how many it refused.
#[allow(clippy::arithmetic_side_effects, reason = "a test's counts")]
fn hostile_rounds(core: &mut Core, watch: &mut Watch, start_page: PageKey, private: u32) -> (usize, usize) {
    let mut random = Random(0x5EED_0309_2026);
    let shapes = shapes(&mut random);
    let samples = samples(start_page, private);
    assert!(shapes.len() > 30, "the reader accepted only {} shapes", shapes.len());
    let (mut taken, mut refused) = (0_usize, 0_usize);
    for round in 0..ROUNDS {
        let bytes = frame(&mut random, &shapes, &samples);
        let mut input = bytes.as_slice();
        // A malformed message ends the interface's reader, and the browser
        // with it (requirement 4.3): refused, not acted on.
        let Ok(Some(intent)) = read_from_interface(&mut input) else {
            refused += 1;
            continue;
        };
        taken += 1;
        let page_intent = is_page_intent(&intent);
        let confirmation = is_confirmation(&intent);
        let described = format!("round {round}: {intent:?}");
        let before = (channels(&core.session), core.settings.clone(), core.session.bookmarks.text());
        let generation = core.generation;
        // An error is the browser ending, as for a write that failed; a
        // panic is the fault the harness is for.
        let result = core.intent(intent);
        if page_intent {
            assert_ne!(result, Err(String::from(NO_BROKER)), "a page intent asked to start a helper or hand a page on: {described}");
        }
        watch.check_intent(core, page_intent, confirmation, &described);
        // The event loop's timed work and flush follow every event.
        if matches!(result, Ok(true)) {
            let _ = core.tick().and_then(|()| core.flush());
            watch.check_timed(core, &described);
        }
        watch.note_channels(core, generation);
        if !matches!(result, Ok(true)) {
            // The last tab closed: the browser would end. Open another and
            // go on, as a new start would.
            if core.session.tabs.is_empty() {
                core.session.open("https://news.invalid/teletext/").unwrap();
            }
            continue;
        }
        if page_intent {
            assert_eq!(
                (channels(&core.session), core.settings.clone(), core.session.bookmarks.text()),
                before,
                "a page intent moved a tab, changed a setting or the bookmarks: {described}"
            );
        }
        check_helpers(core, &described);
    }
    (taken, refused)
}

/// Every message the core sent the interface, once the core has gone,
/// checked for the cookie's value; how many there were.
#[allow(clippy::arithmetic_side_effects, reason = "a test's counts")]
fn sent_without_the_cookie(written: &mpsc::Receiver<Vec<u8>>) -> usize {
    let mut bytes = Vec::new();
    // The writing thread ends once all is written, and its sink with it.
    while let Ok(chunk) = written.recv() {
        bytes.extend(chunk);
    }
    let mut output = bytes.as_slice();
    let mut messages = 0_usize;
    while let Ok(Some(message)) = read_from_core(&mut output) {
        messages += 1;
        assert!(!format!("{message:?}").contains(SECRET), "a cookie reached the interface: {message:?}");
    }
    assert!(!String::from_utf8_lossy(&bytes).contains(SECRET), "a cookie's value in the bytes sent");
    messages
}

/// The file channels the harness opens: the stand-in broker reads any
/// path, and marks its page with the path (`watch`). A file address is an
/// absolute path, which on Windows begins with a drive: there,
/// "/hostile-harness/..." is read as a host name.
#[cfg(not(windows))]
const FILE_CHANNEL: &str = "/hostile-harness/news.tti";
#[cfg(not(windows))]
const PRIVATE_FILE_CHANNEL: &str = "/hostile-harness/private.tti";
#[cfg(windows)]
const FILE_CHANNEL: &str = r"C:\hostile-harness\news.tti";
#[cfg(windows)]
const PRIVATE_FILE_CHANNEL: &str = r"C:\hostile-harness\private.tti";

/// Finding F-98's hand-off, before the rounds: a web link's question in
/// the strip is not answered by page intents, and, confirmed, hands its
/// address on through the broker once, as the stand-in sees; so the
/// rounds' check that nothing else does is one that would see it.
#[cfg(all(unix, feature = "links"))]
fn a_confirmed_web_link_is_handed_on(core: &mut Core, watch: &mut Watch) {
    use neoteletext::links::{Channel, DWELL_MILLISECONDS, Follow, PROMPT_DELAY_MILLISECONDS};
    use protocol::links::{Candidate, Fastext, Found, Spot};
    const WEB: &str = "https://www.example.invalid/";
    let tab = core.session.active_tab().unwrap();
    let (channel, address) = (tab.channel.id, tab.address().unwrap().clone());
    let key = PageKey { channel, page: 0x100 };
    let web = Candidate { row: 5, column: 0, length: 7, double: false, light: false, target: Found::Declared(format!("web:{WEB}")) };
    core.links.opened(Some(key));
    core.links.found(key, vec![web], Vec::new(), Fastext::default());
    core.links.image(key, 1, &Channel { id: channel, address: &address, exists: &|_| true }, 0);
    let spot = Spot { key, version: 1, column: 1, row: 5 };
    core.links.point(Some(spot));
    let Ok(Follow::Ask(prompt)) = core.links.click(spot, Some(channel), DWELL_MILLISECONDS) else { panic!("a web link asks first") };
    // Past the question's half second, measured from the core's start.
    std::thread::sleep(std::time::Duration::from_millis(PROMPT_DELAY_MILLISECONDS + DWELL_MILLISECONDS + 100));
    watch.take();
    // Page intents while the question stands hand nothing on.
    for intent in [ToCore::Point(Some(spot)), ToCore::Visible(vec![key]), ToCore::Point(None)] {
        let described = format!("{intent:?}, with a web link's question in the strip");
        assert_eq!(core.intent(intent), Ok(true));
        watch.check_intent(core, true, false, &described);
    }
    assert_eq!(core.intent(ToCore::Confirm(prompt.id)), Ok(true));
    let handed: Vec<_> = watch.take().into_iter().filter(|request| matches!(request, confine::broker::Request::OpenWeb(_))).collect();
    assert_eq!(handed, [confine::broker::Request::OpenWeb(WEB.into())], "the confirmed link, handed on once");
}

#[test]
#[allow(clippy::arithmetic_side_effects, reason = "a test's counts")]
fn the_core_stands_against_a_hostile_interface() {
    let folder = std::env::temp_dir().join(format!("neoteletext-hostile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let (sender, written) = mpsc::channel();
    let (events, _incoming) = mpsc::channel();
    let (broker, mut watch) = Watch::start();
    // File channels are read through the broker, as `run` has them read.
    let session = match &broker {
        Some(broker) => {
            let reader = std::sync::Arc::clone(broker);
            Session::with_reader(Bookmarks::default(), Box::new(move |path: &std::path::Path| reader.read_source(path).map(crate::source::Source::from_bytes)))
        }
        None => Session::new(Bookmarks::default()),
    };
    let mut core = Core::new(session, Vec::new(), (Settings::default(), Some(folder.clone())), events, Outbox::new(Sink(sender)), broker);
    // Bookmarks, and the start page that lists them, with its bookmark
    // pages that a page intent must not open by itself (finding F-67).
    core.session.bookmarks.add(neoteletext::bookmarks::BAR, "Weather", "https://weather.invalid/teletext/").unwrap();
    core.session.open("https://news.invalid/teletext/").unwrap();
    core.session.open("https://sport.invalid/teletext/").unwrap();
    core.session.open(FILE_CHANNEL).unwrap();
    core.session.open(neoteletext::address::START_PAGE).unwrap();
    let start_page = PageKey { channel: core.session.tabs[3].channel.id, page: 0x201 };
    // A second window, private, whose tab a page intent must not move or
    // reach (findings F-65, F-83, F-84, F-98), with a file channel whose
    // bytes no other channel's decoder may be given.
    let first = core.session.window;
    let private = core.session.new_window(true).unwrap();
    core.session.open_private("https://secret.invalid/teletext/").unwrap();
    core.session.open_private(PRIVATE_FILE_CHANNEL).unwrap();
    core.switch_window(first);
    // A cookie in the first channel's jar, which the interface must never see.
    let channel = core.session.tabs[0].channel.id;
    let set = FromFetcher::CookieSet {
        host: "news.invalid".into(),
        path: "/teletext/".into(),
        scope: None,
        secure: true,
        line: format!("session={SECRET}; Path=/teletext/; Secure").into_bytes(),
    };
    core.cookie_reply(channel, set).unwrap();
    // Pages on screen, so that cells, links and frames have somewhere to be.
    let page = PageKey { channel, page: 0x100 };
    core.intent(ToCore::Visible(vec![page])).unwrap();
    #[cfg(all(unix, feature = "links"))]
    a_confirmed_web_link_is_handed_on(&mut core, &mut watch);

    // First the real messages a compromised interface would send at once:
    // a start-page bookmark opened by a page intent, and a tab moved into
    // the private window.
    watch.take();
    for intent in [ToCore::Opened(Some(start_page)), ToCore::TabToWindow { tab: 0, window: private, index: 0 }] {
        let page_intent = is_page_intent(&intent);
        let described = format!("{intent:?}");
        let before = (channels(&core.session), core.session.bookmarks.text());
        let result = core.intent(intent);
        assert_ne!(result, Err(String::from(NO_BROKER)), "{described} asked to start a helper or hand a page on");
        watch.check_intent(&core, page_intent, false, &described);
        assert_eq!((channels(&core.session), core.session.bookmarks.text()), before, "{described} moved a tab");
    }
    // Every channel's helpers, started as a tab is chosen, as the user
    // would; what they are told is checked at the end. Without the
    // stand-in broker, as on Windows, nothing can start them. What they
    // asked of the broker is checked here, as theirs: left for the first
    // round to take, it was laid to that round's intent.
    #[cfg(unix)]
    {
        let generation = core.generation;
        let described = "SelectTab(2), before the rounds";
        core.intent(ToCore::SelectTab(2)).unwrap();
        watch.check_intent(&core, false, false, described);
        core.tick().and_then(|()| core.flush()).unwrap();
        watch.check_timed(&core, described);
        watch.note_channels(&core, generation);
    }

    let (taken, refused) = hostile_rounds(&mut core, &mut watch, start_page, private);
    core.to_interface.flush().unwrap();
    drop(core);
    let messages = sent_without_the_cookie(&written);
    // The harness reached the core often, not only the reader.
    assert!(taken > ROUNDS / 5 && refused > 0 && messages > 0, "accepted {taken}, refused {refused}, sent {messages}");
    #[cfg(unix)]
    {
        // The broker was reached: helpers started, files read, and what the
        // helpers were sent, read by their own readers, with a file
        // channel's decoder given its own file.
        let (starts, reads) = (watch.starts, watch.reads);
        let to_helpers = watch.finish();
        assert!(
            starts > 10 && reads > 0 && to_helpers.decoders > 5 && to_helpers.own_files > 0 && to_helpers.messages > 50,
            "started {starts}, read {reads}, sent {to_helpers:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&folder);
}
