//! Tests of the pipes written by threads of their own.
#![allow(clippy::unwrap_used, reason = "a test")]

use super::*;
use std::sync::mpsc;
use std::time::Duration;

/// A sink that writes only when told to, counting what it got.
struct Gated {
    gate: mpsc::Receiver<()>,
    written: mpsc::Sender<usize>,
}

impl Write for Gated {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.gate.recv().map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        let _ = self.written.send(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_flush_queues_one_chunk_and_the_thread_writes_it() {
    let (open, gate) = mpsc::channel();
    let (written, got) = mpsc::channel();
    let mut outbox = Outbox::new(Gated { gate, written });
    outbox.write_all(b"ab").unwrap();
    outbox.write_all(b"cd").unwrap();
    outbox.flush().unwrap();
    assert!(got.recv_timeout(Duration::from_millis(100)).is_err(), "nothing is written until the sink allows it");
    open.send(()).unwrap();
    assert_eq!(got.recv_timeout(Duration::from_secs(5)).unwrap(), 4, "both writes went as one chunk");
}

/// A process that stops reading is given up, and the core's thread is
/// never held: every flush returns at once.
#[test]
fn a_process_that_stops_reading_is_given_up_without_waiting() {
    let (_open, gate) = mpsc::channel();
    let (written, _got) = mpsc::channel();
    let mut outbox = Outbox::new(Gated { gate, written });
    let chunk = vec![0_u8; MAX_QUEUED_BYTES / 4];
    let started = std::time::Instant::now();
    for _ in 0..4 {
        outbox.write_all(&chunk).unwrap();
        outbox.flush().unwrap();
    }
    outbox.write_all(b"x").unwrap();
    let refused = outbox.flush().unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::WouldBlock);
    assert!(started.elapsed() < Duration::from_secs(1), "never waited on the process");
    outbox.write_all(b"y").unwrap();
    assert_eq!(outbox.flush().unwrap_err().kind(), io::ErrorKind::BrokenPipe, "given up for good");
}

#[test]
fn closing_writes_everything_first_and_a_broken_pipe_is_reported() {
    let (open, gate) = mpsc::channel();
    let (written, got) = mpsc::channel();
    let mut outbox = Outbox::new(Gated { gate, written });
    outbox.write_all(b"last words").unwrap();
    open.send(()).unwrap();
    outbox.close();
    assert_eq!(got.try_recv().unwrap(), 10, "written before close returned");

    let (open, gate) = mpsc::channel();
    let (written, _got) = mpsc::channel();
    let mut outbox = Outbox::new(Gated { gate, written });
    drop(open);
    outbox.write_all(b"x").unwrap();
    outbox.flush().unwrap();
    // The thread finds the pipe broken; the next flush says so.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        outbox.write_all(b"y").unwrap();
        if let Err(error) = outbox.flush() {
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the break was never reported");
        thread::yield_now();
    }
}
