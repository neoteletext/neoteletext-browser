//! A helper the trusted core started: its pipes and the process.

use std::io;
use std::process::ExitStatus;

use crate::Report;

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

/// The process itself, as each platform holds it.
#[cfg(not(windows))]
type Inner = std::process::Child;
#[cfg(windows)]
type Inner = crate::windows::Inner;

/// A helper the trusted core started: its pipes, taken by whoever reads
/// and writes them, and the process to end and wait for.
#[derive(Debug)]
pub struct Process {
    /// What the trusted core writes to the helper.
    pub stdin: Option<Input>,
    /// What the helper writes to the trusted core.
    pub stdout: Option<Output>,
    inner: Inner,
    /// How the helper was confined, for the trusted core to report.
    pub report: Report,
    /// The broker's end of the interface's own channel (DEC-1551 item 3),
    /// which the core never sees: the interface alone has one.
    pub(crate) broker_side: Option<crate::broker::Channel>,
}

impl Process {
    /// A started helper, from its pipes, its process and its report.
    pub(crate) fn new(stdin: Option<Input>, stdout: Option<Output>, inner: Inner, report: Report) -> Self {
        Self { stdin, stdout, inner, report, broker_side: None }
    }

    /// Ends the helper at once.
    ///
    /// # Errors
    /// When the operating system refuses, or the helper already ended.
    pub fn kill(&mut self) -> io::Result<()> {
        self.inner.kill()
    }

    /// Waits for the helper to end, and reaps it.
    ///
    /// # Errors
    /// When the operating system refuses.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait()
    }

    /// The processor time the helper has used since it started, in
    /// microseconds, from the operating system's own account of it
    /// (REQ-6626 item 1 a): on Linux the first figure of
    /// `/proc/<pid>/schedstat`, the nanoseconds it has run on a processor;
    /// on macOS its user and system time by `proc_pid_rusage`; on Windows
    /// its kernel and user time by `GetProcessTimes` on the handle held.
    ///
    /// # Errors
    /// When the account cannot be read.
    pub fn processor_time_us(&self) -> io::Result<u64> {
        #[cfg(target_os = "linux")]
        {
            const NANOSECONDS_PER_MICROSECOND: u64 = 1_000;
            let account = std::fs::read_to_string(format!("/proc/{}/schedstat", self.inner.id()))?;
            let nanoseconds: u64 = account
                .split_whitespace()
                .next()
                .and_then(|figure| figure.parse().ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "an account that is not three figures"))?;
            Ok(nanoseconds / NANOSECONDS_PER_MICROSECOND)
        }
        #[cfg(target_os = "macos")]
        {
            crate::unix::processor_time_us(self.inner.id())
        }
        #[cfg(windows)]
        {
            self.inner.processor_time_us()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        Err(io::Error::new(io::ErrorKind::Unsupported, "a helper's processor time is read on Linux, macOS and Windows"))
    }

    /// The pipes as files, taken: for a helper started in this process,
    /// whose pipes the core reads and writes as it does a brokered one's.
    pub(crate) fn take_files(&mut self) -> (Option<std::fs::File>, Option<std::fs::File>) {
        #[cfg(unix)]
        let (stdin, stdout) = (
            self.stdin.take().map(|pipe| std::fs::File::from(std::os::fd::OwnedFd::from(pipe))),
            self.stdout.take().map(|pipe| std::fs::File::from(std::os::fd::OwnedFd::from(pipe))),
        );
        #[cfg(windows)]
        let (stdin, stdout) = (self.stdin.take(), self.stdout.take());
        (stdin, stdout)
    }

    /// The process, for duplicating handles into it (decision 56).
    #[cfg(windows)]
    pub(crate) fn raw_process(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.inner.raw_process()
    }
}
