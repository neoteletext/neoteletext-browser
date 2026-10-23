//! A confined helper's process and job, and how its command line and
//! environment are written for `CreateProcess`.

#![expect(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::process::ExitStatusExt as _;
use std::process::ExitStatus;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, TerminateProcess, WaitForSingleObject};

use super::handles::wide;

/// A helper's process and its job.
#[derive(Debug)]
pub struct Inner {
    process: HANDLE,
    job: HANDLE,
}

impl Inner {
    /// A helper's process and the job it runs in, both owned from here on.
    pub(super) fn new(process: HANDLE, job: HANDLE) -> Self {
        Self { process, job }
    }

    /// The process, for duplicating handles into it.
    pub(crate) fn raw_process(&self) -> HANDLE {
        self.process
    }

    pub fn kill(&mut self) -> io::Result<()> {
        // SAFETY: the process handle this value owns.
        if unsafe { TerminateProcess(self.process, 1) } == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        // SAFETY: the process handle this value owns.
        if unsafe { WaitForSingleObject(self.process, INFINITE) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut code = 0_u32;
        // SAFETY: the process handle this value owns; the code is written to a live local.
        if unsafe { GetExitCodeProcess(self.process, &raw mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(ExitStatus::from_raw(code))
    }
}

impl Drop for Inner {
    /// Closing the job ends the helper, if it still runs.
    fn drop(&mut self) {
        // SAFETY: both handles are owned by this value and closed once, here.
        unsafe {
            CloseHandle(self.process);
            CloseHandle(self.job);
        }
    }
}

/// The command line as `CreateProcess` reads it: each argument quoted,
/// with its quotes and the backslashes before them escaped.
pub(super) fn command_line(program: &OsStr, arguments: &[OsString]) -> Vec<u16> {
    let mut line = String::new();
    for (index, argument) in std::iter::once(program).chain(arguments.iter().map(OsString::as_os_str)).enumerate() {
        if index > 0 {
            line.push(' ');
        }
        line.push('"');
        let text = argument.to_string_lossy();
        let mut backslashes = 0_usize;
        for character in text.chars() {
            match character {
                '\\' => backslashes = backslashes.saturating_add(1),
                '"' => {
                    line.extend(std::iter::repeat_n('\\', backslashes.saturating_mul(2).saturating_add(1)));
                    backslashes = 0;
                    line.push('"');
                    continue;
                }
                _ => {
                    line.extend(std::iter::repeat_n('\\', backslashes));
                    backslashes = 0;
                }
            }
            if character != '\\' {
                line.push(character);
            }
        }
        line.extend(std::iter::repeat_n('\\', backslashes.saturating_mul(2)));
        line.push('"');
    }
    wide(line)
}

/// The environment block: `NAME=value` strings, each zero-terminated,
/// then one more zero. An empty block is two zeros (finding F-58):
/// `CreateProcess` reads up to the first two zeros in a row, so one alone
/// would have it read past the block.
pub(super) fn environment_block(environment: &[(OsString, OsString)]) -> Vec<u16> {
    let mut block = Vec::new();
    for (name, value) in environment {
        block.extend(name.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    if environment.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_quoted_as_the_c_runtime_reads_them() {
        let line = command_line(
            OsStr::new(r"C:\Program Files\NeoTeletext\neoteletext-fetcher.exe"),
            &[OsString::from("http://a.example/x y"), OsString::from(r#"say "hi" \"#)],
        );
        let text = String::from_utf16_lossy(&line[..line.len().saturating_sub(1)]);
        assert_eq!(text, r#""C:\Program Files\NeoTeletext\neoteletext-fetcher.exe" "http://a.example/x y" "say \"hi\" \\""#);
    }

    #[test]
    fn the_environment_block_ends_with_two_zeros() {
        let block = environment_block(&[(OsString::from("A"), OsString::from("1"))]);
        assert_eq!(String::from_utf16_lossy(&block[..3]), "A=1");
        assert_eq!(&block[3..], &[0, 0]);
    }

    /// Finding F-58: an empty environment is two zeros, not one, which
    /// `CreateProcess` would read past.
    #[test]
    fn an_empty_environment_block_is_two_zeros() {
        assert_eq!(environment_block(&[]), [0, 0]);
    }
}
