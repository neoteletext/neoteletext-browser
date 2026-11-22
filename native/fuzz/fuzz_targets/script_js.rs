//! Arbitrary JavaScript in the script container (script-api.md section
//! 11.4, target 3): the input is run as a page's script, then stepped
//! with ticks and input, under time limits shortened so that an input
//! that never ends costs little. Whatever the script does, the host does
//! not panic; every call returns within its limit and a wide margin (the
//! fuzzer's instruments slow everything, and its machine is busy); the
//! container's memory stays within its cap and the host's own within its
//! bound; and a container that was stopped is never entered again.
#![no_main]

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use libfuzzer_sys::fuzz_target;
use neoteletext_script::limits::{MAX_BATCH_BYTES, MAX_HOST_STATE_BYTES, MAX_LOG_LINES, WASM_MEMORY_BYTES};
use neoteletext_script::ops::EventKind;
use neoteletext_script::{Claims, Container, ContextId, ContextOpen, Engine, EventPayload, LinkError, PointerClaim, Renderer, Settings, Step, StepEvent, TimeLimits};

/// The limits the inputs run under: the browser's shapes, a tenth to a
/// twentieth of their lengths.
const LIMITS: TimeLimits = TimeLimits {
    start: Duration::from_millis(60),
    callback: Duration::from_millis(10),
    step: Duration::from_millis(12),
    second: Duration::from_millis(100),
    close: Duration::from_millis(10),
};
/// How far past a limit a call may return before it is a fault.
const MARGIN: Duration = Duration::from_millis(1500);
/// The steps each input is given.
const STEPS: u32 = 6;

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| Engine::new().unwrap_or_else(|stop| panic!("the module does not compile: {stop}")))
}

fuzz_target!(|data: &[u8]| {
    let mut container = Container::new(engine(), &Settings { time: LIMITS, watchdog: None }, 0).unwrap_or_else(|stop| panic!("no container: {stop}"));
    let open = ContextOpen {
        context: ContextId::Page,
        api: 1,
        renderer: Some(Renderer::Teletext),
        page: 0x100,
        subcode: None,
        claims: Claims { keys: vec!["Enter".into()], text: true, pointer: PointerClaim::Move, wheel: true, tick_divisor: 1, network: false },
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [3; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        sounds: vec![1],
        sound_allowed: true,
    };
    let began = Instant::now();
    let opened = container.open(open, data);
    assert!(began.elapsed() <= LIMITS.start + MARGIN, "the top level took {:?}", began.elapsed());
    let mut stopped = matches!(opened, Err(LinkError::Stopped(..)));
    for at in 0..STEPS {
        let events = vec![
            StepEvent { kind: EventKind::KEY_DOWN, ints: [0; 4], payload: EventPayload::Text("Enter".into()) },
            StepEvent { kind: EventKind::POINTER, ints: [1, 3, 4, 0], payload: EventPayload::Ints(vec![0, 1, 0, 0]) },
            StepEvent { kind: EventKind::TEXT, ints: [0; 4], payload: EventPayload::Text("x".into()) },
        ];
        let callbacks = container.costs().callbacks;
        let began = Instant::now();
        let batch = container.step(&Step { sequence: at, tick: u64::from(at), is_tick: true, sound_allowed: true, events, ..Step::default() });
        // A step starts nothing past its budget, and no callback outlives its limit.
        assert!(began.elapsed() <= LIMITS.step + LIMITS.callback + MARGIN, "a step took {:?}", began.elapsed());
        if stopped {
            assert!(batch.is_err() && container.costs().callbacks == callbacks, "a stopped container was entered again");
        }
        match batch {
            Ok(batch) => {
                assert!(batch.log.len() <= MAX_LOG_LINES && batch.verdicts.len() == 3);
                let bytes: usize = batch.log.iter().map(|line| line.text.len()).sum();
                assert!(bytes <= MAX_BATCH_BYTES);
                stopped = container.stopped().is_some();
            }
            Err(error) => assert!(matches!(error, LinkError::Stopped(..)), "{error}"),
        }
        assert!(container.memory().0 <= WASM_MEMORY_BYTES && container.host_bytes() <= MAX_HOST_STATE_BYTES);
    }
    container.close(ContextId::Page);
});
