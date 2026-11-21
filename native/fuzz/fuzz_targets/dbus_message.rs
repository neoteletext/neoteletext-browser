//! A D-Bus message from the session bus (DEC-7168): the chooser and the
//! broker read what any of the user's processes on the bus may send. What
//! is read is within the bounds, and what is written back reads the same.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(message) = dbus::Message::decode(data) else { return };
    assert!(data.len() <= dbus::MAX_MESSAGE_BYTES);
    if let Ok(written) = message.encode() {
        assert_eq!(dbus::Message::decode(&written).as_ref(), Ok(&message));
    }
});
