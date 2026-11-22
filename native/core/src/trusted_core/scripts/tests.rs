//! The script hosts and the relay (`script-api.md` section 11.1;
//! TODO-7292's checks): a script runs in the real program, confined,
//! started through the broker's wire for the trusted core and relayed to
//! its decoder; a step not answered kills the process and the channel's
//! scripts are shown stopped; a kind a script host may not write is not
//! relayed; a request reaches the partition's web client with its tab
//! and token and only when the network was reported.

#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects, clippy::indexing_slicing, clippy::panic, reason = "a test")]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use confine::broker::stand_in::{StandIn, Started};
use confine::broker::{Program, Request};
use neoteletext::bookmarks::Bookmarks;
use neoteletext::outbox::Outbox;
use neoteletext::settings::Settings;
use protocol::fetcher::PROCESS_TAB;
use protocol::script::link::{Asked, HostLink, Received};
use protocol::script::relay::{DecoderInput, DecoderOutput, HostInput, read_decoder_input, read_decoder_output, read_host_input};
use protocol::script::{
    Batch, ContextFlags, ContextOpen, Level as PageLevel, Renderer, Step, StepFlags, ToScriptHost, write_to_decoder as write_link_to_decoder, write_to_host,
};
use protocol::web::{
    BodySource, CacheMode, Credentials, FromWebClient, Mode, PARTITION_TOKEN_BYTES, PartitionToken, RedirectMode, Referrer, ReferrerPolicy, RequestKind, Start,
    ToWebClient, read_to_client, write_from_client, write_request,
};

use super::super::run::handle;
use super::super::test_support::{Sink, decoder_watched, stand_in};
use super::*;
use crate::session::Session;

/// Builds the script-host program for this test's profile, as a release
/// puts it beside the broker's, and gives the folder it is in. Cargo is
/// run without what the test runner set for the package under test, but
/// for where it keeps things, so that this build does not change what
/// the next ordinary build must rebuild (`tests/reference_channel/site.rs`).
fn script_host_built() -> std::path::PathBuf {
    let mut build = std::process::Command::new(env!("CARGO"));
    for (name, _) in std::env::vars_os() {
        let runners = name.to_str().is_some_and(|name| name == "OUT_DIR" || (name.starts_with("CARGO_") && !matches!(name, "CARGO_HOME" | "CARGO_TARGET_DIR")));
        if runners {
            build.env_remove(name);
        }
    }
    build.args(["build", "--quiet", "--package", "neoteletext-script-host"]);
    if !cfg!(debug_assertions) {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success(), "the script host did not build");
    // The test runs from target/<profile>/deps; the programs are beside it.
    let deps = std::env::current_exe().unwrap();
    deps.parent().and_then(|deps| deps.parent()).unwrap().to_path_buf()
}

/// How long anything may take before a test gives up.
const LIMIT: Duration = Duration::from_secs(30);
/// The channel the tests' tab is on.
const CHANNEL: &str = "https://example.test/chan/";
/// The generation `decoder_watched` gives its decoder.
const DECODER: u64 = 1;

/// A trusted core on [`CHANNEL`] with a decoder that reads nothing, whose
/// input the test reads, and a broker stood in for.
struct Test {
    core: Core,
    incoming: mpsc::Receiver<Event>,
    stand_in: StandIn,
    channel: u32,
    /// What the core wrote to the decoder.
    to_decoder: mpsc::Receiver<Vec<u8>>,
    /// Of it, what has not been read as a message yet.
    unread: Vec<u8>,
}

impl Test {
    fn with(broker: Arc<Broker>, stand_in: StandIn) -> Self {
        let (events, incoming) = mpsc::channel();
        let mut core =
            Core::new(Session::new(Bookmarks::default()), Vec::new(), (Settings::default(), None), events, Outbox::new(Sink(mpsc::channel().0)), Some(broker));
        core.session.open(CHANNEL).unwrap();
        let channel = core.active_channel().unwrap();
        let to_decoder = decoder_watched(&mut core, channel);
        Self { core, incoming, stand_in, channel, to_decoder, unread: Vec::new() }
    }

    /// With helpers that are two pipes, each saying `first_words`.
    fn piped(first_words: impl Fn(&Started) -> Option<Vec<u8>> + Send + 'static) -> Self {
        let (broker, stand_in) = StandIn::start(Box::new(|_| Err(String::from("no file"))), Box::new(first_words)).unwrap();
        Self::with(broker, stand_in)
    }

    /// With the real script-host program, built beside this test.
    fn real() -> Self {
        let programs = script_host_built();
        let (broker, stand_in) = StandIn::start_real(Box::new(|_| Err(String::from("no file"))), Box::new(|_| Some(Vec::new())), programs).unwrap();
        Self::with(broker, stand_in)
    }

    /// Gives the core what the decoder wrote, as its reader gives it.
    fn decoder_writes(&mut self, written: &[u8]) {
        let mut input = written;
        while let Some(output) = read_decoder_output(&mut input).unwrap() {
            let DecoderOutput::ToHost(asked) = output else { panic!("not the script link's") };
            self.core.script_asked(self.channel, DECODER, &asked);
        }
    }

    /// Handles the next event, and the scripts' timed work after it.
    fn pump(&mut self, within: Duration) -> bool {
        let Ok(event) = self.incoming.recv_timeout(within) else { return false };
        handle(&mut self.core, event).unwrap();
        self.core.scripts_tick(Instant::now());
        true
    }

    /// The next message of the script link the decoder was given, handling
    /// events until it comes.
    fn decoder_reads(&mut self) -> Option<FromScriptHost> {
        let deadline = Instant::now() + LIMIT;
        loop {
            if let Some(decoder) = self.core.decoders.get_mut(&self.channel) {
                decoder.flush().unwrap();
            }
            while let Ok(chunk) = self.to_decoder.recv_timeout(Duration::from_millis(20)) {
                self.unread.extend(chunk);
            }
            if self.unread.len() >= 5 {
                let length = u32::from_be_bytes([self.unread[1], self.unread[2], self.unread[3], self.unread[4]]) as usize;
                if self.unread.len() >= 5 + length {
                    let frame: Vec<u8> = self.unread.drain(..5 + length).collect();
                    let Some(DecoderInput::Link(message)) = read_decoder_input(&mut frame.as_slice()).unwrap() else { panic!("not the script link's") };
                    return Some(message);
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            self.pump(Duration::from_millis(100));
        }
    }

    /// The helpers started of `program`.
    fn started(&self, program: Program) -> Vec<Started> {
        self.stand_in.started().into_iter().filter(|helper| helper.program == program).collect()
    }

    /// Handles events until `request` has been made of the broker.
    fn until_asked(&mut self, wanted: impl Fn(&Request) -> bool) {
        let deadline = Instant::now() + LIMIT;
        while Instant::now() < deadline {
            if self.stand_in.take_requests().iter().any(&wanted) {
                return;
            }
            self.pump(Duration::from_millis(50));
        }
        panic!("the broker was never asked");
    }
}

fn opening(source: &[u8]) -> ContextOpen {
    ContextOpen {
        context: ContextId::Page,
        api: 1,
        renderer: Renderer::Teletext,
        level: PageLevel::TwoFive,
        page: 0x100,
        subcode: 0,
        flags: ContextFlags::default(),
        tick_divisor: 0,
        language: "en".into(),
        channel_url: CHANNEL.into(),
        page_url: format!("{CHANNEL}100.tti"),
        script_url: format!("{CHANNEL}page.js"),
        channel_title: "Example".into(),
        seed: [7; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: Vec::new(),
        source_bytes: u32::try_from(source.len()).unwrap(),
    }
}

fn step(sequence: u32) -> Step {
    Step { sequence, tick: u64::from(sequence), flags: StepFlags { tick: true, ..StepFlags::default() }, ..Step::default() }
}

fn ready() -> Vec<u8> {
    let mut written = Vec::new();
    write_link_to_decoder(&mut written, &FromScriptHost::Ready { link_version: protocol::script::LINK_VERSION, api_max: 1 }).unwrap();
    written
}

fn hello() -> Vec<u8> {
    let mut written = Vec::new();
    write_to_host(&mut written, &ToScriptHost::Hello { link_version: protocol::script::LINK_VERSION }).unwrap();
    written
}

fn stepped(sequence: u32) -> Vec<u8> {
    let mut written = Vec::new();
    write_to_host(&mut written, &ToScriptHost::Step(Box::new(step(sequence)))).unwrap();
    written
}

/// TODO-7292's "done" and section 11.1: a script runs in the script-host
/// program, started confined through the broker's wire at the trusted
/// core's request when the decoder greets, and what it does is relayed
/// to its decoder; told to end, the process is ended and nothing is
/// marked stopped. A script whose start was stopped for time, as on a
/// machine with no processor free, is not this test's to judge: the run
/// is made again.
#[test]
fn a_script_runs_in_a_confined_process_the_broker_started_and_is_relayed_to_its_decoder() {
    for _ in 0..5 {
        let mut test = Test::real();
        let mut link = HostLink::new();
        let source = b"console.log('page ' + nt.page.number.toString(16) + ' runs'); nt.addEventListener('close', () => console.log('closed'));";
        let mut written = Vec::new();
        link.open(&mut written, &opening(source), source).unwrap();
        link.step(&mut written, &step(1)).unwrap();
        assert_eq!(test.core.script_state(test.channel), ScriptState::None);
        test.decoder_writes(&written);
        assert_eq!(test.core.script_state(test.channel), ScriptState::Running);
        let hosts = test.started(Program::ScriptHost);
        assert_eq!(hosts.len(), 1, "one script host for the (tab, channel)");
        assert!(hosts[0].arguments.is_empty() && !hosts[0].local_network && hosts[0].udp_port.is_none(), "{:?}", hosts[0]);
        #[cfg(target_os = "linux")]
        assert_eq!(hosts[0].report, "confines itself at its start");
        assert_eq!(link.receive(test.decoder_reads().unwrap()), Received::Ready { api_max: 1 });
        let batch = match link.receive(test.decoder_reads().unwrap()) {
            Received::Batch { asked: Asked::Step(1), batch } => batch,
            Received::Stopped(stopped) if stopped.reason == FailReason::Time => continue,
            other => panic!("{other:?}"),
        };
        let lines: Vec<String> = batch.log.unwrap_or_default().into_iter().map(|line| line.text).collect();
        assert_eq!(lines, vec!["page 100 runs"]);
        // The page is left: its close's own batch, then the end.
        let mut written = Vec::new();
        link.close(&mut written, ContextId::Page).unwrap();
        test.decoder_writes(&written);
        let Received::Batch { asked: Asked::Close(ContextId::Page), batch } = link.receive(test.decoder_reads().unwrap()) else {
            panic!("no batch for the close")
        };
        assert_eq!(batch.log.unwrap_or_default().into_iter().map(|line| line.text).collect::<Vec<_>>(), vec!["closed"]);
        assert_eq!(test.core.scripts_due_at(), None, "everything asked was answered");
        let mut written = Vec::new();
        link.shutdown(&mut written).unwrap();
        test.decoder_writes(&written);
        assert_eq!(test.core.script_state(test.channel), ScriptState::None);
        // It ends by itself, told to: the broker is asked to wait for it.
        test.until_asked(|request| matches!(request, Request::Wait(id) if *id == hosts[0].id));
        return;
    }
    panic!("stopped for time in five tries: this machine has no processor free");
}

/// Section 11.1 and TODO-7292's check: a script host that does not
/// answer a step within `STEP_DEADLINE` is killed, its decoder is told
/// FAILED for time, the channel's scripts are shown stopped, and no
/// script host is started for the channel again. The script host here
/// is two pipes that say READY and then nothing.
#[test]
fn a_script_host_that_does_not_answer_a_step_is_killed_and_its_scripts_stay_stopped() {
    let mut test = Test::piped(|_| Some(ready()));
    test.decoder_writes(&hello());
    assert_eq!(test.decoder_reads(), Some(FromScriptHost::Ready { link_version: protocol::script::LINK_VERSION, api_max: 1 }));
    assert_eq!(test.core.scripts_due_at(), None, "nothing is asked yet");
    let asked = Instant::now();
    test.decoder_writes(&stepped(1));
    let due = test.core.scripts_due_at().unwrap();
    assert!(due >= asked + STEP_DEADLINE && due <= Instant::now() + STEP_DEADLINE, "the deadline is STEP_DEADLINE from the step");
    // Not before its deadline.
    test.core.scripts_tick(due.checked_sub(Duration::from_millis(1)).unwrap());
    assert_eq!(test.core.script_state(test.channel), ScriptState::Running);
    test.core.scripts_tick(due);
    assert_eq!(test.core.script_state(test.channel), ScriptState::Stopped(FailReason::Time));
    let Some(FromScriptHost::Failed { context: ContextId::Channel, reason: FailReason::Time, text }) = test.decoder_reads() else {
        panic!("the decoder was not told")
    };
    assert!(text.starts_with("STEP_DEADLINE"), "{text}");
    let host = test.started(Program::ScriptHost)[0].id;
    test.until_asked(|request| matches!(request, Request::Kill(id) if *id == host));
    // The decoder greets again, as a replaced decoder would: told again,
    // and nothing is started.
    test.decoder_writes(&hello());
    test.decoder_writes(&stepped(2));
    assert!(matches!(test.decoder_reads(), Some(FromScriptHost::Failed { reason: FailReason::Time, .. })));
    assert_eq!(test.started(Program::ScriptHost).len(), 1, "a stopped channel's script host was started again");
    assert_eq!(test.core.script_state(test.channel), ScriptState::Stopped(FailReason::Time));
}

/// Section 11.1: an opening gives a script host `START_DEADLINE`, since
/// a context's start may take that long; and one that falls more than
/// `MAX_STEPS_UNANSWERED` behind is killed whatever the clock says.
#[test]
fn an_opening_has_the_start_deadline_and_a_script_host_too_far_behind_is_killed() {
    let mut test = Test::piped(|_| Some(ready()));
    let mut link = HostLink::new();
    let mut written = Vec::new();
    link.open(&mut written, &opening(b"1"), b"1").unwrap();
    link.step(&mut written, &step(1)).unwrap();
    let asked = Instant::now();
    test.decoder_writes(&written);
    let due = test.core.scripts_due_at().unwrap();
    assert!(due >= asked + START_DEADLINE && due <= Instant::now() + START_DEADLINE);
    test.core.scripts_tick(asked + STEP_DEADLINE + Duration::from_millis(50));
    assert_eq!(test.core.script_state(test.channel), ScriptState::Running, "killed while its context may still be starting");
    for sequence in 2..=u32::try_from(MAX_STEPS_UNANSWERED).unwrap() {
        test.decoder_writes(&stepped(sequence));
    }
    assert_eq!(test.core.script_state(test.channel), ScriptState::Running);
    test.decoder_writes(&stepped(999));
    assert_eq!(test.core.script_state(test.channel), ScriptState::Stopped(FailReason::Time));
}

/// TODO-7292's check and section 11.1: a frame of a kind a script host
/// may not write is not relayed. A script host that writes READY, then a
/// message only the trusted core may give a web client (`TabCloses`),
/// then a batch, has its READY passed on and nothing after: it is killed,
/// and the decoder is told the link failed.
#[test]
fn a_frame_of_a_kind_a_script_host_may_not_write_is_not_relayed() {
    // `TabCloses` (kind 239) as a web client reads it after its tab: on
    // a script host's pipe it is no message.
    let mut said = ready();
    said.extend_from_slice(&[239, 0, 0, 0, 0]);
    write_link_to_decoder(&mut said, &FromScriptHost::Batch(Box::new(Batch::new(1)))).unwrap();
    let mut test = Test::piped(move |_| Some(said.clone()));
    test.decoder_writes(&hello());
    test.decoder_writes(&stepped(1));
    assert_eq!(test.decoder_reads(), Some(FromScriptHost::Ready { link_version: protocol::script::LINK_VERSION, api_max: 1 }));
    let Some(FromScriptHost::Failed { context: ContextId::Channel, reason: FailReason::Link, text }) = test.decoder_reads() else {
        panic!("the decoder was not told")
    };
    assert!(text.contains("message kind 239"), "{text}");
    assert_eq!(test.core.script_state(test.channel), ScriptState::Stopped(FailReason::Link));
    // Nothing more reaches the decoder: not the batch that followed.
    while test.pump(Duration::from_millis(300)) {}
    test.core.decoders.get_mut(&test.channel).unwrap().flush().unwrap();
    assert!(test.to_decoder.recv_timeout(Duration::from_millis(200)).is_err() && test.unread.is_empty(), "something was relayed after the refused frame");
    assert_eq!(test.started(Program::Fetcher), []);
}

fn start() -> Start {
    Start {
        id: 7,
        context: ContextId::Page,
        kind: RequestKind::Fetch,
        method: "GET".into(),
        url: format!("{CHANNEL}data.json"),
        client: format!("{CHANNEL}100.tti"),
        headers: Vec::new(),
        mode: Mode::Cors,
        credentials: Credentials::SameOrigin,
        cache: CacheMode::Default,
        redirect: RedirectMode::Follow,
        referrer: Referrer::Client,
        referrer_policy: ReferrerPolicy::Unset,
        integrity: String::new(),
        keepalive: false,
        body: BodySource::None,
        protocols: Vec::new(),
        last_event_id: String::new(),
        target_space: None,
    }
}

/// What a script host that asks the network says: READY, then a `Start`.
fn asks_the_network() -> Vec<u8> {
    let mut said = ready();
    write_request(&mut said, &ToWebClient::Start(Box::new(start()))).unwrap();
    said
}

/// Section 11.1: the network's kinds are relayed only for a channel
/// whose decoder reported the network. A script host that asks without
/// it is killed, and no web client is started.
#[test]
fn a_request_from_a_script_host_without_the_network_is_not_relayed() {
    let mut test = Test::piped(|_| Some(asks_the_network()));
    test.decoder_writes(&hello());
    assert!(matches!(test.decoder_reads(), Some(FromScriptHost::Ready { .. })));
    let Some(FromScriptHost::Failed { reason: FailReason::Link, text, .. }) = test.decoder_reads() else { panic!("the decoder was not told") };
    assert!(text.contains("asked for the network"), "{text}");
    assert!(test.started(Program::Fetcher).is_empty(), "a web client was started");
}

/// Section 11.1 and TODO-7420: the viewer leaves a page whose script has
/// the network for one whose script has none. The decoder reports the
/// new page's claims while the old page's context is still closing, and
/// what that context writes to finish with the network (an abort of a
/// request under way, the close of a socket or an event stream, the word
/// that the context closes, and a keepalive request started in its
/// `close` handling) is relayed for [`NETWORK_DRAIN`] and stops nothing.
/// After it, the web client is told to end what it still holds for the
/// channel; a message that only ends something is dropped; and one that
/// starts something is the breach it always was.
#[test]
fn a_context_that_had_the_network_finishes_with_it_and_starts_nothing_after() {
    let said = |message: &ToWebClient| {
        let mut bytes = Vec::new();
        write_request(&mut bytes, message).unwrap();
        read_host_output(&mut bytes.as_slice()).unwrap().unwrap()
    };
    let mut test = Test::piped(|helper| match helper.program {
        Program::ScriptHost => Some(ready()),
        _ => Some(Vec::new()),
    });
    test.decoder_writes(&hello());
    let channel = test.channel;
    let generation = test.core.scripts.hosts[&channel].generation;
    test.core.script_network(channel, true);
    test.core.script_said(channel, generation, said(&ToWebClient::Start(Box::new(start()))));
    assert_eq!(test.started(Program::Fetcher).len(), 1, "the page's request reached a web client");
    // The next page's claims: no network.
    test.core.script_network(channel, false);
    let finishing = [
        ToWebClient::Abort(7),
        ToWebClient::SocketClose { id: 8, code: 1000, reason: String::new() },
        ToWebClient::ContextCloses(ContextId::Page),
        ToWebClient::Start(Box::new(Start { keepalive: true, ..start() })),
    ];
    for message in &finishing {
        test.core.script_said(channel, generation, said(message));
        assert_eq!(test.core.script_state(channel), ScriptState::Running, "{message:?} is the closing context's own");
    }
    assert!(test.core.scripts.web.serves(channel), "and was relayed to its web client's tab");
    assert!(test.core.scripts_due_at().is_some(), "the trusted core wakes when the time to finish is up");
    // The time to finish is up.
    let host = test.core.scripts.hosts.get_mut(&channel).unwrap();
    host.network_left = Instant::now().checked_sub(NETWORK_DRAIN + Duration::from_millis(1));
    test.core.scripts_tick(Instant::now());
    assert!(!test.core.scripts.web.serves(channel), "the web client was told to end what it held for the channel");
    assert_eq!(test.core.scripts_due_at(), None);
    for message in &finishing[..3] {
        test.core.script_said(channel, generation, said(message));
        assert_eq!(test.core.script_state(channel), ScriptState::Running, "{message:?} only ends something: dropped");
    }
    assert!(!test.core.scripts.web.serves(channel), "and not relayed");
    test.core.script_said(channel, generation, said(&ToWebClient::Start(Box::new(start()))));
    assert_eq!(test.core.script_state(channel), ScriptState::Stopped(FailReason::Link), "a context without the network starts nothing");
    // A page with the network again has it again, at once.
    let mut test = Test::piped(|helper| match helper.program {
        Program::ScriptHost => Some(ready()),
        _ => Some(Vec::new()),
    });
    test.decoder_writes(&hello());
    let generation = test.core.scripts.hosts[&test.channel].generation;
    test.core.script_network(test.channel, true);
    test.core.script_network(test.channel, false);
    test.core.script_network(test.channel, true);
    test.core.script_said(test.channel, generation, said(&ToWebClient::Start(Box::new(start()))));
    assert_eq!(test.core.script_state(test.channel), ScriptState::Running);
}

/// TODO-7419 (section 12.3): a JavaScript page has no page file, and the
/// "not found" its fetcher reports for it is no fault: the warning is
/// held, and never said once the page's script comes. It is said when
/// the script fails to come, when none has come in [`REFUSAL_HELD`], and
/// at once for any other status.
#[test]
fn a_page_file_not_found_is_no_warning_when_its_script_comes() {
    use protocol::decoder::script::SourcePiece;
    use protocol::fetcher::{FromFetcher, RefusedFile};
    use protocol::rules::PageSub;
    let mut test = Test::piped(|_| Some(Vec::new()));
    let channel = test.channel;
    let said = |test: &mut Test| {
        test.core.console.set_open(true);
        format!("{:?}", test.core.console.updates(Some(channel)))
    };
    let script = |page: u16| {
        FromFetcher::Script(SourcePiece {
            level: ContextId::Page,
            page: PageSub::new(page, 0),
            name: "p.js".into(),
            total: 2,
            offset: 0,
            bytes: b"//".to_vec(),
        })
    };
    // A JavaScript page: its file is not found, and its script comes.
    test.core.refused(channel, RefusedFile::Page(0x101), Some(NOT_FOUND)).unwrap();
    test.core.script_message(channel, script(0x101));
    test.core.scripts_tick(Instant::now() + REFUSAL_HELD);
    assert!(!said(&mut test).contains("page 101 was refused"), "nothing is said of a JavaScript page's missing file");
    // A page whose script could not be fetched either.
    test.core.refused(channel, RefusedFile::Page(0x102), Some(NOT_FOUND)).unwrap();
    assert!(test.core.scripts_due_at().is_some(), "the trusted core wakes to say it");
    test.core.script_message(channel, FromFetcher::ScriptFailed { level: ContextId::Page, page: PageSub::new(0x102, 0), name: "p.js".into() });
    assert!(said(&mut test).contains("page 102 was refused: status 404"));
    // A page that is missing and has no script: said when none has come.
    test.core.refused(channel, RefusedFile::Page(0x103), Some(NOT_FOUND)).unwrap();
    test.core.scripts_tick(Instant::now());
    assert!(!said(&mut test).contains("page 103 was refused"), "held for its script");
    test.core.scripts_tick(Instant::now() + REFUSAL_HELD);
    assert!(said(&mut test).contains("page 103 was refused: status 404"));
    assert_eq!(test.core.scripts_due_at(), None);
    // Any other answer is said at once.
    test.core.refused(channel, RefusedFile::Page(0x104), Some(500)).unwrap();
    assert!(said(&mut test).contains("page 104 was refused: status 500"));
}

/// DEC-7315 version 2 (TODO-7395 step 1): the script of a page opened
/// from disk has the network. Its request starts a web client of that
/// page alone, with the origin `null` for its identity and this
/// computer's address space, whose sandbox reaches this computer; the
/// script host is not ended.
#[test]
fn a_file_pages_request_starts_a_web_client_of_the_origin_null() {
    let mut test = Test::piped(|helper| match helper.program {
        Program::ScriptHost => Some(asks_the_network()),
        _ => Some(Vec::new()),
    });
    // The channel as one opened from disk is: with no web address.
    for tab in test.core.session.all_tabs_mut() {
        tab.channel.web = None;
    }
    test.decoder_writes(&hello());
    test.core.script_network(test.channel, true);
    assert!(matches!(test.decoder_reads(), Some(FromScriptHost::Ready { .. })));
    while test.pump(Duration::from_millis(500)) {}
    assert_eq!(test.core.script_state(test.channel), ScriptState::Running);
    let clients = test.started(Program::Fetcher);
    assert_eq!(clients.len(), 1);
    let [identity, flag, _token, space_flag, space] = clients[0].arguments.as_slice() else { panic!("{:?}", clients[0].arguments) };
    assert_eq!((identity.as_str(), flag.as_str()), (protocol::web::FILE_PAGE_IDENTITY, "--web-client"));
    assert_eq!((space_flag.as_str(), space.as_str()), ("--page-space", "loopback"));
    assert!(clients[0].local_network, "its sandbox reaches this computer and the local network");
    // The partition is the page's own: another page from disk, or a web
    // channel, is never served by it.
    let own = test.core.partition_of(test.channel).unwrap();
    let mut other = own.clone();
    other.file = Some(test.channel + 1);
    assert_ne!(own, other);
    assert_eq!(own.file, Some(test.channel));
}

/// Sections 11.1 and 11.3 (DEC-7268, DEC-7317 items 4 and 5, DEC-7333
/// item 1): with the network reported, a script host's request reaches
/// its partition's one web client, started with the channel's identity
/// and a token, in the client form: after the script host's tab, and a
/// `Start` with that token. The web client's answer for that tab is
/// passed to the script host as it stands; its `Confined`, which is the
/// trusted core's, and an answer for a tab it does not serve are not.
#[test]
fn a_request_reaches_the_partitions_web_client_with_its_tab_and_token_and_its_answer_comes_back() {
    let mut test = Test::piped(|helper| match helper.program {
        Program::ScriptHost => Some(asks_the_network()),
        Program::Fetcher => {
            let mut said = Vec::new();
            write_from_client(&mut said, PROCESS_TAB, &FromWebClient::Confined).unwrap();
            write_from_client(&mut said, 99, &FromWebClient::End(8)).unwrap();
            write_from_client(&mut said, 1, &FromWebClient::End(7)).unwrap();
            Some(said)
        }
        _ => Some(Vec::new()),
    });
    test.decoder_writes(&hello());
    test.core.script_network(test.channel, true);
    assert!(matches!(test.decoder_reads(), Some(FromScriptHost::Ready { .. })));
    // The request, then the answers: every event there is.
    while test.pump(Duration::from_millis(500)) {}
    assert_eq!(test.core.script_state(test.channel), ScriptState::Running);
    let clients = test.started(Program::Fetcher);
    assert_eq!(clients.len(), 1, "one web client for the partition");
    let [channel, flag, token] = clients[0].arguments.as_slice() else { panic!("{:?}", clients[0].arguments) };
    assert_eq!((channel.as_str(), flag.as_str()), (CHANNEL, "--web-client"));
    assert!(token.len() == 2 * PARTITION_TOKEN_BYTES && token.bytes().all(|byte| byte.is_ascii_hexdigit()), "{token}");
    let mut bytes = [0_u8; PARTITION_TOKEN_BYTES];
    for (byte, pair) in bytes.iter_mut().zip(token.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    let token = PartitionToken::new(bytes);
    // The pipes close, and what each helper was written is read.
    let Test { mut core, stand_in, incoming, .. } = test;
    core.end_scripts();
    drop((core, incoming));
    let written = stand_in.finish(LIMIT).unwrap();
    let to_client = &written.iter().find(|written| written.helper.program == Program::Fetcher).unwrap().bytes;
    let mut input = to_client.as_slice();
    assert_eq!(read_to_client(&mut input, &token).unwrap(), Some((1, ToWebClient::Start(Box::new(start())))), "the request, with the tab and the token");
    assert!(read_to_client(&mut input, &PartitionToken::new([0; PARTITION_TOKEN_BYTES])).is_ok(), "nothing after it needs a token");
    let to_host = &written.iter().find(|written| written.helper.program == Program::ScriptHost).unwrap().bytes;
    let mut input = to_host.as_slice();
    assert!(matches!(read_host_input(&mut input).unwrap(), Some(HostInput::Link(ToScriptHost::Hello { .. }))));
    assert_eq!(read_host_input(&mut input).unwrap(), Some(HostInput::Answer(FromWebClient::End(7))), "its own tab's answer");
    assert_eq!(read_host_input(&mut input).unwrap(), None, "another tab's answer, or the web client's own word, reached the script host");
}

/// Section 3.2 rule 5(b) and TODO-7338 item 1: a script host that ended
/// with its watchdog's status is read as stopped for time, and any other
/// untold end as the link's; either way its decoder is told and the
/// channel's scripts stay stopped.
#[test]
fn a_script_host_its_watchdog_ended_is_read_as_stopped_for_time() {
    let exit = |code: &str| {
        let mut child = stand_in("/bin/sh", &["-c", &format!("exit {code}")], confine::Role::Script);
        child.wait().unwrap()
    };
    assert!(ended_by_its_watchdog(&exit("70")));
    assert!(!ended_by_its_watchdog(&exit("1")) && !ended_by_its_watchdog(&exit("0")) && !ended_by_its_watchdog(&exit("7")));
    let mut test = Test::piped(|_| Some(ready()));
    test.core.script_host_exited(test.channel, DECODER, &exit("70"));
    assert_eq!(test.core.script_state(test.channel), ScriptState::Stopped(FailReason::Time));
    assert!(matches!(test.decoder_reads(), Some(FromScriptHost::Failed { context: ContextId::Channel, reason: FailReason::Time, .. })));
    let mut test = Test::piped(|_| Some(ready()));
    test.core.script_host_exited(test.channel, DECODER, &exit("1"));
    assert_eq!(test.core.script_state(test.channel), ScriptState::Stopped(FailReason::Link));
}

/// Section 11.1: a script host is ended when its decoder is, and its
/// channel's scripts are not marked stopped, so the channel's next
/// decoder starts them again; a channel no tab holds is forgotten.
#[test]
fn a_script_host_goes_with_its_decoder_and_starts_again_with_the_next() {
    let mut test = Test::piped(|_| Some(ready()));
    test.decoder_writes(&hello());
    assert_eq!(test.core.script_state(test.channel), ScriptState::Running);
    let first = test.started(Program::ScriptHost)[0].id;
    // The channel's decoder is replaced, as when its tab wakes.
    let child = stand_in("/bin/sleep", &["60"], confine::Role::Decoder);
    let (sender, written) = mpsc::channel();
    test.core.decoders.insert(test.channel, Decoder::new(child, Outbox::new(Sink(sender)), DECODER + 1));
    test.to_decoder = written;
    test.core.scripts_tick(Instant::now());
    assert_eq!(test.core.script_state(test.channel), ScriptState::None);
    test.until_asked(|request| matches!(request, Request::Kill(id) if *id == first));
    // The old decoder's messages are no one's; the new one's start a new
    // script host.
    test.decoder_writes(&hello());
    assert_eq!(test.started(Program::ScriptHost).len(), 1);
    let greeting = hello();
    let Some(DecoderOutput::ToHost(asked)) = read_decoder_output(&mut greeting.as_slice()).unwrap() else { panic!("not the script link's") };
    test.core.script_asked(test.channel, DECODER + 1, &asked);
    assert_eq!(test.started(Program::ScriptHost).len(), 2);
    assert_eq!(test.core.script_state(test.channel), ScriptState::Running);
}
