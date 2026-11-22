//! The script-host process (`neoteletext-common/docs/script-api.md`
//! section 11.1; DEC-7266, TODO-7292): it runs one (tab, channel)'s
//! scripts. The broker starts it at the trusted core's request, with no
//! arguments; it reads the script link's messages and the web client's
//! answers on standard input and writes its own on standard output, and
//! has nothing else: no file, no network, no program, and the operating
//! system's limit on its memory.
//!
//! It takes no option. In particular none leaves it unconfined (the
//! security review's finding S-17, TODO-7284): a script host that cannot
//! confine itself does not run.
//!
//! Its exit status is 0 when it was told to end or its input ended,
//! [`WATCHDOG_EXIT_STATUS`] when its watchdog ended it because an entry
//! into the script engine ran past its limit, and 1 otherwise.

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::io::{BufReader, BufWriter};
use std::process::ExitCode;

use neoteletext_script::watchdog::WATCHDOG_EXIT_STATUS;
use neoteletext_script::{Engine, Settings, TimeLimits, Watchdog};
use ring::rand::{SecureRandom, SystemRandom};
use script_host::{End, serve};

/// The stack of the thread the scripts run on. An optimised build runs
/// the interpreter's instructions as tail calls and needs little; a
/// build with debug assertions runs them in a plain loop whose frames
/// are large (DEC-7334 item 5; TODO-7338 item 5). 8 MiB is the main
/// thread's usual size on Linux and macOS, given here by name so that
/// the scripts' thread has it on every platform, Windows' 1 MiB main
/// thread included, and so that it is counted once in the process's
/// memory limit.
const SCRIPT_STACK_BYTES: usize = 8 * 1024 * 1024;
// The stack is part of what the process may hold beside a container's
// memory at its largest.
const _: () = assert!(
    SCRIPT_STACK_BYTES + neoteletext_script::limits::WASM_MEMORY_BYTES + neoteletext_script::limits::MAX_HOST_STATE_BYTES
        < neoteletext_script::limits::SCRIPT_PROCESS_MEMORY_BYTES
);
// The watchdog's status is no status the program gives for anything else.
const _: () = assert!(WATCHDOG_EXIT_STATUS != 0 && WATCHDOG_EXIT_STATUS != 1);

/// Compiles the engine, starts the watchdog and serves the pipes.
fn run() -> Result<End, String> {
    let engine = Engine::new().map_err(|stop| format!("the script engine's module does not compile: {stop}"))?;
    // The prelude's bytecode now, while the opening is still arriving,
    // and not within the first context's opening (rdm TODO-7386).
    engine.prelude(neoteletext_script::ops::Parts::ALL).map_err(|stop| format!("the script engine's prelude does not compile: {stop}"))?;
    // Every container is given the watchdog: without it only the
    // container's own clock checks hold the limits (section 3.2 rule
    // 5(b); TODO-7338 item 1).
    let watchdog = Watchdog::ending_the_process().map_err(|error| format!("cannot start the watchdog: {error}"))?;
    let settings = Settings { time: TimeLimits::default(), watchdog: Some(watchdog) };
    let random = SystemRandom::new();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    Ok(serve(&mut BufReader::new(stdin.lock()), &mut BufWriter::new(stdout.lock()), &engine, &settings, &mut |seed| random.fill(seed).is_ok()))
}

fn main() -> ExitCode {
    // First, before any thread and before anything is read: its pipes,
    // its memory limit and nothing else (section 11.1).
    match confine::apply(confine::Role::Script, None) {
        Ok(report) => eprintln!("neoteletext-script-host: confined: {report}"),
        Err(error) => {
            eprintln!("neoteletext-script-host: {error}");
            return ExitCode::FAILURE;
        }
    }
    let scripts = std::thread::Builder::new().name("scripts".into()).stack_size(SCRIPT_STACK_BYTES).spawn(run);
    match scripts.map(std::thread::JoinHandle::join) {
        Ok(Ok(Ok(End::Told))) => ExitCode::SUCCESS,
        Ok(Ok(Ok(End::Stopped(reason)))) => {
            eprintln!("neoteletext-script-host: the scripts were stopped ({reason:?})");
            ExitCode::FAILURE
        }
        Ok(Ok(Ok(End::Link(error)) | Err(error))) => {
            eprintln!("neoteletext-script-host: {error}");
            ExitCode::FAILURE
        }
        Ok(Err(_)) => {
            eprintln!("neoteletext-script-host: the scripts' thread ended abnormally");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("neoteletext-script-host: cannot start the scripts' thread: {error}");
            ExitCode::FAILURE
        }
    }
}
