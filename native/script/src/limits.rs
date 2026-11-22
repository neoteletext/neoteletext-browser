//! Every bound of the script library's core (script-api.md section 10.6),
//! each defined once, here. A script is untrusted input: each length,
//! count and number it passes is held to one of these, or to the engine's
//! own constant named here, before it is used. A call that would pass one
//! throws a `RangeError` whose message begins with the bound's name, and
//! changes nothing.

use core::time::Duration;

pub mod regis;

/// A byte as the whole number a host call's argument is compared with.
const fn wide(byte: u8) -> i32 {
    byte as i32
}

/// The largest script taken, in bytes.
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_SCRIPT_BYTES: usize = protocol::script::MAX_SCRIPT_BYTES;

/// The WebAssembly linear memory one container's QuickJS may grow to: the
/// container's memory cap, enforced by the interpreter's store limits
/// (DEC-7256; one container holds both contexts).
pub const WASM_MEMORY_BYTES: usize = 64 * 1024 * 1024;
/// QuickJS's own heap limit inside that memory, below it so that a script
/// that allocates without end meets QuickJS's out-of-memory error before
/// the container refuses to grow.
pub const QUICKJS_HEAP_BYTES: i32 = 48 * 1024 * 1024;
/// QuickJS's stack limit, below the 1 MiB stack the module is linked with
/// (`qjs/build.sh`, `STACK_BYTES`), so deep recursion throws a
/// `RangeError` in the script before the WebAssembly stack runs out.
pub const QUICKJS_STACK_BYTES: i32 = 768 * 1024;
/// The deepest the WebAssembly interpreter's own call stack may grow, in
/// calls: QuickJS recurses in C for each JavaScript call, and this holds
/// a recursion that QuickJS's check misses.
pub const WASM_MAX_RECURSION: usize = 16 * 1024;

/// The fuel the interpreter is given at a time, after which it stops and
/// the clock is read (script-api.md section 3.2 rule 5(a)). Fuel is how
/// the interpreter is made to stop; the clock is the limit, because fuel
/// is not time. Measured on chunko on 22 November 2026 by the crate's
/// `tests/measure.rs`, which prints the figures: `QuickJS`'s loop over
/// bytecode spends about 600 million units a millisecond (wasmi charges
/// the loop's large block whole at each instruction of the script), and
/// loops inside `QuickJS`'s own C and the C library (filling a typed
/// array, searching a string, sorting) about 5 million. This much is
/// therefore spent in about 7 microseconds of plain JavaScript and in
/// under a millisecond of the slowest work found; stopping and going on
/// so often costs under a tenth of a script's speed. The specification's
/// value, 1,000,000,000, is the most this may be: it is two milliseconds
/// of bytecode and a fifth of a second of a C loop, which is the overrun
/// the security review measured.
pub const FUEL_SLICE: u64 = 4_000_000;
/// The fuel the interpreter is charged for each byte a WebAssembly
/// bulk-memory instruction would copy, were one ever to run: one unit a
/// byte, against wasmi's own one unit for 64 bytes. The module holds no
/// such instruction and the engine refuses one that does, so this is the
/// fallback script-api.md section 3.2 rule 5(a) keeps in place.
pub const BULK_BYTES_PER_FUEL: u32 = 1;
/// What the interpreter charges for translating and validating a
/// function on its first call, in fuel a byte: nothing. wasmi's own
/// charges (7 and 2) are taken in one piece when a function is first
/// called, and where a slice has less fuel left than that it ends the
/// call with an error that cannot be resumed (wasmi 2.0.2,
/// `engine/code_map`), which would stop a container for nothing it did.
/// The work is bounded without fuel: each function is translated once,
/// and the largest, QuickJS's interpreter loop, takes about 2 ms.
pub const FUEL_PER_BYTE_TRANSLATED: u32 = 0;
/// See [`FUEL_PER_BYTE_TRANSLATED`].
pub const FUEL_PER_BYTE_VALIDATED: u32 = 0;

/// The longest a context's top level may run before the container is
/// stopped (DEC-7256).
pub const START_TIME_LIMIT: Duration = Duration::from_secs(1);
/// The longest one callback, with its microtasks, may run before the
/// container is stopped (DEC-7256).
pub const CALLBACK_TIME_LIMIT: Duration = Duration::from_millis(50);
/// The time everything one step runs may use, in both contexts: past it no
/// further callback of the step starts (script-api.md section 3.2 rule 2;
/// TODO-7271).
pub const STEP_TIME_BUDGET: Duration = Duration::from_millis(12);
/// The script time a container may use in any [`TICKS_PER_SECOND`]
/// consecutive ticks before it is stopped for good (rule 3; TODO-7271).
pub const SECOND_TIME_BUDGET: Duration = Duration::from_millis(500);
/// The longest the one handling of `close` may run.
pub const CLOSE_TIME_LIMIT: Duration = Duration::from_millis(50);
/// How far past its limit a callback or a top level may be before the
/// watchdog thread ends the process (rule 5(b)).
pub const WATCHDOG_GRACE: Duration = Duration::from_millis(10);
/// The most an entry may run past its limit before the container's own
/// clock checks stop it, on a machine with a processor free for it: the
/// longest piece of work between two reads of the clock (a slice of
/// [`FUEL_SLICE`] fuel, [`HOST_COPY_CHECK_BYTES`] copied, a mebibyte of
/// memory added, one function translated on its first call) with room
/// over. The stated worst case the security review asked for (finding
/// S-04, TODO-7270 item 4): the crate's tests hold every limit to it,
/// and the watchdog's grace is no shorter.
pub const TIME_CHECK_MARGIN: Duration = Duration::from_millis(10);
const _: () = assert!(TIME_CHECK_MARGIN.as_nanos() <= WATCHDOG_GRACE.as_nanos());
/// The longest a container may take to be made: the module instantiated,
/// the C library and QuickJS started. Nothing of a script runs in it.
pub const CONTAINER_START_LIMIT: Duration = START_TIME_LIMIT;
/// The bytes a host call or the module's own memory functions copy or
/// process between two reads of the clock (rule 6). The module's
/// `COPY_CHECK_BYTES` (`qjs/mem.c`) is the same number, compared at each
/// container's start.
pub const HOST_COPY_CHECK_BYTES: usize = 65_536;
/// The handler failures in a row after which a context is stopped.
pub const MAX_CONSECUTIVE_FAILURES: u32 = 16;

/// The most whole numbers one host call takes after its operation: the
/// shim's `MAX_INTS` less the operation itself.
pub const MAX_HOST_INTS: usize = 23;
/// The most bytes of one host call's payload.
pub const MAX_HOST_BYTES: usize = 256 * 1024;
/// The longest result a host call returns: the shim's `OUT_CAPACITY`.
pub const MAX_RESULT_BYTES: usize = 1024 * 1024;
/// The longest error text taken back from the shim: its `ERROR_CAPACITY`.
pub const MAX_ERROR_BYTES: usize = 2048;
/// The contexts of a container: the channel's and the page's.
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const CONTEXTS: usize = protocol::script::CONTEXTS;

/// The presentation clock (neoteletext-common's ANIM-1): 60 ticks a
/// second, the rule table's constant by name.
pub const TICKS_PER_SECOND: u64 = neoteletext_rules::limits::TICKS_PER_SECOND as u64;
/// The largest tick divisor a manifest may ask for.
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_TICK_DIVISOR: u8 = protocol::script::MAX_TICK_DIVISOR;
/// The most timers a context may have.
pub const MAX_TIMERS: usize = 32;
/// The longest a timer may be set to: one hour of ticks.
pub const MAX_TIMER_TICKS: i32 = 216_000;
const _: () = assert!(MAX_TIMER_TICKS as u64 == 60 * 60 * TICKS_PER_SECOND);

/// The most events waiting for a context (the rule table's bound).
pub const MAX_QUEUED_EVENTS: usize = neoteletext_rules::limits::MAX_QUEUED_EVENTS;
/// The most events handled in one step (the rule table's bound).
pub const MAX_EVENTS_PER_TICK: usize = neoteletext_rules::limits::MAX_EVENTS_PER_TICK;
/// The most keys followed as held (the rule table's bound).
pub const MAX_KEYS_DOWN: usize = neoteletext_rules::limits::MAX_KEYS_DOWN;
/// The most things named under the pointer (the rule table's bound).
pub const MAX_POINTER_OBJECTS: usize = neoteletext_rules::limits::MAX_POINTER_OBJECTS;
/// The most keys a manifest may claim (the browser's protocol's bound).
pub const MAX_KEY_ASKS: usize = protocol::script::MAX_KEY_ASKS;
/// The longest key name, in bytes: the protocol crate's constant of the
/// same name is the one definition (`protocol::script`).
pub const MAX_KEY_NAME_BYTES: usize = protocol::script::MAX_KEY_NAME_BYTES;
/// The most events one step may carry to the host: a step's input and the
/// lost releases of every key that can be held.
pub const MAX_STEP_EVENTS: usize = protocol::script::MAX_STEP_EVENTS;
const _: () = assert!(MAX_STEP_EVENTS == MAX_EVENTS_PER_TICK + MAX_KEYS_DOWN);
/// The longest payload of one event of a step.
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_EVENT_PAYLOAD_BYTES: usize = protocol::script::MAX_EVENT_PAYLOAD_BYTES;
/// The longest committed text one `text` event carries.
pub const MAX_TEXT_EVENT_BYTES: usize = 4096;
const _: () = assert!(MAX_TEXT_EVENT_BYTES <= MAX_EVENT_PAYLOAD_BYTES);

/// The deepest scopes nest.
pub const MAX_SCOPE_DEPTH: usize = 8;
/// The most scopes a context may hold open.
pub const MAX_SCOPES: usize = 64;

/// The most keys of the shared store.
pub const MAX_SHARED_KEYS: usize = 64;
/// The longest key of the shared store, in bytes.
pub const MAX_SHARED_KEY_BYTES: usize = 64;
/// The longest value of the shared store, as JSON text.
pub const MAX_SHARED_VALUE_BYTES: usize = 16_384;
/// The most bytes of the shared store's keys and values together.
pub const MAX_SHARED_BYTES: usize = 65_536;

/// The most log lines kept between two steps, and the longest of each in
/// characters; more are counted and dropped.
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_LOG_LINES: usize = protocol::script::MAX_LOG_LINES;
/// See [`MAX_LOG_LINES`].
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_LOG_CHARS: usize = protocol::script::MAX_LOG_CHARS;
/// The most buffer descriptors one `fd_write` reads (the security
/// review's finding S-09, TODO-7275): QuickJS's and the C library's own
/// writes use one or two.
pub const MAX_WRITE_VECTORS: usize = 16;
/// The most bytes one `fd_write` reads: no more than the log could keep,
/// at UTF-8's four bytes a character.
pub const MAX_WRITE_BYTES: usize = MAX_LOG_LINES * MAX_LOG_CHARS * 4;
/// The highest log level: 0 debug, 1 log, 2 info, 3 warn, 4 error.
pub const MAX_LOG_LEVEL: i32 = wide(protocol::script::batch::MAX_LOG_LEVEL);

/// The longest page title, in bytes of UTF-8.
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_TITLE_BYTES: usize = protocol::script::MAX_TITLE_BYTES;
/// The longest label of a thing, in bytes.
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_LABEL_BYTES: usize = protocol::script::MAX_LABEL_BYTES;
/// The longest address, in bytes (the web client's bound).
/// The web client's constant of the same name is the one definition (`protocol::web`).
pub const MAX_URL_BYTES: usize = protocol::web::MAX_URL_BYTES;
/// The most navigations a context may ask for in a second.
pub const MAX_NAVIGATIONS_PER_SECOND: u32 = 2;
/// The longest language tag of a context, in bytes.
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_LANGUAGE_BYTES: usize = protocol::script::MAX_LANGUAGE_BYTES;

/// The audio channels of a view (the engine's audio profile, by name).
#[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap, reason = "the profile's four channels, far inside an i32, as the assertion below holds")]
pub const AUDIO_CHANNELS: i32 = neoteletext_rules::limits::AUDIO_CHANNELS as i32;
const _: () = assert!(neoteletext_rules::limits::AUDIO_CHANNELS <= 8);
/// The highest handle of a channel's sound.
pub const MAX_SOUND_NUMBER: i32 = 255;
const _: () = assert!(MAX_SOUND_NUMBER as usize == protocol::script::MAX_SOUND_HANDLES);
/// The largest variant of a synthesizer family: the rule table's
/// `MAX_VARIANT`, which the batch's reader holds a start to.
pub const MAX_SYNTH_VARIANT: i32 = wide(neoteletext_rules::limits::MAX_VARIANT);
/// The loudest gain.
pub const MAX_GAIN: i32 = wide(neoteletext_rules::limits::MAX_GAIN);
/// The most sound operations (plays, stops, notes, MIDI) one callback may ask for.
pub const MAX_SOUND_OPERATIONS_PER_CALLBACK: u32 = 64;
/// The most sound operations one step may ask for.
pub const MAX_SOUND_OPERATIONS_PER_STEP: u32 = 64;
/// The most sound operations in any second.
pub const MAX_SOUND_OPERATIONS_PER_SECOND: u32 = 256;
/// The longest a synthesizer note may be held, in ticks: ten seconds.
pub const MAX_NOTE_TICKS: i32 = 600;
/// MIDI's largest data byte (MIDI 1.0 detailed specification, section
/// "Data bytes": seven bits).
pub const MAX_MIDI_DATA: i32 = wide(neoteletext_rules::limits::MAX_MIDI_DATA);
/// The largest value of a pitch bend, 14 bits (MIDI 1.0, pitch bend change).
pub const MAX_MIDI_BEND: i32 = 16_383;

/// The most bytes one step's output may be: the engine's largest frame,
/// by name, so that a batch is always one frame.
pub const MAX_BATCH_BYTES: usize = protocol::script::batch::MAX_BATCH_BYTES;
const _: () = assert!(MAX_BATCH_BYTES == neoteletext_web::consts::MAX_FRAME_BYTES);
/// The most bytes the host holds for one container outside the
/// WebAssembly memory: tables, staged output, the log, the shared store
/// (script-api.md section 10.1 rule 7; the security review's finding
/// S-06, TODO-7272).
pub const MAX_HOST_STATE_BYTES: usize = 8 * 1024 * 1024;
/// The operating system's limit on a script-host process's memory, set by
/// the program that runs the library (work package F3).
pub const SCRIPT_PROCESS_MEMORY_BYTES: usize = 128 * 1024 * 1024;

/// The most redefinitions of one colour entry in a second: three flashes
/// (section 10.5; the security review's finding S-10, TODO-7276: more
/// than three flashes a second is unsafe for viewers).
pub const MAX_COLOUR_SETS_PER_SECOND: usize = 6;

/// The most random bytes one call gives: the Web Cryptography standard's
/// quota for `getRandomValues`.
pub const MAX_RANDOM_BYTES: usize = 65_536;
/// The most things a context may mark as having a listener (operation LISTEN).
pub const MAX_LISTENS: usize = 256;
/// The most kinds of event a thing may be listened to for.
pub const MAX_LISTEN_KINDS: i32 = 64;

/// The network events a step may give a context.
pub const MAX_NETWORK_EVENTS_PER_STEP: usize = 32;
/// The most things a step may report as painted, as ending a cycle, as
/// sound cues or ends, and as faults (the STEP message's bounds).
/// The protocol crate's `MAX_WATCHED_PAINTINGS` is the one definition.
pub const MAX_STEP_PAINTED: usize = protocol::script::MAX_WATCHED_PAINTINGS;
/// See [`MAX_STEP_PAINTED`].
/// The protocol crate's `MAX_CYCLE_ENDS` is the one definition.
pub const MAX_STEP_CYCLE_ENDS: usize = protocol::script::MAX_CYCLE_ENDS;
/// See [`MAX_STEP_PAINTED`].
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_STEP_SOUND_REPORTS: usize = protocol::script::MAX_STEP_SOUND_REPORTS;
/// See [`MAX_STEP_PAINTED`].
/// The protocol crate's constant of the same name is the one definition (`protocol::script`).
pub const MAX_STEP_FAULTS: usize = protocol::script::MAX_STEP_FAULTS;

// The step and second budgets are budgets of whole callbacks, so each is
// at least nothing and the second's holds several steps'.
const _: () = assert!(STEP_TIME_BUDGET.as_nanos() < SECOND_TIME_BUDGET.as_nanos() && SECOND_TIME_BUDGET.as_nanos() < 1_000_000_000);
// The spec's ceiling on the fuel slice.
const _: () = assert!(FUEL_SLICE <= 1_000_000_000);
// A result always fits the shim's buffer, and a payload a result.
const _: () = assert!(MAX_HOST_BYTES <= MAX_RESULT_BYTES && MAX_ERROR_BYTES <= MAX_RESULT_BYTES && MAX_RANDOM_BYTES <= MAX_RESULT_BYTES);
// QuickJS's heap fits the container's memory with room for the module's
// own data, stack and result buffer.
const _: () = assert!((QUICKJS_HEAP_BYTES as usize) + 8 * 1024 * 1024 <= WASM_MEMORY_BYTES);

// What the two halves of the link must agree on beside the bounds taken
// by name above: a step's sound operations fit one batch, the audio
// channels are the batch's voices, and a seed is the message's.
const _: () = assert!(MAX_SOUND_OPERATIONS_PER_STEP as usize == protocol::script::batch::MAX_SOUND_OPERATIONS);
const _: () = assert!(crate::random::SEED_BYTES == protocol::script::SEED_BYTES);
const _: () = assert!(MAX_TEXT_EVENT_BYTES <= protocol::script::MAX_EVENT_PAYLOAD_BYTES);
