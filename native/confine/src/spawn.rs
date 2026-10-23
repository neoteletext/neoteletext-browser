//! The two halves of confinement: [`spawn`], the parent's side, which
//! starts a helper confined, and [`apply`], a process's own side.

use std::ffi::OsString;
use std::io;
use std::path::Path;

use crate::{Process, Report, Role};

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
        crate::linux::apply(role, writable)
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

/// Starts a helper, confined as its role says, with its pipes; standard
/// error is the trusted core's own.
///
/// # Errors
/// When the helper cannot be started, confined.
pub fn spawn(spawn: &Spawn<'_>) -> io::Result<Process> {
    #[cfg(windows)]
    {
        crate::windows::spawn(spawn)
    }
    #[cfg(not(windows))]
    {
        use std::process::Stdio;
        #[cfg(target_os = "macos")]
        let mut command = crate::macos::command(spawn);
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
        Ok(Process::new(child.stdin.take(), child.stdout.take(), child, report))
    }
}

#[cfg(test)]
mod tests;
