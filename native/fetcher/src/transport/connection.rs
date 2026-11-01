//! An open connection's bytes as the loop takes them, and the TLS records
//! given to the writer thread rather than written here.

use std::io::ErrorKind;

use neoteletext_web::port::Received;

use super::Connection;
use super::writer::Outgoing;

impl Connection {
    /// What has arrived, looking without waiting (REQ-INT-135); any failure
    /// is the connection's end.
    pub(super) fn receive(&mut self, buffer: &mut [u8]) -> Received {
        let Self { stream, outgoing, .. } = self;
        let taken = stream.take(buffer, None, |tls, _| if give_records(tls, outgoing) { Ok(()) } else { Err(ErrorKind::BrokenPipe.into()) });
        taken.unwrap_or(Received::Closed)
    }
}

/// Gives the writer thread whatever records rustls has ready, as one
/// piece, so the messages sent together leave in one write (REQ-RCH-010);
/// false when the thread does not take them.
pub(super) fn give_records(tls: &mut rustls::ClientConnection, outgoing: &Outgoing) -> bool {
    let mut records = Vec::new();
    while tls.wants_write() {
        // Writing to memory does not fail.
        if tls.write_tls(&mut records).is_err() {
            return false;
        }
    }
    records.is_empty() || outgoing.give(records)
}
