//! One run (script-api.md section 16): the page is opened in the
//! decoder's engine, the scripts in the script library through
//! [`ScriptLink`], and the clock is stepped tick by tick. Nothing here
//! reads a file or the time of day; the wall clock is read only to time
//! each step for the report and, with `--pace`, to wait for the next
//! tick.
//!
//! A step is what the decoder would send (section 3.2): the tick, the
//! input events the input script places on it, and the reports. Its
//! batch is applied to the page ([`Screen`]), its sound operations to the
//! mixer ([`Speaker`]), its log lines to the log, and what the scripts
//! asked of the network goes to the [`Network`], whose answers are given
//! before the next step.

use core::fmt::Write as _;
use core::time::Duration;
use std::time::Instant;

use neoteletext_script::{Engine, InProcess, LinkError, ScriptLink, Settings, TimeLimits, Watchdog};
use protocol::script::batch::Batch;
use protocol::script::{
    ContextFlags, ContextId, ContextOpen, Event, EventKind, Level, MAX_CYCLE_ENDS, MAX_STEP_EVENTS, MAX_STEP_FAULTS, MAX_STEP_SOUND_REPORTS,
    MAX_TRACKED_DRAWINGS, MAX_WATCHED_PAINTINGS, Renderer, SEED_BYTES, Step, StepFlags,
};

use crate::args::Options;
use crate::blobs::{Blob, find};
use crate::files::{CHANNEL_FILE, CHANNEL_SCRIPT, CHANNEL_SCRIPT_NAME, FROM_DISK, INPUT, PAGE_SCRIPT, PAGE_SCRIPT_NAME, PAGE_T42, PAGE_TTI, channel};
use crate::input::{InputScript, Item};
use crate::limits::{DEFAULT_PAGE, FIXED_EPOCH_SECONDS, MAX_RUN_LOG_BYTES};
use crate::manifest::{self, Manifest, Pointer};
use crate::network::Network;
use crate::report::{Budget, Report, Times, budget_file, sha256};
use crate::screen::{Place, Screen, StandIn};
use crate::sound::Speaker;
use crate::{Failure, Status};

/// The kept log.
pub const LOG_FILE: &str = "log.txt";
/// The kept budget.
pub const BUDGET_FILE: &str = "budget.json";
/// The kept sound.
pub const SOUND_FILE: &str = "sound.wav";
/// The report, as JSON and as text: never kept, since they hold times.
pub const REPORT_JSON: &str = "report.json";
/// See [`REPORT_JSON`].
pub const REPORT_TEXT: &str = "report.txt";
/// A recording the network made.
pub const RECORDING: &str = "network.recording";
/// The run's exit status, one ASCII digit.
pub const STATUS: &str = "status";
/// Why a run failed, for standard error.
pub const MESSAGE: &str = "message";

/// The name of the snapshot taken after `tick`'s step.
#[must_use]
pub fn snapshot_name(tick: u64) -> String {
    format!("tick-{tick:06}.png")
}

/// Whether `name` is a file a run keeps: one that `--expect` compares.
#[must_use]
pub fn is_kept(name: &str) -> bool {
    let snapshot = name
        .strip_prefix("tick-")
        .and_then(|rest| rest.strip_suffix(".png"))
        .is_some_and(|digits| digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_digit()));
    snapshot || matches!(name, LOG_FILE | BUDGET_FILE | SOUND_FILE)
}

/// What a run gives: its report and its files.
#[derive(Debug)]
pub struct Outcome {
    /// The report.
    pub report: Report,
    /// The files: those kept ([`is_kept`]), the two reports and a
    /// recording.
    pub files: Vec<Blob>,
}

const fn renderer_name(renderer: Renderer) -> &'static str {
    match renderer {
        Renderer::Teletext => "teletext",
        Renderer::Regis => "regis",
        Renderer::Naplps => "naplps",
    }
}

const fn context_name(context: ContextId) -> &'static str {
    match context {
        ContextId::Channel => "channel",
        ContextId::Page => "page",
    }
}

/// The names of the log's levels, 0 to 4, as `console` names them.
const LEVEL_NAMES: [&str; 5] = ["debug", "log", "info", "warn", "error"];

/// A context's seed: SHA-256 of the run's seed text, a NUL and the
/// context's code, so that the two contexts' random numbers differ and
/// both follow `--seed` alone.
fn seed(text: &str, context: ContextId) -> [u8; SEED_BYTES] {
    let mut hashed = ring::digest::Context::new(&ring::digest::SHA256);
    hashed.update(text.as_bytes());
    hashed.update(&[0, context.code()]);
    let mut seed = [0_u8; SEED_BYTES];
    for (to, from) in seed.iter_mut().zip(hashed.finish().as_ref()) {
        *to = *from;
    }
    seed
}

/// The run's log: lines kept to a bound, the rest counted.
#[derive(Default)]
struct Log {
    text: String,
    lines: u64,
    dropped: u64,
}

impl Log {
    fn line(&mut self, tick: u64, who: &str, level: &str, text: &str) {
        if self.text.len().saturating_add(text.len()) > MAX_RUN_LOG_BYTES {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.lines = self.lines.saturating_add(1);
        let _ = writeln!(self.text, "{tick:06} {who} {level} {text}");
    }

    fn runner(&mut self, tick: u64, text: &str) {
        self.line(tick, "runner", "note", text);
    }
}

/// What the files given say the run is of.
struct Subject {
    place: Place,
    channel_title: String,
    page_file: Option<(&'static str, Vec<u8>)>,
    from_disk: bool,
}

fn subject(options: &Options, blobs: &[Blob]) -> Result<Subject, Failure> {
    let read = match find(blobs, CHANNEL_FILE) {
        Some(bytes) => Some(channel(bytes, &options.channel_url)?),
        None => None,
    };
    let page = options.page.map(|(page, _)| page).or(read.as_ref().map(|read| read.initial_page)).unwrap_or(DEFAULT_PAGE);
    let subcode = options.page.and_then(|(_, subcode)| subcode);
    let page_file = [(PAGE_TTI, "tti"), (PAGE_T42, "t42")].into_iter().find_map(|(name, extension)| find(blobs, name).map(|bytes| (extension, bytes.to_vec())));
    Ok(Subject {
        // With a page file and no sub-page named, the first the file
        // holds; a JavaScript page has the sub-code 0, no sub-pages.
        place: Place { page, subcode: subcode.unwrap_or(if page_file.is_some() { neoteletext_core::limits::SUBCODE_ANY } else { 0 }) },
        channel_title: read.map(|read| read.title).unwrap_or_default(),
        page_file,
        from_disk: find(blobs, FROM_DISK).is_some(),
    })
}

/// A context's opening, from its manifest and the run.
#[expect(clippy::too_many_arguments, reason = "the opening's sources, each needed and none another's")]
fn opening(
    context: ContextId,
    manifest: &Manifest,
    options: &Options,
    subject: &Subject,
    place: Place,
    renderer: Renderer,
    script_name: &str,
    sounds: &[u8],
    source: &[u8],
) -> ContextOpen {
    let page_name = subject.page_file.as_ref().map_or_else(|| script_name.to_owned(), |(extension, _)| format!("{:03X}.{extension}", place.page));
    ContextOpen {
        context,
        api: manifest.api,
        renderer,
        // The decoder's profile draws at Level 2.5 with the extensions
        // the browser turns on; the script library's own tests open
        // contexts so.
        level: Level::TwoFive,
        page: place.page,
        subcode: place.subcode,
        flags: ContextFlags {
            pointer_press: manifest.pointer != Pointer::None,
            pointer_move: manifest.pointer == Pointer::Move,
            wheel: manifest.wheel,
            text: manifest.text,
            // A page opened from disk has no network (section 12.7).
            network: manifest.network && !subject.from_disk,
            from_disk: subject.from_disk,
            sound_allowed: options.sound_allowed,
        },
        tick_divisor: manifest.tick,
        language: manifest.language.clone().unwrap_or_else(|| "en".to_owned()),
        channel_url: options.channel_url.clone(),
        page_url: format!("{}{page_name}", options.channel_url),
        script_url: format!("{}{script_name}", options.channel_url),
        channel_title: subject.channel_title.clone(),
        seed: seed(&options.seed, context),
        epoch_seconds: FIXED_EPOCH_SECONDS,
        zone_minutes: 0,
        keys: manifest.keys.clone(),
        sounds: sounds.to_vec(),
        source_bytes: u32::try_from(source.len()).unwrap_or(u32::MAX),
    }
}

/// The step of `tick` from the input script's items; gives what did not
/// fit and was dropped.
fn step_of(sequence: u32, page_tick: u64, items: &[&Item], first: Option<Event>, sound_allowed: bool) -> (Step, u64, bool) {
    let mut step = Step { sequence, tick: page_tick, flags: StepFlags { tick: true, sound_allowed, ..StepFlags::default() }, ..Step::default() };
    let (mut dropped, mut closing) = (0_u64, false);
    step.events.extend(first);
    for item in items {
        // Each list is held to the bound its message has; more than a
        // step carries is dropped and counted, as a full queue's are.
        let fits = match item {
            Item::Event(event) => (step.events.len() < MAX_STEP_EVENTS).then(|| step.events.push(event.clone())),
            Item::Hide => {
                step.flags.pausing = true;
                Some(())
            }
            Item::Show => {
                step.flags.resumed = true;
                Some(())
            }
            Item::Close => {
                closing = true;
                Some(())
            }
            Item::Painted(handle) => (step.painted.len() < MAX_WATCHED_PAINTINGS).then(|| step.painted.push(*handle)),
            Item::CycleEnded(handle) => (step.cycle_ended.len() < MAX_CYCLE_ENDS).then(|| step.cycle_ended.push(*handle)),
            Item::Sound(report) => (step.sound.len() < MAX_STEP_SOUND_REPORTS).then(|| step.sound.push(*report)),
            Item::Fault(fault) => (step.faults.len() < MAX_STEP_FAULTS).then(|| step.faults.push(fault.clone())),
            Item::Bounds(bounds) => (step.bounds.len() < MAX_TRACKED_DRAWINGS).then(|| step.bounds.push(*bounds)),
        };
        if fits.is_none() {
            dropped = dropped.saturating_add(1);
        }
    }
    step.flags.closing = closing;
    (step, dropped, closing)
}

/// Everything a run holds while it steps.
struct Running {
    link: InProcess,
    screen: Screen,
    speaker: Speaker,
    log: Log,
    budgets: Vec<(ContextId, Budget)>,
    stopped: Option<String>,
}

impl Running {
    /// Takes a batch in: the page, the mixer, the log.
    fn take(&mut self, tick: u64, batch: &Batch) {
        self.screen.apply(batch);
        for audio in self.screen.take_sounds() {
            self.speaker.page(audio);
        }
        for operation in batch.sound.iter().flatten() {
            self.speaker.script(*operation);
        }
        if let Some(verdicts) = &batch.verdicts {
            // One letter an event: C its default was cancelled, h a
            // listener heard it, - neither.
            let letters: String = verdicts
                .iter()
                .map(|verdict| {
                    if verdict.cancelled {
                        'C'
                    } else if verdict.handled {
                        'h'
                    } else {
                        '-'
                    }
                })
                .collect();
            self.log.runner(tick, &format!("verdicts {letters}"));
        }
        for line in batch.log.iter().flatten() {
            let level = LEVEL_NAMES.get(usize::from(line.level)).copied().unwrap_or("log");
            self.log.line(tick, context_name(line.context), level, &line.text);
        }
        if let Some(title) = &batch.title {
            self.log.runner(tick, &format!("title {title}"));
        }
        if let Some(target) = &batch.navigate {
            // The runner shows one page: where a script asked to go is
            // logged, and the page stays.
            self.log.runner(tick, &format!("navigate {target}"));
        }
        for state in batch.state.iter().flatten() {
            if let Some(reason) = state.stopped
                && self.stopped.is_none()
            {
                self.stopped = Some(format!("the {} context was stopped: {reason:?}", context_name(state.context)));
            }
        }
        for (context, budget) in &mut self.budgets {
            if let Some(container) = self.link.container()
                && container.is_open(*context)
            {
                budget.observe(&container.budget(*context));
            }
        }
    }
}

/// Opens one context; a refusal is status 3, a stop status 2.
fn open(running: &mut Running, open: ContextOpen, source: &[u8]) -> Result<(), Failure> {
    let context = open.context;
    running.link.open(open, source).map_err(|error| match error {
        LinkError::Stopped(..) => Failure { status: Status::Stopped, message: format!("the {} script: {error}", context_name(context)) },
        LinkError::Refused(_) | LinkError::Context(_) => Failure::refused(format!("the {} script: {error}", context_name(context))),
    })?;
    running.budgets.push((context, Budget::default()));
    Ok(())
}

/// Runs what `options` asks for on the files `blobs` hold.
///
/// # Errors
/// A run that could not begin: status 4 for a file the engine or a
/// reader refuses, status 3 for a script or manifest refused, status 2
/// for a script stopped at its top level. A run that began always gives
/// its outcome, whose report says how it ended.
#[expect(clippy::too_many_lines, reason = "the run from its opening to its files, in the order things happen")]
pub fn run(options: &Options, blobs: &[Blob], mut network: Box<dyn Network>) -> Result<Outcome, Failure> {
    let began = Instant::now();
    let subject = subject(options, blobs)?;
    let input = match find(blobs, INPUT) {
        Some(bytes) => crate::input::parse(bytes).map_err(Failure::usage)?,
        None => InputScript::default(),
    };
    let page_source = find(blobs, PAGE_SCRIPT);
    let channel_source = find(blobs, CHANNEL_SCRIPT);
    let read_manifest = |source: Option<&[u8]>, which: &str| {
        source.map(|source| manifest::read(source).map_err(|why| Failure::refused(format!("the {which} script: {why}")))).transpose()
    };
    let page_manifest = read_manifest(page_source, "page")?;
    let channel_manifest = read_manifest(channel_source, "channel")?;
    if channel_manifest.as_ref().is_some_and(|manifest| manifest.renderer.is_some()) {
        return Err(Failure::refused("the channel script: a channel-level script's manifest has no `renderer` (section 3.1)"));
    }

    // The page, in the decoder's engine.
    let mut screen = Screen::new(subject.place, Box::new(StandIn::default()));
    let renderer = if let Some((_, bytes)) = &subject.page_file {
        screen.page_file(bytes);
        screen.settle_subpage();
        let of_file = screen.renderer();
        if let Some(asked) = page_manifest.as_ref().and_then(|manifest| manifest.renderer)
            && asked != of_file
        {
            return Err(Failure::refused(format!(
                "the page script: its manifest's renderer is {}, and its page file is drawn by {} (section 3.1)",
                renderer_name(asked),
                renderer_name(of_file)
            )));
        }
        of_file
    } else {
        let renderer = page_manifest.as_ref().and_then(|manifest| manifest.renderer).unwrap_or(Renderer::Teletext);
        screen.blank(renderer);
        renderer
    };
    let place =
        Place { page: subject.place.page, subcode: if screen.place().subcode == neoteletext_core::limits::SUBCODE_ANY { 0 } else { screen.place().subcode } };
    let mut speaker = Speaker::new();
    for audio in screen.take_sounds() {
        speaker.page(audio);
    }
    let sounds = speaker.handles();

    // The scripts, through the one link.
    let engine = Engine::new().map_err(|stop| Failure { status: Status::Stopped, message: format!("the script engine could not start: {stop}") })?;
    let settings = if options.strict_time {
        // Behind every entry, as in the browser's script host (section
        // 3.2 rule 5b): a callback that does not return ends this process
        // with the status of a stopped script.
        let watchdog = Watchdog::start(|| std::process::exit(i32::from(Status::Stopped.code())))
            .map_err(|error| Failure::usage(format!("the watchdog thread could not start: {error}")))?;
        Settings { time: TimeLimits::default(), watchdog: Some(watchdog) }
    } else {
        Settings { time: TimeLimits::unhurried(), watchdog: None }
    };
    let mut running = Running { link: InProcess::new(engine, settings), screen, speaker, log: Log::default(), budgets: Vec::new(), stopped: None };
    let name_of = |blob: &str, fallback: &str| find(blobs, blob).and_then(|name| core::str::from_utf8(name).ok()).unwrap_or(fallback).to_owned();
    // The channel context first, then the page's (section 3.1).
    if let (Some(source), Some(manifest)) = (channel_source, &channel_manifest) {
        let name = name_of(CHANNEL_SCRIPT_NAME, "channel.js");
        open(&mut running, opening(ContextId::Channel, manifest, options, &subject, place, renderer, &name, &sounds, source), source)?;
    }
    if let (Some(source), Some(manifest)) = (page_source, &page_manifest) {
        let name = name_of(PAGE_SCRIPT_NAME, "page.js");
        open(&mut running, opening(ContextId::Page, manifest, options, &subject, place, renderer, &name, &sounds, source), source)?;
    }
    let start_micros = u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX);

    // The channel context is told the page is shown, in its first step.
    let mut first_event = channel_source.map(|_| Event {
        kind: EventKind::PageShow,
        numbers: [i32::from(place.page), i32::from(place.subcode), i32::from(renderer.code()), 0],
        payload: format!("{}{:03X}", options.channel_url, place.page).into_bytes(),
    });
    let snapshot_ticks = options.snapshot_ticks();
    let mut files: Vec<Blob> = Vec::new();
    let (mut times, mut events, mut dropped) = (Vec::new(), 0_u64, 0_u64);
    let (mut sound_allowed, mut hidden, mut page_tick, mut sequence, mut ticks_run) = (options.sound_allowed, false, 0_u64, 0_u32, 0_u64);
    let paced = Instant::now();
    for tick in 0..options.ticks {
        if options.pace
            && let Some(due) = paced.checked_add(Duration::from_micros(tick.saturating_mul(crate::limits::MICROSECONDS_PER_TICK)))
        {
            // Waiting for the tick's own moment, and no longer.
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
        ticks_run = tick.saturating_add(1);
        running.screen.set_tick(tick);
        let items: Vec<&Item> = input.at(tick).collect();
        sound_allowed |= items.iter().any(|item| item.is_press());
        let shown_again = items.iter().any(|item| matches!(item, Item::Show));
        let mut closing = false;
        // A hidden tab's scripts are paused: no step, no tick, no timer,
        // and the page's tick count stands still (section 3.2).
        if !hidden || shown_again {
            for answer in network.answers(tick) {
                running.link.network(answer);
            }
            let (step, lost, closes) = step_of(sequence, page_tick, &items, first_event.take(), sound_allowed);
            closing = closes;
            dropped = dropped.saturating_add(lost);
            events = events.saturating_add(u64::try_from(step.events.len()).unwrap_or(u64::MAX));
            hidden = step.flags.pausing;
            let stepped = Instant::now();
            let answered = running.link.step(step);
            times.push(stepped.elapsed());
            sequence = sequence.wrapping_add(1);
            page_tick = page_tick.saturating_add(1);
            match answered {
                Ok(batch) => running.take(tick, &batch),
                Err(error) => {
                    running.stopped.get_or_insert_with(|| error.to_string());
                    running.log.runner(tick, &format!("stopped: {error}"));
                }
            }
            for message in running.link.take_requests() {
                network.request(tick, message);
            }
        } else {
            dropped = dropped.saturating_add(u64::try_from(items.len()).unwrap_or(u64::MAX));
        }
        running.speaker.tick();
        if snapshot_ticks.binary_search(&tick).is_ok() {
            match running.screen.snapshot() {
                Ok(png) => files.push((snapshot_name(tick), png)),
                Err(why) => running.log.runner(tick, &format!("no snapshot: {why}")),
            }
        }
        if closing || running.stopped.is_some() {
            break;
        }
    }
    // The contexts end: the page's, then the channel's, each with its
    // one handling of `close`; what that staged is the last batch.
    let memory = running.link.container().map_or((0, 0), |container| container.memory());
    for context in [ContextId::Page, ContextId::Channel] {
        running.link.close(context);
        let staged = match running.link.container() {
            Some(container) => Some(container.drain(sequence)),
            None => running.link.take_closing(sequence),
        };
        if let Some(batch) = staged {
            running.take(ticks_run, &batch);
        }
    }

    let status = if running.stopped.is_some() { Status::Stopped } else { Status::Matched };
    let budgets: Vec<(&'static str, Budget)> = running.budgets.iter().map(|(context, budget)| (context_name(*context), budget.clone())).collect();
    files.push((LOG_FILE.to_owned(), core::mem::take(&mut running.log.text).into_bytes()));
    files.push((BUDGET_FILE.to_owned(), budget_file(&budgets)));
    files.push((SOUND_FILE.to_owned(), running.speaker.wav()));
    if let Some(recording) = network.recording() {
        files.push((RECORDING.to_owned(), recording));
    }
    let report = Report {
        status,
        message: running.stopped.clone().unwrap_or_default(),
        seed: options.seed.clone(),
        ticks: ticks_run,
        renderer: renderer_name(renderer),
        times: Times::of(&mut times),
        start_micros,
        output_bytes: running.screen.output_bytes,
        left_out: running.screen.left_out.clone(),
        network: network.counts(),
        deterministic: network.deterministic(),
        events,
        events_dropped: dropped,
        log_lines: running.log.lines,
        log_dropped: running.log.dropped,
        sound_operations: running.speaker.operations,
        sound_peak: running.speaker.peak,
        memory,
        budgets,
        files: files.iter().filter(|(name, _)| is_kept(name)).map(|(name, bytes)| (name.clone(), sha256(bytes))).collect(),
    };
    files.push((REPORT_JSON.to_owned(), report.json().into_bytes()));
    files.push((REPORT_TEXT.to_owned(), report.text().into_bytes()));
    Ok(Outcome { report, files })
}
