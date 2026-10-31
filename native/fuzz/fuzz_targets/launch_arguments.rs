//! A command line from a registered link or file (REQ-1673): a marker comes
//! alone with one argument, or the launch is refused. NUL splits arguments.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::ffi::OsString;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = core::str::from_utf8(data) else { return };
    let arguments: Vec<OsString> = text.split('\0').map(OsString::from).collect();
    if let Ok(neoteletext::launch::Launch::Link(_) | neoteletext::launch::Launch::File(_)) = neoteletext::launch::read(&arguments) {
        assert!(arguments.len() <= 2);
    }
});
