//! A stand-in broker for the launcher's tests (`tests/launcher.rs`), never
//! shipped: a test copies it into a fake version as its broker, with a
//! version mark and a behaviour appended to its bytes, as
//! `\x02behaviour=<word>\x02`. It does what the word says: `healthy`
//! reports health and ends; `crash` ends at once with a failure; `hang`
//! never reports; `quiet` ends successfully without reporting. Each run
//! appends its version and behaviour to the file its first argument
//! names.

use std::io::Write as _;
use std::time::Duration;

/// The behaviour's marker in this program's bytes.
const START: &[u8] = b"\x02behaviour=";
/// How long a hanging version hangs: longer than any test waits.
const HANG: Duration = Duration::from_secs(3600);

fn main() -> std::process::ExitCode {
    let own = std::env::current_exe().and_then(std::fs::read).unwrap_or_default();
    let behaviour = own
        .windows(START.len())
        .rposition(|window| window == START)
        .and_then(|at| own.get(at + START.len()..))
        .and_then(|rest| rest.split(|&byte| byte == 0x02).next())
        .map(|word| String::from_utf8_lossy(word).into_owned())
        .unwrap_or_default();
    let version = update::mark::find(&own).map(|version| version.to_string()).unwrap_or_default();
    if let Some(log) = std::env::args_os().nth(1)
        && let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(log)
    {
        let _ = writeln!(file, "{version} {behaviour}");
    }
    match behaviour.as_str() {
        "healthy" => {
            print!("{}", update::layout::HEALTHY_LINE);
            let _ = std::io::stdout().flush();
            std::process::ExitCode::SUCCESS
        }
        "hang" => {
            std::thread::sleep(HANG);
            std::process::ExitCode::SUCCESS
        }
        "quiet" => std::process::ExitCode::SUCCESS,
        _ => std::process::ExitCode::from(3),
    }
}
