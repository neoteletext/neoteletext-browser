//! The shell's `ShellExecuteW`, the one call of this crate that is
//! `unsafe`.
#![allow(unsafe_code, reason = "ShellExecuteW is a foreign function: the one call, with its reason")]

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::OpenError;

/// `ShellExecuteW` reports success with a value above 32 (its
/// documentation in the Windows SDK, "Return value").
const SUCCESS_ABOVE: isize = 32;

/// UTF-16 with the terminating zero the call needs.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(crate) fn open(address: &str) -> Result<(), OpenError> {
    let verb = wide("open");
    let file = wide(address);
    // SAFETY: both strings are zero-terminated UTF-16 that live until the
    // call returns; a null window, null parameters and a null directory are
    // allowed by the call's documentation. The address is an https or http
    // address (`crate::acceptable`), which the shell opens in the default
    // browser and never runs.
    let result = unsafe { ShellExecuteW(0, verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    if result > SUCCESS_ABOVE { Ok(()) } else { Err(OpenError::Failed(format!("ShellExecuteW returned {result}"))) }
}
