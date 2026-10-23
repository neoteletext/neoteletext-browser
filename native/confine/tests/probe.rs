//! Each role confined on Linux, as a helper confines itself, and what it can
//! still do: run through the single-threaded probe program.

#![cfg(target_os = "linux")]

use std::process::Command;

/// The probe's one line, run as `role` with `environment` added.
fn probe(role: &str, environment: &[(&str, &str)]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_confine-probe"));
    command.env("NEOTELETEXT_CONFINE_PROBE", role);
    for (name, value) in environment {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.lines().find(|line| line.starts_with("PROBE")).map_or_else(|| panic!("no probe line in {text}"), str::to_owned)
}

/// Requirement 3.4: a decoder has every layer, and can open no file, start
/// no program and make no connection.
#[test]
fn a_decoder_can_do_nothing_but_compute() {
    let line = probe("decoder", &[]);
    assert!(line.starts_with("PROBE report=network namespace, Landlock, seccomp;"), "{line}");
    for refused in ["file=refused", "program=refused", "tcp=refused", "write=refused"] {
        assert!(line.contains(refused), "{line}");
    }
}

/// Requirement 3.3: a fetcher may connect but not write or start programs.
#[test]
fn a_fetcher_has_the_network_and_nothing_else() {
    let line = probe("network", &[]);
    assert!(line.starts_with("PROBE report=Landlock, seccomp;"), "{line}");
    assert!(line.contains("program=refused") && line.contains("write=refused"), "{line}");
    assert!(line.contains("tcp=no listener") || line.contains("tcp=connected"), "the network is open: {line}");
}

/// `installation-and-updates.md` section 8.1: the updater reaches its one
/// port and no other, writes its staging folder and nothing else, and
/// starts nothing.
#[test]
fn the_updater_reaches_one_port_and_writes_only_its_staging_folder() {
    let staging = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("probe-staging");
    std::fs::create_dir_all(&staging).unwrap();
    let line = probe("updater", &[("NEOTELETEXT_CONFINE_PROBE_WRITABLE", staging.to_str().unwrap())]);
    assert!(line.starts_with("PROBE report=Landlock, seccomp;"), "{line}");
    assert!(line.contains("tcp=no listener") || line.contains("tcp=connected"), "its port is open: {line}");
    for expected in ["other-port=refused", "program=refused", "write=refused", "own-folder=wrote"] {
        assert!(line.contains(expected), "{expected}: {line}");
    }
}

/// Decision 58: a helper that cannot have its network namespace, as on a
/// distribution that forbids user namespaces, is not run without it.
/// Before, it ran and reported "without a network namespace".
#[test]
fn a_helper_without_its_namespace_does_not_run() {
    let line = probe("decoder-without-namespaces", &[]);
    assert!(line.starts_with("PROBE refused: cannot make an empty network namespace"), "{line}");
}

/// Decision 58: Landlock is enforced whole. A file to read (here the
/// display's authority file) was given a directory's rights, which left
/// the interface's ruleset enforced only in part.
#[test]
fn a_file_to_read_keeps_landlock_whole() {
    let line = probe("interface", &[("XAUTHORITY", "/etc/hostname")]);
    assert!(line.starts_with("PROBE report=network namespace, Landlock, seccomp;"), "{line}");
}
