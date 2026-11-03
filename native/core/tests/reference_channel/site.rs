//! The reference channel, `neoteletext-site terminal`, on a port of its own,
//! with what it says, and the programs a test runs, built for its profile:
//! shared by the tests in `core/tests` and the trusted core's hostile
//! interface harness, in every feature shape.

#![allow(dead_code, reason = "each test uses part of this")]

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A process that ends with the test.
pub struct Killed(Child);

impl Killed {
    /// `child`, ended with the test.
    #[must_use]
    pub fn new(child: Child) -> Self {
        Self(child)
    }

    /// Ends the process now, as a fault would.
    pub fn kill(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Drop for Killed {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Cargo, for a test to build with, without what the test runner set for the
/// package under test: `OUT_DIR` and Cargo's own variables, but for where it
/// keeps things. A build script can watch those, as `ring`'s does: with them
/// set, this build changes what the next ordinary build must rebuild, `ring`
/// and all that sits on it, on every run of the tests.
pub fn cargo() -> Command {
    let mut command = Command::new(env!("CARGO"));
    for (name, _) in std::env::vars_os() {
        let runners = name.to_str().is_some_and(|name| name == "OUT_DIR" || (name.starts_with("CARGO_") && !matches!(name, "CARGO_HOME" | "CARGO_TARGET_DIR")));
        if runners {
            command.env_remove(name);
        }
    }
    command
}

/// Builds `packages`' programs for this test's profile, and returns the
/// folder they are in: with their default features, or with `features`
/// alone (`package/feature`, separated by commas) when given.
pub fn build(packages: &[&str], features: Option<&str>) -> PathBuf {
    let mut build = cargo();
    build.args(["build", "--quiet"]);
    for package in packages {
        build.args(["--package", package]);
    }
    if let Some(features) = features {
        build.arg("--no-default-features");
        if !features.is_empty() {
            build.args(["--features", features]);
        }
    }
    if !cfg!(debug_assertions) {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success(), "{packages:?} did not build");
    // The test runs from target/<profile>/deps; the programs are beside it.
    let deps = std::env::current_exe().unwrap();
    deps.parent().and_then(|deps| deps.parent()).unwrap().to_path_buf()
}

/// How many free ports a site tries.
const PORT_ATTEMPTS: usize = 8;
/// A ceiling on a site's first line, which arrives within milliseconds: on 3
/// November 2026 macOS held one freshly built site at `_dyld_start`, before
/// `main`, for over 30 minutes, and a wait with no ceiling held the full gate
/// with it. A site that has not said it serves by then is ended and another
/// started, as one whose port was taken is.
const SITE_SERVES_WITHIN: Duration = Duration::from_secs(10);

/// The reference channel, on a port of its own.
pub struct Site {
    pub port: u16,
    _process: Killed,
}

impl Site {
    /// Starts it, and returns with what it says from now on, a line at a
    /// time: its requests, and each connection's SELECTs and end.
    pub fn start(program: &Path) -> (Self, mpsc::Receiver<String>) {
        // A free port can be taken by another test's site before this one
        // binds it, so a site that could not serve is started again.
        let (port, site, heard) = (0..PORT_ATTEMPTS)
            .find_map(|_| {
                let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
                let mut site = Command::new(program).args(["terminal", &port.to_string()]).stderr(Stdio::piped()).spawn().unwrap();
                let said = BufReader::new(site.stderr.take().unwrap());
                let site = Killed(site);
                let (lines, heard) = mpsc::channel();
                std::thread::spawn(move || {
                    // Read to the end, heard or not: a site that cannot write
                    // would panic in its session.
                    for line in said.lines().map_while(Result::ok) {
                        let _ = lines.send(line);
                    }
                });
                // A site dropped here is killed, which ends its reader.
                match heard.recv_timeout(SITE_SERVES_WITHIN) {
                    Ok(line) if line.contains("serving") => Some((port, site, heard)),
                    _ => None,
                }
            })
            .expect("the site serves on a free port");
        (Self { port, _process: site }, heard)
    }
}

/// What the site said, up to and including its first line holding `words`.
pub fn said_until(heard: &mpsc::Receiver<String>, words: &str, deadline: Instant) -> Vec<String> {
    let mut lines = Vec::new();
    loop {
        match heard.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => {
                let found = line.contains(words);
                lines.push(line);
                if found {
                    return lines;
                }
            }
            Err(error) => panic!("the site never said {words:?} ({error}); it said {lines:?}"),
        }
    }
}
