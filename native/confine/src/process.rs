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
}

impl Process {
    /// A started helper, from its pipes, its process and its report.
    pub(crate) fn new(stdin: Option<Input>, stdout: Option<Output>, inner: Inner, report: Report) -> Self {
        Self { stdin, stdout, inner, report }
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

    /// The process, for duplicating handles into it (decision 56).
    #[cfg(windows)]
    pub(crate) fn raw_process(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.inner.raw_process()
    }
}
