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
use crate::files::{
    CHANNEL_FILE, CHANNEL_SCRIPT, Channel, FOLDER_SCRIPT, FROM_DISK, INPUT, PAGE_FILE_STEM, PAGE_SCRIPT, PAGE_SCRIPT_NAME, PAGE_T42, PAGE_TTI, channel,
};
use crate::input::{InputScript, Item, POINTER_ACTION_MASK};
use crate::limits::{DEFAULT_PAGE, FIXED_EPOCH_SECONDS, MAX_RUN_LOG_BYTES};
use crate::network::Network;
use crate::report::{Budget, Report, Times, budget_file, sha256};
use crate::screen::{DEFAULT_LANGUAGE, Place, Screen};
use crate::sound::Speaker;
use crate::{Failure, Status};
use decoder::scripts::manifest::{self, Manifest};
use neoteletext_script::limits::MAX_POINTER_OBJECTS;
use neoteletext_script::ops::PointerAction;
use protocol::decoder::script::PointerClaim;

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

/// The bytes of a pointer event's payload before the things under it:
/// the pointer's id, the buttons held and its place within the thing,
/// four whole numbers.
const POINTER_FIXED_BYTES: usize = 4 * 4;

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

/// How an error's stack names a place in the script library's prelude,
/// before the line and column.
const PRELUDE_PLACE: &str = "(nt:prelude:";

/// A log line with each place in the prelude (`(nt:prelude:LINE:COLUMN)`
/// in an error's stack) written without its line and column. Those are
/// the library's own and move with every change to the prelude; a kept
/// log is to change when its page's behaviour does. A place in the
/// page's own script is left whole.
fn without_prelude_places(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once(PRELUDE_PLACE) {
        out.push_str(before);
        out.push_str("(nt:prelude");
        // Past the line and the column, to the closing bracket.
        let numbers = after.find(|character: char| !(character.is_ascii_digit() || character == ':')).unwrap_or(after.len());
        rest = after.get(numbers..).unwrap_or_default();
    }
    out.push_str(rest);
    out
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
        let text = &without_prelude_places(text);
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
    channel: Option<Channel>,
    page_file: Option<Vec<u8>>,
    from_disk: bool,
}

fn subject(options: &Options, blobs: &[Blob]) -> Result<Subject, Failure> {
    let read = match find(blobs, CHANNEL_FILE) {
        Some(bytes) => Some(channel(bytes, &options.channel_url)?),
        None => None,
    };
    // A page file opened by itself is the page its name says.
    let named = find(blobs, PAGE_FILE_STEM)
        .and_then(|stem| core::str::from_utf8(stem).ok())
        .and_then(|stem| u16::from_str_radix(stem, 16).ok())
        .filter(|page| neoteletext_core::types::PageAddress::from_mpp(*page).is_some());
    let page = options.page.map(|(page, _)| page).or(named).or(read.as_ref().map(|read| read.initial_page)).unwrap_or(DEFAULT_PAGE);
    let subcode = options.page.and_then(|(_, subcode)| subcode);
    let page_file = [PAGE_TTI, PAGE_T42].into_iter().find_map(|name| find(blobs, name).map(<[u8]>::to_vec));
    Ok(Subject {
        // With a page file and no sub-page named, the first the file
        // holds; a JavaScript page has the sub-code 0, no sub-pages.
        place: Place { page, subcode: subcode.unwrap_or(if page_file.is_some() { neoteletext_core::limits::SUBCODE_ANY } else { 0 }) },
        channel: read,
        page_file,
        from_disk: find(blobs, FROM_DISK).is_some(),
    })
}

/// A script to run: its file name and its bytes.
type Script<'a> = (String, &'a [u8]);

/// The scripts of the page shown, the page-level one and the
/// channel-level one, as the engine's readers name them (section 12):
/// the page file's own reference (`in_page_file`, its `JS` line or
/// `SCRIPT_REF` record as the decoder read it) before the channel file's
/// entry for exactly that page and sub-code, by the engine's rule
/// (`neoteletext_web::scripts::chosen`); and the channel file's
/// `scripts.channel`. Each is a file of the channel's folder. A run
/// whose target is one script has that script for its page.
fn scripts<'a>(subject: &Subject, place: Place, in_page_file: Option<&str>, blobs: &'a [Blob]) -> Result<(Option<Script<'a>>, Option<Script<'a>>), Failure> {
    use neoteletext_core::types::ScriptName;
    let direct = |blob: &str, name_blob: &str, fallback: &str| {
        find(blobs, blob).map(|source| (find(blobs, name_blob).and_then(|name| core::str::from_utf8(name).ok()).unwrap_or(fallback).to_owned(), source))
    };
    let named = |name: &str| -> Result<Script<'a>, Failure> {
        find(blobs, &format!("{FOLDER_SCRIPT}{name}"))
            .map(|source| (name.to_owned(), source))
            .ok_or_else(|| Failure::usage(format!("{name}: the channel names this script, and its folder holds no such file")))
    };
    let parsed = |name: Option<&str>| name.and_then(|name| ScriptName::parse(name.as_bytes()));
    let in_channel_file = subject.channel.as_ref().and_then(|channel| channel.page_script(place.page, place.subcode));
    let chosen = neoteletext_web::scripts::chosen(parsed(in_page_file), parsed(in_channel_file));
    let page = match direct(PAGE_SCRIPT, PAGE_SCRIPT_NAME, "page.js") {
        Some(script) => Some(script),
        None => chosen.map(|name| named(name.as_str())).transpose()?,
    };
    let channel = match direct(CHANNEL_SCRIPT, "", "channel.js") {
        Some(script) => Some(script),
        None => subject.channel.as_ref().and_then(|channel| channel.channel_script.as_deref()).map(named).transpose()?,
    };
    Ok((page, channel))
}

/// A page's address, as the decoder says it: its channel's with its
/// number, and its sub-code when it has sub-pages.
fn page_url(channel_url: &str, place: Place) -> String {
    if place.subcode == 0 { format!("{channel_url}{:03X}", place.page) } else { format!("{channel_url}{:03X}/{:04X}", place.page, place.subcode) }
}

/// A context's opening, from its manifest and the run, as the decoder
/// makes one (`decoder::scripts`).
#[expect(clippy::too_many_arguments, reason = "the opening's sources, each needed and none another's")]
fn opening(
    context: ContextId,
    manifest: &Manifest,
    options: &Options,
    subject: &Subject,
    place: Place,
    renderer: Renderer,
    level: Level,
    script: &Script<'_>,
) -> ContextOpen {
    let (script_name, source) = script;
    ContextOpen {
        context,
        api: manifest.api,
        renderer,
        level,
        page: place.page,
        subcode: place.subcode,
        flags: ContextFlags {
            pointer_press: manifest.pointer >= PointerClaim::Press,
            pointer_move: manifest.pointer >= PointerClaim::Move,
            wheel: manifest.wheel,
            text: manifest.text,
            network: manifest.network,
            from_disk: subject.from_disk,
            sound_allowed: options.sound_allowed,
        },
        tick_divisor: manifest.tick,
        language: manifest.language.clone().unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned()),
        channel_url: options.channel_url.clone(),
        page_url: page_url(&options.channel_url, place),
        script_url: format!("{}{script_name}", options.channel_url),
        channel_title: subject.channel.as_ref().map(|channel| channel.title.clone()).unwrap_or_default(),
        seed: seed(&options.seed, context),
        epoch_seconds: FIXED_EPOCH_SECONDS,
        zone_minutes: 0,
        keys: manifest.keys.clone(),
        // The decoder names no sounds in an opening.
        sounds: Vec::new(),
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
            // The pause is a step of its own, after this one.
            Item::Hide => Some(()),
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
    /// An input script's item as the step carries it: a pointer line that
    /// names no thing is given the things under it by the decoder's own
    /// picking, topmost first, as the browser's decoder gives them
    /// (section 4.4).
    fn picked(&self, item: &Item) -> Item {
        let Item::Event(event) = item else { return item.clone() };
        let [first, x, y, thing] = event.numbers;
        let action = first & POINTER_ACTION_MASK;
        let placed = [PointerAction::DOWN, PointerAction::UP, PointerAction::MOVE].iter().any(|one| one.number() == action);
        if event.kind != EventKind::Pointer || thing != 0 || !placed || event.payload.len() > POINTER_FIXED_BYTES {
            return item.clone();
        }
        let things = self.screen.pick(x, y);
        let Some((hit, others)) = things.split_first() else { return item.clone() };
        let mut event = event.clone();
        event.numbers = [first, x, y, *hit];
        event.payload.extend(others.iter().take(MAX_POINTER_OBJECTS).flat_map(|other| other.to_be_bytes()));
        Item::Event(event)
    }

    /// Takes a batch in: the page, the mixer, the log.
    fn take(&mut self, tick: u64, batch: &Batch, draws: bool) {
        // The decoder's side of the step: a batch it refuses, or a rate
        // passed, fails the scripts as it does in the browser.
        if let Err(why) = self.screen.apply(tick, batch, draws) {
            self.log.runner(tick, &format!("stopped: {why}"));
            self.stopped.get_or_insert(why);
        }
        for audio in self.screen.take_sounds() {
            self.speaker.voice(audio);
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
    // The page file, in the decoder's engine, read by its readers.
    let mut screen = Screen::new(subject.place);
    if let Some(bytes) = &subject.page_file {
        screen.page_file(bytes);
    }
    screen.settle_subpage();
    let place = screen.place();
    let in_page_file =
        screen.script_names().into_iter().rev().find(|(page, subcode, _)| (*page, *subcode) == (place.page, place.subcode)).and_then(|(_, _, name)| name);
    if subject.channel.as_ref().is_some_and(|channel| channel.scripts_ignored) {
        return Err(Failure::usage(
            "nothing to run: the channel is live (its channel.ttx has the member `push`), and a live channel runs no script (section 12.1)",
        ));
    }
    let (page_script, channel_script) = scripts(&subject, place, in_page_file.as_deref(), blobs)?;
    if page_script.is_none() && channel_script.is_none() {
        return Err(Failure::usage(format!(
            "nothing to run: page {:03X} has no script. Give a script (.js), or name the page's in the channel file's `scripts` or in its page file",
            place.page
        )));
    }
    // A manifest is read by the decoder's own reader, before any code runs.
    let read_manifest = |script: Option<&Script<'_>>, which: &str| {
        let read = |(name, source): &Script<'_>| {
            let manifest = manifest::read(source).map_err(|why| Failure::refused(format!("the {which} script {name} was not run: {why}")))?;
            if manifest.api > neoteletext_script::API_VERSION {
                return Err(Failure::refused(format!(
                    "the {which} script {name} was not run: its manifest names a later version of the API than this runner has"
                )));
            }
            Ok(manifest)
        };
        script.map(read).transpose()
    };
    let page_manifest = read_manifest(page_script.as_ref(), "page")?;
    let channel_manifest = read_manifest(channel_script.as_ref(), "channel")?;
    // Section 3.1, as the decoder decides it: for a script attached to a
    // page file the manifest's renderer equals the page file's or is
    // absent; a JavaScript page draws on the renderer its manifest names.
    let held = screen.held_renderer();
    let named = page_manifest.as_ref().and_then(|manifest| manifest.renderer);
    if let (Some(named), Some(held)) = (named, held)
        && named != held
    {
        return Err(Failure::refused(format!(
            "the page script: its manifest names another renderer ({}) than its page file's ({})",
            renderer_name(named),
            renderer_name(held)
        )));
    }
    let renderer = held.or(named).unwrap_or(Renderer::Teletext);
    let level = screen.level();
    let mut speaker = Speaker::new();
    for audio in screen.take_sounds() {
        speaker.voice(audio);
    }

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
    // The channel context first, then the page's (section 3.1); the
    // channel context is told the renderer of the page as the decoder
    // then holds it.
    if let (Some(script), Some(manifest)) = (&channel_script, &channel_manifest) {
        let told = held.unwrap_or(Renderer::Teletext);
        open(&mut running, opening(ContextId::Channel, manifest, options, &subject, place, told, level, script), script.1)?;
    }
    if let (Some(script), Some(manifest)) = (&page_script, &page_manifest) {
        open(&mut running, opening(ContextId::Page, manifest, options, &subject, place, renderer, level, script), script.1)?;
    }
    let language = page_manifest.as_ref().and_then(|manifest| manifest.language.clone()).unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned());
    running.screen.open(renderer, &language);
    let channel_source = channel_script.as_ref();
    let start_micros = u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX);

    // The channel context is told the page is shown, in its first step.
    let mut first_event = channel_source.map(|_| Event {
        kind: EventKind::PageShow,
        numbers: [i32::from(place.page), i32::from(place.subcode), i32::from(renderer.code()), 0],
        payload: page_url(&options.channel_url, place).into_bytes(),
    });
    let snapshot_ticks = options.snapshot_ticks();
    let mut files: Vec<Blob> = Vec::new();
    let (mut times, mut events, mut dropped) = (Vec::new(), 0_u64, 0_u64);
    let (mut sound_allowed, mut hidden, mut page_tick, mut sequence, mut ticks_run) = (options.sound_allowed, false, 0_u64, 0_u32, 0_u64);
    // The keys the scripts were given as down and not yet as up.
    let mut held_keys: Vec<String> = Vec::new();
    let paced = Instant::now();
    for tick in 0..options.ticks {
        if (options.pace || network.real_time())
            && let Some(due) = paced.checked_add(Duration::from_micros(tick.saturating_mul(crate::limits::MICROSECONDS_PER_TICK)))
        {
            // Waiting for the tick's own moment, and no longer.
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
        ticks_run = tick.saturating_add(1);
        running.screen.set_tick(tick);
        let picked: Vec<Item> = input.at(tick).map(|item| running.picked(item)).collect();
        let items: Vec<&Item> = picked.iter().collect();
        sound_allowed |= items.iter().any(|item| item.is_press());
        let shown_again = items.iter().any(|item| matches!(item, Item::Show));
        let mut closing = false;
        // A hidden tab's scripts are paused: no step, no tick, no timer,
        // and the page's tick count stands still (section 3.2).
        if !hidden || shown_again {
            for answer in network.answers(tick) {
                running.link.network(answer);
            }
            let (mut step, lost, closes) = step_of(sequence, page_tick, &items, first_event.take(), sound_allowed);
            // The decoder's report of the watched drawings that became
            // whole, beside any the input script gives.
            for handle in running.screen.painted() {
                if step.painted.len() < MAX_WATCHED_PAINTINGS && !step.painted.contains(&handle) {
                    step.painted.push(handle);
                }
            }
            closing = closes;
            dropped = dropped.saturating_add(lost);
            events = events.saturating_add(u64::try_from(step.events.len()).unwrap_or(u64::MAX));
            let hides = items.iter().any(|item| matches!(item, Item::Hide));
            for event in &step.events {
                let name = String::from_utf8_lossy(&event.payload).into_owned();
                match event.kind {
                    EventKind::KeyDown if !held_keys.contains(&name) => held_keys.push(name),
                    EventKind::KeyUp => held_keys.retain(|held| *held != name),
                    _ => {}
                }
            }
            let stepped = Instant::now();
            let answered = running.link.step(step);
            times.push(stepped.elapsed());
            sequence = sequence.wrapping_add(1);
            page_tick = page_tick.saturating_add(1);
            match answered {
                Ok(batch) => running.take(tick, &batch, true),
                Err(error) => {
                    running.stopped.get_or_insert_with(|| error.to_string());
                    running.log.runner(tick, &format!("stopped: {error}"));
                }
            }
            // The tab is hidden: as the decoder does it, one more step
            // that is no tick, the last before the pause, carrying the
            // release of every key held, marked as not seen (section 4.7).
            if hides && running.stopped.is_none() {
                let releases: Vec<Event> = held_keys.drain(..).take(MAX_STEP_EVENTS).map(|name| decoder::scripts::events::lost_release(&name)).collect();
                events = events.saturating_add(u64::try_from(releases.len()).unwrap_or(u64::MAX));
                let pause = Step {
                    sequence,
                    tick: page_tick.saturating_sub(1),
                    flags: StepFlags { pausing: true, sound_allowed, ..StepFlags::default() },
                    events: releases,
                    ..Step::default()
                };
                sequence = sequence.wrapping_add(1);
                match running.link.step(pause) {
                    Ok(batch) => running.take(tick, &batch, true),
                    Err(error) => {
                        running.stopped.get_or_insert_with(|| error.to_string());
                        running.log.runner(tick, &format!("stopped: {error}"));
                    }
                }
                hidden = true;
            } else if shown_again {
                hidden = false;
            }
            for message in running.link.take_requests() {
                network.request(tick, message);
            }
        } else {
            dropped = dropped.saturating_add(u64::try_from(items.len()).unwrap_or(u64::MAX));
        }
        running.screen.paint();
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
            // A close's batch draws nothing; its sound stops are carried out.
            running.take(ticks_run, &batch, false);
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
