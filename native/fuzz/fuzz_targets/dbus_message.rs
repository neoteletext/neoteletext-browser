//! A D-Bus message from the session bus (DEC-7168): the chooser and the
//! broker read what any of the user's processes on the bus may send. What
//! is read is within the bounds, its decoded values among them (at most
//! `MAX_DECODED_NODES_PER_BYTE` for each byte, TODO-7173 finding A1), and
//! what is written back reads the same. The gate runs it with inputs up to
//! `MAX_MESSAGE_BYTES` (`scripts/check.sh`, `fuzz_options`).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(message) = dbus::Message::decode(data) else { return };
    assert!(data.len() <= dbus::MAX_MESSAGE_BYTES);
    assert!(dbus::nodes(&message.body) <= data.len() * dbus::MAX_DECODED_NODES_PER_BYTE);
    if let Ok(written) = message.encode() {
        assert_eq!(dbus::Message::decode(&written).as_ref(), Ok(&message));
    }
});
