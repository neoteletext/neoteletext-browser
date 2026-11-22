//! The WebAssembly container (DEC-7246, DEC-7256; script-api.md sections
//! 3.2 and 10.1): QuickJS compiled to WebAssembly (`qjs/nt-quickjs.wasm`,
//! built by `qjs/build.sh`), interpreted by wasmi under a memory cap and
//! wall-clock limits.
//!
//! The module's only ways out are the host call `nt.call`, the clock check
//! `nt.check`, and the few system calls of WASI preview 1 that the C
//! library's start-up, clock and standard streams import. Each is provided
//! here by hand and no other import exists, so a module that asked for any
//! other would fail to link and never run.
//!
//! **Time** (section 3.2 rules 5 and 6; the security review's finding
//! S-04, TODO-7270). Every entry into the module has a deadline on the
//! monotonic clock, kept in the store. The clock is read against it:
//! whenever a slice of [`FUEL_SLICE`] fuel is spent; before and after
//! every host call and every WASI call; for every
//! [`HOST_COPY_CHECK_BYTES`] a host call copies; and by the module's own
//! memory functions for every [`HOST_COPY_CHECK_BYTES`] they copy or fill
//! (`nt.check`). The module holds no bulk-memory instruction and the
//! engine refuses one that does, so no single instruction does unbounded
//! work. A watchdog thread ([`crate::watchdog`]) stands behind all of
//! this.
//!
//! **Traps** (rule 5 of section 10.1; finding S-09, TODO-7275). Every
//! call into the module, the small ones that allocate and free included,
//! goes through [`Machine::enter`] and so has its own fuel and deadline. A
//! trap of any kind, from any of them, stops the container for good: the
//! module's state is then not trusted, and it is never entered again.

use std::time::{Duration, Instant};

use wasmi::errors::HostError;
use wasmi::{
    Caller, CompilationMode, Config, CustomFuelCosts, Extern, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode, TypedFunc,
    TypedResumableCall, WasmParams, WasmResults,
};

use crate::host::{Host, Payload, Reply};
use crate::limits::{
    BULK_BYTES_PER_FUEL, CONTAINER_START_LIMIT, CONTEXTS, FUEL_PER_BYTE_TRANSLATED, FUEL_PER_BYTE_VALIDATED, FUEL_SLICE, HOST_COPY_CHECK_BYTES,
    MAX_ERROR_BYTES, MAX_EVENT_PAYLOAD_BYTES, MAX_HOST_BYTES, MAX_HOST_INTS, MAX_RESULT_BYTES, MAX_SCRIPT_BYTES, MAX_WRITE_BYTES, MAX_WRITE_VECTORS,
    QUICKJS_HEAP_BYTES, QUICKJS_STACK_BYTES, WASM_MAX_RECURSION, WASM_MEMORY_BYTES,
};
use protocol::script::{ContextId, FailReason};

use crate::link::Context as _;
use crate::watchdog::Watchdog;

/// The module: QuickJS, the host-boundary shim and the module's own
/// memory functions, built by `qjs/build.sh`. The gates check this file
/// against its recorded SHA-256 and rebuild it (TODO-7277).
pub const MODULE: &[u8] = include_bytes!("../qjs/nt-quickjs.wasm");

/// The module the host's own functions are imported from.
const NT: &str = "nt";
/// The WASI preview 1 module name.
const WASI: &str = "wasi_snapshot_preview1";
/// The module's imports, each by its module and name: the closed list
/// (the crate's tests compare the module with it).
pub const IMPORTS: [(&str, &str); 7] =
    [(NT, "call"), (NT, "check"), (WASI, "clock_time_get"), (WASI, "fd_write"), (WASI, "fd_close"), (WASI, "fd_seek"), (WASI, "fd_fdstat_get")];
/// WASI's error numbers (wasi-libc `__wasi_errno_t`): success, a bad file
/// descriptor.
const ERRNO_SUCCESS: i32 = 0;
const ERRNO_BADF: i32 = 8;
/// WASI's clocks (`__wasi_clockid_t`): the time of day; the others count
/// from the container's start.
const CLOCK_REALTIME: i32 = 0;
/// The standard output and error descriptors, the only two written to.
const STDOUT: i32 = 1;
const STDERR: i32 = 2;
/// The bytes of one buffer descriptor of `fd_write` (`__wasi_ciovec_t`: a
/// pointer and a length, 32 bits each).
const WRITE_VECTOR_BYTES: usize = 8;
/// The log level engine output is given: a warning.
const ENGINE_LOG_LEVEL: u8 = 3;
/// What a payload is, to the shim and from it (`qjs/shim.c`).
const PAYLOAD_NONE: i32 = 0;
const PAYLOAD_STRING: i32 = 1;
const PAYLOAD_BYTES: i32 = 2;
const PAYLOAD_INTS: i32 = 3;
/// A host call's result words (`qjs/shim.c`): status, what the result is,
/// a whole-number result, the length of a string or byte result.
const RESULT_WORDS: usize = 4;
const STATUS_OK: i32 = 0;
const STATUS_RANGE: i32 = 1;
const STATUS_TYPE: i32 = 2;
const STATUS_ERROR: i32 = 3;
const RESULT_UNDEFINED: i32 = 0;
const RESULT_INT: i32 = 1;
const RESULT_STRING: i32 = 2;
const RESULT_BYTES: i32 = 3;
/// The bits of an entry's result above the dispatch function's own
/// (`qjs/shim.c`): something was thrown and not caught; a promise was
/// rejected and nobody handled it; QuickJS's heap was full.
pub const ENTRY_THREW: i32 = 1 << 8;
/// See [`ENTRY_THREW`].
pub const ENTRY_REJECTED: i32 = 1 << 9;
/// See [`ENTRY_THREW`].
pub const ENTRY_MEMORY: i32 = 1 << 10;
/// The shim's `nt_limit` questions, and what each must answer.
const SHIM_LIMITS: [(i32, usize, &str); 5] = [
    (1, MAX_HOST_INTS + 1, "MAX_HOST_INTS"),
    (2, MAX_RESULT_BYTES, "MAX_RESULT_BYTES"),
    (3, MAX_ERROR_BYTES, "MAX_ERROR_BYTES"),
    (4, HOST_COPY_CHECK_BYTES, "HOST_COPY_CHECK_BYTES"),
    (5, CONTEXTS, "CONTEXTS"),
];
/// The bytes of the scratch space the host keeps in the module's memory
/// for an event's payload and an error's text.
const SCRATCH_BYTES: usize = {
    let step = if MAX_EVENT_PAYLOAD_BYTES > MAX_ERROR_BYTES { MAX_EVENT_PAYLOAD_BYTES } else { MAX_ERROR_BYTES };
    // The network's events are the longest (work package W2).
    if crate::host::web::MAX_NETWORK_EVENT_BYTES > step { crate::host::web::MAX_NETWORK_EVENT_BYTES } else { step }
};
/// The longest name a script is run under, in bytes.
const MAX_SCRIPT_NAME_BYTES: usize = 256;

/// Why a container was stopped for good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// An entry passed its time limit, or the container its budget.
    Time,
    /// QuickJS's heap was full, or the container's memory was refused.
    Memory,
    /// The interpreter's stack limit was passed.
    Stack,
    /// The module trapped, or the host found its state wrong.
    Trap(String),
    /// The container could not be made: a fault of the module or the
    /// build, not of any script.
    Container(String),
}

impl Stop {
    /// The reason, as the `FAILED` message gives it.
    #[must_use]
    pub const fn reason(&self) -> FailReason {
        match self {
            Self::Time => FailReason::Time,
            Self::Memory => FailReason::Memory,
            Self::Stack => FailReason::Stack,
            Self::Trap(_) | Self::Container(_) => FailReason::Trap,
        }
    }
}

impl core::fmt::Display for Stop {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Time => write!(formatter, "the script ran past its time limit and was stopped"),
            Self::Memory => write!(formatter, "the script used all the memory it may and was stopped"),
            Self::Stack => write!(formatter, "the script's calls nested past the interpreter's limit and it was stopped"),
            Self::Trap(message) => write!(formatter, "the container trapped and was stopped: {message}"),
            Self::Container(message) => write!(formatter, "the container could not be made: {message}"),
        }
    }
}

impl core::error::Error for Stop {}

/// The host error an import returns when the entry's deadline has passed:
/// it ends the call as a trap that [`Machine::enter`] reads as time.
#[derive(Debug)]
struct TimeUp;

impl core::fmt::Display for TimeUp {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "the time limit passed")
    }
}

impl HostError for TimeUp {}

/// What the store holds beside the module.
pub struct State {
    /// The container's host: its tables, timers and outputs.
    pub host: Host,
    limits: StoreLimits,
    /// When the entry running now must have ended.
    deadline: Option<Instant>,
    /// Where the shim keeps QuickJS's heap count in the module's memory.
    heap_address: Option<usize>,
}

impl State {
    /// A trap when the entry's deadline has passed.
    fn check_time(&self) -> Result<(), wasmi::Error> {
        match self.deadline {
            Some(deadline) if Instant::now() > deadline => Err(wasmi::Error::host(TimeUp)),
            _ => Ok(()),
        }
    }
}

/// Reads `length` bytes at `pointer`, or none when they are not all inside `memory`.
fn read(memory: &[u8], pointer: i32, length: usize) -> Option<&[u8]> {
    let start = usize::try_from(u32::from_ne_bytes(pointer.to_ne_bytes())).ok()?;
    memory.get(start..start.checked_add(length)?)
}

/// Writes `bytes` at `pointer`; false when they do not fit.
fn write(memory: &mut [u8], pointer: i32, bytes: &[u8]) -> bool {
    let Ok(start) = usize::try_from(u32::from_ne_bytes(pointer.to_ne_bytes())) else { return false };
    let Some(end) = start.checked_add(bytes.len()) else { return false };
    match memory.get_mut(start..end) {
        Some(target) => {
            target.copy_from_slice(bytes);
            true
        }
        None => false,
    }
}

/// A trap for an import given a pointer or a length outside the module's
/// memory, or a count past its bound: the module is not behaving as the
/// shim does, so its state is not trusted (rule 5).
fn outside(what: &str) -> wasmi::Error {
    wasmi::Error::new(format!("an import was called with {what}"))
}

/// The module's memory and the store's state.
fn memory_of<'a>(caller: &'a mut Caller<'_, State>) -> Result<(&'a mut [u8], &'a mut State), wasmi::Error> {
    let Some(Extern::Memory(memory)) = caller.get_export("memory") else { return Err(wasmi::Error::new("the module exports no memory")) };
    Ok(memory.data_and_store_mut(caller))
}

/// The arguments of the import `nt.call`, as the shim passes them.
struct CallArguments {
    context: i32,
    op: i32,
    ints: i32,
    n_ints: i32,
    payload: i32,
    n_payload: i32,
    payload_kind: i32,
    result: i32,
    out: i32,
    out_capacity: i32,
}

/// The host call (section 10.1): bounds each length before a byte is read
/// from the module's memory, gives the operation to the host, and writes
/// its reply back, bounded too. The clock is read before and after, and
/// for every [`HOST_COPY_CHECK_BYTES`] copied.
fn host_call(caller: &mut Caller<'_, State>, arguments: &CallArguments) -> Result<(), wasmi::Error> {
    let (data, state) = memory_of(caller)?;
    state.check_time()?;
    let count = usize::try_from(arguments.n_ints).map_err(|_| outside("a negative count of whole numbers"))?;
    if count > MAX_HOST_INTS {
        return Err(outside("more whole numbers than MAX_HOST_INTS"));
    }
    let raw = read(data, arguments.ints, count.saturating_mul(size_of::<i32>())).ok_or_else(|| outside("whole numbers outside its memory"))?;
    let mut numbers = [0_i32; MAX_HOST_INTS];
    for (slot, word) in numbers.iter_mut().zip(raw.as_chunks::<4>().0) {
        *slot = i32::from_le_bytes(*word);
    }
    let numbers = numbers.get(..count).unwrap_or(&[]);
    if let Some(address) = state.heap_address
        && let Some(bytes) = data.get(address..address.saturating_add(size_of::<u32>()))
    {
        state.host.heap_bytes = usize::try_from(u32::from_le_bytes(bytes.try_into().unwrap_or_default())).unwrap_or(usize::MAX);
    }
    let length = usize::try_from(arguments.n_payload).map_err(|_| outside("a negative payload length"))?;
    let reply = if length > MAX_HOST_BYTES {
        // Bound before copying: nothing of the payload is read.
        Reply::Range(format!("MAX_HOST_BYTES: more than {MAX_HOST_BYTES} bytes in one host call"))
    } else {
        let bytes = read(data, arguments.payload, length).ok_or_else(|| outside("a payload outside its memory"))?;
        // The operations read the payload where it lies; the clock is read
        // for each piece of it they may go through.
        for _ in 0..length.checked_div(HOST_COPY_CHECK_BYTES).unwrap_or(0) {
            state.check_time()?;
        }
        let payload = match arguments.payload_kind {
            PAYLOAD_NONE => Payload::None,
            PAYLOAD_STRING => Payload::Text(bytes),
            PAYLOAD_BYTES => Payload::Bytes(bytes),
            PAYLOAD_INTS => Payload::Ints(bytes),
            _ => return Err(outside("a payload of no known kind")),
        };
        state.host.call(arguments.context, arguments.op, numbers, payload)
    };
    let capacity = usize::try_from(arguments.out_capacity).unwrap_or(0).min(MAX_RESULT_BYTES);
    let (status, kind, value, bytes): (i32, i32, i32, Vec<u8>) = match reply {
        Reply::Undefined => (STATUS_OK, RESULT_UNDEFINED, 0, Vec::new()),
        Reply::Int(value) => (STATUS_OK, RESULT_INT, value, Vec::new()),
        Reply::Text(text) => (STATUS_OK, RESULT_STRING, 0, text.into_bytes()),
        Reply::Bytes(bytes) => (STATUS_OK, RESULT_BYTES, 0, bytes),
        Reply::Range(message) => (STATUS_RANGE, RESULT_STRING, 0, message.into_bytes()),
        Reply::Type(message) => (STATUS_TYPE, RESULT_STRING, 0, message.into_bytes()),
        Reply::Error(message) => (STATUS_ERROR, RESULT_STRING, 0, message.into_bytes()),
    };
    // A result longer than the shim's buffer is the host's own fault; it
    // is never written past the bound.
    let bytes = bytes.get(..bytes.len().min(capacity)).unwrap_or(&[]);
    for piece in bytes.chunks(HOST_COPY_CHECK_BYTES).skip(1) {
        let _ = piece;
        state.check_time()?;
    }
    if !write(data, arguments.out, bytes) {
        return Err(outside("a result buffer outside its memory"));
    }
    let mut words = [0_u8; RESULT_WORDS * size_of::<i32>()];
    let length = i32::try_from(bytes.len()).unwrap_or(0);
    for (slot, word) in words.as_chunks_mut::<4>().0.iter_mut().zip([status, kind, value, length]) {
        *slot = word.to_le_bytes();
    }
    if !write(data, arguments.result, &words) {
        return Err(outside("result words outside its memory"));
    }
    state.check_time()
}

/// `fd_write`: standard output and error go to the log, and nothing else
/// is written. Bounded by what the log could keep before anything is read
/// (finding S-09 item 1, TODO-7275): at most [`MAX_WRITE_VECTORS`] buffer
/// descriptors and [`MAX_WRITE_BYTES`] bytes, whatever the module names.
fn fd_write(caller: &mut Caller<'_, State>, descriptor: i32, vectors: i32, count: i32, written: i32) -> Result<i32, wasmi::Error> {
    let (data, state) = memory_of(caller)?;
    state.check_time()?;
    if descriptor != STDOUT && descriptor != STDERR {
        return Ok(ERRNO_BADF);
    }
    let count = usize::try_from(count).map_err(|_| outside("a negative count of buffers"))?.min(MAX_WRITE_VECTORS);
    let table = read(data, vectors, count.saturating_mul(WRITE_VECTOR_BYTES)).ok_or_else(|| outside("buffer descriptors outside its memory"))?;
    let mut text = Vec::new();
    // What the module is told was written: all of every buffer looked at,
    // so that the C library does not come back with the rest.
    let mut taken = 0_u32;
    for pair in table.as_chunks::<WRITE_VECTOR_BYTES>().0 {
        let (pointer, length) = pair.split_at(size_of::<i32>());
        let pointer = i32::from_le_bytes(pointer.try_into().unwrap_or_default());
        let length = u32::from_le_bytes(length.try_into().unwrap_or_default());
        taken = taken.saturating_add(length);
        let room = MAX_WRITE_BYTES.saturating_sub(text.len());
        let wanted = usize::try_from(length).unwrap_or(usize::MAX).min(room);
        let bytes = read(data, pointer, wanted).ok_or_else(|| outside("a buffer outside its memory"))?;
        text.extend_from_slice(bytes);
    }
    for line in String::from_utf8_lossy(&text).lines().filter(|line| !line.trim().is_empty()) {
        state.host.log_line(ENGINE_LOG_LEVEL, None, line);
    }
    if !write(data, written, &taken.to_le_bytes()) {
        return Err(outside("a count outside its memory"));
    }
    state.check_time()?;
    Ok(ERRNO_SUCCESS)
}

/// `clock_time_get`: the clock a script is given, which moves only with
/// the presentation tick (departures D13 and D14), as nanoseconds.
fn clock_time_get(caller: &mut Caller<'_, State>, clock: i32, time: i32) -> Result<i32, wasmi::Error> {
    let (data, state) = memory_of(caller)?;
    state.check_time()?;
    let now = if clock == CLOCK_REALTIME { state.host.clock_now_ms().saturating_mul(1_000_000) } else { state.host.monotonic_ns() };
    if !write(data, time, &now.to_le_bytes()) {
        return Err(outside("a time outside its memory"));
    }
    Ok(ERRNO_SUCCESS)
}

/// Defines the imports: the host call, the clock check and the WASI calls.
fn link(linker: &mut Linker<State>) -> Result<(), wasmi::Error> {
    #[expect(clippy::too_many_arguments, reason = "the shim's import takes these ten words")]
    fn call(
        mut caller: Caller<'_, State>,
        context: i32,
        op: i32,
        ints: i32,
        n_ints: i32,
        payload: i32,
        n_payload: i32,
        payload_kind: i32,
        result: i32,
        out: i32,
        out_capacity: i32,
    ) -> Result<(), wasmi::Error> {
        host_call(&mut caller, &CallArguments { context, op, ints, n_ints, payload, n_payload, payload_kind, result, out, out_capacity })
    }
    linker.func_wrap(NT, "call", call)?;
    linker.func_wrap(NT, "check", |caller: Caller<'_, State>| -> Result<(), wasmi::Error> { caller.data().check_time() })?;
    linker.func_wrap(WASI, "clock_time_get", |mut caller: Caller<'_, State>, clock: i32, _precision: i64, time: i32| -> Result<i32, wasmi::Error> {
        clock_time_get(&mut caller, clock, time)
    })?;
    linker.func_wrap(
        WASI,
        "fd_write",
        |mut caller: Caller<'_, State>, descriptor: i32, vectors: i32, count: i32, written: i32| -> Result<i32, wasmi::Error> {
            fd_write(&mut caller, descriptor, vectors, count, written)
        },
    )?;
    // No file exists: each is refused, with the clock read as for any import.
    linker.func_wrap(WASI, "fd_close", |caller: Caller<'_, State>, _descriptor: i32| -> Result<i32, wasmi::Error> {
        caller.data().check_time()?;
        Ok(ERRNO_BADF)
    })?;
    linker.func_wrap(WASI, "fd_seek", |caller: Caller<'_, State>, _descriptor: i32, _offset: i64, _whence: i32, _new: i32| -> Result<i32, wasmi::Error> {
        caller.data().check_time()?;
        Ok(ERRNO_BADF)
    })?;
    linker.func_wrap(WASI, "fd_fdstat_get", |caller: Caller<'_, State>, _descriptor: i32, _stat: i32| -> Result<i32, wasmi::Error> {
        caller.data().check_time()?;
        Ok(ERRNO_BADF)
    })?;
    Ok(())
}

/// The engine every container of a process shares: its compiled module is
/// kept, so a second container starts without compiling again.
#[derive(Clone)]
pub struct Engine {
    interpreter: wasmi::Engine,
    module: Module,
    /// How long compiling the module took.
    pub compile_time: Duration,
}

impl Engine {
    /// Compiles the script engine's module.
    ///
    /// # Errors
    /// When the module does not compile, which is a fault of the build.
    pub fn new() -> Result<Self, Stop> {
        Self::with_module(MODULE)
    }

    /// Compiles `module` as the script engine's module would be: for the
    /// tests and fuzz targets that give the imports a module of their own.
    ///
    /// # Errors
    /// When the module does not compile, or holds what the engine refuses.
    pub fn with_module(module: &[u8]) -> Result<Self, Stop> {
        let began = Instant::now();
        let interpreter = wasmi::Engine::new(&Self::config());
        let module = Module::new(&interpreter, module).map_err(|error| Stop::Container(error.to_string()))?;
        Ok(Self { interpreter, module, compile_time: began.elapsed() })
    }

    /// The interpreter's settings: fuel metering on; the call depth
    /// capped; and no WebAssembly proposal the module does not need. Bulk
    /// memory above all is off, so that a module holding `memory.copy` or
    /// `memory.fill` is refused when it is compiled (section 3.2 rule
    /// 5(a)), and should one ever run its bytes are charged in full.
    fn config() -> Config {
        let mut config = Config::default();
        config.consume_fuel(true);
        config.set_max_recursion_depth(WASM_MAX_RECURSION);
        config.compilation_mode(CompilationMode::LazyTranslation);
        config.wasm_bulk_memory(false);
        config.wasm_tail_call(false);
        config.wasm_multi_memory(false);
        config.wasm_extended_const(false);
        config.wasm_custom_page_sizes(false);
        config.wasm_wide_arithmetic(false);
        config.fuel_cost(CustomFuelCosts {
            bytes_copied_per_fuel: BULK_BYTES_PER_FUEL,
            fuel_per_bytes_translated: FUEL_PER_BYTE_TRANSLATED,
            fuel_per_bytes_validated: FUEL_PER_BYTE_VALIDATED,
        });
        config
    }

    /// The module's imports, each as its module and name.
    #[must_use]
    pub fn imports(&self) -> Vec<(String, String)> {
        self.module.imports().map(|import| (import.module().to_owned(), import.name().to_owned())).collect()
    }
}

/// The exports the host calls.
struct Exports {
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<i32, ()>,
    context_open: TypedFunc<(i32, i64), i32>,
    context_close: TypedFunc<i32, i32>,
    eval: TypedFunc<(i32, i32, i32, i32, i32), i32>,
    event: TypedFunc<EventArguments, i32>,
    error: TypedFunc<(i32, i32), i32>,
}

/// `nt_event`'s arguments: the context, the kind, four whole numbers, the
/// payload's pointer, length and kind.
type EventArguments = (i32, i32, i32, i32, i32, i32, i32, i32, i32);

/// An event's payload, as the host gives it to the module.
#[derive(Debug, Clone, Copy)]
pub enum Given<'a> {
    /// None: `undefined`.
    None,
    /// A string.
    Text(&'a str),
    /// A `Uint8Array`.
    Bytes(&'a [u8]),
    /// An `Int32Array`.
    Ints(&'a [i32]),
}

/// How an entry into a context ended, when the container goes on.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    /// The dispatch function's own bits (see [`crate::ops::DispatchBit`]).
    pub dispatch: i32,
    /// Something was thrown and nobody caught it.
    pub threw: bool,
    /// A promise was rejected and nobody handled it.
    pub rejected: bool,
    /// The first failure's description.
    pub error: Option<String>,
    /// How long it ran on the monotonic clock, host calls included.
    pub elapsed: Duration,
}

/// One container: an instance of the module, with its store.
pub struct Machine {
    store: Store<State>,
    memory: Memory,
    exports: Exports,
    /// The host's scratch space in the module's memory.
    scratch: i32,
    stopped: Option<Stop>,
    watchdog: Option<Watchdog>,
    /// How long making the container took: instantiating the module and
    /// starting the C library and QuickJS.
    pub start_time: Duration,
    /// The longest any one slice of fuel took to spend, and how many
    /// slices have been spent: the measurements [`FUEL_SLICE`] was chosen by.
    pub longest_slice: Duration,
    /// See `longest_slice`.
    pub slices: u64,
}

/// The stop a failed call into the module means.
fn stop_of(error: &wasmi::Error) -> Stop {
    if error.downcast_ref::<TimeUp>().is_some() {
        return Stop::Time;
    }
    match error.as_trap_code() {
        Some(TrapCode::StackOverflow) => Stop::Stack,
        Some(TrapCode::GrowthOperationLimited) => Stop::Memory,
        _ => Stop::Trap(error.to_string()),
    }
}

impl Machine {
    /// A new container: instantiates the module and starts the C library
    /// and QuickJS, with local time `zone_minutes` east of UTC. No context
    /// exists yet.
    ///
    /// # Errors
    /// When the container cannot be made.
    pub fn new(engine: &Engine, host: Host, zone_minutes: i16, watchdog: Option<Watchdog>) -> Result<Self, Stop> {
        let began = Instant::now();
        let container = |error: wasmi::Error| Stop::Container(error.to_string());
        let limits = StoreLimitsBuilder::new().memory_size(WASM_MEMORY_BYTES).instances(1).memories(1).tables(1).trap_on_grow_failure(false).build();
        let mut store = Store::new(&engine.interpreter, State { host, limits, deadline: None, heap_address: None });
        store.limiter(|state| &mut state.limits);
        let mut linker = Linker::new(&engine.interpreter);
        link(&mut linker).map_err(container)?;
        let instance: Instance = linker.instantiate_and_start(&mut store, &engine.module).map_err(container)?;
        let memory = instance.get_memory(&store, "memory").ok_or_else(|| Stop::Container("the module exports no memory".into()))?;
        let initialise = instance.get_typed_func::<(), ()>(&store, "_initialize").map_err(container)?;
        let init = instance.get_typed_func::<(i32, i32, i32), i32>(&store, "nt_init").map_err(container)?;
        let limit = instance.get_typed_func::<i32, i32>(&store, "nt_limit").map_err(container)?;
        let heap = instance.get_typed_func::<(), i32>(&store, "nt_heap_address").map_err(container)?;
        let exports = Exports {
            alloc: instance.get_typed_func(&store, "nt_alloc").map_err(container)?,
            free: instance.get_typed_func(&store, "nt_free").map_err(container)?,
            context_open: instance.get_typed_func(&store, "nt_context_open").map_err(container)?,
            context_close: instance.get_typed_func(&store, "nt_context_close").map_err(container)?,
            eval: instance.get_typed_func(&store, "nt_eval").map_err(container)?,
            event: instance.get_typed_func(&store, "nt_event").map_err(container)?,
            error: instance.get_typed_func(&store, "nt_error").map_err(container)?,
        };
        let mut machine =
            Self { store, memory, exports, scratch: 0, stopped: None, watchdog, start_time: Duration::ZERO, longest_slice: Duration::ZERO, slices: 0 };
        let deadline = began.checked_add(CONTAINER_START_LIMIT);
        let as_container = |stop: Stop| Stop::Container(stop.to_string());
        machine.enter(initialise, (), deadline).map_err(as_container)?;
        // The module and this host were compiled with the same bounds.
        for (question, expected, name) in SHIM_LIMITS {
            let answer = machine.enter(limit, question, deadline).map_err(as_container)?;
            if usize::try_from(answer).ok() != Some(expected) {
                return Err(Stop::Container(format!("the module was built with {name} as {answer}, not {expected}")));
            }
        }
        let started = machine.enter(init, (QUICKJS_HEAP_BYTES, QUICKJS_STACK_BYTES, i32::from(zone_minutes)), deadline).map_err(as_container)?;
        if started != 0 {
            return Err(Stop::Container(format!("QuickJS did not start ({started})")));
        }
        let address = machine.enter(heap, (), deadline).map_err(as_container)?;
        machine.store.data_mut().heap_address = usize::try_from(address).ok();
        let scratch = i32::try_from(SCRATCH_BYTES).map_err(|_| Stop::Container("the scratch space is too large".into()))?;
        machine.scratch = machine.enter(machine.exports.alloc, scratch, deadline).map_err(as_container)?;
        if machine.scratch == 0 {
            return Err(Stop::Container("the module has no room for the host's scratch space".into()));
        }
        machine.start_time = began.elapsed();
        Ok(machine)
    }

    /// Calls `function` with `parameters` under `deadline`, in slices of
    /// [`FUEL_SLICE`] fuel: when a slice is spent the call stops, the
    /// clock is read, and it resumes only before the deadline. The
    /// watchdog is told of the deadline first and of the return after.
    /// Every call into the module is made here, so every one is bounded
    /// and any trap from any one stops the container for good.
    ///
    /// # Errors
    /// The stop, which is also kept: the container is not entered again.
    fn enter<P: WasmParams, R: WasmResults>(&mut self, function: TypedFunc<P, R>, parameters: P, deadline: Option<Instant>) -> Result<R, Stop> {
        if let Some(stop) = &self.stopped {
            return Err(stop.clone());
        }
        let result = self.bounded(function, parameters, deadline);
        self.store.data_mut().deadline = None;
        if let Some(watchdog) = &self.watchdog {
            watchdog.disarm();
        }
        if let Err(stop) = &result {
            self.stopped = Some(stop.clone());
        }
        result
    }

    fn bounded<P: WasmParams, R: WasmResults>(&mut self, function: TypedFunc<P, R>, parameters: P, deadline: Option<Instant>) -> Result<R, Stop> {
        let Some(deadline) = deadline else { return Err(Stop::Time) };
        if Instant::now() > deadline {
            return Err(Stop::Time);
        }
        self.store.data_mut().deadline = Some(deadline);
        if let Some(watchdog) = &self.watchdog {
            watchdog.arm(deadline);
        }
        self.store.set_fuel(FUEL_SLICE).map_err(|error| Stop::Trap(error.to_string()))?;
        let mut slice_began = Instant::now();
        let mut call = function.call_resumable(&mut self.store, parameters).map_err(|error| stop_of(&error))?;
        loop {
            match call {
                TypedResumableCall::Finished(results) => return Ok(results),
                TypedResumableCall::HostTrap(trap) => return Err(stop_of(trap.host_error())),
                TypedResumableCall::OutOfFuel(more) => {
                    let now = Instant::now();
                    self.longest_slice = self.longest_slice.max(now.saturating_duration_since(slice_began));
                    self.slices = self.slices.saturating_add(1);
                    slice_began = now;
                    if now > deadline {
                        return Err(Stop::Time);
                    }
                    // A function translated on its first call may need
                    // more than a slice at once.
                    self.store.set_fuel(FUEL_SLICE.max(more.required_fuel())).map_err(|error| Stop::Trap(error.to_string()))?;
                    call = more.resume(&mut self.store).map_err(|error| stop_of(&error))?;
                }
            }
        }
    }

    /// The host.
    pub fn host(&mut self) -> &mut Host {
        &mut self.store.data_mut().host
    }

    /// The host, to read.
    #[must_use]
    pub fn host_ref(&self) -> &Host {
        &self.store.data().host
    }

    /// Why the container was stopped, if it was.
    #[must_use]
    pub const fn stopped(&self) -> Option<&Stop> {
        self.stopped.as_ref()
    }

    /// Stops the container for good, for a reason the host found itself
    /// (a budget passed).
    pub fn stop(&mut self, stop: Stop) {
        self.stopped.get_or_insert(stop);
    }

    /// The bytes of the container's linear memory.
    #[must_use]
    pub fn linear_bytes(&self) -> usize {
        self.memory.data_size(&self.store)
    }

    /// QuickJS's heap in use, read from the module's memory.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        let Some(address) = self.store.data().heap_address else { return 0 };
        let data = self.memory.data(&self.store);
        data.get(address..address.saturating_add(size_of::<u32>()))
            .map_or(0, |bytes| usize::try_from(u32::from_le_bytes(bytes.try_into().unwrap_or_default())).unwrap_or(0))
    }

    /// Writes `bytes` into the module's memory at `pointer`.
    fn put(&mut self, pointer: i32, bytes: &[u8]) -> Result<(), Stop> {
        if write(self.memory.data_mut(&mut self.store), pointer, bytes) {
            Ok(())
        } else {
            let stop = Stop::Trap("the module's memory is too small for what the host gives it".into());
            self.stopped = Some(stop.clone());
            Err(stop)
        }
    }

    /// Makes context `context`, its `Math.random` seeded with `seed`.
    ///
    /// # Errors
    /// The stop, when the container is or becomes stopped.
    pub fn open_context(&mut self, context: ContextId, seed: i64, deadline: Option<Instant>) -> Result<(), Stop> {
        let made = self.enter(self.exports.context_open, (context.number(), seed), deadline)?;
        if made == 0 {
            return Ok(());
        }
        // QuickJS could not make a realm: its heap is full.
        let stop = if made == -4 || made == -5 { Stop::Memory } else { Stop::Trap(format!("the module refused to make the context ({made})")) };
        self.stopped = Some(stop.clone());
        Err(stop)
    }

    /// Ends context `context`: its realm is freed and the collector run.
    ///
    /// # Errors
    /// The stop, when the container is or becomes stopped.
    pub fn close_context(&mut self, context: ContextId, deadline: Option<Instant>) -> Result<(), Stop> {
        self.enter(self.exports.context_close, context.number(), deadline).map(|_| ())
    }

    /// Reads an entry's result: its bits, and the error the shim kept.
    fn finish(&mut self, bits: i32, began: Instant, deadline: Option<Instant>) -> Result<Entry, Stop> {
        if bits < 0 {
            let stop = Stop::Trap(format!("the module refused the entry ({bits})"));
            self.stopped = Some(stop.clone());
            return Err(stop);
        }
        let mut entry =
            Entry { dispatch: bits & 0xFF, threw: bits & ENTRY_THREW != 0, rejected: bits & ENTRY_REJECTED != 0, error: None, elapsed: Duration::ZERO };
        if entry.threw || entry.rejected {
            let capacity = i32::try_from(MAX_ERROR_BYTES.min(SCRATCH_BYTES)).unwrap_or(0);
            let length = self.enter(self.exports.error, (self.scratch, capacity), deadline)?;
            let length = usize::try_from(length).unwrap_or(0).min(MAX_ERROR_BYTES);
            entry.error = read(self.memory.data(&self.store), self.scratch, length).map(|bytes| String::from_utf8_lossy(bytes).into_owned());
        }
        entry.elapsed = began.elapsed();
        if bits & ENTRY_MEMORY != 0 {
            // QuickJS's heap was full during the entry: the memory limit
            // was passed, and the container is stopped for good.
            self.stopped = Some(Stop::Memory);
            return Err(Stop::Memory);
        }
        Ok(entry)
    }

    /// Runs `source` in `context` as a classic script named `name`, in
    /// strict mode when `strict`, to end before `deadline`.
    ///
    /// # Errors
    /// The stop, when the container is or becomes stopped. A script that
    /// throws is not a stop: the entry says so.
    pub fn eval(&mut self, context: ContextId, source: &[u8], name: &str, strict: bool, deadline: Option<Instant>) -> Result<Entry, Stop> {
        let began = Instant::now();
        if source.len() > MAX_SCRIPT_BYTES {
            return Err(Stop::Trap("a script longer than MAX_SCRIPT_BYTES was given to the container".into()));
        }
        let name = name.get(..name.len().min(MAX_SCRIPT_NAME_BYTES)).unwrap_or("script");
        let length = i32::try_from(source.len()).unwrap_or(0);
        let name_length = i32::try_from(name.len()).unwrap_or(0);
        // The source and its name, each with a NUL after it.
        let code = self.enter(self.exports.alloc, length.saturating_add(1), deadline)?;
        let label = self.enter(self.exports.alloc, name_length.saturating_add(1), deadline)?;
        if code == 0 || label == 0 {
            self.stopped = Some(Stop::Memory);
            return Err(Stop::Memory);
        }
        self.put(code, source)?;
        self.put(code.saturating_add(length), &[0])?;
        self.put(label, name.as_bytes())?;
        self.put(label.saturating_add(name_length), &[0])?;
        let bits = self.enter(self.exports.eval, (context.number(), code, length, label, i32::from(strict)), deadline)?;
        self.enter(self.exports.free, code, deadline)?;
        self.enter(self.exports.free, label, deadline)?;
        self.finish(bits, began, deadline)
    }

    /// Gives `context` one event, to end before `deadline`.
    ///
    /// # Errors
    /// The stop, when the container is or becomes stopped. A handler that
    /// throws is not a stop: the entry says so.
    pub fn event(&mut self, context: ContextId, kind: i32, numbers: [i32; 4], payload: Given<'_>, deadline: Option<Instant>) -> Result<Entry, Stop> {
        let began = Instant::now();
        let mut words = Vec::new();
        let (bytes, payload_kind): (&[u8], i32) = match payload {
            Given::None => (&[], PAYLOAD_NONE),
            Given::Text(text) => (text.as_bytes(), PAYLOAD_STRING),
            Given::Bytes(bytes) => (bytes, PAYLOAD_BYTES),
            Given::Ints(ints) => {
                words.extend(ints.iter().flat_map(|word| word.to_le_bytes()));
                (&words, PAYLOAD_INTS)
            }
        };
        // A payload longer than the scratch space is cut: the link's
        // bounds keep one from arriving.
        let bytes = bytes.get(..bytes.len().min(SCRATCH_BYTES)).unwrap_or(&[]);
        self.put(self.scratch, bytes)?;
        let [a, b, c, d] = numbers;
        let length = i32::try_from(bytes.len()).unwrap_or(0);
        let bits = self.enter(self.exports.event, (context.number(), kind, a, b, c, d, self.scratch, length, payload_kind), deadline)?;
        self.finish(bits, began, deadline)
    }
}

/// The imports alone, over a memory of the caller's making: the module of
/// [`crate::probe::forwarder`], each of whose exports passes its
/// arguments straight to an import. What a module that has been taken
/// over could ask of the host, for the tests and the fuzz target of the
/// imports (finding S-09 item 5, TODO-7275).
pub struct Probe {
    store: Store<State>,
    instance: Instance,
}

impl Probe {
    /// A probe whose memory is `pages` pages, starting as `image`.
    ///
    /// # Errors
    /// When the forwarding module does not compile or start.
    pub fn new(image: &[u8], pages: usize, host: Host) -> Result<Self, Stop> {
        let container = |error: wasmi::Error| Stop::Container(error.to_string());
        let engine = Engine::with_module(&crate::probe::forwarder(image, pages))?;
        let limits = StoreLimitsBuilder::new().memory_size(WASM_MEMORY_BYTES).instances(1).memories(1).tables(1).build();
        let mut store = Store::new(&engine.interpreter, State { host, limits, deadline: None, heap_address: None });
        store.limiter(|state| &mut state.limits);
        let mut linker = Linker::new(&engine.interpreter);
        link(&mut linker).map_err(container)?;
        store.set_fuel(FUEL_SLICE).map_err(container)?;
        let instance = linker.instantiate_and_start(&mut store, &engine.module).map_err(container)?;
        Ok(Self { store, instance })
    }

    /// Calls the import numbered `import` in [`IMPORTS`] with `arguments`
    /// (each cut to the width its parameter has; missing ones are 0),
    /// to end within `limit`. The import's result, if it has one.
    ///
    /// # Errors
    /// The stop the call would be for a container: a trap, or time.
    pub fn call(&mut self, import: usize, arguments: &[i64], limit: Duration) -> Result<Option<i32>, Stop> {
        let Some((from, what)) = IMPORTS.get(import) else { return Err(Stop::Container("no such import".into())) };
        let Some((parameters, results)) = crate::probe::SIGNATURES.get(import) else { return Err(Stop::Container("no such import".into())) };
        let function = self.instance.get_func(&self.store, &format!("{from}.{what}")).ok_or_else(|| Stop::Container("the forwarder lacks an export".into()))?;
        let values: Vec<wasmi::Val> = (0..parameters.len())
            .map(|index| {
                let value = arguments.get(index).copied().unwrap_or(0);
                if crate::probe::is_wide(import, index) {
                    wasmi::Val::I64(value)
                } else {
                    wasmi::Val::I32(i32::from_ne_bytes(value.to_ne_bytes().first_chunk::<4>().copied().unwrap_or_default()))
                }
            })
            .collect();
        let mut out = vec![wasmi::Val::I32(0); results.len()];
        self.store.data_mut().deadline = Instant::now().checked_add(limit);
        self.store.set_fuel(FUEL_SLICE).map_err(|error| Stop::Trap(error.to_string()))?;
        let called = function.call(&mut self.store, &values, &mut out);
        self.store.data_mut().deadline = None;
        called.map_err(|error| stop_of(&error))?;
        Ok(out.first().and_then(wasmi::Val::i32))
    }

    /// The probe's memory.
    pub fn memory(&mut self) -> &mut [u8] {
        match self.instance.get_memory(&self.store, "memory") {
            Some(memory) => memory.data_mut(&mut self.store),
            None => &mut [],
        }
    }

    /// The host.
    pub fn host(&mut self) -> &mut Host {
        &mut self.store.data_mut().host
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::unwrap_used, clippy::indexing_slicing, reason = "tests")]
mod tests {
    use super::*;

    fn machine() -> Machine {
        Machine::new(&Engine::new().unwrap(), Host::new(), 0, None).unwrap()
    }

    fn soon() -> Option<Instant> {
        Instant::now().checked_add(Duration::from_secs(1))
    }

    /// Section 10.1 rule 5; finding S-09 items 2 and 3 (TODO-7275): a trap
    /// from any call into the module, `nt_free` included, stops the
    /// container for good, and it is never entered again. The spike
    /// ignored `free`'s traps (`let _ = self.exports.free.call(...)`).
    #[test]
    fn a_trap_in_free_stops_the_container_for_good() {
        let mut machine = machine();
        // A pointer far outside the module's memory: the allocator reads
        // the block's header there and the interpreter traps.
        let freed = machine.enter(machine.exports.free, i32::from_ne_bytes(0xFFFF_FF00_u32.to_ne_bytes()), soon());
        assert!(matches!(freed, Err(Stop::Trap(_))), "{freed:?}");
        assert!(matches!(machine.stopped(), Some(Stop::Trap(_))));
        // Never entered again: every later call answers the same stop
        // without running anything.
        let again = machine.enter(machine.exports.alloc, 16, soon());
        assert_eq!(again.err(), machine.stopped().cloned());
        assert!(matches!(machine.open_context(ContextId::Page, 1, soon()), Err(Stop::Trap(_))));
        assert!(matches!(machine.eval(ContextId::Page, b"1", "x", true, soon()), Err(Stop::Trap(_))));
    }

    /// The same for `nt_alloc`: an allocation the container's memory cap
    /// refuses is not a trap (it answers null), but a call past its
    /// deadline is a stop like any other.
    #[test]
    fn a_small_call_past_its_deadline_stops_the_container() {
        let mut machine = machine();
        let past = Instant::now().checked_sub(Duration::from_millis(1));
        assert_eq!(machine.enter(machine.exports.alloc, 16, past).err(), Some(Stop::Time));
        assert_eq!(machine.stopped(), Some(&Stop::Time));
    }

    /// Section 3.2 rule 5(a); TODO-7285 item 1: the engine refuses a
    /// module that holds a bulk-memory instruction, whatever the build
    /// script's own check said.
    #[test]
    fn a_module_with_a_bulk_memory_instruction_is_refused() {
        let refused = Engine::with_module(&crate::probe::bulk_memory_module());
        let Err(Stop::Container(message)) = refused else { panic!("the module was accepted") };
        assert!(message.contains("bulk memory"), "{message}");
    }

    /// DEC-7256 item 4, as rebuilt: the module imports exactly the closed
    /// list, so nothing else of the host can be reached from it.
    #[test]
    fn the_module_imports_exactly_the_closed_list() {
        let found = Engine::new().unwrap().imports();
        let expected: Vec<(String, String)> = IMPORTS.iter().map(|(from, what)| ((*from).to_owned(), (*what).to_owned())).collect();
        let mut sorted = (found.clone(), expected.clone());
        sorted.0.sort();
        sorted.1.sort();
        assert_eq!(sorted.0, sorted.1, "the module imports {found:?}");
    }
}
