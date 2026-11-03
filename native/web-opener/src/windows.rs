//! The shell's `ShellExecuteW` and the system's `SystemParametersInfoW`,
//! the two calls of this crate that are `unsafe`.
#![allow(unsafe_code, reason = "ShellExecuteW and SystemParametersInfoW are foreign functions: two calls, each with its reason")]

use windows_sys::Win32::Foundation::BOOL;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETCLIENTAREAANIMATION, SW_SHOWNORMAL, SystemParametersInfoW};

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

/// Whether the viewer turned off "Animation effects" (Settings,
/// Accessibility, Visual effects), which Windows reads as
/// `SPI_GETCLIENTAREAANIMATION`: the Windows SDK's documentation of
/// `SystemParametersInfoW` says `pvParam` points to a `BOOL` that receives
/// true when client-area animations are enabled. A call that fails is no
/// preference, so motion stays as it always was.
pub(crate) fn reduced_motion() -> bool {
    let mut animations: BOOL = 1;
    // SAFETY: `pvParam` points to a `BOOL` that lives in this frame until
    // the call returns, which is what this action writes and all it
    // writes; `uiParam` is unused by it and zero, and `fWinIni` is zero as
    // a read changes no setting.
    let succeeded = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, std::ptr::from_mut(&mut animations).cast(), 0) };
    succeeded != 0 && animations == 0
}
