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
use neoteletext_script::limits::{MAX_HOST_STATE_BYTES, WASM_MEMORY_BYTES};
use neoteletext_script::{Container, Engine, LinkError, Settings, TimeLimits};
use protocol::script::{ContextFlags, ContextId, ContextOpen, Event, EventKind, FromScriptHost, Level, Renderer, Step, StepFlags, write_to_decoder};

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
    // An empty script is no script, and one past the bound is refused
    // before it is run: neither is this target's to try.
    let Ok(source_bytes) = u32::try_from(data.len()) else { return };
    let open = ContextOpen {
        context: ContextId::Page,
        api: 1,
        renderer: Renderer::Teletext,
        level: Level::TwoFive,
        page: 0x100,
        subcode: 0,
        flags: ContextFlags { pointer_press: true, pointer_move: true, wheel: true, text: true, network: false, from_disk: false, sound_allowed: true },
        tick_divisor: 1,
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [3; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: vec!["Enter".into()],
        sounds: vec![1],
        source_bytes,
    };
    if open.check().is_err() {
        return;
    }
    let began = Instant::now();
    let opened = container.open(open, data);
    assert!(began.elapsed() <= LIMITS.start + MARGIN, "the top level took {:?}", began.elapsed());
    let mut stopped = matches!(opened, Err(LinkError::Stopped(..)));
    for at in 0..STEPS {
        let events = vec![
            Event { kind: EventKind::KeyDown, numbers: [0; 4], payload: b"Enter".to_vec() },
            Event { kind: EventKind::Pointer, numbers: [1, 3, 4, 0], payload: [0_i32, 1, 0, 0].iter().flat_map(|number| number.to_be_bytes()).collect() },
            Event { kind: EventKind::Text, numbers: [0; 4], payload: b"x".to_vec() },
        ];
        let callbacks = container.costs().callbacks;
        let began = Instant::now();
        let batch = container.step(&Step {
            sequence: at,
            tick: u64::from(at),
            flags: StepFlags { tick: true, sound_allowed: true, ..StepFlags::default() },
            events,
            ..Step::default()
        });
        // A step starts nothing past its budget, and no callback outlives its limit.
        assert!(began.elapsed() <= LIMITS.step + LIMITS.callback + MARGIN, "a step took {:?}", began.elapsed());
        if stopped {
            assert!(batch.is_err() && container.costs().callbacks == callbacks, "a stopped container was entered again");
        }
        match batch {
            Ok(batch) => {
                // Whatever the script did, its step's output is a message
                // the link takes whole, with a verdict for each event.
                assert!(batch.verdicts.as_ref().is_some_and(|verdicts| verdicts.len() == 3));
                let mut written = Vec::new();
                if let Err(error) = write_to_decoder(&mut written, &FromScriptHost::Batch(batch.clone().into())) {
                    panic!("the batch is not a message the link takes: {error:?}\n{batch:#?}");
                }
                stopped = container.stopped().is_some();
            }
            Err(error) => assert!(matches!(error, LinkError::Stopped(..)), "{error}"),
        }
        assert!(container.memory().0 <= WASM_MEMORY_BYTES && container.host_bytes() <= MAX_HOST_STATE_BYTES);
    }
    container.close(ContextId::Page);
});
