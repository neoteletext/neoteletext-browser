//! Passing open files between processes on macOS and Linux: a message on
//! a local-domain socket carries descriptors with it (`SCM_RIGHTS`,
//! `unix(7)`). The broker passes the trusted core the pipes of each helper
//! it starts this way. The standard library's ancillary-data interface is
//! not yet stable, so the two system calls are made here. And the local
//! time zone's offset, which the standard library does not offer, from
//! the C library's own `localtime_r`.

use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

use crate::{Process, Report, Spawn};

mod channel;
mod descriptors;
mod time;

pub(crate) use channel::Channel;
pub(crate) use descriptors::{receive, send};
pub(crate) use time::local_offset;

/// The command that runs a helper: under its profile on macOS; as it is
/// on Linux, where it confines itself at its start.
fn command(spawn: &Spawn<'_>) -> Command {
    #[cfg(target_os = "macos")]
    let command = crate::macos::command(spawn);
    #[cfg(not(target_os = "macos"))]
    let command = {
        let mut command = Command::new(spawn.program);
        command.args(spawn.arguments);
        command
    };
    command
}

/// Starts a helper, confined as its role says, with its pipes; standard
/// error is the trusted core's own.
pub(crate) fn spawn(spawn: &Spawn<'_>) -> io::Result<Process> {
    let mut command = command(spawn);
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

/// Starts the trusted core confined, its standard input the broker's
/// socket to it.
pub(crate) fn start_core(spawn: &Spawn<'_>) -> io::Result<(Process, Channel)> {
    let (ours, theirs) = UnixStream::pair()?;
    let mut command = command(spawn);
    command.env_clear();
    for (name, value) in spawn.environment {
        command.env(name, value);
    }
    let child = command.stdin(Stdio::from(OwnedFd::from(theirs))).stdout(Stdio::null()).stderr(Stdio::inherit()).spawn()?;
    #[cfg(target_os = "macos")]
    let report = Report { layers: vec![String::from("sandbox-exec")] };
    #[cfg(not(target_os = "macos"))]
    let report = Report { layers: vec![String::from("confines itself at its start")] };
    let process = Process::new(None, None, child, report);
    Ok((process, Channel::new(ours)))
}
