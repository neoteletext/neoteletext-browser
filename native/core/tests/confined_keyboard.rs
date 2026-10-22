//! The phone keyboard process started confined as the trusted core
//! starts it, reaching its relay: name resolution and TLS inside the
//! sandbox of the network role, on the platform this runs on (on Windows,
//! the `AppContainer` with the internet client capability). It needs the
//! internet, so it runs only when asked: `cargo test -- --ignored`.

#![cfg(feature = "phone-keyboard")]

use std::ffi::OsString;
use std::io::BufReader;
use std::process::Command;
use std::time::{Duration, Instant};

use confine::{Role, Spawn};
use protocol::keyboard::{FromKeyboard, read_from_keyboard};

/// How long the relay has to answer, including the handshake.
const RELAY_TIME: Duration = Duration::from_secs(30);

/// Builds the keyboard as the browser is built, and finds it beside this
/// test, as `latency.rs` does.
fn keyboard() -> std::path::PathBuf {
    let mut build = Command::new(env!("CARGO"));
    build.args(["build", "--quiet", "--package", "neoteletext-keyboard"]);
    if !cfg!(debug_assertions) {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success(), "the keyboard did not build");
    let deps = std::env::current_exe().unwrap();
    let directory = deps.parent().and_then(|deps| deps.parent()).unwrap();
    directory.join(format!("neoteletext-keyboard{}", std::env::consts::EXE_SUFFIX))
}

/// Requirements 3.3 and 3.7: the network role, confined, still resolves
/// its relay's name and completes TLS, and says so with the pairing's
/// code; and it ends when its pipe does.
#[test]
#[ignore = "needs the internet"]
fn the_keyboard_reaches_its_relay_confined() {
    let program = keyboard();
    let arguments: [OsString; 0] = [];
    let environment = confine::environment(Role::Network);
    let mut process = confine::spawn(&Spawn {
        program: &program,
        arguments: &arguments,
        role: Role::Network,
        environment: &environment,
        local_network: false,
        writable: None,
    })
    .unwrap();
    eprintln!("started: {}", process.report);
    let mut output = BufReader::new(process.stdout.take().unwrap());
    let started = Instant::now();
    let mut joined = false;
    while started.elapsed() < RELAY_TIME {
        match read_from_keyboard(&mut output) {
            Ok(Some(FromKeyboard::Join { qr, .. })) => {
                assert!(qr.size > 0);
                joined = true;
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(error) => panic!("the keyboard broke the protocol: {error}"),
        }
    }
    drop(process.stdin.take());
    let _ = process.kill();
    let _ = process.wait();
    assert!(joined, "no Join within {RELAY_TIME:?}: the relay was not reached from inside the sandbox");
}
