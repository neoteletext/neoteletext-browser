//! The helpers' sandboxes on Windows, which the trusted core applies as
//! it starts them, as every sandboxed program on Windows does since
//! Vista: the token is the sandbox.
//!
//! - **A job object** with kill-on-close, one process at most and the UI
//!   restrictions the role allows: the helper dies with the trusted core
//!   and can start nothing.
//! - **Process mitigation policies:** DEP, ASLR, control-flow guard,
//!   strict handle checks, no extension points, no remote or
//!   low-integrity images, no dynamic code (Rust needs none), no child
//!   processes; and for a helper with no window, the Win32k system-call
//!   filter, so it cannot reach the window system at all.
//! - **An `AppContainer`** for every process: the decoder and the trusted
//!   core in a less privileged one with no capabilities, the interface
//!   and the sound helper with none (decision 90: the interface's window
//!   and desktop are reached from a container, and it reaches no network
//!   and none of the user's files), the fetcher and keyboard with the
//!   internet client capability (and the private network one when the
//!   user allowed local addresses). A container's token is at low
//!   integrity, so it writes nothing of the user's either. The program's
//!   folder must grant read to `ALL APPLICATION PACKAGES`, which an
//!   installation under Program Files does and the installer sets
//!   elsewhere; when the `AppContainer` cannot be made or started, the
//!   process is not started at all (decision 58: security is never
//!   downgraded).
//!
//! Only the two pipe handles are inherited, by the handle list; the
//! environment is the one the trusted core chose, and nothing else.

#![expect(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]

use std::fs::File;
use std::io;
use std::os::windows::io::FromRawHandle as _;
use std::path::Path;

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
use windows_sys::Win32::System::Threading::{ResumeThread, TerminateProcess};

use crate::{Process, Report, Role, Spawn};

mod acl;
mod attributes;
mod channel;
mod container;
mod handles;
mod job;
mod process;
mod time;

pub(crate) use channel::Channel;
pub use process::Inner;
pub(crate) use time::local_offset;

use acl::grant;
use attributes::start;
use container::{INTERNET_CLIENT_SID, PRIVATE_NETWORK_SID, container};
use handles::{Owned, Sid, keep_ours, last_error, pipe};
use job::job;

/// Grants `ALL APPLICATION PACKAGES` read and run on `folder` and
/// everything in it, as Program Files grants it, so the containers can
/// start the browser's programs from the user's own folder; the
/// installer's step (`installation-and-updates.md` section 4.2). No
/// administrator: the user owns the folder.
///
/// # Errors
/// When the folder's access list cannot be changed.
pub fn grant_packages_read(folder: &Path) -> io::Result<()> {
    acl::grant_packages_read(folder)
}

/// Removes the browser's `AppContainer` profiles, for the uninstaller
/// (requirement 13.20).
pub fn remove_containers() {
    container::remove_containers();
}

/// A process's own side: its parent started it with the token, job and
/// policies that confine it.
pub(crate) fn apply() -> Report {
    Report { layers: vec![String::from("its parent's token, job and policies")] }
}

/// Starts a helper confined as its role says.
pub(crate) fn spawn(spawn: &Spawn<'_>) -> io::Result<Process> {
    let (their_stdin, our_stdin) = pipe()?;
    let (our_stdout, their_stdout) = pipe()?;
    keep_ours(our_stdin.raw())?;
    keep_ours(our_stdout.raw())?;
    // SAFETY: GetStdHandle takes a constant and returns a handle this process already
    // has, or none; it is only compared and passed on, never closed.
    let stderr = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    let mut inherited = vec![their_stdin.raw(), their_stdout.raw()];
    if stderr != 0 && stderr != INVALID_HANDLE_VALUE {
        inherited.push(stderr);
    }
    let job = job(spawn.role)?;
    let mut report = Report { layers: vec![String::from("job object"), String::from("mitigation policies"), String::from("no child processes")] };
    // Every role runs in its container or not at all (decision 58); the
    // interface too, with no capabilities (decision 90).
    let sid = container(spawn.role).map_err(|error| io::Error::new(error.kind(), format!("cannot make its AppContainer: {error}")))?;
    let mut capabilities = Vec::new();
    // A container cannot be told a port: the updater's host is pinned by
    // TLS (`installation-and-updates.md` section 8.1).
    if matches!(spawn.role, Role::Network | Role::Updater { .. }) {
        capabilities.push(Sid::parse(INTERNET_CLIENT_SID)?);
        if spawn.local_network {
            capabilities.push(Sid::parse(PRIVATE_NETWORK_SID)?);
        }
    }
    if matches!(spawn.role, Role::Core | Role::Updater { .. })
        && let Some(folder) = spawn.writable
    {
        grant(folder, &sid)?;
    }
    let information = start(spawn, &inherited, their_stdin.raw(), their_stdout.raw(), &sid, &capabilities)
        .map_err(|error| io::Error::new(error.kind(), format!("cannot start in its AppContainer: {error}")))?;
    report.layers.push(String::from(if matches!(spawn.role, Role::Decoder | Role::Core) { "less privileged AppContainer" } else { "AppContainer" }));
    let process = Owned::new(information.hProcess);
    let thread = Owned::new(information.hThread);
    // Into the job before it runs a single instruction.
    // SAFETY: the job and the process are handles this function made and owns, alive to
    // the end of it.
    if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
        let error = last_error();
        // SAFETY: the suspended process this function started, its handle still owned here.
        unsafe { TerminateProcess(process.raw(), 1) };
        return Err(error);
    }
    // SAFETY: the process's first thread, started suspended, its handle owned here.
    if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
        let error = last_error();
        // SAFETY: as above.
        unsafe { TerminateProcess(process.raw(), 1) };
        return Err(error);
    }
    drop(thread);
    drop(their_stdin);
    drop(their_stdout);
    // SAFETY: our end of the input pipe, made by `pipe` and owned only by `our_stdin`,
    // forgotten below so the file becomes its one owner.
    let stdin = unsafe { File::from_raw_handle(our_stdin.raw() as _) };
    // SAFETY: likewise our end of the output pipe.
    let stdout = unsafe { File::from_raw_handle(our_stdout.raw() as _) };
    std::mem::forget(our_stdin);
    std::mem::forget(our_stdout);
    let inner = Inner::new(process.raw(), job.raw());
    std::mem::forget(process);
    std::mem::forget(job);
    Ok(Process::new(Some(stdin), Some(stdout), inner, report))
}

/// Starts the trusted core confined, its standard input and output the
/// broker's two pipes to it.
pub(crate) fn start_core(spawn: &Spawn<'_>) -> io::Result<(Process, Channel)> {
    let mut process = self::spawn(spawn)?;
    let writer = process.stdin.take().ok_or_else(|| io::Error::other("the core has no input"))?;
    let reader = process.stdout.take().ok_or_else(|| io::Error::other("the core has no output"))?;
    Ok((process, Channel::new(reader, writer)))
}
