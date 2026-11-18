//! The browser's window reaches the screen on macOS (TODO-1258): the broker
//! starts the trusted core confined, which starts the interface under its
//! own profile, and the window server is then asked whether a window of the
//! interface's process is on the screen. Once the window was drawn and never
//! ordered onto the screen, with nothing failing, because the interface's
//! profile lacked the window manager's service (commit `4151484`); only a
//! person looking saw it.
//!
//! It needs a desktop session, and it shows the browser's window for a
//! moment, so it is run by hand:
//! `cargo nextest run -p neoteletext --test window_on_screen --run-ignored only`.
//! The window is opened without taking the focus (`NEOTELETEXT_BACKGROUND`),
//! which only a debug build reads, so a release build has no such check.
#![cfg(all(target_os = "macos", debug_assertions))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "a test")]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use confine::Role;
use confine::broker::{Handlers, serve, start_core};

mod reference_channel;
use reference_channel::cargo;

/// A ceiling on the window's arrival, which takes about a second: the check
/// ends as soon as it is on the screen.
const WINDOW_WITHIN: Duration = Duration::from_secs(30);
/// How often the processes and the window server are asked meanwhile.
const POLL: Duration = Duration::from_millis(250);
/// The level of an application's ordinary windows, `kCGNormalWindowLevel`.
const NORMAL_WINDOW_LEVEL: i64 = 0;

/// The channel the browser is asked to open: a file, which the test's broker
/// refuses to read.
const NO_SUCH_FILE: &str = "/nonexistent/window-on-screen.tti";

/// Asks the window server, through `CGWindowListCopyWindowInfo` from an
/// outside program, for the windows on the screen: one line each, its
/// owner's process number, level, width and height.
const WINDOWS_ON_SCREEN: &str = "ObjC.import('CoreGraphics');
function run() {
    const list = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, $.kCGNullWindowID)));
    return list.map(w => [w.kCGWindowOwnerPID, w.kCGWindowLayer, w.kCGWindowBounds.Width, w.kCGWindowBounds.Height].join(' ')).join('\\n');
}";

/// The helpers the browser starts, built beside the core, where the broker
/// finds them.
const HELPERS: [&str; 4] = ["neoteletext-interface", "neoteletext-decoder", "neoteletext-fetcher", "neoteletext-sound"];

/// The processes `parent` started whose command line holds `pattern`.
fn children(parent: u32, pattern: &str) -> Vec<u32> {
    let output = Command::new("/usr/bin/pgrep").args(["-P", &parent.to_string(), "-f", "--", pattern]).output().unwrap();
    String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| line.trim().parse().ok()).collect()
}

/// The process of the application the person at the computer is using,
/// as `LaunchServices` names it.
fn front_application() -> Option<u32> {
    let front = Command::new("/usr/bin/lsappinfo").arg("front").output().ok()?;
    let name = String::from_utf8_lossy(&front.stdout).trim().to_owned();
    let info = Command::new("/usr/bin/lsappinfo").args(["info", "-only", "pid", &name]).output().ok()?;
    String::from_utf8_lossy(&info.stdout).trim().strip_prefix("\"pid\"=")?.parse().ok()
}

/// Ends the trusted core this test's broker started, however the test ends:
/// the broker's service then ends every helper it started, the window with
/// them.
struct EndsTheCore;

impl Drop for EndsTheCore {
    fn drop(&mut self) {
        for core in children(std::process::id(), "--core") {
            let _ = Command::new("/bin/kill").args(["-TERM", &core.to_string()]).status();
        }
    }
}

/// Whether a window of process `owner`, of an ordinary level and of some
/// size, is on the screen.
fn on_screen(owner: u32) -> bool {
    let output = Command::new("/usr/bin/osascript").args(["-l", "JavaScript", "-e", WINDOWS_ON_SCREEN]).output().unwrap();
    assert!(output.status.success(), "the window server could not be asked: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let fields: Vec<i64> = line.split(' ').filter_map(|field| field.parse().ok()).collect();
        matches!(fields[..], [pid, level, width, height] if pid == i64::from(owner) && level == NORMAL_WINDOW_LEVEL && width > 0 && height > 0)
    })
}

/// REQ-674 and REQ-673: the interface, confined by its profile as the browser
/// confines it, puts its window on the screen. Fails when the profile lacks
/// `com.apple.windowmanager.server` (checked by hand on 3 November 2026), as
/// the code before commit `4151484` did.
#[test]
#[ignore = "needs a macOS desktop session, and shows the browser's window; run by hand"]
fn the_interface_s_window_reaches_the_screen() {
    let mut build = cargo();
    build.args(["build", "--quiet"]);
    for helper in HELPERS {
        build.args(["--package", helper]);
    }
    assert!(build.status().unwrap().success(), "the helpers did not build");

    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext"));
    let programs = program.parent().unwrap().to_path_buf();
    // A home of the test's own, so that the core neither reads nor writes
    // the person's settings and history.
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("window-on-screen-home");
    let settings: PathBuf = home.join("Library/Application Support/NeoTeletext");
    std::fs::create_dir_all(&settings).unwrap();
    let mut core_environment: Vec<(OsString, OsString)> = confine::environment(Role::Core).into_iter().filter(|(name, _)| name != "HOME").collect();
    core_environment.push((OsString::from("HOME"), home.clone().into_os_string()));
    let front = front_application();
    // A file to open, which this test's broker refuses to read, so the tab
    // shows its failure and nothing reaches the network.
    let arguments = [OsString::from("--core"), OsString::from(NO_SUCH_FILE)];
    let core = start_core(program, &arguments, &settings, &core_environment).unwrap();
    let ends_the_core = EndsTheCore;

    let serving = thread::spawn(move || {
        let refuse_read = |_: &Path| Err(String::from("this test reads nothing"));
        let refuse_web = |_: &str| Err(String::from("this test opens nothing"));
        let refuse_key = || Err(String::from("this test remembers no channel"));
        let refuse_health = || Err(String::from("this test has no launcher"));
        let refuse_state = || Err(String::from("this test has no launcher"));
        let refuse_updater = |_: &[String]| Err(String::from("this test has no launcher"));
        let environment = |role: Role| {
            let mut environment = confine::environment(role);
            if role == Role::Interface {
                environment.push((OsString::from("NEOTELETEXT_BACKGROUND"), OsString::from("1")));
            }
            environment
        };
        let handlers = Handlers {
            read_source: &refuse_read,
            open_web: &refuse_web,
            environment: &environment,
            viewer_keys_key: &refuse_key,
            healthy: &refuse_health,
            update_state: &refuse_state,
            updater: &refuse_updater,
            choose: &|_, _, _| Err(String::from("this test shows no dialog")),
            openings: &|| None,
            read_picture: &refuse_read,
            read_font: &refuse_read,
            save: &|_, _| Err(String::from("this test saves nothing")),
            declared: Arc::new(|_| {}),
        };
        serve(core, &programs, &handlers)
    });

    let own = std::process::id();
    let deadline = Instant::now() + WINDOW_WITHIN;
    let mut interface = None;
    let mut seen = false;
    while Instant::now() < deadline {
        interface = interface.or_else(|| children(own, "neoteletext-interface").first().copied());
        if let Some(pid) = interface
            && on_screen(pid)
        {
            seen = true;
            break;
        }
        thread::sleep(POLL);
    }
    let front_meanwhile = front_application();
    drop(ends_the_core);
    let _ = serving.join();
    let interface = interface.expect("the core did not start the interface");
    assert!(seen, "no window of the interface (process {interface}) reached the screen within {WINDOW_WITHIN:?}");
    assert!(front_meanwhile != Some(interface), "the window took the focus from the application in use, process {front:?}");
}
