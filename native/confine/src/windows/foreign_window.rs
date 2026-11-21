//! A window named by its handle, which means the same in every process on
//! the desktop: where the pointer is on it. The chooser's dialog is owned by
//! the interface's window through its handle too (DEC-1551 item 5), in
//! `native/dialog`.

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// Where the pointer is on `window`, in the window's pixels, from its top
/// left corner. A drag from another application tells the window nothing as
/// it moves, so the system is asked.
#[must_use]
pub fn pointer_in_window(window: isize) -> Option<(f64, f64)> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a live POINT for the length of each call, and a
    // handle that names no window is refused by the system.
    let found = unsafe { GetCursorPos(&raw mut point) != 0 && ScreenToClient(window, &raw mut point) != 0 };
    found.then_some((f64::from(point.x), f64::from(point.y)))
}
