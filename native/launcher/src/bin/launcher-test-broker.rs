//! A stand-in broker for the launcher's tests (`tests/launcher.rs`) and the
//! end-to-end tests (`scripts/updates-e2e.*`), never shipped: a test copies
//! it into a fake version as its broker, with a version mark and a
//! behaviour, `\x02behaviour=<word>\x02`, either appended to its bytes or,
//! by `--make`, written in place into a slot it carries (a macOS program
//! cannot grow: `codesign` refuses bytes after its last segment). It does
//! what the word says, after saying it started as the broker does before
//! it starts the trusted core: `healthy` reports health and ends; `crash`
//! ends at once with a failure; `hang` never reports; `quits` ends
//! successfully, as a user quitting does; `slow` starts only after
//! [`SLOW_START`], then reports health. Two never say they started: `quiet`
//! ends successfully, `mute` hangs. Each run appends its version and
//! behaviour to the file its first argument names.

use std::io::Write as _;
use std::time::Duration;

/// The behaviour's marker, `\x02behaviour=`, reversed, so that the only
/// copy in this program's bytes in reading order is the one a test wrote.
const START_REVERSED: &[u8] = b"=ruoivaheb\x02";
/// The byte that ends the behaviour.
const END: u8 = 0x02;
/// The version mark's start, `\x01neoteletext-version\x01`, reversed for
/// the same reason (`update::mark` keeps its own copy reversed).
const MARK_START_REVERSED: &[u8] = b"\x01noisrev-txeteletoen\x01";
/// The byte that ends the version.
const MARK_END: u8 = 0x01;
/// How long a hanging version hangs: longer than any test waits.
const HANG: Duration = Duration::from_secs(3600);
/// How long a slow version takes to start: longer than the short health
/// deadline the launcher's test gives it, far shorter than its start
/// deadline.
const SLOW_START: Duration = Duration::from_secs(6);
/// The slot `--make` fills: its length is the most a mark and a behaviour
/// may take, and its text is what `--make` finds.
const SLOT_BYTES: usize = 64;
#[used]
static SLOT: [u8; SLOT_BYTES] = *b"launcher-test-broker: the slot for a version mark and behaviour.";

fn forward(reversed: &[u8]) -> Vec<u8> {
    reversed.iter().rev().copied().collect()
}

/// `--make <version> <behaviour> <out>`: a copy of this program at `out`,
/// its slot holding the version's mark and the behaviour.
fn make(own: &[u8], version: &str, behaviour: &str, out: &str) -> Result<(), String> {
    let slot = std::hint::black_box(&SLOT);
    let at = own.windows(SLOT_BYTES).position(|window| window == slot.as_slice()).ok_or("the slot is not in this program")?;
    let mut filled = forward(MARK_START_REVERSED);
    filled.extend_from_slice(version.as_bytes());
    filled.push(MARK_END);
    filled.extend_from_slice(&forward(START_REVERSED));
    filled.extend_from_slice(behaviour.as_bytes());
    filled.push(END);
    if filled.len() > SLOT_BYTES {
        return Err(String::from("the mark and behaviour do not fit the slot"));
    }
    filled.resize(SLOT_BYTES, 0);
    let mut copy = own.to_vec();
    copy.get_mut(at..at.saturating_add(SLOT_BYTES)).ok_or("the slot is cut short")?.copy_from_slice(&filled);
    std::fs::write(out, &copy).map_err(|error| format!("{out}: {error}"))?;
    let own_path = std::env::current_exe().map_err(|error| error.to_string())?;
    let permissions = std::fs::metadata(own_path).map_err(|error| error.to_string())?.permissions();
    std::fs::set_permissions(out, permissions).map_err(|error| format!("{out}: {error}"))
}

fn main() -> std::process::ExitCode {
    let own = std::env::current_exe().and_then(std::fs::read).unwrap_or_default();
    let arguments: Vec<String> = std::env::args_os().skip(1).map(|argument| argument.to_string_lossy().into_owned()).collect();
    if let [flag, version, behaviour, out] = arguments.as_slice()
        && flag == "--make"
    {
        return match make(&own, version, behaviour, out) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("launcher-test-broker: {error}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    let start = forward(START_REVERSED);
    let behaviour = own
        .windows(start.len())
        .rposition(|window| window == start.as_slice())
        .and_then(|at| own.get(at.saturating_add(start.len())..))
        .and_then(|rest| rest.split(|&byte| byte == END).next())
        .map(|word| String::from_utf8_lossy(word).into_owned())
        .unwrap_or_default();
    let version = update::mark::find(&own).map(|version| version.to_string()).unwrap_or_default();
    if let Some(log) = arguments.first()
        && let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(log)
    {
        let _ = writeln!(file, "{version} {behaviour}");
    }
    let say = |line: &str| {
        print!("{line}");
        let _ = std::io::stdout().flush();
    };
    match behaviour.as_str() {
        "quiet" => return std::process::ExitCode::SUCCESS,
        "mute" => {
            std::thread::sleep(HANG);
            return std::process::ExitCode::SUCCESS;
        }
        "slow" => std::thread::sleep(SLOW_START),
        _ => {}
    }
    say(update::layout::STARTED_LINE);
    match behaviour.as_str() {
        "healthy" | "slow" => {
            say(update::layout::HEALTHY_LINE);
            std::process::ExitCode::SUCCESS
        }
        "hang" => {
            std::thread::sleep(HANG);
            std::process::ExitCode::SUCCESS
        }
        "quits" => std::process::ExitCode::SUCCESS,
        _ => std::process::ExitCode::from(3),
    }
}
