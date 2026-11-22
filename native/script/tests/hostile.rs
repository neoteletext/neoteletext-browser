//! The security review's hostile scripts (`tests/hostile/`), every one
//! held to its bound under the limits the browser's script host runs
//! with (script-api.md section 3.2 rule 5 and section 10.1; rdm TODO-7267
//! and its findings TODO-7270, TODO-7271, TODO-7272, TODO-7275 and
//! TODO-7279). The measurements the review made of the spike are beside
//! each: a script that ran 3.03 s under a 50 ms limit there must be
//! stopped within the limit and [`TIME_CHECK_MARGIN`] here.
//!
//! Times are taken on the wall clock less the time the test's thread
//! waited for a processor, since a limit cannot be held through that.
mod common;

use std::time::Duration;

use common::{Took, all_lines, limited, lines, opening, tick, timed};
use neoteletext_script::limits::{
    CALLBACK_TIME_LIMIT, MAX_BATCH_BYTES, MAX_HOST_STATE_BYTES, MAX_SOUND_OPERATIONS_PER_CALLBACK, START_TIME_LIMIT, TIME_CHECK_MARGIN, WASM_MEMORY_BYTES,
};
use neoteletext_script::{Batch, Container, ContextId, LinkError, SoundOperation, StopReason};

macro_rules! hostile {
    ($name:literal) => {
        include_str!(concat!("hostile/", $name, ".js"))
    };
}

/// Opens `source` as a page's script, timing it.
fn open(source: &str) -> (Container, Result<(), LinkError>, Took) {
    let mut container = limited();
    let (opened, took) = timed(|| container.open(opening(ContextId::Page), source.as_bytes()));
    (container, opened, took)
}

/// Asserts a run was stopped by its time limit: no sooner than `limit`
/// on the wall clock, and no later than the margin past it once its
/// waits for a processor are left out.
#[track_caller]
fn assert_within(took: Took, limit: Duration, what: &str) {
    assert!(took.wall >= limit, "{what} was stopped after {:?}, before its limit of {limit:?}", took.wall);
    assert!(took.running <= limit + TIME_CHECK_MARGIN, "{what} ran {:?} under a limit of {limit:?}: more than {TIME_CHECK_MARGIN:?} past it", took.running);
}

/// Asserts the container is stopped for good for `reason`: its state says
/// so, and it is never entered again.
#[track_caller]
fn assert_stopped_for_good(container: &mut Container, reason: StopReason) {
    assert_eq!(container.stop_reason(), Some(reason));
    let callbacks = container.costs().callbacks;
    assert!(matches!(container.step(&tick(900, 900)), Err(LinkError::Stopped(stopped, _)) if stopped == reason));
    assert!(matches!(container.open(opening(ContextId::Channel), b"nt.log('ran after the stop')"), Err(LinkError::Stopped(stopped, _)) if stopped == reason));
    assert_eq!(container.costs().callbacks, callbacks, "a stopped container was entered again");
}

/// Runs a script whose top level never ends: it must be stopped by the
/// start limit.
#[track_caller]
fn stopped_at_its_top_level(name: &str, source: &str) {
    let (mut container, opened, took) = open(source);
    assert!(matches!(opened, Err(LinkError::Stopped(StopReason::Time, _))), "{name}: {opened:?}");
    assert_within(took, START_TIME_LIMIT, name);
    assert_stopped_for_good(&mut container, StopReason::Time);
}

/// Runs a script that sets a timer for the next tick whose callback never
/// ends: the first step runs nothing, the second must be stopped by the
/// callback limit, and its batch must still be answered and say so.
#[track_caller]
fn stopped_in_its_callback(name: &str, source: &str) -> Batch {
    let (mut container, opened, _) = open(source);
    opened.unwrap_or_else(|error| panic!("{name} did not open: {error}"));
    container.step(&tick(1, 0)).unwrap();
    let (batch, took) = timed(|| container.step(&tick(2, 1)));
    let batch = batch.unwrap_or_else(|error| panic!("{name}: the step in which the container stopped was not answered: {error}"));
    assert_within(took, CALLBACK_TIME_LIMIT, name);
    assert_eq!(batch.states.iter().map(|state| state.stopped).collect::<Vec<_>>(), vec![Some(StopReason::Time)], "{name}");
    assert!(batch.log.iter().any(|line| line.level == 4 && line.text.contains("time limit")), "{name}: {:?}", lines(&batch));
    assert_stopped_for_good(&mut container, StopReason::Time);
    batch
}

/// Runs a script that sets a timer for the next tick and gives the step
/// the timer fires in, with how long it took and the container after it.
fn after_its_callback(source: &str) -> (Container, Batch, Took) {
    let (mut container, opened, _) = open(source);
    opened.unwrap();
    container.step(&tick(1, 0)).unwrap();
    let (batch, took) = timed(|| container.step(&tick(2, 1)));
    (container, batch.unwrap(), took)
}

/// S-04 (TODO-7270): the review measured 3.07 s under the 1 s start limit
/// for the copy at the top level.
#[test]
fn a_top_level_that_never_ends_is_stopped_at_the_start_limit() {
    stopped_at_its_top_level("toploop.js", hostile!("toploop"));
    stopped_at_its_top_level("copy_top.js", hostile!("copy_top"));
    // The promise chain that never ends: microtasks run to exhaustion
    // inside the limit of the entry that made them.
    stopped_at_its_top_level("jobs_carry.js", hostile!("jobs_carry"));
}

/// S-04 (TODO-7270): under the 50 ms callback limit the review measured
/// 3.03 s for `copyWithin` over 40 MiB, 1.94 s for `fill` and 208 ms for a
/// loop of host calls.
#[test]
fn a_callback_that_never_ends_is_stopped_at_the_callback_limit() {
    stopped_in_its_callback("loop.js", hostile!("loop"));
    stopped_in_its_callback("copy_cb.js", hostile!("copy_cb"));
    stopped_in_its_callback("fill_cb.js", hostile!("fill_cb"));
    stopped_in_its_callback("budget.js", hostile!("budget"));
    stopped_in_its_callback("promises.js", hostile!("promises"));
}

/// S-09 item 4 (TODO-7275): the text of what was thrown is taken without
/// running the page's code after the callback has ended. Thrown inside a
/// callback, the prelude describes it within the callback's own time: an
/// object is described as JSON, so its `toString` never runs; a `stack`
/// getter that never returns is the callback never returning, and is
/// stopped at the callback's limit as the review measured.
#[test]
fn describing_what_was_thrown_runs_no_page_code_after_the_callback() {
    let (container, batch, took) = after_its_callback(hostile!("throwobj"));
    assert!(took.running < CALLBACK_TIME_LIMIT, "throwobj.js ran {took:?}: its toString was called");
    assert_eq!(lines(&batch), vec!["uncaught: {}"]);
    assert_eq!(container.stop_reason(), None);
    stopped_in_its_callback("throwstack.js", hostile!("throwstack"));
    // Thrown at the top level, where the shim describes it after the
    // script has returned: neither the toString nor the getter is called.
    for (source, expected) in [
        ("throw { toString() { for (;;) {} } };", "uncaught: a value was thrown: an object"),
        ("const e = new Error('x'); Object.defineProperty(e, 'stack', { get() { for (;;) {} } }); throw e;", "uncaught: Error: x"),
        ("const e = new Error('x'); Object.defineProperty(e, 'message', { get() { for (;;) {} } }); throw e;", "uncaught: Error\n"),
        (
            "throw new Proxy(new Error('x'), { get() { for (;;) {} }, getOwnPropertyDescriptor() { for (;;) {} }, getPrototypeOf() { for (;;) {} } });",
            "uncaught: a value was thrown: an object",
        ),
        ("class Mine extends Error { get name() { for (;;) {} } } throw new Mine('x');", "uncaught: Error: x"),
        ("Promise.reject({ toString() { for (;;) {} } });", "unhandled rejection: a value was thrown: an object"),
        ("throw 'a string';", "uncaught: a string was thrown: a string"),
        ("throw null;", "uncaught: a value was thrown: null"),
    ] {
        let (mut container, opened, took) = open(source);
        opened.unwrap();
        assert!(took.running < START_TIME_LIMIT / 2, "{source} ran {took:?}: the page's code was called to describe what it threw");
        let log = lines(&container.step(&tick(1, 0)).unwrap());
        // The stack, where there is one, follows on the next line.
        let first = log[0].replace("     at", "\n    at");
        assert!(first == expected || first.starts_with(&format!("{}\n", expected.trim_end())), "{source} logged {log:?}");
        assert_eq!(container.stop_reason(), None, "{source}");
    }
}

/// Section 3.1's table: a memory limit passed stops the container for
/// good. The scripts that allocate without end are stopped by memory or,
/// if their callback's time runs out first, by time; either way within
/// the callback's limit, and with the container's memory within its cap.
#[test]
fn allocation_without_end_is_stopped_within_the_memory_cap() {
    for (name, source) in [("alloc.js", hostile!("alloc")), ("bigstring.js", hostile!("bigstring"))] {
        let (mut container, batch, took) = after_its_callback(source);
        assert!(took.running <= CALLBACK_TIME_LIMIT + TIME_CHECK_MARGIN, "{name} ran {took:?}");
        assert!(container.memory().0 <= WASM_MEMORY_BYTES, "{name}: the container's memory is {} bytes", container.memory().0);
        match container.stop_reason() {
            Some(reason) => {
                assert!(matches!(reason, StopReason::Memory | StopReason::Time), "{name}: {reason:?}");
                assert_stopped_for_good(&mut container, reason);
            }
            // A string doubled without end meets the engine's longest
            // string first, an ordinary error that ends the callback.
            None => assert!(lines(&batch).iter().any(|line| line.contains("string too long")), "{name} was neither stopped nor failed: {:?}", lines(&batch)),
        }
    }
    // Typed arrays reach the heap's limit long before the start limit.
    let (mut container, opened, took) = open("const keep = []; for (;;) keep.push(new Uint8Array(4 * 1024 * 1024));");
    assert!(matches!(opened, Err(LinkError::Stopped(StopReason::Memory, _))), "{opened:?}");
    assert!(took.running < START_TIME_LIMIT);
    assert_stopped_for_good(&mut container, StopReason::Memory);
}

/// Section 3.1's table and section 10.1 rule 5: recursion without end
/// meets the interpreter's stack limit, a trap that stops the container
/// for good, whether it recurses directly or through native callbacks.
#[test]
fn recursion_without_end_is_stopped_at_the_stack_limit() {
    for (name, source) in [("recurse.js", hostile!("recurse")), ("recurse2.js", hostile!("recurse2"))] {
        let (mut container, batch, took) = after_its_callback(source);
        assert!(took.running <= CALLBACK_TIME_LIMIT + TIME_CHECK_MARGIN, "{name} ran {took:?}");
        assert_eq!(container.stop_reason(), Some(StopReason::Stack), "{name}: {:?}", lines(&batch));
        assert_stopped_for_good(&mut container, StopReason::Stack);
    }
}

/// The review's lead (section 3.5): a callback made 2,000 sound stops in
/// the spike. Here the 65th throws, naming the bound, and the batch holds
/// no more than the bound.
#[test]
fn sound_stops_are_held_to_the_callbacks_rate() {
    let (container, batch, _) = after_its_callback(hostile!("soundstop"));
    assert_eq!(batch.sound, vec![SoundOperation::Stop { voice: 0 }; MAX_SOUND_OPERATIONS_PER_CALLBACK as usize]);
    let log = lines(&batch);
    assert!(log[0].starts_with("uncaught: RangeError: MAX_SOUND_OPERATIONS_PER_CALLBACK: "), "{log:?}");
    assert!(!log.iter().any(|line| line == "stopped"));
    assert_eq!(container.stop_reason(), None);
}

/// S-06 (TODO-7272) and the review's request floods: the scripts that
/// made the spike's host hold 130 MB through ReGIS drawings, and make
/// 1,240 requests in a step, use parts of the API the shared core does
/// not have (work packages R1 and W2, which owe their own regression
/// tests with these same scripts). Here they find nothing to call: no
/// request is made, nothing is drawn, the host holds no more than its
/// bound, and thirty-one failing timers a tick stop the context after
/// sixteen failures in a row.
#[test]
fn scripts_for_parts_of_the_api_not_yet_built_find_nothing_to_call() {
    for (name, source) in [
        ("regismem.js", hostile!("regismem")),
        ("regisstage.js", hostile!("regisstage")),
        ("fetchabort.js", hostile!("fetchabort")),
        ("fetchabort2.js", hostile!("fetchabort2")),
        ("sse.js", hostile!("sse")),
        ("sse2.js", hostile!("sse2")),
    ] {
        let (mut container, opened, _) = open(source);
        opened.unwrap();
        let batches: Vec<_> = (0..4).map(|at| container.step(&tick(at, u64::from(at)))).collect();
        let log = all_lines(&batches);
        assert!(
            log.iter()
                .any(|line| line.starts_with("uncaught: ") && (line.contains("not a function") || line.contains("not defined") || line.contains("undefined"))),
            "{name}: {log:?}"
        );
        assert!(!log.iter().any(|line| line.starts_with("issued") || line.starts_with("opened")), "{name}: {log:?}");
        assert_eq!(container.take_requests(), Vec::new(), "{name} made a request");
        assert!(container.host_bytes() <= MAX_HOST_STATE_BYTES, "{name}: the host holds {} bytes", container.host_bytes());
        for batch in batches.iter().flatten() {
            assert!(batch.sound.is_empty() && batch.navigate.is_none() && batch.title.is_none(), "{name}");
            let bytes: usize = batch.log.iter().map(|line| line.text.len()).sum();
            assert!(bytes <= MAX_BATCH_BYTES, "{name}: a batch of {bytes} bytes of log");
        }
        assert_eq!(container.stop_reason(), None, "{name}");
        if name.ends_with("2.js") {
            let stopped = batches.last().unwrap().as_ref().unwrap().states[0].stopped;
            assert_eq!(stopped, Some(StopReason::Failures), "{name}");
        }
    }
}

/// The review's survey of the realm: no shared memory and no `Atomics`
/// (DEC-7282 item 11), and no clock finer than a tick (departures D13 and
/// D14): neither `Date` nor `performance.now()` moves within a callback.
#[test]
fn the_realm_has_no_shared_memory_and_no_fine_clock() {
    let (mut container, opened, _) = open(hostile!("globals"));
    opened.unwrap();
    let log = all_lines(&[container.step(&tick(1, 0)), container.step(&tick(2, 1))]);
    let names: String = log.iter().filter_map(|line| line.strip_prefix("globals ")).collect();
    let globals: Vec<&str> = names.split(',').collect();
    for absent in ["SharedArrayBuffer", "Atomics", "WeakRef", "FinalizationRegistry", "__host", "__setDispatch", "__setRejection", "os", "std", "gc", "print"] {
        assert!(!globals.contains(&absent), "{absent} is a global: {globals:?}");
    }
    for present in
        ["nt", "crypto", "performance", "setTimeout", "structuredClone", "console", "atob", "queueMicrotask", "EventTarget", "AbortController", "DOMException"]
    {
        assert!(globals.contains(&present), "{present} is not a global: {globals:?}");
    }
    assert_eq!(&log[log.len() - 2..], ["date 0 perf 0 now 1", "atomics: ReferenceError"]);
}

/// S-13 (TODO-7279): the review found three pages started in the same
/// second with one `Math.random` sequence. Here the sequence follows the
/// context's seed, which the trusted core draws afresh for each.
#[test]
fn pages_started_in_the_same_second_have_different_random_sequences() {
    let sequence = |seed: u8| {
        let mut container = limited();
        let mut open = opening(ContextId::Page);
        open.seed = [seed; 32];
        container.open(open, hostile!("rand").as_bytes()).unwrap();
        lines(&container.step(&tick(1, 0)).unwrap())
    };
    let (first, second, third) = (sequence(1), sequence(2), sequence(3));
    assert!(first[0].starts_with("r=0.") && first != second && second != third && first != third, "{first:?} {second:?} {third:?}");
}
