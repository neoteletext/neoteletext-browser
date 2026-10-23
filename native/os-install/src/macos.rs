//! CoreFoundation's alert, which a program without a window may show:
//! `CFUserNotificationDisplayAlert` (Apple's `CFUserNotification.h`). The
//! launcher has no `NSApplication`, so `AppKit`'s alerts are not open to it.

#![allow(unsafe_code, reason = "CoreFoundation's C interface, which Rust cannot check (requirement 10.12)")]

use std::ffi::c_void;
use std::ptr;

/// `CFStringRef`, `CFAllocatorRef`, `CFURLRef`: opaque pointers.
type CfRef = *const c_void;

/// `kCFStringEncodingUTF8` (`CFString.h`).
const UTF8: u32 = 0x0800_0100;
/// `kCFUserNotificationNoteAlertLevel` (`CFUserNotification.h`).
const NOTE_ALERT_LEVEL: usize = 1;
/// `kCFUserNotificationDefaultResponse`: the first button.
const DEFAULT_RESPONSE: usize = 0;
/// The response's low two bits name the button pressed (`CFUserNotification.h`).
const BUTTON_MASK: usize = 0x3;
/// No time-out: the alert waits for the user.
const NO_TIMEOUT: f64 = 0.0;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithBytes(allocator: CfRef, bytes: *const u8, length: isize, encoding: u32, external: u8) -> CfRef;
    fn CFRelease(object: CfRef);
    fn CFUserNotificationDisplayAlert(
        timeout: f64,
        flags: usize,
        icon: CfRef,
        sound: CfRef,
        localisation: CfRef,
        header: CfRef,
        message: CfRef,
        default_button: CfRef,
        alternate_button: CfRef,
        other_button: CfRef,
        response: *mut usize,
    ) -> i32;
}

/// A `CFString` of `text`, released when dropped.
struct CfString(CfRef);

impl CfString {
    fn new(text: &str) -> Self {
        let length = isize::try_from(text.len()).unwrap_or(0);
        // SAFETY: the bytes are valid for `length` bytes and UTF-8; a null
        // allocator is the default one; CoreFoundation copies the bytes.
        Self(unsafe { CFStringCreateWithBytes(ptr::null(), text.as_ptr(), length, UTF8, 0) })
    }
}

impl Drop for CfString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a string this module created and still owns, released once.
            unsafe { CFRelease(self.0) };
        }
    }
}

/// Shows the alert; the button pressed, `DEFAULT_RESPONSE` for the first.
fn alert(title: &str, message: &str, first: &str, second: Option<&str>) -> Option<usize> {
    let (title, message, first) = (CfString::new(title), CfString::new(message), CfString::new(first));
    let second = second.map(CfString::new);
    let mut response = 0_usize;
    // SAFETY: every string is a live CFString or null, which the call
    // allows for the optional ones; `response` is a valid place to write.
    let result = unsafe {
        CFUserNotificationDisplayAlert(
            NO_TIMEOUT,
            NOTE_ALERT_LEVEL,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            title.0,
            message.0,
            first.0,
            second.as_ref().map_or(ptr::null(), |second| second.0),
            ptr::null(),
            &raw mut response,
        )
    };
    (result == 0).then_some(response & BUTTON_MASK)
}

pub fn ask(title: &str, question: &str, yes: &str, no: &str) -> bool {
    alert(title, question, yes, Some(no)) == Some(DEFAULT_RESPONSE)
}

pub fn tell(title: &str, message: &str) {
    let _ = alert(title, message, "OK", None);
}
