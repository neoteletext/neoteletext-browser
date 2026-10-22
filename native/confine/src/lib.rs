//! The processes' sandboxes (requirements 3.3, 3.4, 3.6 and 3.7; decision
//! 52): every helper of the trusted core runs with only what it is for,
//! on every platform, by the platform's own means.
//!
//! - **macOS:** the trusted core starts each helper under a `sandbox-exec`
//!   profile that starts from "deny everything" (`macos`).
//! - **Windows:** the trusted core starts each helper in a job object,
//!   with the process mitigation policies, and in an `AppContainer`, or
//!   for the interface, which needs the desktop, with a restricted token
//!   at low integrity (`windows`).
//! - **Linux:** each helper confines itself first thing: an empty network
//!   namespace for those without the network, Landlock and a seccomp
//!   system-call filter (`linux`).
//!
//! Every layer is required (decision 58: security is paramount, and is
//! never weakened or downgraded). A machine that cannot give a process
//! every layer its role names gets an error and no process, never a
//! process with less; there is no fallback and no switch to one.
//!
//! The two halves are [`spawn`], the parent's side, and [`apply`], a
//! process's own side; each is a no-op on the platforms where the other
//! does the work. The helpers receive everything over their pipes, so no
//! sandbox here has to let anything through on request: they only deny.
//!
//! The parent is the broker (decision 56, `broker`): the one unconfined
//! process, which starts the trusted core confined and every helper for
//! it, so that the core too is confined on every platform.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::Path;
use std::process::ExitStatus;

pub mod broker;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// What a helper is for, which is all it may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// A decoder: its pipes and nothing else.
    Decoder,
    /// A fetcher or the phone keyboard: the network, with the system's
    /// name resolution, and nothing else.
    Network,
    /// The interface: the window system, fonts, text input, the
    /// pasteboard and the Dock; no network, no sound (decision 55).
    Interface,
    /// The trusted core: its settings folder and nothing else; no
    /// network, no programs (decision 56).
    Core,
    /// The sound helper: the sound output and nothing else (decision 55).
    Sound,
}

impl Role {
    /// The helper's name in what it reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Decoder => "decoder",
            Self::Network => "fetcher",
            Self::Interface => "interface",
            Self::Core => "trusted core",
            Self::Sound => "sound",
        }
    }
}

/// What confines a process, as it reports it: every layer its role names,
/// since a process with fewer is not started (decision 58).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The layers in force, in words ("seccomp", "Landlock").
    pub layers: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.layers.join(", "))
    }
}

/// Confines the calling process as `role` says, where a process confines
/// itself: Linux. Elsewhere the broker confined it at its start, and this
/// says so. `writable` is the one folder it may write (the core's
/// settings). Called first thing, before any thread exists: a filter
/// applies to the thread that sets it and those it then starts.
///
/// # Errors
/// When any layer cannot be applied, or the platform has no sandbox; the
/// process then ends, and its parent reports it (decision 58).
pub fn apply(role: Role, writable: Option<&Path>) -> Result<Report, String> {
    #[cfg(target_os = "linux")]
    {
        linux::apply(role, writable)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (role, writable);
        Ok(Report { layers: vec![String::from("sandbox-exec, by its parent")] })
    }
    #[cfg(windows)]
    {
        let _ = (role, writable);
        Ok(Report { layers: vec![String::from("its parent's token, job and policies")] })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (role, writable);
        Err(String::from("this platform has no sandbox for the browser, so nothing is run on it"))
    }
}

/// The most a time zone is ahead of or behind UTC, in seconds: 18 hours,
/// the bound of ISO 8601's offsets and of every zone in use.
pub const MAX_OFFSET: i32 = 18 * 60 * 60;

/// The time, and the local time zone's offset from UTC then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// Seconds since 1970, UTC.
    pub utc: i64,
    /// Seconds the local time is ahead of UTC.
    pub offset: i32,
}

impl LocalTime {
    /// Whether the offset is one a zone can have.
    #[must_use]
    pub fn is_plausible(&self) -> bool {
        (-MAX_OFFSET..=MAX_OFFSET).contains(&self.offset)
    }
}

/// The time now and the local offset, by the operating system's own rules
/// for the user's time zone. The broker reads them for the confined core,
/// which may not read the system's time-zone files (decision 56).
///
/// # Errors
/// When the clock or the zone cannot be read.
pub fn local_time() -> io::Result<LocalTime> {
    let utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)
        .and_then(|since| i64::try_from(since.as_secs()).map_err(io::Error::other))?;
    #[cfg(unix)]
    let offset = unix::local_offset(utc)?;
    #[cfg(windows)]
    let offset = windows::local_offset()?;
    #[cfg(not(any(unix, windows)))]
    let offset = 0;
    let time = LocalTime { utc, offset };
    if time.is_plausible() { Ok(time) } else { Err(io::Error::other("an offset no zone has")) }
}

/// How the trusted core starts a helper.
#[derive(Debug)]
pub struct Spawn<'a> {
    /// The helper's program, beside the trusted core's.
    pub program: &'a Path,
    /// Its arguments.
    pub arguments: &'a [OsString],
    /// What it is for.
    pub role: Role,
    /// The whole of its environment: nothing the trusted core has is
    /// passed on unasked.
    pub environment: &'a [(OsString, OsString)],
    /// Whether a network helper may reach the local network, which the
    /// user allowed (requirement 5.10).
    pub local_network: bool,
    /// The one folder it may write: the trusted core's settings.
    pub writable: Option<&'a Path>,
}

/// What a process needs of the environment to start at all on this
/// platform; on Windows an `AppContainer` also needs the user's profile
/// folders, or it cannot be created.
#[cfg(windows)]
const ENVIRONMENT_NEEDED: &[&str] = &[
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "PROGRAMDATA",
    "ALLUSERSPROFILE",
    "USERNAME",
    "HOMEDRIVE",
    "HOMEPATH",
];
#[cfg(not(windows))]
const ENVIRONMENT_NEEDED: &[&str] = &[];
/// What the interface needs besides: the window system, the user's
/// folders for fonts and preferences, the locale, and its own
/// diagnostics.
const ENVIRONMENT_WINDOW: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "TMPDIR",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XAUTHORITY",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_DATA_DIRS",
    "DBUS_SESSION_BUS_ADDRESS",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "NEOTELETEXT_FRAME_TRACE",
];
/// What the sound helper needs: the sound server and the user's sound
/// configuration, and its own diagnostics.
const ENVIRONMENT_SOUND: &[&str] =
    &["HOME", "XDG_RUNTIME_DIR", "XDG_CONFIG_HOME", "PULSE_SERVER", "PULSE_RUNTIME_PATH", "PIPEWIRE_RUNTIME_DIR", "NEOTELETEXT_AUDIO_TRACE"];
/// What the trusted core needs: where its settings are, the phone
/// keyboard's relay for tests, and the update host for tests.
const ENVIRONMENT_CORE: &[&str] = &["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "NEOTELETEXT_QRKEYBOARD_RELAY"];

/// The environment a process of `role` is given, taken from this
/// process's: only what it needs, nothing else.
#[must_use]
pub fn environment(role: Role) -> Vec<(OsString, OsString)> {
    let extra: &[&str] = match role {
        Role::Interface => ENVIRONMENT_WINDOW,
        Role::Sound => ENVIRONMENT_SOUND,
        Role::Core => ENVIRONMENT_CORE,
        Role::Decoder | Role::Network => &[],
    };
    let mut names: Vec<&str> = ENVIRONMENT_NEEDED.iter().chain(extra).copied().collect();
    names.sort_unstable();
    names.dedup();
    names.into_iter().filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value))).collect()
}

/// The helper's end of the pipe the trusted core writes.
#[cfg(not(windows))]
pub type Input = std::process::ChildStdin;
/// The helper's end of the pipe the trusted core writes.
#[cfg(windows)]
pub type Input = std::fs::File;
/// The helper's end of the pipe the trusted core reads.
#[cfg(not(windows))]
pub type Output = std::process::ChildStdout;
/// The helper's end of the pipe the trusted core reads.
#[cfg(windows)]
pub type Output = std::fs::File;

/// A helper the trusted core started: its pipes, taken by whoever reads
/// and writes them, and the process to end and wait for.
#[derive(Debug)]
pub struct Process {
    /// What the trusted core writes to the helper.
    pub stdin: Option<Input>,
    /// What the helper writes to the trusted core.
    pub stdout: Option<Output>,
    #[cfg(not(windows))]
    inner: std::process::Child,
    #[cfg(windows)]
    inner: windows::Inner,
    /// How the helper was confined, for the trusted core to report.
    pub report: Report,
}

impl Process {
    /// Ends the helper at once.
    ///
    /// # Errors
    /// When the operating system refuses, or the helper already ended.
    pub fn kill(&mut self) -> io::Result<()> {
        #[cfg(not(windows))]
        {
            self.inner.kill()
        }
        #[cfg(windows)]
        {
            self.inner.kill()
        }
    }

    /// Waits for the helper to end, and reaps it.
    ///
    /// # Errors
    /// When the operating system refuses.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        #[cfg(not(windows))]
        {
            self.inner.wait()
        }
        #[cfg(windows)]
        {
            self.inner.wait()
        }
    }
}

/// Starts a helper, confined as its role says, with its pipes; standard
/// error is the trusted core's own.
///
/// # Errors
/// When the helper cannot be started, confined.
pub fn spawn(spawn: &Spawn<'_>) -> io::Result<Process> {
    #[cfg(windows)]
    {
        windows::spawn(spawn)
    }
    #[cfg(not(windows))]
    {
        use std::process::Stdio;
        #[cfg(target_os = "macos")]
        let mut command = macos::command(spawn);
        #[cfg(not(target_os = "macos"))]
        let mut command = {
            let mut command = std::process::Command::new(spawn.program);
            command.args(spawn.arguments);
            command
        };
        command.env_clear();
        for (name, value) in spawn.environment {
            command.env(name, value);
        }
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()?;
        #[cfg(target_os = "macos")]
        let report = Report { layers: vec![String::from("sandbox-exec")] };
        #[cfg(not(target_os = "macos"))]
        let report = Report { layers: vec![String::from("confines itself at its start")] };
        Ok(Process { stdin: child.stdin.take(), stdout: child.stdout.take(), inner: child, report })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let arguments = [OsString::from("--exact"), OsString::from("tests::environment_probe"), OsString::from("--nocapture")];
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
        for leaked in ["HOME", "PATH", "USER", "SHELL", "TMPDIR", "USERPROFILE", "APPDATA"].into_iter().filter(|name| !passed.iter().any(|given| given == name))
        {
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
        let arguments = [OsString::from("--exact"), OsString::from("tests::environment_probe")];
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
}
