//! What the script library's tests share: one compiled engine, contexts
//! opened with plain settings, and steps.
#![allow(dead_code, reason = "each test file uses its own part of this")]

use std::sync::OnceLock;

use neoteletext_script::{Container, Engine, LinkError, Settings, TimeLimits};
use protocol::script::batch::{ContextState, LogLine, SoundOperation};
use protocol::script::{
    Batch, ContextFlags, ContextId, ContextOpen, Event, EventKind, FromScriptHost, Level, Renderer, Step, StepFlags, read_to_decoder, write_to_decoder,
};

/// The seed the tests give a context unless they give their own.
pub const SEED: [u8; 32] = [42; 32];
/// The channel the tests' contexts belong to.
pub const CHANNEL: &str = "https://example.test/chan/";

/// The engine, compiled once for the test program.
pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let engine = Engine::new().expect("the module compiles");
        // As the script host does when it starts: the prelude's bytecode
        // is made before any context opens, so no test's timing of an
        // opening holds the one compilation.
        engine.prelude(neoteletext_script::ops::Parts::ALL).expect("the prelude compiles");
        engine
    })
}

/// A context's opening for `context` with nothing claimed. Its script's
/// length is set when it is opened ([`Opens::open_with`]).
pub fn opening(context: ContextId) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer: Renderer::Teletext,
        level: Level::TwoFive,
        page: 0x100,
        subcode: 0,
        flags: ContextFlags { sound_allowed: true, ..ContextFlags::default() },
        tick_divisor: 0,
        language: "en".into(),
        channel_url: CHANNEL.into(),
        page_url: format!("{CHANNEL}100.tti"),
        script_url: format!("{CHANNEL}{}.js", if context == ContextId::Page { "page" } else { "channel" }),
        channel_title: "Example".into(),
        seed: SEED,
        epoch_seconds: 1_800_000_000,
        zone_minutes: 600,
        keys: Vec::new(),
        sounds: vec![1, 2],
        source_bytes: 1,
    }
}

/// Claims everything for `open`: the keys named, text, pointer moves, the
/// wheel and every tick.
pub fn claim(open: &mut ContextOpen, keys: &[&str]) {
    open.keys = keys.iter().map(|key| (*key).to_owned()).collect();
    open.flags = ContextFlags { pointer_press: true, pointer_move: true, wheel: true, text: true, ..open.flags };
    open.tick_divisor = 1;
}

/// Opening a context with a script, the opening's length made the script's.
pub trait Opens {
    /// Opens `open` with `source` as its script.
    fn open_with(&mut self, open: ContextOpen, source: &[u8]) -> Result<(), LinkError>;
}

impl Opens for Container {
    fn open_with(&mut self, mut open: ContextOpen, source: &[u8]) -> Result<(), LinkError> {
        open.source_bytes = u32::try_from(source.len()).unwrap();
        self.open(open, source)
    }
}

/// A new container whose scripts no clock hurries: for the tests of what
/// a script sees, which must not depend on how fast this machine is.
pub fn container() -> Container {
    Container::new(engine(), &Settings { time: TimeLimits::unhurried(), watchdog: None }, 600).expect("the container starts")
}

/// A new container under the limits the browser's script host runs with:
/// for the tests of the limits themselves.
pub fn limited() -> Container {
    Container::new(engine(), &Settings::default(), 600).expect("the container starts")
}

/// A container with a page context running `source`, everything claimed.
pub fn page(source: &str) -> Container {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    claim(&mut open, &["Enter", "a", "ArrowLeft"]);
    container.open_with(open, source.as_bytes()).expect("the page opens");
    container
}

/// A tick step.
pub fn tick(sequence: u32, tick: u64) -> Step {
    Step { sequence, tick, flags: StepFlags { tick: true, sound_allowed: true, ..StepFlags::default() }, ..Step::default() }
}

/// Steps `container` through the ticks `from..to`, giving each batch.
pub fn run(container: &mut Container, from: u64, to: u64) -> Vec<Result<Batch, LinkError>> {
    (from..to).map(|at| container.step(&tick(u32::try_from(at).unwrap(), at))).collect()
}

/// A tick step carrying `events`.
pub fn with_events(sequence: u32, at: u64, events: Vec<Event>) -> Step {
    Step { events, ..tick(sequence, at) }
}

/// Every log line of `batch`, as text.
pub fn lines(batch: &Batch) -> Vec<String> {
    view(batch).log.iter().map(|line| line.text.clone()).collect()
}

/// Every log line of a run's batches that were answered.
pub fn all_lines(batches: &[Result<Batch, LinkError>]) -> Vec<String> {
    batches.iter().filter_map(|batch| batch.as_ref().ok()).flat_map(lines).collect()
}

/// What each test script starts with: `check(name, ok)` logs a failure,
/// `throws(name, Kind, starts, fn)` checks that `fn` throws a `Kind` whose
/// message starts with `starts`, and `done()` logs how many checks passed.
pub const CHECKS: &str = r#"
let __passed = 0;
function check(name, ok) { if (ok === true) __passed++; else console.error("FAIL " + name); }
function equal(name, got, want) { if (got === want) __passed++; else console.error("FAIL " + name + ": got " + JSON.stringify(got) + ", want " + JSON.stringify(want)); }
function throws(name, Kind, starts, fn) {
  try { fn(); console.error("FAIL " + name + ": nothing was thrown"); }
  catch (error) {
    if (error instanceof Kind && String(error.message).startsWith(starts)) __passed++;
    else console.error("FAIL " + name + ": threw " + (error && error.name) + ": " + (error && error.message));
  }
}
function done() { console.log("passed " + __passed); }
"#;

/// Asserts that a run's log has no failed check and that exactly `passed`
/// checks passed in all.
#[track_caller]
pub fn assert_checks(lines: &[String], passed: u32) {
    let failures: Vec<&String> = lines.iter().filter(|line| line.contains("FAIL") || line.starts_with("uncaught")).collect();
    assert!(failures.is_empty(), "{failures:#?}");
    let counted: u32 = lines.iter().filter_map(|line| line.strip_prefix("passed ")).map(|count| count.parse::<u32>().unwrap()).sum();
    assert_eq!(counted, passed, "the checks that passed, in {lines:#?}");
}

/// A script: the checks, then `body`.
pub fn script(body: &str) -> String {
    format!("{CHECKS}\n{body}")
}

/// The time this thread has spent waiting for a processor, where the
/// system says (Linux's `/proc/thread-self/schedstat`, second field,
/// nanoseconds); zero elsewhere. A limit on the wall clock cannot be held
/// through a wait for a processor on a machine that has none free, so the
/// timing tests take the wait out of what they measure.
pub fn waited_for_a_processor() -> std::time::Duration {
    std::fs::read_to_string("/proc/thread-self/schedstat")
        .ok()
        .and_then(|text| text.split_whitespace().nth(1).and_then(|field| field.parse::<u64>().ok()))
        .map_or(std::time::Duration::ZERO, std::time::Duration::from_nanos)
}

/// How long something took: on the wall clock, and of that the part its
/// thread was not waiting for a processor.
#[derive(Debug, Clone, Copy)]
pub struct Took {
    /// On the wall clock.
    pub wall: std::time::Duration,
    /// On the wall clock, less the waits for a processor.
    pub running: std::time::Duration,
}

/// Runs `work` and gives its result with how long it took.
pub fn timed<T>(work: impl FnOnce() -> T) -> (T, Took) {
    let waited = waited_for_a_processor();
    let began = std::time::Instant::now();
    let result = work();
    let wall = began.elapsed();
    (result, Took { wall, running: wall.saturating_sub(waited_for_a_processor().saturating_sub(waited)) })
}

/// An event with text for its payload: a key's name, typed text, an address.
pub fn text_event(kind: EventKind, numbers: [i32; 4], text: &str) -> Event {
    Event { kind, numbers, payload: text.as_bytes().to_vec() }
}

/// An event with whole numbers for its payload, four bytes each,
/// big-endian as the message writes every number.
pub fn ints_event(kind: EventKind, numbers: [i32; 4], ints: &[i32]) -> Event {
    Event { kind, numbers, payload: ints.iter().flat_map(|number| number.to_be_bytes()).collect() }
}

/// An event with no payload.
pub fn plain_event(kind: EventKind, numbers: [i32; 4]) -> Event {
    Event { kind, numbers, payload: Vec::new() }
}

/// A key event: its name, and its modifier bits.
pub fn key(kind: EventKind, name: &str, bits: i32) -> Event {
    text_event(kind, [bits, 0, 0, 0], name)
}

/// A batch's sections as plain lists, an absent section an empty one.
pub struct View {
    /// Each event's verdict: bit 0 cancelled, bit 1 handled.
    pub verdicts: Vec<u8>,
    /// The sound operations.
    pub sound: Vec<SoundOperation>,
    /// The log.
    pub log: Vec<LogLine>,
    /// Each context's state.
    pub states: Vec<ContextState>,
}

/// A batch's sections as lists. Every batch a test looks at is also
/// written as the message a script host sends and read back as the
/// decoder reads it: what the library makes is always a message the
/// link takes, whole.
#[track_caller]
pub fn view(batch: &Batch) -> View {
    let mut written = Vec::new();
    write_to_decoder(&mut written, &FromScriptHost::Batch(Box::new(batch.clone())))
        .unwrap_or_else(|error| panic!("the batch is not a message the link takes: {error:?}\n{batch:#?}"));
    let read = read_to_decoder(&mut written.as_slice()).unwrap();
    assert_eq!(read, Some(FromScriptHost::Batch(Box::new(batch.clone()))), "the batch is not read back as it was written");
    View {
        verdicts: batch.verdicts.iter().flatten().map(|verdict| u8::from(verdict.cancelled) | (u8::from(verdict.handled) << 1)).collect(),
        sound: batch.sound.clone().unwrap_or_default(),
        log: batch.log.clone().unwrap_or_default(),
        states: batch.state.clone().unwrap_or_default(),
    }
}
