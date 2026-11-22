//! The decoder's side of scripts and the script-host program together
//! (script-api.md sections 3.1, 3.2, 10.5 and 11.2; work package F4,
//! TODO-7293's "done"): a script page on each renderer runs in the real
//! script host, confined as the broker starts it, and draws in the real
//! decoder's engine. The test stands where the trusted core does: it
//! passes the script link's messages between the two by their kind
//! alone, and keeps the decoder's reports.
//!
//! The decoder may not link the script library (requirement 1.3), so the
//! bounds and the packing the two must agree on are held equal here.

use std::ffi::OsString;
use std::io::{BufReader, Write};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use confine::{Input, Process, Role, Spawn};
use decoder::Engine;
use decoder::scripts::{Scripts, events, limits};
use neoteletext_script::ops::{KeyBit, PointerAction};
use protocol::ProtocolError;
use protocol::decoder::FromDecoder;
use protocol::decoder::script::{ChannelFacts, ScriptOrder, ScriptReport, SourcePiece};
use protocol::rules::{KeyAction, PageSub, RuleInput};
use protocol::script::relay::{DecoderOutput, HostOutput, read_decoder_output, read_host_output};
use protocol::script::{ContextId, FailReason, FromScriptHost, Renderer, read_to_decoder};

/// How long a page has to draw: far longer than a loaded machine needs.
const PATIENCE: Duration = Duration::from_secs(20);
/// How long the test waits for the script host between two wakes of the
/// decoder: less than a tick.
const TURN: Duration = Duration::from_millis(5);

/// A decoder's engine and its scripts, a confined script host, and the
/// test between them as the trusted core is.
struct Browser {
    scripts: Scripts,
    engine: Engine,
    host: Process,
    to_host: Input,
    from_host: Receiver<Result<FromScriptHost, ProtocolError>>,
    reports: Vec<ScriptReport>,
    /// The script host ended, or wrote what it may not.
    ended: bool,
}

impl Browser {
    fn new(page_file: &[u8]) -> Self {
        let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-script-host"));
        let environment = confine::environment(Role::Script);
        let arguments: [OsString; 0] = [];
        let mut host = confine::spawn(&Spawn {
            program,
            arguments: &arguments,
            role: Role::Script,
            environment: &environment,
            local_network: false,
            udp_port: None,
            writable: None,
        })
        .unwrap();
        let mut output = BufReader::new(host.stdout.take().unwrap());
        let (said, from_host) = channel();
        std::thread::spawn(move || {
            // The trusted core's relay: a message for the decoder is
            // passed on by its kind, and read only by the decoder.
            while let Ok(Some(HostOutput::ToDecoder(relayed))) = read_host_output(&mut output) {
                let read = read_to_decoder(&mut relayed.frame()).and_then(|message| message.ok_or(ProtocolError::BadValue("an empty frame")));
                if said.send(read).is_err() {
                    return;
                }
            }
        });
        let mut engine = Engine::new();
        if !page_file.is_empty() {
            engine.feed(page_file);
            engine.finish();
        }
        let to_host = host.stdin.take().unwrap();
        Self { scripts: Scripts::new(), engine, host, to_host, from_host, reports: Vec::new(), ended: false }
    }

    /// Passes on what the decoder wrote: the script link's messages to
    /// the script host, unread; its reports kept.
    fn pass(&mut self, out: &[u8]) {
        let mut input = out;
        while let Some(message) = read_decoder_output(&mut input).unwrap() {
            match message {
                DecoderOutput::Core(FromDecoder::Script(report)) => self.reports.push(report),
                DecoderOutput::Core(_) => {}
                DecoderOutput::ToHost(relayed) => {
                    // A script host that has ended takes nothing more.
                    let _ = self.to_host.write_all(relayed.frame()).and_then(|()| self.to_host.flush());
                }
            }
        }
    }

    fn order(&mut self, order: ScriptOrder) {
        let mut out = Vec::new();
        self.scripts.order(order, &mut self.engine, &mut out).unwrap();
        self.pass(&out);
    }

    /// Gives the script of `page` and shows the page.
    fn open(&mut self, page: PageSub, name: &str, source: &[u8]) {
        self.order(ScriptOrder::Channel(ChannelFacts {
            channel_url: "https://example.test/harbour/".into(),
            channel_title: "Harbour".into(),
            ..ChannelFacts::default()
        }));
        let piece =
            SourcePiece { level: ContextId::Page, page, name: name.into(), total: u32::try_from(source.len()).unwrap(), offset: 0, bytes: source.to_vec() };
        self.order(ScriptOrder::Source(piece));
        let mut out = Vec::new();
        self.scripts.page_opened(Some(page), &mut self.engine, &mut out).unwrap();
        self.pass(&out);
    }

    /// The trusted core says which page is open, or that none is.
    fn page(&mut self, page: Option<PageSub>) {
        let mut out = Vec::new();
        self.scripts.page_opened(page, &mut self.engine, &mut out).unwrap();
        self.pass(&out);
    }

    /// The trusted core says the tab is shown, or hidden.
    fn shown(&mut self, shown: bool) {
        self.order(ScriptOrder::TabShown(shown));
    }

    fn key(&mut self, page: PageSub, sequence: u32, name: char) {
        let mut out = Vec::new();
        let input = RuleInput::Key { action: KeyAction::Press, named: false, code: u32::from(name), base: None, mods: 0 };
        self.scripts.input(page, sequence, &input, &mut self.engine, &mut out).unwrap();
        self.pass(&out);
    }

    /// Runs the decoder's loop, as `decoder::process::run` does for its
    /// scripts, until `done` or [`PATIENCE`]: it wakes for what is due
    /// and takes each message of the script host as it comes.
    fn run_until(&mut self, done: impl Fn(&mut Self) -> bool) -> bool {
        let began = Instant::now();
        while began.elapsed() < PATIENCE {
            if done(self) {
                return true;
            }
            let mut out = Vec::new();
            self.scripts.wake(&mut self.engine, &mut out).unwrap();
            match self.from_host.recv_timeout(TURN) {
                Ok(message) => {
                    self.scripts.received(message, &mut self.engine, &mut out).unwrap();
                }
                Err(RecvTimeoutError::Timeout) => {}
                // The script host ended untold: the trusted core tells the
                // decoder with a FAILED of its own (DEC-7357 item 7).
                Err(RecvTimeoutError::Disconnected) if !self.ended => {
                    self.ended = true;
                    let failed = FromScriptHost::Failed { context: ContextId::Channel, reason: FailReason::Time, text: "the script host ended".into() };
                    self.scripts.received(Ok(failed), &mut self.engine, &mut out).unwrap();
                }
                Err(RecvTimeoutError::Disconnected) => std::thread::sleep(TURN),
            }
            self.pass(&out);
        }
        done(self)
    }

    /// Row `row` of `page` as the engine's store holds it, as text.
    fn row(&self, page: u16, row: usize) -> String {
        let pages = self.engine.export();
        let Some(held) = pages.iter().find(|held| held.page == page) else { return String::new() };
        held.rows[row].iter().map(|code| char::from(code & 0x7F)).collect::<String>().trim().to_owned()
    }

    fn logged(&self) -> Vec<String> {
        self.reports.iter().filter_map(|report| if let ScriptReport::Log { text, .. } = report { Some(text.clone()) } else { None }).collect()
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.host.kill();
        let _ = self.host.wait();
    }
}

/// How often a page whose script was stopped for time is tried again:
/// a script host's limits are on the wall clock, and on a machine with
/// no processor free a script's start alone can pass its limit, as the
/// script-host program's own tests allow for.
const STARVED_TRIES: usize = 5;

/// What `page` gives, tried again while its script was stopped for time
/// before it drew.
fn drawn(page: impl Fn() -> Browser) -> Browser {
    let mut browser = page();
    for _ in 1..STARVED_TRIES {
        if browser.scripts.stopped() != Some(FailReason::Time) {
            break;
        }
        browser = page();
    }
    browser
}

/// TODO-7293's "done", teletext: a page file is shown first and its
/// script then draws over it through a separate script host (section
/// 12.3); a claimed key reaches the script and its verdict comes back
/// (section 4.1); rows the script did not write, and row 24, stay the
/// page file's.
#[test]
fn a_teletext_script_page_draws_in_the_decoder_through_a_confined_script_host() {
    let page = PageSub::new(0x100, 0);
    let file = b"PN,10000\nSC,0000\nPS,8000\nOL,1,FILE ROW ONE\nOL,24,FILE KEYS\n";
    let script = br#"/*! nt {"api":1,"keys":["1"],"tick":1} */
        nt.teletext.page.put(5, 0, "HELLO FROM JS");
        let ticks = 0;
        nt.addEventListener("tick", () => { ticks += 1; if (ticks === 3) nt.teletext.page.put(7, 0, "THREE TICKS"); });
        nt.addEventListener("keydown", (e) => { e.preventDefault(); nt.teletext.page.put(6, 0, "KEY " + e.key); });"#;
    let mut browser = drawn(|| {
        let mut browser = Browser::new(file);
        browser.open(page, "100.js", script);
        browser.run_until(|browser| browser.row(0x100, 7) == "THREE TICKS" || browser.scripts.stopped().is_some());
        browser
    });
    assert_eq!(browser.row(0x100, 7), "THREE TICKS", "reports: {:?}", browser.reports);
    assert_eq!((browser.row(0x100, 1), browser.row(0x100, 5), browser.row(0x100, 24)), ("FILE ROW ONE".into(), "HELLO FROM JS".into(), "FILE KEYS".into()));
    assert!(matches!(&browser.reports[0], ScriptReport::Claims(claims) if claims.running && claims.keys == ["1"] && !claims.network), "{:?}", browser.reports);

    browser.key(page, 41, '1');
    assert!(browser.run_until(|browser| browser.row(0x100, 6) == "KEY 1"), "reports: {:?}", browser.reports);
    assert!(browser.reports.contains(&ScriptReport::Verdict { sequence: 41, cancelled: true, handled: true }), "{:?}", browser.reports);
    assert!(browser.scripts.stopped().is_none());
    assert_eq!(browser.logged(), Vec::<String>::new());
}

/// TODO-7293's "done", ReGIS: a JavaScript page (section 12.3, no page
/// file) opens in the renderer its manifest names, and what its script
/// draws is fed to the open sub-page's one ReGIS stream and painted by
/// the decoder's renderer.
#[test]
fn a_regis_script_page_draws_in_the_decoder() {
    let page = PageSub::new(0x200, 0);
    let script = br#"/*! nt {"api":1,"renderer":"regis"} */
        nt.regis.drawing({ x: 100, y: 100 }).draw((p) => p.ink(2).rect(0, 0, 50, 50));"#;
    let mut browser = drawn(|| {
        let mut browser = Browser::new(b"");
        browser.open(page, "200.js", script);
        browser.run_until(|browser| browser.engine.regis().stream_end(0x200, 0).is_some() || browser.scripts.stopped().is_some());
        browser
    });
    assert!(browser.engine.regis().stream_end(0x200, 0).is_some(), "reports: {:?}", browser.reports);
    assert_eq!(browser.engine.script_renderer(0x200, 0), Some(Renderer::Regis));
    let now = browser.engine.now_us();
    let (regis, naplps) = browser.engine.take_pictures(0x200, Some(0), now);
    assert!(regis.is_some() && naplps.is_none(), "the decoder's ReGIS renderer painted the page");
    assert_eq!(browser.logged(), Vec::<String>::new());
    assert!(browser.scripts.stopped().is_none());
}

/// TODO-7293's "done", NAPLPS: the script library's own worked page, a
/// JavaScript page, draws in the decoder's NAPLPS renderer.
#[test]
fn a_naplps_script_page_draws_in_the_decoder() {
    let page = PageSub::new(0x300, 0);
    let script = include_bytes!("../../script/tests/naplps_page/harbour-hatched.js");
    let mut browser = drawn(|| {
        let mut browser = Browser::new(b"");
        browser.open(page, "300.js", script);
        browser.run_until(|browser| browser.engine.naplps().stream_end(0x300, 0).is_some() || browser.scripts.stopped().is_some());
        browser
    });
    assert!(browser.engine.naplps().stream_end(0x300, 0).is_some(), "reports: {:?}", browser.reports);
    assert_eq!(browser.engine.script_renderer(0x300, 0), Some(Renderer::Naplps));
    let now = browser.engine.now_us();
    let (regis, naplps) = browser.engine.take_pictures(0x300, Some(0), now);
    assert!(regis.is_none() && naplps.is_some(), "the decoder's NAPLPS renderer painted the page");
    assert!(browser.scripts.stopped().is_none(), "{:?}", browser.reports);
}

/// The security review's hostile script `toploop.js` against the
/// decoder's side (TODO-7336; section 3.2 rule 5): a script whose top
/// level never returns is stopped for good, the decoder tells the
/// trusted core, nothing of it is drawn, and the page file stays.
#[test]
fn a_script_that_never_returns_is_stopped_and_the_page_file_stays() {
    let page = PageSub::new(0x100, 0);
    let script = b"/*! nt {\"api\":1,\"keys\":[\"1\"]} */ nt.teletext.page.put(5, 0, 'NEVER'); for (;;) {}";
    let mut browser = Browser::new(b"PN,10000\nSC,0000\nPS,8000\nOL,1,FILE ROW ONE\n");
    browser.open(page, "100.js", script);
    assert!(browser.run_until(|browser| browser.scripts.stopped().is_some()), "reports: {:?}", browser.reports);
    assert!(browser.reports.iter().any(|report| matches!(report, ScriptReport::Stopped { reason: FailReason::Time, .. })), "{:?}", browser.reports);
    assert!(matches!(browser.reports.last(), Some(ScriptReport::Claims(claims)) if !claims.running), "{:?}", browser.reports);
    assert_eq!((browser.row(0x100, 1), browser.row(0x100, 5)), ("FILE ROW ONE".into(), String::new()));
}

/// Section 3.1's table of lifetimes, end to end through the real script
/// host (TODO-7405): a change of the open page inside the channel ends
/// the page's context after its one handling of `close` and opens the
/// new page's, while the channel context lives on and hears `pagehide`
/// then `pageshow`; between the two pages the trusted core tells the
/// decoder that no page is open, which pauses nothing; and a tab hidden
/// and shown again pauses and resumes both contexts, each hearing
/// `visibilitychange` before the pause and after it. Nothing of this
/// stops a script.
#[test]
fn a_change_of_page_and_a_hidden_tab_stop_no_script() {
    let (first, second) = (PageSub::new(0x100, 0), PageSub::new(0x101, 0));
    let file = b"PN,10000\nSC,0000\nPS,8000\nOL,1,PAGE ONE\n\nPN,10100\nSC,0000\nPS,8000\nOL,1,PAGE TWO\n";
    let channel = br#"/*! nt {"api":1} */
        nt.addEventListener("pageshow", (e) => console.log("channel pageshow " + e.page.number));
        nt.addEventListener("pagehide", (e) => console.log("channel pagehide " + e.page.number));
        nt.addEventListener("visibilitychange", () => console.log("channel hidden " + nt.hidden));"#;
    let page_one = br#"/*! nt {"api":1} */
        nt.addEventListener("close", () => console.log("one closed"));
        console.log("one runs");"#;
    let page_two = br#"/*! nt {"api":1} */
        nt.addEventListener("visibilitychange", () => console.log("two hidden " + nt.hidden));
        console.log("two runs");"#;
    let source = |level: ContextId, page: PageSub, name: &str, bytes: &[u8]| {
        ScriptOrder::Source(SourcePiece { level, page, name: name.into(), total: u32::try_from(bytes.len()).unwrap(), offset: 0, bytes: bytes.to_vec() })
    };
    let said = |browser: &Browser, line: &str| browser.logged().iter().any(|logged| logged == line);
    let mut browser = drawn(|| {
        let mut browser = Browser::new(file);
        browser.order(source(ContextId::Channel, first, "channel.js", channel));
        browser.order(source(ContextId::Page, second, "two.js", page_two));
        browser.open(first, "one.js", page_one);
        browser.run_until(|browser| (said(browser, "one runs") && said(browser, "channel pageshow 100")) || browser.scripts.stopped().is_some());
        browser
    });
    assert!(said(&browser, "one runs") && said(&browser, "channel pageshow 100"), "{:?}", browser.reports);
    // The page changes: the trusted core says no page is open while the
    // new one is fetched and drawn, then names it.
    browser.page(None);
    browser.page(Some(second));
    browser.run_until(|browser| (said(browser, "two runs") && said(browser, "channel pageshow 101")) || browser.scripts.stopped().is_some());
    assert_eq!(browser.scripts.stopped(), None, "a change of page stops no script: {:?}", browser.reports);
    let logged = browser.logged();
    let place = |line: &str| logged.iter().position(|logged| logged == line).unwrap_or_else(|| panic!("{line} was not logged: {logged:?}"));
    assert!(place("one closed") < place("two runs"), "the old page's context closes before the new one opens: {logged:?}");
    assert!(place("channel pagehide 100") < place("channel pageshow 101"), "{logged:?}");
    assert!(!logged.iter().any(|line| line.contains("hidden")), "a change of page hides no tab: {logged:?}");
    assert!(browser.scripts.running());
    // The tab is hidden, then shown again.
    browser.shown(false);
    browser.run_until(|browser| (said(browser, "channel hidden true") && said(browser, "two hidden true")) || browser.scripts.stopped().is_some());
    assert_eq!(browser.scripts.stopped(), None, "a hidden tab pauses its scripts and stops none: {:?}", browser.reports);
    assert!(said(&browser, "channel hidden true") && said(&browser, "two hidden true"), "{:?}", browser.logged());
    browser.shown(true);
    browser.run_until(|browser| (said(browser, "channel hidden false") && said(browser, "two hidden false")) || browser.scripts.stopped().is_some());
    assert_eq!(browser.scripts.stopped(), None, "{:?}", browser.reports);
    assert!(said(&browser, "channel hidden false") && said(&browser, "two hidden false"), "{:?}", browser.logged());
    assert!(browser.scripts.running());
}

/// Section 4.1 and principle 5 ("Fail open"), timed end to end through
/// the real script host (work package F5, TODO-7294): a script that
/// takes a key and never answers does not hold it. Its handler never
/// returns; the script host is ended for time, and the decoder reports
/// the key as not cancelled, so the trusted core takes its default. From
/// the key to that report is less than the trusted core's own deadline
/// for an answer, twice [`STEP_DEADLINE`] (`script_gate::VERDICT_DEADLINE`,
/// which holds whatever a decoder does), and the page file stays.
#[test]
fn a_key_a_script_takes_and_never_answers_keeps_its_default_in_time() {
    use protocol::script::relay::STEP_DEADLINE;
    let page = PageSub::new(0x100, 0);
    let script = br#"/*! nt {"api":1,"keys":["1"]} */
        nt.addEventListener("keydown", () => { for (;;) {} });
        console.log("listening");"#;
    let mut waits = Vec::new();
    for _ in 0..STARVED_TRIES {
        let mut browser = drawn(|| {
            let mut browser = Browser::new(b"PN,10000\nSC,0000\nPS,8000\nOL,1,FILE ROW ONE\n");
            browser.open(page, "100.js", script);
            browser.run_until(|browser| browser.logged().iter().any(|line| line == "listening") || browser.scripts.stopped().is_some());
            browser
        });
        assert!(browser.scripts.stopped().is_none(), "the script never started: {:?}", browser.reports);
        let pressed = Instant::now();
        browser.key(page, 77, '1');
        let answered = |browser: &mut Browser| browser.reports.iter().any(|report| matches!(report, ScriptReport::Verdict { sequence: 77, .. }));
        assert!(browser.run_until(answered), "the key was never answered: {:?}", browser.reports);
        let waited = pressed.elapsed();
        assert!(browser.reports.contains(&ScriptReport::Verdict { sequence: 77, cancelled: false, handled: false }), "{:?}", browser.reports);
        assert!(browser.run_until(|browser| browser.scripts.stopped().is_some()));
        assert_eq!(browser.scripts.stopped(), Some(FailReason::Time));
        assert_eq!(browser.row(0x100, 1), "FILE ROW ONE");
        // A key after it is answered at once: nothing is the script's.
        browser.key(page, 78, '1');
        assert!(browser.reports.contains(&ScriptReport::Verdict { sequence: 78, cancelled: false, handled: false }));
        println!("from the key to its default: {waited:?}");
        waits.push(waited);
        if waited < 2 * STEP_DEADLINE {
            return;
        }
    }
    panic!("a key waited past the trusted core's deadline in every try, on a machine with no processor free: {waits:?}");
}

/// Sections 4.2 and 4.4 (TODO-7294), through the real script host: a
/// contact of a touch screen reaches a script as a pointer of its own
/// number and kind, and a navigation of the browser's own as a
/// `navigate` event that cannot be cancelled.
#[test]
fn a_touch_and_the_browsers_navigation_reach_a_script_as_the_specification_says() {
    use protocol::decoder::script::{NavigationCause, PointerKind, ScriptPointer};
    use protocol::rules::PointerAction as Told;
    let page = PageSub::new(0x100, 0);
    let script = br#"/*! nt {"api":1,"pointer":"press"} */
        nt.addEventListener("pointerdown", (e) => console.log("down " + e.pointerType + " " + e.pointerId + " " + e.isPrimary));
        nt.addEventListener("navigate", (e) => { e.preventDefault(); console.log("navigate " + e.cause + " " + e.cancelable + " " + e.defaultPrevented); });
        console.log("listening");"#;
    let mut browser = drawn(|| {
        let mut browser = Browser::new(b"PN,10000\nSC,0000\nPS,8000\nOL,1,FILE ROW ONE\n");
        browser.open(page, "100.js", script);
        browser.run_until(|browser| browser.logged().iter().any(|line| line == "listening") || browser.scripts.stopped().is_some());
        browser
    });
    assert!(browser.scripts.stopped().is_none(), "{:?}", browser.reports);
    let touch = ScriptPointer { sequence: 5, action: Told::Press, button: 0, x: 24, y: 40, pointer: 3, kind: PointerKind::Touch, primary: false };
    browser.order(ScriptOrder::Pointer(touch));
    browser.order(ScriptOrder::Navigating { cause: NavigationCause::Browser, target: "101".into() });
    let heard = |browser: &mut Browser| browser.logged().iter().any(|line| line.starts_with("navigate")) || browser.scripts.stopped().is_some();
    assert!(browser.run_until(heard), "{:?}", browser.reports);
    let logged = browser.logged();
    assert!(logged.contains(&"down touch 3 false".to_owned()), "{logged:?}");
    assert!(logged.contains(&"navigate browser false false".to_owned()), "{logged:?}");
    assert!(browser.reports.contains(&ScriptReport::Verdict { sequence: 5, cancelled: false, handled: true }), "{:?}", browser.reports);
}

/// Sections 10.3, 10.5 and 10.6: the bounds the decoder holds as the
/// authority and the packing of the events it makes are the script
/// library's, which the decoder may not link.
#[test]
fn the_decoders_bounds_and_event_packing_are_the_script_librarys() {
    use neoteletext_script::host::naplps::limits as naplps;
    use neoteletext_script::limits as library;
    assert_eq!(limits::MAX_NAVIGATIONS_PER_SECOND, usize::try_from(library::MAX_NAVIGATIONS_PER_SECOND).unwrap());
    assert_eq!(limits::MAX_SOUND_OPERATIONS_PER_SECOND, usize::try_from(library::MAX_SOUND_OPERATIONS_PER_SECOND).unwrap());
    assert_eq!(limits::MAX_COLOUR_SETS_PER_SECOND, library::MAX_COLOUR_SETS_PER_SECOND);
    assert_eq!(limits::MAX_QUEUED_EVENTS, library::MAX_QUEUED_EVENTS);
    assert_eq!((limits::NAPLPS_REPAINT_TICKS, limits::NAPLPS_MAX_BYTES_PER_SECOND), (naplps::NAPLPS_REPAINT_TICKS, naplps::NAPLPS_MAX_BYTES_PER_SECOND));
    assert_eq!(limits::SECOND_TICKS, u64::from(neoteletext_rules::limits::TICKS_PER_SECOND));
    let keys = [events::KEY_SHIFT, events::KEY_CTRL, events::KEY_ALT, events::KEY_META, events::KEY_REPEAT_OR_LOST];
    assert_eq!(keys, [KeyBit::SHIFT, KeyBit::CTRL, KeyBit::ALT, KeyBit::META, KeyBit::REPEAT_OR_LOST].map(KeyBit::number));
    let pointer = [events::POINTER_DOWN, events::POINTER_UP, events::POINTER_MOVE, events::POINTER_ENTER, events::POINTER_LEAVE, events::POINTER_CANCEL];
    assert_eq!(
        pointer,
        [PointerAction::DOWN, PointerAction::UP, PointerAction::MOVE, PointerAction::ENTER, PointerAction::LEAVE, PointerAction::CANCEL]
            .map(PointerAction::number)
    );
}

/// The numbered drawing the decoder's ReGIS renderer picks at `x`, `y` of
/// `page` as it now paints it; 0 for none.
fn regis_drawing_at(browser: &mut Browser, page: u16, x: u16, y: u16) -> u16 {
    let now = browser.engine.now_us();
    // The pick is of the picture as last composed.
    let _ = browser.engine.take_pictures(page, Some(0), now);
    browser.engine.regis().drawing_at(page, Some(0), x, y)
}

/// A ReGIS JavaScript page that draws one detectable square at 100, 100
/// and hides it when the key 1 is pressed.
const REGIS_SQUARE_PAGE: &[u8] = br#"/*! nt {"api":1,"renderer":"regis","keys":["1"]} */
    const square = nt.regis.drawing({ x: 100, y: 100, detectable: true });
    square.draw((p) => p.ink(2).fill((q) => q.rect(0, 0, 50, 50)));
    nt.addEventListener("keydown", () => { square.visible = false; console.log("hidden"); });
    console.log("square drawn");"#;

/// A channel-level script that draws a detectable mark in the corner of
/// every ReGIS page shown, as a channel's clock is drawn.
const REGIS_CHANNEL_MARK: &[u8] = br#"/*! nt {"api":1} */
    nt.addEventListener("pageshow", (e) => {
      if (e.page.renderer !== "regis") return;
      nt.regis.drawing({ x: 700, y: 0, detectable: true }).draw((p) => p.ink(3).fill((q) => q.rect(0, 0, 40, 20)));
      console.log("mark on " + e.page.number);
    });"#;

/// Section 12.3 and section 3.1 (rdm TODO-7424): a ReGIS JavaScript page
/// reached from another page draws as it does opened by its address. The
/// page's one ReGIS stream begins when its page context opens, and what
/// the channel context drew on the page before goes with that page: the
/// news that the page before was left, which the channel context hears
/// after the new page's context has opened, takes nothing of the new
/// page's drawing with it. And a page come back to begins its stream
/// again, so that nothing of its earlier visit shows: here a square the
/// first visit hid.
#[test]
fn a_regis_script_page_reached_from_another_page_draws_and_a_page_come_back_to_begins_again() {
    let (first, second) = (PageSub::new(0x200, 0), PageSub::new(0x210, 0));
    let source = |level: ContextId, page: PageSub, name: &str, bytes: &[u8]| {
        ScriptOrder::Source(SourcePiece { level, page, name: name.into(), total: u32::try_from(bytes.len()).unwrap(), offset: 0, bytes: bytes.to_vec() })
    };
    let said = |browser: &Browser, line: &str| browser.logged().iter().filter(|logged| *logged == line).count();
    let mut browser = drawn(|| {
        let mut browser = Browser::new(b"");
        browser.order(source(ContextId::Channel, first, "channel.js", REGIS_CHANNEL_MARK));
        browser.order(source(ContextId::Page, second, "210.js", REGIS_SQUARE_PAGE));
        browser.open(first, "200.js", REGIS_SQUARE_PAGE);
        browser.run_until(|browser| (said(browser, "square drawn") == 1 && said(browser, "mark on 200") == 1) || browser.scripts.stopped().is_some());
        browser
    });
    // Opened by its address: the page's square and the channel's mark.
    assert!(
        browser.run_until(|browser| regis_drawing_at(browser, 0x200, 120, 120) != 0 && regis_drawing_at(browser, 0x200, 710, 10) != 0),
        "{:?}",
        browser.reports
    );
    let (square, mark) = (regis_drawing_at(&mut browser, 0x200, 120, 120), regis_drawing_at(&mut browser, 0x200, 710, 10));
    // The first visit hides its square.
    browser.key(first, 7, '1');
    assert!(browser.run_until(|browser| said(browser, "hidden") == 1 && regis_drawing_at(browser, 0x200, 120, 120) == 0), "{:?}", browser.reports);

    // The second page is reached from the first.
    browser.page(None);
    browser.page(Some(second));
    browser.run_until(|browser| (said(browser, "square drawn") == 2 && said(browser, "mark on 210") == 1) || browser.scripts.stopped().is_some());
    assert_eq!(browser.scripts.stopped(), None, "{:?}", browser.reports);
    let drew = browser.run_until(|browser| regis_drawing_at(browser, 0x210, 120, 120) != 0 && regis_drawing_at(browser, 0x210, 710, 10) != 0);
    let found = (regis_drawing_at(&mut browser, 0x210, 120, 120), regis_drawing_at(&mut browser, 0x210, 710, 10));
    assert!(drew, "the page reached from another shows drawings {found:?} at its square and at the channel's mark: {:?}", browser.logged());
    assert_eq!(found, (square, mark), "the page's drawings and the channel's are numbered as on a page opened by its address");

    // Back to the first page: its stream begins again, and the square
    // its first visit hid is drawn.
    browser.page(None);
    browser.page(Some(first));
    browser.run_until(|browser| (said(browser, "square drawn") == 3 && said(browser, "mark on 200") == 2) || browser.scripts.stopped().is_some());
    let drew = browser.run_until(|browser| regis_drawing_at(browser, 0x200, 120, 120) != 0 && regis_drawing_at(browser, 0x200, 710, 10) != 0);
    assert!(drew, "the page come back to shows what its earlier visit left: {:?}", browser.logged());
    let faults: Vec<String> = browser.logged().into_iter().filter(|line| line.contains("Error") || line.contains("commit")).collect();
    assert_eq!(faults, Vec::<String>::new());
    assert!(browser.scripts.stopped().is_none());
}
