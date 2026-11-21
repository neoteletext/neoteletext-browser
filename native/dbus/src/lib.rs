//! A minimal D-Bus client (DEC-7168), written to the D-Bus Specification
//! (freedesktop.org, version 0.43): the session bus's address
//! ("Server Addresses"), SASL EXTERNAL authentication ("Authentication
//! Protocol"), the message format ("Message Protocol") and the method calls
//! and signals two callers need. The browser speaks D-Bus only on Linux,
//! and only from its two unconfined processes: the chooser, for the
//! desktop portal's `FileChooser` and `Settings`, and the broker, for the
//! Secret Service that keeps the viewer key file's sealing key.
//!
//! Everything read from the bus is untrusted: a message is at most
//! [`MAX_MESSAGE_BYTES`], a signature at most [`MAX_SIGNATURE_BYTES`] and
//! nested at most [`MAX_DEPTH`] containers deep, and every string, padding
//! byte and header field is checked as the specification says it must be.
//! The decoder is fuzzed (`native/fuzz`, `dbus_message`).
//!
//! It passes no file descriptors, serves no objects and keeps no state but
//! one connection's serial number and the signals that came in while a
//! reply was awaited.

// No panicking constructs: this crate reads what any process of the user's
// on the session bus may send. The same list is in each such crate;
// scripts/check.sh checks that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

mod address;
#[cfg(unix)]
mod connection;
mod error;
mod message;
mod value;
mod wire;

pub use address::{Address, parse_addresses};
#[cfg(unix)]
pub use connection::{CALL_TIMEOUT, Connection, MAX_WAITING_SIGNALS};
pub use error::Error;
pub use message::{Kind, MAX_MESSAGE_BYTES, Message};
pub use value::{MAX_ARRAY_DEPTH, MAX_DEPTH, MAX_SIGNATURE_BYTES, MAX_STRUCT_DEPTH, Type, Value, parse_signature, valid_object_path, wipe};
