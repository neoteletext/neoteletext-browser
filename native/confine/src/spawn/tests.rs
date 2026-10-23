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
