//! The sandbox from the parent's side: what a helper is given, and that
//! it is not started when it cannot be confined (decision 58).

use super::*;
use crate::environment;
use std::io::{BufRead as _, BufReader};

/// What a spawned copy of this test binary does: prints its environment.
const PROBE: &str = "NEOTELETEXT_CONFINE_ENVIRONMENT_PROBE";

/// The probe: run by `a_helper_gets_only_the_environment_it_is_given`,
/// never on its own.
#[test]
fn environment_probe() {
    if std::env::var_os(PROBE).is_none() {
        return;
    }
    for (name, value) in std::env::vars_os() {
        println!("ENV {}={}", name.to_string_lossy(), value.to_string_lossy());
    }
}

/// Requirement 3.7: a helper's environment is exactly what the trusted
/// core passes; nothing of the core's own leaks. Before this, every
/// helper inherited the core's whole environment. It is given what a
/// decoder is given: on Windows a container cannot be made without the
/// user's profile folders, and without its container a decoder is not
/// started (decision 58).
#[test]
fn a_helper_gets_only_the_environment_it_is_given() {
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::environment_probe"), OsString::from("--nocapture")];
    let mut environment = environment(Role::Decoder);
    environment.extend([(OsString::from(PROBE), OsString::from("1")), (OsString::from("NEOTELETEXT_GIVEN"), OsString::from("yes"))]);
    let mut process =
        spawn(&Spawn { program: &program, arguments: &arguments, role: Role::Decoder, environment: &environment, local_network: false, writable: None })
            .unwrap();
    let output = process.stdout.take().unwrap();
    let names: Vec<String> = BufReader::new(output)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| line.strip_prefix("ENV ").map(|rest| rest.split('=').next().unwrap_or("").to_owned()))
        .collect();
    let _ = process.wait();
    assert!(names.contains(&String::from("NEOTELETEXT_GIVEN")), "{names:?}");
    // Nothing of the core's environment that was not passed, such as
    // its path or shell, is there.
    let passed: Vec<String> = environment.iter().map(|(name, _)| name.to_string_lossy().into_owned()).collect();
    for leaked in ["HOME", "PATH", "USER", "SHELL", "TMPDIR", "USERPROFILE", "APPDATA"].into_iter().filter(|name| !passed.iter().any(|given| given == name)) {
        assert!(!names.contains(&String::from(leaked)), "{leaked} leaked: {names:?}");
    }
    assert!(!process.report.layers.is_empty(), "{}", process.report);
}

/// Decision 58: a helper whose container cannot be made is not
/// started. Without the user's profile folders in its environment an
/// `AppContainer` cannot be created; the code before started the helper
/// with the restricted token alone instead, which is how this crate's
/// own environment test had run unseen without its container.
#[cfg(windows)]
#[test]
fn a_helper_whose_container_cannot_be_made_is_not_started() {
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::environment_probe")];
    let started = spawn(&Spawn { program: &program, arguments: &arguments, role: Role::Decoder, environment: &[], local_network: false, writable: None });
    let refused = match started {
        Ok(mut process) => {
            let _ = process.kill();
            let _ = process.wait();
            format!("started: {}", process.report)
        }
        Err(error) => error.to_string(),
    };
    assert!(refused.starts_with("cannot start in its AppContainer"), "{refused}");
}

#[test]
fn a_report_reads_as_a_list() {
    let report = Report { layers: vec!["network namespace".into(), "Landlock".into(), "seccomp".into()] };
    assert_eq!(report.to_string(), "network namespace, Landlock, seccomp");
}

/// What a spawned copy of this test binary reaches as the interface: the
/// listener's port, and the file it must not read.
#[cfg(windows)]
const INTERFACE_PORT: &str = "NEOTELETEXT_CONFINE_INTERFACE_PORT";
#[cfg(windows)]
const INTERFACE_FILE: &str = "NEOTELETEXT_CONFINE_INTERFACE_FILE";
/// An address on the internet the interface must not reach (Cloudflare's
/// resolver, which answers TLS on 443).
#[cfg(windows)]
const INTERNET: &str = "1.1.1.1:443";
/// How long a connection may take to be refused or not answered.
#[cfg(windows)]
const CONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The interface probe: run by
/// `the_interface_reaches_no_network_and_no_user_file`, never on its own.
#[cfg(windows)]
#[test]
fn interface_probe() {
    let (Some(port), Some(file)) = (std::env::var_os(INTERFACE_PORT), std::env::var_os(INTERFACE_FILE)) else {
        return;
    };
    let reach = |address: String| {
        address.parse().ok().and_then(|address| std::net::TcpStream::connect_timeout(&address, CONNECT_WAIT).ok()).map_or("refused", |_| "connected")
    };
    let loopback = reach(format!("127.0.0.1:{}", port.to_string_lossy()));
    let internet = reach(String::from(INTERNET));
    let read = std::fs::read(file).map_or("refused", |_| "read");
    println!("REACH loopback={loopback}; internet={internet}; file={read}");
}

/// Decision 90 (finding F-04): the interface runs in an `AppContainer`
/// with no capabilities, so it reaches no network and reads none of the
/// user's files. Before, under its restricted token at low integrity, it
/// connected to this test's listener and read a file in the user's
/// profile folder.
#[cfg(windows)]
#[test]
fn the_interface_reaches_no_network_and_no_user_file() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let profile = std::path::PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
    let file = profile.join("neoteletext-confine-probe.txt");
    std::fs::write(&file, b"the user's").unwrap();
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::interface_probe"), OsString::from("--nocapture")];
    let mut environment = environment(Role::Interface);
    environment.extend([(OsString::from(INTERFACE_PORT), OsString::from(port.to_string())), (OsString::from(INTERFACE_FILE), file.clone().into_os_string())]);
    let mut process =
        spawn(&Spawn { program: &program, arguments: &arguments, role: Role::Interface, environment: &environment, local_network: false, writable: None })
            .unwrap();
    let output = process.stdout.take().unwrap();
    let line = BufReader::new(output).lines().map_while(Result::ok).find(|line| line.starts_with("REACH")).unwrap_or_default();
    let _ = process.wait();
    let _ = std::fs::remove_file(&file);
    drop(listener);
    for expected in ["loopback=refused", "internet=refused", "file=refused"] {
        assert!(line.contains(expected), "{expected}: {line} ({})", process.report);
    }
    assert!(process.report.layers.iter().any(|layer| layer == "AppContainer"), "{}", process.report);
}
