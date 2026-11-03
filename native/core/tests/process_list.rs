//! Requirement 3.1 (REQ-667): the browser started whole runs exactly the
//! processes it names, each its own program: the broker, the trusted core,
//! the interface, the sound helper, a decoder for each channel awake (a
//! tab's channel sleeps without one by decision 47) and a fetcher for each
//! web channel, and no phone keyboard until one is opened.
//!
//! The browser opens a window, so this runs only in a desktop session: on
//! Linux under an X or Wayland display, on macOS in the logged-in session,
//! on Windows in the logged-in desktop session (not the `ssh` session,
//! which has no window station; `DEVELOPMENT.md` section 12.1). It is
//! ignored otherwise, and run by hand with `--ignored` where there is one.
#![cfg(all(feature = "interaction", feature = "links"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "a test")]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

mod reference_channel;
use reference_channel::{Killed, Site, cargo, programs};

/// A ceiling on the browser's start: its helpers start within a second.
const STARTS_WITHIN: Duration = Duration::from_secs(30);
/// How long the browser is watched, once its helpers have started, for one
/// more it must not start: all of them started within 0.3 seconds of the
/// browser on Linux on 4 November 2026.
const NOTHING_MORE_FOR: Duration = Duration::from_secs(3);
/// Where the broker's line for each helper it was asked to start begins
/// (`trusted_core/processes.rs`, `start_helper`).
const STARTED: &str = "neoteletext: the ";
const STARTED_CONFINED: &str = " started confined: ";
/// What the broker says when the sound helper ended for want of an output
/// (`trusted_core/run.rs`).
const SOUND_ENDED: &str = "no sound: the sound helper ended";

/// The browser's other programs, built beside the broker for this test's
/// profile, as a release puts them beside it.
fn build_helpers() {
    let mut build = cargo();
    build.args(["build", "--quiet", "--package", "neoteletext-decoder", "--package", "neoteletext-interface", "--package", "neoteletext-sound"]);
    if !cfg!(debug_assertions) {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success(), "the helpers did not build");
}

/// One process: its parent and its command line.
#[derive(Clone, Debug)]
struct Process {
    parent: u32,
    command: String,
}

/// Every process on this computer, by number.
#[cfg(target_os = "linux")]
fn processes() -> BTreeMap<u32, Process> {
    let mut all = BTreeMap::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else { continue };
        let (Ok(stat), Ok(cmdline)) = (std::fs::read_to_string(entry.path().join("stat")), std::fs::read(entry.path().join("cmdline"))) else { continue };
        // proc(5): the parent is the second field after the command's
        // closing parenthesis, which the command itself may contain.
        let Some(parent) = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(1)).and_then(|parent| parent.parse().ok()) else {
            continue;
        };
        let command = cmdline
            .split(|&byte| byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        all.insert(pid, Process { parent, command });
    }
    all
}

/// Every process on this computer, by number, as `ps` lists them.
#[cfg(target_os = "macos")]
fn processes() -> BTreeMap<u32, Process> {
    let listed = Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid=,command="]).output().unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            Some((pid, Process { parent, command: fields.collect::<Vec<_>>().join(" ") }))
        })
        .collect()
}

/// Every process on this computer, by number, as Windows lists them.
#[cfg(windows)]
fn processes() -> BTreeMap<u32, Process> {
    let script = "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId) $($_.CommandLine)\" }";
    let listed = Command::new("powershell").args(["-NoProfile", "-Command", script]).output().unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            Some((pid, Process { parent, command: fields.collect::<Vec<_>>().join(" ") }))
        })
        .collect()
}

/// The processes descended from `root`, and `root` itself.
fn family(root: u32) -> BTreeMap<u32, Process> {
    let all = processes();
    let mut family: BTreeMap<u32, Process> = all.iter().filter(|(pid, _)| **pid == root).map(|(pid, process)| (*pid, process.clone())).collect();
    loop {
        let more: Vec<(u32, Process)> = all
            .iter()
            .filter(|(pid, process)| !family.contains_key(pid) && family.contains_key(&process.parent))
            .map(|(pid, process)| (*pid, process.clone()))
            .collect();
        if more.is_empty() {
            return family;
        }
        family.extend(more);
    }
}

/// What each process is, by the program it runs: the broker and the core
/// are one program, the core started with `--core`.
fn role(command: &str) -> String {
    let mut words = command.split_whitespace();
    let program = words.next().map(Path::new).and_then(Path::file_stem).and_then(|stem| stem.to_str()).unwrap_or_default().to_owned();
    if program == "neoteletext" && words.next() == Some("--core") { String::from("neoteletext --core") } else { program }
}

/// How many processes run each program. A `neoteletext` started by a
/// `neoteletext` is the core whatever its command line shows: on Windows,
/// in the virtual machine on 4 November 2026, the listing did not show the
/// core's `--core`, and the census counted two brokers.
fn census(family: &BTreeMap<u32, Process>) -> BTreeMap<String, usize> {
    let mut census = BTreeMap::new();
    for process in family.values() {
        let mut named = role(&process.command);
        let parent_is_the_program = family.get(&process.parent).is_some_and(|parent| role(&parent.command) == "neoteletext");
        if named == "neoteletext" && parent_is_the_program {
            named = String::from("neoteletext --core");
        }
        *census.entry(named).or_default() += 1;
    }
    census
}

// REQ-667 (requirement 3.1), TODO-1244: started whole with a file channel and
// a web channel, each in a tab, the browser runs one broker, one trusted core,
// one interface, one sound helper, a decoder for each channel and a fetcher
// for the web channel, and nothing else: no phone keyboard, which is started
// only when the viewer opens it.
#[test]
#[ignore = "needs a desktop session: run by hand with --ignored on each platform's (Linux X or Wayland, macOS logged in, the Windows virtual machine's desktop through desktop-run.ps1)"]
fn the_browser_started_whole_runs_exactly_the_processes_it_names() {
    let (site_program, _) = programs();
    build_helpers();
    let (site, _heard) = Site::start(&site_program);
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("process-list");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let page = folder.join("news.tti");
    std::fs::write(&page, "PN,10000\r\nOL,1,A page of a file\r\n").unwrap();

    let broker = PathBuf::from(env!("CARGO_BIN_EXE_neoteletext"));
    let web = format!("http://127.0.0.1:{}/terminal/", site.port);
    let mut command = Command::new(&broker);
    // The file first: the first address opened is the tab in front, which
    // is awake; the web channel's tab behind it is awake because its channel
    // holds a push connection. A file tab behind others, never shown, would
    // sleep with no decoder (decision 47, `sleep.rs`), as it should.
    command.args(["--allow-local", "127.0.0.1"]).arg(&page).arg(&web).stderr(Stdio::piped()).stdout(Stdio::null());
    // A settings folder of the test's own: the user's are never touched.
    command.env("HOME", &folder).env("XDG_CONFIG_HOME", folder.join("config")).env("APPDATA", folder.join("appdata"));
    let mut browser = command.spawn().unwrap();
    let root = browser.id();
    let said = BufReader::new(browser.stderr.take().unwrap());
    let mut browser = Killed::new(browser);

    // Each helper the core asks for is said as it starts; the list is read
    // once all have, so a helper that is never asked for is never waited for.
    let (lines, heard) = mpsc::channel();
    std::thread::spawn(move || {
        for line in said.lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                return;
            }
        }
    });
    let expected_starts: BTreeMap<&str, usize> = [("decoder", 2), ("fetcher", 1), ("interface", 1), ("sound helper", 1)].into_iter().collect();
    let mut starts: BTreeMap<String, usize> = BTreeMap::new();
    let deadline = Instant::now() + STARTS_WITHIN;
    let mut log = Vec::new();
    while expected_starts.iter().any(|(name, count)| starts.get(*name).copied().unwrap_or(0) < *count) {
        let line = heard
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|_| panic!("not every helper started: {starts:?}\n{}", log.join("\n")));
        if let Some(name) = line.strip_prefix(STARTED).and_then(|rest| rest.split_once(STARTED_CONFINED)).map(|(name, _)| name.to_owned()) {
            *starts.entry(name).or_default() += 1;
        }
        log.push(line);
    }

    // Nothing more starts: a helper started beyond the expected ones would
    // come within moments of them.
    let quiet_until = Instant::now() + NOTHING_MORE_FOR;
    while let Ok(line) = heard.recv_timeout(quiet_until.saturating_duration_since(Instant::now())) {
        if let Some(name) = line.strip_prefix(STARTED).and_then(|rest| rest.split_once(STARTED_CONFINED)).map(|(name, _)| name.to_owned()) {
            *starts.entry(name).or_default() += 1;
        }
        log.push(line);
    }
    let started: BTreeMap<&str, usize> = starts.iter().map(|(name, count)| (name.as_str(), *count)).collect();
    assert_eq!(started, expected_starts, "the helpers the core asked the broker for:\n{}", log.join("\n"));

    let family = family(root);
    let census = census(&family);
    // A sound helper with no output device to open says so and ends
    // (decision 55): a desktop with no sound, as a headless one, has none
    // left running, which is right, and is waited for to be said.
    let sound = if census.contains_key("neoteletext-sound") {
        1
    } else {
        while !log.iter().any(|line| line.contains(SOUND_ENDED)) {
            let line = heard
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|_| panic!("no sound helper runs, and none was said to end:\n{}", log.join("\n")));
            log.push(line);
        }
        eprintln!("no sound helper: this desktop has no sound output");
        0
    };
    let expected: BTreeMap<String, usize> = [
        ("neoteletext", 1),
        ("neoteletext --core", 1),
        ("neoteletext-interface", 1),
        ("neoteletext-sound", sound),
        ("neoteletext-decoder", 2),
        ("neoteletext-fetcher", 1),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(name, count)| (name.to_owned(), count))
    .collect();
    let listed: Vec<String> = family.iter().map(|(pid, process)| format!("{pid} (parent {}): {}", process.parent, process.command)).collect();
    assert_eq!(census, expected, "the processes the browser runs:\n{}", listed.join("\n"));
    browser.kill();
    // A helper outlives the broker only if confinement failed to tie it to
    // its parent; none may be left behind.
    for pid in family.keys().filter(|pid| **pid != root) {
        let _ = Command::new(if cfg!(windows) { "taskkill" } else { "kill" })
            .args(if cfg!(windows) { vec!["/F", "/PID"] } else { vec!["-9"] })
            .arg(pid.to_string())
            .status();
    }
}
