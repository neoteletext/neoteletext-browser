//! The processes' sandboxes (requirements 3.3, 3.4, 3.6 and 3.7; decision
//! 52): every helper of the trusted core runs with only what it is for,
//! on every platform, by the platform's own means.
//!
//! - **macOS:** the trusted core starts each helper under a `sandbox-exec`
//!   profile that starts from "deny everything" (`macos`).
//! - **Windows:** the trusted core starts each helper with a restricted
//!   token at low integrity, in a job object, with the process mitigation
//!   policies, and the decoder and the fetcher in an `AppContainer`
//!   (`windows`).
//! - **Linux:** each helper confines itself first thing: Landlock and an
//!   empty network namespace where the kernel allows them, and a seccomp
//!   system-call filter always (`linux`). The filter is the one layer every
//!   Linux has had since 2012, so it is the one layer a helper refuses to
//!   run without; the others are reported when they are missing.
//!
//! The two halves are [`spawn`], the trusted core's side, and [`apply`],
//! a helper's side; each is a no-op on the platforms where the other does
//! the work. The helpers receive everything over their pipes, so no
//! sandbox here has to let anything through on request: they only deny.

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

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
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
    /// pasteboard, the sound output and the Dock; no network.
    Interface,
}

impl Role {
    /// The helper's name in what it reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Decoder => "decoder",
            Self::Network => "fetcher",
            Self::Interface => "interface",
        }
    }
}

/// What confines a process, as it reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// The layers in force, in words ("seccomp", "Landlock 5").
    pub layers: Vec<String>,
    /// Layers the platform could not give, and why.
    pub missing: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.layers.is_empty() {
            f.write_str("not confined")?;
        } else {
            f.write_str(&self.layers.join(", "))?;
        }
        for missing in &self.missing {
            write!(f, "; without {missing}")?;
        }
        Ok(())
    }
}

/// Confines the calling process as `role` says, where a process confines
/// itself: Linux. Elsewhere the trusted core confined it at its start,
/// and this says so. Called first thing, before any thread exists: a
/// filter applies to the thread that sets it and those it then starts.
///
/// # Errors
/// When the one layer a helper must have cannot be applied; the helper
/// then ends, and the trusted core reports it.
pub fn apply(role: Role) -> Result<Report, String> {
    #[cfg(target_os = "linux")]
    {
        linux::apply(role)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = role;
        Ok(Report { layers: vec![String::from("sandbox-exec, by the trusted core")], missing: Vec::new() })
    }
    #[cfg(windows)]
    {
        let _ = role;
        Ok(Report { layers: vec![String::from("the trusted core's token, job and policies")], missing: Vec::new() })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = role;
        Ok(Report { layers: Vec::new(), missing: vec![String::from("a sandbox on this platform")] })
    }
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
/// What the interface needs besides: the window system, the sound
/// server, the user's folders for fonts and preferences, the locale, and
/// its own diagnostics.
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
    "PULSE_SERVER",
    "PULSE_RUNTIME_PATH",
    "PIPEWIRE_RUNTIME_DIR",
    "NEOTELETEXT_AUDIO_TRACE",
    "NEOTELETEXT_FRAME_TRACE",
];

/// The environment a helper of `role` is given, taken from this
/// process's: only what it needs, nothing else of the trusted core's.
#[must_use]
pub fn environment(role: Role) -> Vec<(OsString, OsString)> {
    ENVIRONMENT_NEEDED
        .iter()
        .chain(if role == Role::Interface { ENVIRONMENT_WINDOW } else { &[] })
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect()
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
        let report = Report { layers: vec![String::from("sandbox-exec")], missing: Vec::new() };
        #[cfg(not(target_os = "macos"))]
        let report = Report { layers: vec![String::from("confines itself at its start")], missing: Vec::new() };
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
    /// helper inherited the core's whole environment.
    #[test]
    fn a_helper_gets_only_the_environment_it_is_given() {
        let program = std::env::current_exe().unwrap();
        let arguments = [OsString::from("--exact"), OsString::from("tests::environment_probe"), OsString::from("--nocapture")];
        let environment = [(OsString::from(PROBE), OsString::from("1")), (OsString::from("NEOTELETEXT_GIVEN"), OsString::from("yes"))];
        let mut process =
            spawn(&Spawn { program: &program, arguments: &arguments, role: Role::Decoder, environment: &environment, local_network: false }).unwrap();
        let output = process.stdout.take().unwrap();
        let names: Vec<String> = BufReader::new(output)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| line.strip_prefix("ENV ").map(|rest| rest.split('=').next().unwrap_or("").to_owned()))
            .collect();
        let _ = process.wait();
        assert!(names.contains(&String::from("NEOTELETEXT_GIVEN")), "{names:?}");
        // What the platform adds of its own is not the core's: nothing of
        // the core's environment, such as its home or path, is there.
        for leaked in ["HOME", "PATH", "USER", "SHELL", "TMPDIR", "USERPROFILE", "APPDATA"] {
            assert!(!names.contains(&String::from(leaked)), "{leaked} leaked: {names:?}");
        }
        assert!(!process.report.layers.is_empty(), "{}", process.report);
    }

    #[test]
    fn a_report_reads_as_a_sentence() {
        let report =
            Report { layers: vec!["seccomp".into(), "Landlock 5".into()], missing: vec!["a network namespace (user namespaces are restricted)".into()] };
        assert_eq!(report.to_string(), "seccomp, Landlock 5; without a network namespace (user namespaces are restricted)");
        assert_eq!(Report::default().to_string(), "not confined");
    }
}
