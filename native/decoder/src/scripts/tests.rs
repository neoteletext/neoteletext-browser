//! The decoder's side of scripts against a script host played by the
//! test (script-api.md sections 3.1, 3.2, 4.1, 10.5 and 11.2; TODO-7293's
//! lifecycle checks): what the decoder writes for its script host and
//! reports to the trusted core, read back from its output as the trusted
//! core reads it.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::panic, reason = "tests")]

use super::*;
use protocol::decoder::script::SourcePiece;
use protocol::script::batch::{ContextState, TeletextRow, Verdict};
use protocol::script::relay::{DecoderOutput, HostInput, START_HUNG_AFTER, STEP_DEADLINE, STEP_HUNG_AFTER, read_decoder_output, read_host_input};
use protocol::script::{LINK_VERSION, ToScriptHost};

const TTI: &[u8] = b"PN,10000\nSC,0000\nPS,8000\nOL,1,FILE ROW ONE\n\nPN,10100\nSC,0000\nPS,8000\nOL,1,Sport results\n";
const CHANNEL: &[u8] = b"/*! nt {\"api\":1,\"keys\":[\"i\"],\"network\":true} */";
const PAGE: &[u8] = b"/*! nt {\"api\":1,\"keys\":[\"1\",\"ArrowLeft\"],\"pointer\":\"press\",\"tick\":2,\"text\":true} */";

fn at(page: u16) -> PageSub {
    PageSub::new(page, 0)
}

/// What the decoder wrote, split as the trusted core splits it: its own
/// reports, and the script link's messages for the script host.
fn written(out: &mut Vec<u8>) -> (Vec<ScriptReport>, Vec<ToScriptHost>) {
    let mut input = out.as_slice();
    let (mut reports, mut link) = (Vec::new(), Vec::new());
    while let Some(message) = read_decoder_output(&mut input).unwrap() {
        match message {
            DecoderOutput::Core(FromDecoder::Script(report)) => reports.push(report),
            DecoderOutput::Core(_) => {}
            DecoderOutput::ToHost(relayed) => {
                let Some(HostInput::Link(message)) = read_host_input(&mut relayed.frame()).unwrap() else { panic!("a link message") };
                link.push(message);
            }
        }
    }
    out.clear();
    (reports, link)
}

fn source(level: ContextId, page: u16, name: &str, bytes: &[u8]) -> ScriptOrder {
    ScriptOrder::Source(SourcePiece { level, page: at(page), name: name.into(), total: u32::try_from(bytes.len()).unwrap(), offset: 0, bytes: bytes.to_vec() })
}

struct Fixture {
    scripts: Scripts,
    engine: Engine,
    out: Vec<u8>,
}

impl Fixture {
    /// A decoder with pages 100 and 101, its clock held at the start.
    fn new() -> Self {
        let mut engine = Engine::new();
        engine.feed(TTI);
        engine.finish();
        engine.set_now_us(0);
        Self { scripts: Scripts::new(), engine, out: Vec::new() }
    }

    fn order(&mut self, order: ScriptOrder) {
        self.scripts.order(order, &mut self.engine, &mut self.out).unwrap();
    }

    fn show(&mut self, page: Option<u16>) {
        self.scripts.page_opened(page.map(at), &mut self.engine, &mut self.out).unwrap();
    }

    /// Moves the decoder's clock to presentation tick `tick` and wakes.
    fn at_tick(&mut self, tick: u32) {
        self.engine.set_now_us(if tick == 0 { 0 } else { due_after(tick - 1) });
        self.scripts.wake(&mut self.engine, &mut self.out).unwrap();
    }

    fn host(&mut self, message: FromScriptHost) -> Outcome {
        self.scripts.received(Ok(message), &mut self.engine, &mut self.out).unwrap()
    }

    fn answer(&mut self, batch: Batch) -> Outcome {
        self.host(FromScriptHost::Batch(Box::new(batch)))
    }

    /// The channel's and page 100's scripts given, page 100 shown, and
    /// the script host ready: what was written for it, and reported.
    fn started() -> (Self, Vec<ScriptReport>, Vec<ToScriptHost>) {
        let mut fixture = Self::new();
        fixture.order(ScriptOrder::Channel(ChannelFacts {
            channel_url: "https://example.test/harbour/".into(),
            channel_title: "Harbour".into(),
            ..ChannelFacts::default()
        }));
        fixture.order(source(ContextId::Channel, 0x100, "channel.js", CHANNEL));
        fixture.order(source(ContextId::Page, 0x100, "100.js", PAGE));
        fixture.show(Some(0x100));
        fixture.host(FromScriptHost::Ready { link_version: LINK_VERSION, api_max: 1 });
        let (reports, link) = written(&mut fixture.out);
        (fixture, reports, link)
    }

    fn steps(&mut self) -> Vec<Step> {
        written(&mut self.out).1.into_iter().filter_map(|message| if let ToScriptHost::Step(step) = message { Some(*step) } else { None }).collect()
    }
}

fn key(name: char, action: KeyAction) -> RuleInput {
    RuleInput::Key { action, named: false, code: u32::from(name), base: None, mods: 1 }
}

/// Section 3.1's table, "the tab opens the channel", and section 4.6: the
/// channel context is opened first and the page's after it, each with
/// what its manifest claims and what the trusted core said of the
/// channel; the claims of the two together are reported after the
/// opening, so that the script host exists when its network is allowed;
/// and the first step is asked at once, carrying `pageshow`.
#[test]
fn the_contexts_open_channel_first_with_their_manifests_claims() {
    let (mut fixture, reports, link) = Fixture::started();
    let opens: Vec<&ContextOpen> = link.iter().filter_map(|message| if let ToScriptHost::ContextOpen(open) = message { Some(&**open) } else { None }).collect();
    assert!(matches!(link[0], ToScriptHost::Hello { link_version: LINK_VERSION }));
    assert_eq!(opens.iter().map(|open| open.context).collect::<Vec<_>>(), vec![ContextId::Channel, ContextId::Page]);
    let (channel, page) = (opens[0], opens[1]);
    assert_eq!((channel.keys.clone(), channel.flags.network, channel.tick_divisor), (vec!["i".to_owned()], true, 0));
    assert_eq!(
        (page.keys.len(), page.flags.pointer_press, page.flags.pointer_move, page.flags.text, page.flags.network, page.tick_divisor),
        (2, true, false, true, false, 2)
    );
    assert_eq!((page.page, page.subcode, page.renderer, page.language.as_str()), (0x100, 0, Renderer::Teletext, "en"));
    assert_eq!(
        (page.channel_url.as_str(), page.page_url.as_str(), page.script_url.as_str()),
        ("https://example.test/harbour/", "https://example.test/harbour/100", "https://example.test/harbour/100.js")
    );
    assert_eq!(page.channel_title, "Harbour");
    let sources = link.iter().filter(|message| matches!(message, ToScriptHost::Source { .. })).count();
    assert_eq!(sources, 2);
    let claims = ScriptClaims {
        page: at(0x100),
        running: true,
        network: true,
        text: true,
        pointer: PointerClaim::Press,
        wheel: false,
        keys: vec!["i".into(), "1".into(), "ArrowLeft".into()],
    };
    assert_eq!(reports, vec![ScriptReport::Claims(claims)]);
    let steps: Vec<&Step> = link.iter().filter_map(|message| if let ToScriptHost::Step(step) = message { Some(&**step) } else { None }).collect();
    assert_eq!(steps.len(), 1, "one step, asked at once");
    assert_eq!((steps[0].sequence, steps[0].tick), (0, 0));
    assert_eq!(steps[0].events.iter().map(|event| event.kind).collect::<Vec<_>>(), vec![EventKind::PageShow]);
    assert_eq!((steps[0].events[0].numbers, steps[0].events[0].payload.as_slice()), ([0x100, 0, 1, 0], &b"https://example.test/harbour/100"[..]));
    assert!(fixture.scripts.running() && fixture.scripts.page_context().is_some());
    fixture.at_tick(1);
    assert!(fixture.steps().is_empty(), "nothing is asked while no context asked for the tick");
}

/// Section 3.1: "an unknown member, a duplicate member, a wrong type or a
/// value out of range refuses the script whole. A file with no manifest
/// is refused." Nothing is said to a script host for it, and the
/// developer is told why.
#[test]
fn a_script_with_no_manifest_or_a_wrong_one_is_never_opened() {
    for refused in [&b"nt.log('x')"[..], b"/*! nt {\"api\":1,\"any\":true} */", b"/*! nt {\"api\":1,\"renderer\":\"regis\"} */"] {
        let mut fixture = Fixture::new();
        fixture.order(source(ContextId::Page, 0x100, "100.js", refused));
        fixture.show(Some(0x100));
        let (reports, link) = written(&mut fixture.out);
        assert!(link.is_empty(), "{link:?}");
        assert!(matches!(reports.as_slice(), [ScriptReport::Log { level: 0, text, .. }] if text.starts_with("the script 100.js was not run")), "{reports:?}");
        assert!(!fixture.scripts.running());
        fixture.show(Some(0x100));
        assert!(written(&mut fixture.out).0.is_empty(), "a refused script is not read again at every turn");
    }
}

/// Section 3.1's table, "the page or sub-page changes inside the
/// channel": the page context is closed and the channel context lives on
/// and hears `pagehide` then `pageshow`; a page with no script of its own
/// has no page context; and nothing a page context's batch held is drawn
/// once its page is left.
#[test]
fn a_page_change_ends_the_page_context_and_the_channel_context_lives_on() {
    let (mut fixture, _, _) = Fixture::started();
    fixture.show(Some(0x101));
    let (reports, link) = written(&mut fixture.out);
    assert!(matches!(link[0], ToScriptHost::ContextClose(ContextId::Page)), "{link:?}");
    // `pagehide` in a step of its own, ahead of anything of the next
    // page: the script host drops the page's renderer state at it, and a
    // new page's context, opened after, keeps what its top level draws.
    let ToScriptHost::Step(hidden) = &link[1] else { panic!("a step for the page hidden: {link:?}") };
    assert_eq!(hidden.events.iter().map(|event| (event.kind, event.numbers[0])).collect::<Vec<_>>(), vec![(EventKind::PageHide, 0x100)]);
    let ToScriptHost::Step(step) = &link[2] else { panic!("a step for the page shown: {link:?}") };
    assert_eq!(step.events.iter().map(|event| (event.kind, event.numbers[0])).collect::<Vec<_>>(), vec![(EventKind::PageShow, 0x101)]);
    let claims = ScriptClaims { page: at(0x101), running: true, network: true, text: false, pointer: PointerClaim::None, wheel: false, keys: vec!["i".into()] };
    assert_eq!(reports, vec![ScriptReport::Claims(claims)]);
    assert!(fixture.scripts.page_context().is_none());

    // The first step's batch, asked while page 100 was open, arrives
    // late: it draws nothing, on page 100 or on page 101.
    let late = Batch { teletext_rows: Some(vec![TeletextRow { row: 1, codes: [b'X'; 40] }]), ..Batch::new(0) };
    let outcome = fixture.answer(late);
    assert!(!outcome.applied && outcome.arrived.is_empty());
    // The close's batch, then the page-change step's, which the channel
    // context may draw with on the page now shown.
    assert!(!fixture.answer(Batch::new(0)).applied, "a close's batch draws nothing");
    let drawn = Batch { teletext_rows: Some(vec![TeletextRow { row: 2, codes: [b'C'; 40] }]), ..Batch::new(1) };
    assert!(fixture.answer(drawn).applied);
    assert!(fixture.scripts.stopped().is_none());

    // Back to page 100: its script, still held, starts a new context.
    fixture.show(Some(0x100));
    let (_, link) = written(&mut fixture.out);
    assert!(link.iter().any(|message| matches!(message, ToScriptHost::ContextOpen(open) if open.context == ContextId::Page && open.page == 0x100)), "{link:?}");
}

/// Section 3.1's table, "the tab is hidden", and section 13 item 6: one
/// step says the pause, with `visibilitychange` and the release of every
/// key held; no step is asked while paused; the first after it says
/// `resumed`, and the tick count goes on from where it stopped.
#[test]
fn a_hidden_tab_pauses_the_contexts_and_the_tick_does_not_jump() {
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(Batch::new(0));
    fixture.scripts.input(at(0x100), 7, &key('1', KeyAction::Press), &mut fixture.engine, &mut fixture.out).unwrap();
    let pressed = fixture.steps();
    assert_eq!(pressed.len(), 1);
    fixture.answer(Batch { verdicts: Some(vec![Verdict { cancelled: true, handled: true }]), ..Batch::new(1) });
    written(&mut fixture.out);
    fixture.at_tick(2);
    let ticked = fixture.steps();
    assert_eq!(ticked.iter().map(|step| (step.tick, step.flags.tick)).collect::<Vec<_>>(), vec![(2, true)], "the page's manifest asked every second tick");
    fixture.answer(Batch::new(2));

    // No page open between two pages pauses nothing and asks no step
    // (TODO-7405): only a hidden tab does.
    fixture.show(None);
    assert!(fixture.steps().is_empty() && fixture.scripts.running() && fixture.scripts.stopped().is_none());
    fixture.show(Some(0x100));
    assert!(fixture.steps().is_empty(), "the same page named again changes nothing");

    fixture.order(ScriptOrder::TabShown(false));
    let pausing = fixture.steps();
    assert_eq!(pausing.len(), 1);
    assert!(pausing[0].flags.pausing);
    // The script host makes `visibilitychange` itself from the step's
    // flag, and refuses the event from the decoder: none is sent.
    let events: Vec<(EventKind, i32)> = pausing[0].events.iter().map(|event| (event.kind, event.numbers[0])).collect();
    assert_eq!(events, vec![(EventKind::KeyUp, events::KEY_REPEAT_OR_LOST)]);
    fixture.order(ScriptOrder::TabShown(false));
    assert!(fixture.steps().is_empty(), "hidden twice is hidden once");
    assert_eq!(pausing[0].events[0].payload, b"1");
    fixture.answer(Batch::new(3));
    fixture.at_tick(40);
    assert!(fixture.steps().is_empty(), "paused: no step");
    assert!(fixture.scripts.due_us(&fixture.engine).is_none(), "and nothing to wake for");

    fixture.engine.set_now_us(due_after(99));
    fixture.order(ScriptOrder::TabShown(true));
    let resumed = fixture.steps();
    assert_eq!(resumed.len(), 1);
    assert!(resumed[0].flags.resumed && !resumed[0].flags.pausing);
    assert!(resumed[0].tick <= 4, "the ticks of the pause never happened: {}", resumed[0].tick);
}

/// Sections 4.1 and 4.6: a key a manifest names waits for its step and is
/// answered with the batch's verdict; a key none names is answered at
/// once as not cancelled and never reaches the script host; and a stopped
/// script leaves every default in force (section 1 principle 5).
#[test]
fn a_claimed_key_takes_its_verdict_and_every_other_keeps_its_default() {
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(Batch::new(0));
    written(&mut fixture.out);
    fixture.scripts.input(at(0x100), 11, &key('q', KeyAction::Press), &mut fixture.engine, &mut fixture.out).unwrap();
    let (reports, link) = written(&mut fixture.out);
    assert_eq!(reports, vec![ScriptReport::Verdict { sequence: 11, cancelled: false, handled: false }]);
    assert!(link.is_empty(), "an unclaimed key is not sent");

    fixture.scripts.input(at(0x100), 12, &key('1', KeyAction::Press), &mut fixture.engine, &mut fixture.out).unwrap();
    let (reports, link) = written(&mut fixture.out);
    assert!(reports.is_empty(), "the verdict waits for the script");
    let [ToScriptHost::Step(step)] = link.as_slice() else { panic!("one step: {link:?}") };
    assert_eq!(
        (step.events[0].kind, step.events[0].numbers[0], step.events[0].payload.as_slice(), step.flags.tick),
        (EventKind::KeyDown, events::KEY_SHIFT, &b"1"[..], false)
    );
    fixture.answer(Batch { verdicts: Some(vec![Verdict { cancelled: true, handled: true }]), ..Batch::new(1) });
    assert_eq!(written(&mut fixture.out).0, vec![ScriptReport::Verdict { sequence: 12, cancelled: true, handled: true }]);

    // A key on another page than the one open is not the script's.
    fixture.scripts.input(at(0x101), 13, &key('1', KeyAction::Press), &mut fixture.engine, &mut fixture.out).unwrap();
    assert_eq!(written(&mut fixture.out).0, vec![ScriptReport::Verdict { sequence: 13, cancelled: false, handled: false }]);

    // The scripts stop while a key waits: its default stands.
    fixture.scripts.input(at(0x100), 14, &key('1', KeyAction::Press), &mut fixture.engine, &mut fixture.out).unwrap();
    fixture.host(FromScriptHost::Failed { context: ContextId::Channel, reason: FailReason::Time, text: "too long".into() });
    let (reports, _) = written(&mut fixture.out);
    assert_eq!(
        reports,
        vec![
            ScriptReport::Verdict { sequence: 14, cancelled: false, handled: false },
            ScriptReport::Stopped { reason: FailReason::Time, text: "too long".into() },
            ScriptReport::Claims(ScriptClaims::none(at(0x100))),
        ]
    );
    assert_eq!(fixture.scripts.stopped(), Some(FailReason::Time));
    fixture.scripts.input(at(0x100), 15, &key('1', KeyAction::Press), &mut fixture.engine, &mut fixture.out).unwrap();
    let (reports, link) = written(&mut fixture.out);
    assert_eq!((reports, link.len()), (vec![ScriptReport::Verdict { sequence: 15, cancelled: false, handled: false }], 0));
}

/// Section 10.5 (TODO-7293 step 5): the decoder's rates are the
/// authoritative ones. A script host whose batches pass one is failed:
/// the batch that passed it is not applied, the trusted core is told to
/// end the script host, and nothing more is asked of it.
#[test]
fn a_script_host_that_passes_a_rate_is_failed_and_its_batch_not_applied() {
    let (mut fixture, _, _) = Fixture::started();
    let go = |sequence: u32| Batch { navigate: Some("101".into()), ..Batch::new(sequence) };
    fixture.answer(go(0));
    for sequence in 1..=2 {
        fixture.scripts.input(at(0x100), sequence, &key('1', KeyAction::Press), &mut fixture.engine, &mut fixture.out).unwrap();
        written(&mut fixture.out);
        let over = Batch { teletext_rows: Some(vec![TeletextRow { row: 1, codes: [b'Z'; 40] }]), ..go(sequence) };
        let outcome = fixture.answer(over);
        let (reports, _) = written(&mut fixture.out);
        if sequence < 2 {
            assert!(outcome.applied && fixture.scripts.stopped().is_none());
            continue;
        }
        assert!(!outcome.applied, "the batch that passed the rate draws nothing");
        assert!(reports.contains(&ScriptReport::Verdict { sequence: 2, cancelled: false, handled: false }));
        assert!(
            reports
                .iter()
                .any(|report| matches!(report, ScriptReport::Stopped { reason: FailReason::Link, text } if text.starts_with("MAX_NAVIGATIONS_PER_SECOND"))),
            "{reports:?}"
        );
    }
    assert_eq!(fixture.scripts.stopped(), Some(FailReason::Link));
    fixture.at_tick(10);
    assert!(written(&mut fixture.out).1.is_empty(), "nothing more is asked");
}

/// Section 11.2: a batch nothing asked for, one with more verdicts than
/// its step had events, and a message the reader refused each stop the
/// scripts; and section 3.2 rule 5(b): so does a step not answered by
/// its deadline.
#[test]
fn a_broken_link_or_a_late_answer_stops_the_scripts() {
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(Batch::new(0));
    fixture.answer(Batch::new(0));
    assert_eq!(fixture.scripts.stopped(), Some(FailReason::Link), "a batch nothing asked for");

    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(Batch { verdicts: Some(vec![Verdict::default(); 2]), ..Batch::new(0) });
    assert_eq!(fixture.scripts.stopped(), Some(FailReason::Link), "more verdicts than events");

    let (mut fixture, _, _) = Fixture::started();
    fixture.scripts.received(Err(ProtocolError::BadValue("row")), &mut fixture.engine, &mut fixture.out).unwrap();
    assert_eq!(fixture.scripts.stopped(), Some(FailReason::Link), "a message its reader refused");

    let (mut fixture, _, _) = Fixture::started();
    assert!(fixture.scripts.due_us(&fixture.engine).is_some(), "the loop wakes for the deadline");
    // A script host that has just been opened and answers nothing is not
    // stopped for a step's time, which a start on a busy machine passes
    // without any fault (DEC-7357 as revised): only once it has been
    // silent for `START_HUNG_AFTER` is it taken for hung.
    let opened = Instant::now();
    fixture.scripts.wake_at(opened + 2 * STEP_HUNG_AFTER, &mut fixture.engine, &mut fixture.out).unwrap();
    assert_eq!(fixture.scripts.stopped(), None, "stopped while it may still be starting");
    fixture.scripts.wake_at(opened + START_HUNG_AFTER, &mut fixture.engine, &mut fixture.out).unwrap();
    assert_eq!(fixture.scripts.stopped(), Some(FailReason::Time));
    let (reports, _) = written(&mut fixture.out);
    assert!(
        reports.iter().any(|report| matches!(report, ScriptReport::Stopped { reason: FailReason::Time, text } if text.starts_with("STEP_HUNG_AFTER"))),
        "{reports:?}"
    );
}

/// Sections 3.2 and 11.2: a context's earliest timer, which its batch
/// says, is what the decoder wakes for, and it never polls; a context
/// that a batch says stopped no longer claims anything.
#[test]
fn the_decoder_wakes_for_the_earliest_timer_and_a_stopped_context_claims_nothing() {
    let mut fixture = Fixture::new();
    fixture.order(source(ContextId::Channel, 0x100, "channel.js", CHANNEL));
    fixture.show(Some(0x100));
    fixture.host(FromScriptHost::Ready { link_version: LINK_VERSION, api_max: 1 });
    written(&mut fixture.out);
    let state = |next_due, stopped| vec![ContextState { context: ContextId::Channel, stopped, failures: 0, next_due }];
    fixture.answer(Batch { state: Some(state(Some(30), None)), ..Batch::new(0) });
    assert_eq!(fixture.scripts.due_us(&fixture.engine), Some(due_after(29)), "tick 30 and no sooner");
    fixture.at_tick(29);
    assert!(fixture.steps().is_empty(), "a tick before the timer is due");
    fixture.at_tick(30);
    let steps = fixture.steps();
    assert_eq!(steps.iter().map(|step| step.tick).collect::<Vec<_>>(), vec![30]);
    fixture.answer(Batch { state: Some(state(None, Some(FailReason::Failures))), ..Batch::new(1) });
    assert_eq!(written(&mut fixture.out).0, vec![ScriptReport::Claims(ScriptClaims::none(at(0x100)))]);
    assert!(fixture.scripts.due_us(&fixture.engine).is_none(), "no timer, no tick, nothing waits: the loop sleeps");
}

/// Section 12.5 and TODO-7331: a script arrives in pieces, in order; the
/// same bytes arriving again leave the running script running; other
/// bytes end its context and start the new script's; and a script longer
/// than its stated length, or out of order, is dropped.
#[test]
fn a_script_arrives_in_pieces_and_the_same_bytes_again_restart_nothing() {
    let piece = |offset: usize, bytes: &[u8], total: usize| {
        ScriptOrder::Source(SourcePiece {
            level: ContextId::Page,
            page: at(0x100),
            name: "100.js".into(),
            total: u32::try_from(total).unwrap(),
            offset: u32::try_from(offset).unwrap(),
            bytes: bytes.to_vec(),
        })
    };
    let mut fixture = Fixture::new();
    fixture.show(Some(0x100));
    let (head, tail) = PAGE.split_at(10);
    fixture.order(piece(0, head, PAGE.len()));
    assert!(written(&mut fixture.out).1.is_empty(), "not whole yet");
    fixture.order(piece(10, tail, PAGE.len()));
    let (_, link) = written(&mut fixture.out);
    assert!(matches!(&link[1], ToScriptHost::ContextOpen(open) if open.context == ContextId::Page), "{link:?}");

    fixture.order(source(ContextId::Page, 0x100, "100.js", PAGE));
    assert!(written(&mut fixture.out).1.is_empty(), "the same bytes again");
    let other = b"/*! nt {\"api\":1} */ nt.log(2)";
    fixture.order(source(ContextId::Page, 0x100, "100.js", other));
    let (_, link) = written(&mut fixture.out);
    assert!(matches!(link[0], ToScriptHost::ContextClose(ContextId::Page)), "{link:?}");
    assert!(matches!(&link[1], ToScriptHost::ContextOpen(open) if open.keys.is_empty()));

    let mut fixture = Fixture::new();
    fixture.show(Some(0x100));
    fixture.order(piece(0, head, PAGE.len()));
    fixture.order(piece(11, tail, PAGE.len()));
    fixture.order(piece(10, tail, PAGE.len()));
    assert!(written(&mut fixture.out).1.is_empty(), "a piece out of order drops what arrived");
}

/// Section 3.1's table, "the tab leaves the channel or closes": each
/// context is closed, the page's first, and the script host told to end.
#[test]
fn leaving_the_channel_closes_each_context_and_ends_the_script_host() {
    let (mut fixture, _, _) = Fixture::started();
    fixture.scripts.shutdown(&mut fixture.out).unwrap();
    let (_, link) = written(&mut fixture.out);
    assert!(
        matches!(link.as_slice(), [ToScriptHost::ContextClose(ContextId::Page), ToScriptHost::ContextClose(ContextId::Channel), ToScriptHost::Shutdown]),
        "{link:?}"
    );
}

/// Sections 4.2 and 4.4 (TODO-7294): a pointer the trusted core tells
/// with its number and kind reaches the script as that pointer, a
/// contact of a touch screen beside the mouse; its press is answered
/// with the batch's verdict; and one no manifest claims is answered at
/// once and never reaches the script host.
#[test]
fn a_pointer_is_told_with_its_number_and_kind() {
    use protocol::decoder::script::{PointerKind, ScriptPointer};
    let pointer = |sequence: u32, action: PointerAction, number: u8, kind: PointerKind, primary: bool| {
        ScriptOrder::Pointer(ScriptPointer { sequence, action, button: 0, x: 24, y: 40, pointer: number, kind, primary })
    };
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(Batch::new(0));
    written(&mut fixture.out);
    fixture.order(pointer(21, PointerAction::Press, 3, PointerKind::Touch, false));
    let (reports, link) = written(&mut fixture.out);
    assert!(reports.is_empty(), "the verdict waits for the script");
    let [ToScriptHost::Step(step)] = link.as_slice() else { panic!("one step: {link:?}") };
    let event = &step.events[0];
    let touch = i32::from(PointerKind::Touch.code()) << events::POINTER_KIND_SHIFT;
    assert_eq!((event.kind, event.numbers[0]), (EventKind::Pointer, events::POINTER_DOWN | touch), "a touch, not the primary one");
    // The cell of the dot, on a teletext page: column 2, row 2.
    assert_eq!((event.numbers[1], event.numbers[2]), (2, 2));
    assert_eq!(event.payload[..4], 3_i32.to_be_bytes(), "its own number");
    fixture.answer(Batch { verdicts: Some(vec![Verdict { cancelled: false, handled: true }]), ..Batch::new(1) });
    assert_eq!(written(&mut fixture.out).0, vec![ScriptReport::Verdict { sequence: 21, cancelled: false, handled: true }]);
    // The mouse is pointer 1 and primary; a pen is told as a pen.
    fixture.order(pointer(22, PointerAction::Release, 1, PointerKind::Mouse, true));
    fixture.order(pointer(23, PointerAction::Press, 2, PointerKind::Pen, true));
    let steps = fixture.steps();
    let packed: Vec<i32> = steps.iter().flat_map(|step| step.events.iter().map(|event| event.numbers[0])).collect();
    let pen = i32::from(PointerKind::Pen.code()) << events::POINTER_KIND_SHIFT;
    assert_eq!(packed, [events::POINTER_UP | events::POINTER_PRIMARY, events::POINTER_DOWN | pen | events::POINTER_PRIMARY]);
    // The page's manifest claims presses and not moves: a move over
    // nothing is not sent, and has no verdict to wait for.
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(Batch::new(0));
    written(&mut fixture.out);
    fixture.order(pointer(24, PointerAction::Move, 1, PointerKind::Mouse, true));
    let (reports, link) = written(&mut fixture.out);
    assert!(reports.is_empty() && link.is_empty(), "{reports:?} {link:?}");
    // With only the channel's script, which claims no pointer, a press is
    // answered at once as not cancelled.
    let mut fixture = Fixture::new();
    fixture.order(source(ContextId::Channel, 0x100, "channel.js", CHANNEL));
    fixture.show(Some(0x100));
    fixture.host(FromScriptHost::Ready { link_version: LINK_VERSION, api_max: 1 });
    fixture.answer(Batch::new(0));
    written(&mut fixture.out);
    fixture.order(pointer(25, PointerAction::Press, 1, PointerKind::Mouse, true));
    let (reports, link) = written(&mut fixture.out);
    assert_eq!((reports, link.len()), (vec![ScriptReport::Verdict { sequence: 25, cancelled: false, handled: false }], 0));
}

/// Section 4.2 (TODO-7294; TODO-7338 item 9): when the trusted core says
/// the tab is about to go somewhere, the contexts are given `navigate`
/// with its cause and where, at once and not at a tick; nothing is
/// reported for it, since a navigation of the browser's own cannot be
/// cancelled.
#[test]
fn the_scripts_hear_navigate_before_the_tab_goes() {
    use protocol::decoder::script::NavigationCause;
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(Batch::new(0));
    written(&mut fixture.out);
    fixture.order(ScriptOrder::Navigating { cause: NavigationCause::Browser, target: "101/0002".into() });
    let (reports, link) = written(&mut fixture.out);
    assert_eq!(reports, vec![], "nothing is reported for it");
    let [ToScriptHost::Step(step)] = link.as_slice() else { panic!("one step: {link:?}") };
    let event = &step.events[0];
    assert_eq!((event.kind, event.numbers[0], event.payload.as_slice()), (EventKind::Navigate, i32::from(NavigationCause::Browser.code()), &b"101/0002"[..]));
    assert!(!step.flags.tick);
    // Whatever the script host answers, no verdict is told for it.
    fixture.answer(Batch { verdicts: Some(vec![Verdict { cancelled: true, handled: true }]), ..Batch::new(1) });
    assert_eq!(written(&mut fixture.out).0, vec![]);
    // With no script running there is nobody to tell.
    let mut fixture = Fixture::new();
    fixture.show(Some(0x100));
    fixture.order(ScriptOrder::Navigating { cause: NavigationCause::Browser, target: "101".into() });
    let (reports, link) = written(&mut fixture.out);
    assert!(reports.is_empty() && link.is_empty());
}

/// Section 4.1 and principle 5 (TODO-7294), the decoder's half of "a
/// script that does not answer never holds a key": a claimed key whose
/// step is never answered is reported as not cancelled when the script
/// host is taken for hung, [`STEP_HUNG_AFTER`] after the step was asked
/// and not at [`STEP_DEADLINE`], which a busy machine passes (DEC-7357 as
/// revised). The key itself is not held that long: the trusted core
/// takes its default at `VERDICT_DEADLINE` whatever the decoder says.
#[test]
fn a_key_whose_step_is_never_answered_is_reported_as_not_cancelled_at_the_deadline() {
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(Batch::new(0));
    written(&mut fixture.out);
    let pressed = Instant::now();
    fixture.scripts.input(at(0x100), 31, &key('1', KeyAction::Press), &mut fixture.engine, &mut fixture.out).unwrap();
    assert!(written(&mut fixture.out).0.is_empty(), "the verdict waits");
    // The loop wakes by itself at the deadline, on the engine's clock.
    assert!(fixture.scripts.due_us(&fixture.engine).is_some());
    fixture.scripts.wake(&mut fixture.engine, &mut fixture.out).unwrap();
    assert!(written(&mut fixture.out).0.is_empty(), "the script host still has time");
    fixture.scripts.wake_at(pressed + 2 * STEP_DEADLINE, &mut fixture.engine, &mut fixture.out).unwrap();
    assert!(written(&mut fixture.out).0.is_empty(), "a step's time passed stops no script");
    fixture.scripts.wake_at(Instant::now() + STEP_HUNG_AFTER, &mut fixture.engine, &mut fixture.out).unwrap();
    let reports = written(&mut fixture.out).0;
    assert_eq!(reports.first(), Some(&ScriptReport::Verdict { sequence: 31, cancelled: false, handled: false }), "{reports:?}");
    assert!(reports.iter().any(|report| matches!(report, ScriptReport::Stopped { reason: FailReason::Time, .. })), "{reports:?}");
}

/// DEC-7315 version 2, the owner's ruling on local files and the network: a
/// page file opened from disk runs its script at once, and its script has
/// the network when its manifest asks for it; the context is told the
/// page is a file's, and the file's address is no folder a script's
/// address is made from.
#[test]
fn a_file_opened_from_disk_runs_its_script_with_the_network_its_manifest_asks_for() {
    let mut fixture = Fixture::new();
    fixture.order(ScriptOrder::Channel(ChannelFacts { from_disk: true, ..ChannelFacts::default() }));
    fixture.order(source(ContextId::Page, 0x100, "own.js", b"/*! nt {\"api\":1,\"network\":true} */"));
    fixture.show(Some(0x100));
    let (reports, link) = written(&mut fixture.out);
    let open = link.iter().find_map(|message| if let ToScriptHost::ContextOpen(open) = message { Some(&**open) } else { None }).unwrap();
    assert!(open.flags.network && open.flags.from_disk, "{:?}", open.flags);
    assert_eq!((open.channel_url.as_str(), open.script_url.as_str()), ("", "own.js"));
    assert!(matches!(reports.as_slice(), [ScriptReport::Claims(claims)] if claims.running && claims.network), "{reports:?}");
}

/// Sections 12.4 to 12.6 (DEC-7287 item 10; TODO-7304 steps 3 and 4): a
/// page file that names its script has that script asked for by name,
/// once; the channel file's script for the same page is then not run and
/// the difference is said for the developer; the named script runs when
/// it comes; and a file that names none again gives the page back to the
/// channel file's.
#[test]
fn the_page_files_own_script_wins_over_the_channel_files() {
    let mut fixture = Fixture::new();
    fixture.show(Some(0x100));
    let names = |fixture: &mut Fixture, name: Option<&str>| {
        fixture.scripts.page_file_names(at(0x100), name.map(str::to_owned), &mut fixture.engine, &mut fixture.out).unwrap();
    };
    names(&mut fixture, Some("own.js"));
    assert_eq!(written(&mut fixture.out).0, vec![ScriptReport::Wanted { page: at(0x100), name: "own.js".into() }]);
    names(&mut fixture, Some("own.js"));
    assert!(written(&mut fixture.out).0.is_empty(), "asked once");

    // The channel file's script for the page arrives with the page.
    fixture.order(source(ContextId::Page, 0x100, "other.js", PAGE));
    let (reports, link) = written(&mut fixture.out);
    assert!(link.is_empty(), "the channel file's script is not run: {link:?}");
    assert!(matches!(reports.as_slice(), [ScriptReport::Log { level: 1, text, .. }] if text.contains("own.js") && text.contains("other.js")), "{reports:?}");

    fixture.order(source(ContextId::Page, 0x100, "own.js", b"/*! nt {\"api\":1,\"keys\":[\"o\"]} */"));
    let (reports, link) = written(&mut fixture.out);
    assert!(
        link.iter().any(|message| matches!(message, ToScriptHost::ContextOpen(open) if open.keys == ["o"] && open.script_url.ends_with("own.js"))),
        "{link:?}"
    );
    assert!(matches!(reports.as_slice(), [ScriptReport::Claims(claims)] if claims.keys == ["o"]), "{reports:?}");

    // Another page's file names nothing of this page: exact keys, and
    // nothing inherited by a sub-page.
    fixture.scripts.page_file_names(PageSub::new(0x100, 2), None, &mut fixture.engine, &mut fixture.out).unwrap();
    assert!(written(&mut fixture.out).1.is_empty(), "another sub-code of the page is another page");
    assert!(fixture.scripts.page_context().is_some());

    // The file is read again without its line: its script ends, and the
    // channel file's runs when it next comes with the page.
    names(&mut fixture, None);
    let (_, link) = written(&mut fixture.out);
    assert!(matches!(link.first(), Some(ToScriptHost::ContextClose(ContextId::Page))), "{link:?}");
    fixture.order(source(ContextId::Page, 0x100, "other.js", PAGE));
    let (_, link) = written(&mut fixture.out);
    assert!(link.iter().any(|message| matches!(message, ToScriptHost::ContextOpen(open) if open.script_url.ends_with("other.js"))), "{link:?}");
}

/// DEC-7282 item 10 and section 12.4: a fetched page file's `JS` line is
/// noted with its page and sub-code, a page file with none is noted as
/// naming none, and page bytes of a push connection name no script.
#[test]
fn a_fetched_page_files_js_line_is_read_and_a_pushed_ones_is_not() {
    let mut engine = Engine::new();
    engine.feed(b"PN,10000\nSC,0000\nPS,8000\nJS,own.js\nOL,1,ONE\n\nPN,10100\nSC,0000\nPS,8000\nOL,1,TWO\n");
    engine.finish();
    assert_eq!(engine.take_script_names(), vec![(0x100, 0, Some("own.js".to_owned())), (0x101, 0, None)]);
    assert!(engine.take_script_names().is_empty(), "taken once");
    let page = b"PN,10200\nSC,0000\nPS,8000\nJS,pushed.js\nOL,1,PUSHED\n";
    engine.feed_whole(false, true, page);
    assert!(engine.take_script_names().is_empty(), "a push connection's page bytes name no script");
    engine.feed_whole(false, false, page);
    assert_eq!(engine.take_script_names(), vec![(0x102, 0, Some("pushed.js".to_owned()))]);
}

/// A batch that makes sprite 1: three cells by two at column 10, row 7,
/// of two frames, detectable.
fn sprite_batch(sequence: u32) -> Batch {
    use protocol::script::batch::{Sprite, TeletextObject, Triplet};
    let character = neoteletext_core::packet::TripletFunction::G0Character { column: 0, code: b'X' }.to_triplet();
    let body = vec![Triplet { address: character.address, mode: character.mode, data: character.data }];
    let sprite = Sprite { number: 1, shown: true, detectable: true, layer: 2, order: 0, column: 10, row: 7, width: 3, height: 2, objects: vec![0, 0] };
    Batch { teletext_objects: Some(vec![TeletextObject { number: 0, triplets: body }]), sprites: Some(vec![sprite]), ..Batch::new(sequence) }
}

fn pointer_at(sequence: u32, action: PointerAction, column: u16, row: u16) -> ScriptOrder {
    use protocol::decoder::script::{PointerKind, ScriptPointer};
    let (x, y) = (column * 12 + 1, row * 20 + 1);
    ScriptOrder::Pointer(ScriptPointer { sequence, action, button: 0, x, y, pointer: 1, kind: PointerKind::Mouse, primary: true })
}

/// Section 4.4's table for teletext (TODO-7382 step 2(f), TODO-7379 item
/// 3): a pointer event on a sprite carries the cell within the sprite,
/// from where the sprite stands; and as the pointer comes onto a thing
/// and goes off it the thing hears `pointerenter` and `pointerleave`,
/// under a claim of presses alone, with no move event.
#[test]
fn a_pointer_on_a_sprite_is_told_its_cell_within_it_and_its_coming_and_going() {
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(sprite_batch(0));
    written(&mut fixture.out);
    fixture.order(pointer_at(40, PointerAction::Press, 12, 8));
    let steps = fixture.steps();
    let events: Vec<&Event> = steps.iter().flat_map(|step| step.events.iter()).collect();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!((events[0].kind, events[0].numbers[0] & 15, events[0].numbers[3]), (EventKind::Pointer, events::POINTER_ENTER, 1), "it came onto sprite 1");
    let press = events[1];
    assert_eq!(
        (press.numbers[0] & 15, press.numbers[1], press.numbers[2], press.numbers[3]),
        (events::POINTER_DOWN, 12, 8, 1),
        "the page's cell and the sprite"
    );
    let local: Vec<i32> = press.payload.chunks(4).skip(2).take(2).map(|word| i32::from_be_bytes(word.try_into().unwrap())).collect();
    assert_eq!(local, [2, 1], "the cell within the sprite");
    fixture.answer(Batch { verdicts: Some(vec![Verdict::default(), Verdict { cancelled: false, handled: true }]), ..Batch::new(1) });
    assert_eq!(written(&mut fixture.out).0, vec![ScriptReport::Verdict { sequence: 40, cancelled: false, handled: true }], "only the press is answered");
    // The pointer moves off the sprite: it hears that, and no move.
    fixture.order(pointer_at(41, PointerAction::Move, 30, 3));
    let steps = fixture.steps();
    let events: Vec<&Event> = steps.iter().flat_map(|step| step.events.iter()).collect();
    assert_eq!(events.iter().map(|event| (event.numbers[0] & 15, event.numbers[3])).collect::<Vec<_>>(), vec![(events::POINTER_LEAVE, 1)]);
    // Over nothing, a press is told the page's own cell both ways.
    fixture.answer(Batch::new(2));
    fixture.order(pointer_at(42, PointerAction::Press, 30, 3));
    let steps = fixture.steps();
    let press = &steps[0].events[0];
    let local: Vec<i32> = press.payload.chunks(4).skip(2).take(2).map(|word| i32::from_be_bytes(word.try_into().unwrap())).collect();
    assert_eq!((press.numbers[1], press.numbers[2], press.numbers[3], local), (30, 3, 0, vec![30, 3]));
}

/// Section 11.2 (TODO-7379 item 3; TODO-7382 step 2(f) and (g)): a step
/// reports what the decoder's own renderers say of the things a script
/// asked about: the bounds of a tracked thing as it stands, a cycle that
/// ran once and has ended, by the thing's handle, and an animation
/// operation that did nothing as a fault; and the wheel is a script's
/// only over a thing its batch named to take it.
#[test]
fn a_step_reports_tracked_bounds_an_ended_cycle_a_fault_and_the_wheel_only_on_its_things() {
    use neoteletext_rules::animate::{Operation, OperationKind};
    use neoteletext_rules::table::Target;
    use protocol::script::batch::Watch;
    use protocol::script::{Bounds, FaultReport};
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(sprite_batch(0));
    written(&mut fixture.out);
    let wheel = |sequence: u32| ScriptOrder::Wheel { sequence, x: 11 * 12 + 1, y: 7 * 20 + 1, delta_x: 0, delta_y: 1 };
    // PAGE claims no wheel and the batch named no thing: nothing is sent.
    fixture.order(wheel(50));
    assert_eq!(fixture.steps(), vec![], "no wheel event off a thing that takes it");
    // The script tracks sprite 1, cycles it once and animates a sprite
    // that does not exist.
    fixture.at_tick(2);
    let asked = fixture.steps();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        (asked[0].bounds.as_slice(), asked[0].cycle_ended.as_slice(), asked[0].faults.as_slice()),
        (&[][..], &[][..], &[][..]),
        "nothing asked about yet"
    );
    let cycle = Operation { target: Target::object(1), kind: OperationKind::Cycle { first: 0, last: 1, period: 2, once: true } };
    let nothing = Operation { target: Target::object(9), kind: OperationKind::Show };
    fixture.answer(Batch {
        animate: Some(vec![cycle, nothing]),
        watch: Some(Watch { painted: None, tracked: Some(vec![1, 9]), wheel: Some(vec![1]) }),
        ..Batch::new(asked[0].sequence)
    });
    written(&mut fixture.out);
    assert!(fixture.scripts.due_us(&fixture.engine).is_some(), "the loop wakes to see the cycle end");
    // The decoder's animation runs on to the cycle's last frame.
    let now = due_after(40);
    fixture.engine.animation_showing(&[0x100]);
    fixture.engine.animation_step(&[0x100], now);
    fixture.engine.set_now_us(now);
    fixture.scripts.wake(&mut fixture.engine, &mut fixture.out).unwrap();
    let told = fixture.steps();
    assert_eq!(told.len(), 1, "a step is owed for what there is to report");
    assert_eq!(told[0].cycle_ended, vec![1]);
    assert_eq!(
        told[0].bounds,
        vec![Bounds { handle: 1, x: 120, y: 140, width: 36, height: 40 }],
        "the sprite's cells in the picture's pixels; a thing that is not there has none"
    );
    assert_eq!(told[0].faults, vec![FaultReport { code: limits::FAULT_ANIMATION_SKIPPED.0, bound: limits::FAULT_ANIMATION_SKIPPED.1.to_owned() }]);
    fixture.answer(Batch::new(told[0].sequence));
    written(&mut fixture.out);
    // Told once: the next step says the bounds again and nothing else.
    fixture.at_tick(60);
    let later = fixture.steps();
    assert!(later.iter().all(|step| step.cycle_ended.is_empty() && step.faults.is_empty()), "{later:?}");
}

/// Section 3.5 (TODO-7379 item 3): a context is told the sounds the
/// channel's files have announced, which are the decoder's own store's;
/// and a cue of a voice no script plays on is no script's to hear.
#[test]
fn a_context_is_told_the_sounds_announced_and_hears_only_its_own_voices() {
    let (mut fixture, _, link) = Fixture::started();
    let announced = fixture.engine.script_sounds();
    for open in link.iter().filter_map(|message| if let ToScriptHost::ContextOpen(open) = message { Some(open) } else { None }) {
        assert_eq!(open.sounds, announced);
    }
    fixture.answer(Batch::new(0));
    written(&mut fixture.out);
    fixture.scripts.cue(2, 5, &mut fixture.engine, &mut fixture.out).unwrap();
    assert!(fixture.steps().is_empty(), "voice 2 plays no sound of a script's");
    // A script plays on voice 2: a cue heard on it is told in the step
    // owed at the next tick.
    fixture.at_tick(2);
    let asked = fixture.steps();
    let play = protocol::script::batch::SoundOperation::Play { voice: 2, sound: 1, gain: 100 };
    fixture.answer(Batch { sound: Some(vec![play]), ..Batch::new(asked[0].sequence) });
    written(&mut fixture.out);
    fixture.scripts.cue(2, 5, &mut fixture.engine, &mut fixture.out).unwrap();
    fixture.at_tick(3);
    let told = fixture.steps();
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].sound, vec![protocol::script::SoundReport::Cue { voice: 2, cue: 5 }]);
}

/// Section 13 item 5 (TODO-7379 item 3): a page file fetched again while
/// its script runs does not put the file's rows back over what the
/// script drew: the rows the script wrote are written again at once, and
/// the rows it never wrote are the file's, new.
#[test]
fn a_page_file_fetched_again_does_not_overwrite_what_its_script_drew() {
    let row = |row: u8, text: &str| {
        let mut codes = [b' '; 40];
        codes[..text.len()].copy_from_slice(text.as_bytes());
        TeletextRow { row, codes }
    };
    let shown = |fixture: &Fixture, row: usize| {
        let pages = fixture.engine.export();
        let held = pages.iter().find(|held| held.page == 0x100).unwrap();
        held.rows[row].iter().map(|code| char::from(code & 0x7F)).collect::<String>().trim().to_owned()
    };
    let (mut fixture, _, _) = Fixture::started();
    fixture.answer(Batch { teletext_rows: Some(vec![row(1, "SCRIPT ROW ONE"), row(5, "SCRIPT ROW FIVE")]), ..Batch::new(0) });
    assert_eq!((shown(&fixture, 1), shown(&fixture, 5)), ("SCRIPT ROW ONE".to_owned(), "SCRIPT ROW FIVE".to_owned()));
    // The file comes again, with a row the script never wrote.
    fixture.engine.feed_whole(false, false, b"PN,10000\nSC,0000\nPS,8000\nOL,1,FILE ROW ONE\nOL,5,FILE ROW FIVE\nOL,9,FILE ROW NINE\n");
    assert_eq!(shown(&fixture, 1), "FILE ROW ONE", "the file's packets replaced the page's");
    let names = fixture.engine.take_script_names();
    assert_eq!(names.last(), Some(&(0x100, 0, None)), "the loop is told the page's file came: {names:?}");
    let outcome = fixture.scripts.page_file_names(at(0x100), None, &mut fixture.engine, &mut fixture.out).unwrap();
    assert!(outcome.applied);
    assert_eq!(
        (shown(&fixture, 1), shown(&fixture, 5), shown(&fixture, 9)),
        ("SCRIPT ROW ONE".to_owned(), "SCRIPT ROW FIVE".to_owned(), "FILE ROW NINE".to_owned())
    );
    // Another page's file changes nothing of this one.
    let outcome = fixture.scripts.page_file_names(at(0x101), None, &mut fixture.engine, &mut fixture.out).unwrap();
    assert!(!outcome.applied);
}
