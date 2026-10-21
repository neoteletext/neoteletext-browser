//! The trusted core's pipes to its processes, each written by a thread of
//! its own (`architecture.md` section 6.4): the core's one thread, which
//! owns every tab, never waits on a process that is slow to read. What an
//! event produced is queued whole when the core flushes, and a queue that
//! grows past [`MAX_QUEUED_BYTES`] means the process has stopped reading:
//! the pipe is then given up, and the core replaces a decoder or ends with
//! the interface, rather than stopping with it (`security-analysis.md`
//! section 6.4).

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

/// The most bytes a process may be behind by. A decoder is given at most
/// 8 MB of kept data when it wakes (`sleep::MAX_KEPT_BYTES`), in one
/// flush; twice that is a process that has stopped reading, not one that
/// is busy.
pub const MAX_QUEUED_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
struct State {
    chunks: VecDeque<Vec<u8>>,
    /// The bytes of `chunks`, kept so the bound costs nothing to check.
    bytes: usize,
    /// No more chunks come: the writer ends once these are written.
    closed: bool,
    /// The process stopped reading, or the pipe broke.
    failed: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A pipe to a process, written by its own thread. It is a [`Write`]:
/// what is written waits until [`Write::flush`], which queues it whole;
/// the thread writes the queue to the pipe in order.
pub struct Outbox {
    shared: Arc<Shared>,
    /// Written since the last flush.
    pending: Vec<u8>,
    writer: Option<JoinHandle<()>>,
}

impl Outbox {
    /// A pipe to `sink`, which the writing thread owns and drops, ending
    /// the pipe, once the outbox is closed and written out.
    pub fn new(mut sink: impl Write + Send + 'static) -> Self {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), ready: Condvar::new() });
        let theirs = Arc::clone(&shared);
        let writer = thread::spawn(move || {
            loop {
                let mut state = theirs.lock();
                let chunk = loop {
                    if let Some(chunk) = state.chunks.pop_front() {
                        break chunk;
                    }
                    if state.closed {
                        return;
                    }
                    state = theirs.ready.wait(state).unwrap_or_else(PoisonError::into_inner);
                };
                drop(state);
                let written = sink.write_all(&chunk).and_then(|()| sink.flush());
                let mut state = theirs.lock();
                state.bytes = state.bytes.saturating_sub(chunk.len());
                if written.is_err() {
                    state.failed = true;
                    state.chunks.clear();
                    state.bytes = 0;
                    return;
                }
            }
        });
        Self { shared, pending: Vec::new(), writer: Some(writer) }
    }

    /// Queues what was written and lets the thread end once it is all
    /// written, which ends the pipe.
    fn end(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        let mut state = self.shared.lock();
        if !pending.is_empty() && !state.failed {
            state.bytes = state.bytes.saturating_add(pending.len());
            state.chunks.push_back(pending);
        }
        state.closed = true;
        self.shared.ready.notify_one();
    }

    /// Ends the pipe and waits until everything queued has been written
    /// to it, for a process that is to read it all before it ends.
    pub fn close(mut self) {
        self.end();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

impl Drop for Outbox {
    /// The pipe ends once what was queued is written; the thread is not
    /// waited for, so a process that has stopped reading holds nothing up.
    fn drop(&mut self) {
        self.end();
    }
}

impl Write for Outbox {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    /// Queues what was written since the last flush, as one chunk.
    ///
    /// # Errors
    /// The process stopped reading, the pipe broke, or the outbox was
    /// closed: nothing more can be sent to it.
    fn flush(&mut self) -> io::Result<()> {
        let mut state = self.shared.lock();
        if state.failed {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the process stopped reading"));
        }
        if state.closed {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the pipe was closed"));
        }
        if self.pending.is_empty() {
            return Ok(());
        }
        let queued = state.bytes.saturating_add(self.pending.len());
        if queued > MAX_QUEUED_BYTES {
            // The thread is left to finish the write it is in, if any,
            // then ends; the pending bytes are dropped with the rest.
            state.failed = true;
            state.closed = true;
            state.chunks.clear();
            state.bytes = 0;
            self.shared.ready.notify_one();
            self.pending.clear();
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "the process has stopped reading"));
        }
        state.bytes = queued;
        state.chunks.push_back(std::mem::take(&mut self.pending));
        self.shared.ready.notify_one();
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
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
}
