//! A window of another process, named by its handle, to own a dialog
//! (DEC-1551 item 5). A window handle means the same in every process on the
//! desktop, so the interface's goes to the chooser as a number.

use std::num::NonZeroIsize;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowHandle, WindowsDisplayHandle,
};
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

/// Another process's window.
pub struct ForeignWindow(NonZeroIsize);

impl ForeignWindow {
    /// The window of `handle`, or none for the null handle.
    #[must_use]
    pub fn new(handle: isize) -> Option<Self> {
        NonZeroIsize::new(handle).map(Self)
    }
}

impl HasWindowHandle for ForeignWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the handle is only passed to the system as a dialog's
        // owner, which refuses one that names no window; nothing here reads
        // through it, so it need not outlive anything of this process.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(self.0))) })
    }
}

impl HasDisplayHandle for ForeignWindow {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: Windows' display handle holds nothing.
        Ok(unsafe { DisplayHandle::borrow_raw(RawDisplayHandle::Windows(WindowsDisplayHandle::new())) })
    }
}
