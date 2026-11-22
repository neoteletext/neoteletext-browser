//! What the script library's tests share: one compiled engine, contexts
//! opened with plain settings, and steps.
#![allow(dead_code, reason = "each test file uses its own part of this")]

use std::sync::OnceLock;

use neoteletext_script::{Batch, Claims, Container, ContextId, ContextOpen, Engine, LinkError, PointerClaim, Renderer, Settings, Step, StepEvent, TimeLimits};

/// The seed the tests give a context unless they give their own.
pub const SEED: [u8; 32] = [42; 32];
/// The channel the tests' contexts belong to.
pub const CHANNEL: &str = "https://example.test/chan/";

/// The engine, compiled once for the test program.
pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| Engine::new().expect("the module compiles"))
}

/// A `CONTEXT_OPEN` for `context` with nothing claimed.
pub fn opening(context: ContextId) -> ContextOpen {
    ContextOpen {
        context,
        api: 1,
        renderer: Some(Renderer::Teletext),
        page: 0x100,
        subcode: None,
        claims: Claims::default(),
        language: "en".into(),
        channel_url: CHANNEL.into(),
        page_url: format!("{CHANNEL}100.tti"),
        script_url: format!("{CHANNEL}{}.js", if context == ContextId::Page { "page" } else { "channel" }),
        channel_title: "Example".into(),
        seed: SEED,
        epoch_seconds: 1_800_000_000,
        zone_minutes: 600,
        sounds: vec![1, 2],
        sound_allowed: true,
    }
}

/// Everything claimed: the keys named, text, pointer moves, the wheel and every tick.
pub fn claiming(keys: &[&str]) -> Claims {
    Claims { keys: keys.iter().map(|key| (*key).to_owned()).collect(), text: true, pointer: PointerClaim::Move, wheel: true, tick_divisor: 1, network: false }
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
    open.claims = claiming(&["Enter", "a", "ArrowLeft"]);
    container.open(open, source.as_bytes()).expect("the page opens");
    container
}

/// A tick step.
pub fn tick(sequence: u32, tick: u64) -> Step {
    Step { sequence, tick, is_tick: true, sound_allowed: true, ..Step::default() }
}

/// Steps `container` through the ticks `from..to`, giving each batch.
pub fn run(container: &mut Container, from: u64, to: u64) -> Vec<Result<Batch, LinkError>> {
    (from..to).map(|at| container.step(&tick(u32::try_from(at).unwrap(), at))).collect()
}

/// A tick step carrying `events`.
pub fn with_events(sequence: u32, at: u64, events: Vec<StepEvent>) -> Step {
    Step { events, ..tick(sequence, at) }
}

/// Every log line of `batch`, as text.
pub fn lines(batch: &Batch) -> Vec<String> {
    batch.log.iter().map(|line| line.text.clone()).collect()
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
