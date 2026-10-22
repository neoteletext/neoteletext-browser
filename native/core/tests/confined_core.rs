//! The trusted core started by the broker, confined as the browser starts
//! it (decision 56), on the platform this runs on: it gets past its
//! confinement and its broker's channel to its own code. Given
//! `--allow-local` with no address, that code refuses at once, so the core
//! ends there with its own failure, and nothing is opened.

use std::ffi::OsString;
use std::path::Path;

use confine::Role;
use confine::broker::{Handlers, serve, start_core};

/// How the standard library words the exit of a program that returned
/// `ExitCode::FAILURE`, which is 1 on every platform.
const REFUSED: &str = if cfg!(windows) { "exit code: 1" } else { "exit status: 1" };

/// Requirements 3.1 and 3.7. On Windows, before `shell32.dll` was
/// delay-loaded (`core/build.rs`), the core under Win32k lockdown ended
/// before its first line with `exit code: 0xc0000142`.
#[test]
fn the_trusted_core_starts_confined_and_reaches_its_own_code() {
    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext"));
    let programs = program.parent().unwrap();
    let settings = Path::new(env!("CARGO_TARGET_TMPDIR")).join("confined-core-settings");
    std::fs::create_dir_all(&settings).unwrap();
    let arguments = [OsString::from("--core"), OsString::from("--allow-local")];
    let core = start_core(program, &arguments, &settings, &confine::environment(Role::Core)).unwrap();
    eprintln!("started: {}", core.report);
    let refuse_read = |_: &Path| Err(String::from("this test reads nothing"));
    let refuse_web = |_: &str| Err(String::from("this test opens nothing"));
    let refuse_key = || Err(String::from("this test keeps no cookies"));
    let handlers = Handlers { read_source: &refuse_read, open_web: &refuse_web, environment: &confine::environment, cookie_key: &refuse_key };
    let exit = serve(core, programs, &handlers).unwrap();
    assert_eq!(exit.text, REFUSED);
}
