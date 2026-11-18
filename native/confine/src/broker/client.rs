//! The core's side of the broker: its connection, and the helpers the
//! broker started for it.

use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use super::wire::{Exit, Reply, Request, decode_reply, encode_request};
use super::{Channel, ChooseKind, MAX_UPDATE_STATE, Program, SEALING_KEY_BYTES};
use crate::{LocalTime, Process, Report};

/// The core's connection to its broker.
pub struct Broker {
    channel: Mutex<Channel>,
}

impl fmt::Debug for Broker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Broker")
    }
}

/// A helper the broker started for the core, or one the core started
/// itself in a test.
#[derive(Debug)]
pub struct Helper {
    /// What the core writes to the helper.
    pub stdin: Option<File>,
    /// What the helper writes to the core.
    pub stdout: Option<File>,
    /// How it was confined.
    pub report: Report,
    how: How,
}

#[derive(Debug)]
enum How {
    Brokered { id: u32, broker: Arc<Broker> },
    Local(Process),
}

impl Helper {
    /// A helper started in this process with [`crate::spawn`], for tests.
    #[must_use]
    pub fn local(mut process: Process) -> Self {
        let (stdin, stdout) = process.take_files();
        let report = process.report.clone();
        Self { stdin, stdout, report, how: How::Local(process) }
    }

    /// Ends it at once.
    ///
    /// # Errors
    /// When it cannot be ended.
    pub fn kill(&mut self) -> io::Result<()> {
        match &mut self.how {
            How::Brokered { id, broker } => match broker.ask(&Request::Kill(*id))? {
                (Reply::Done, _) => Ok(()),
                (Reply::Failed(reason), _) => Err(io::Error::other(reason)),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, "an unexpected reply")),
            },
            How::Local(process) => process.kill(),
        }
    }

    /// The processor time it has used since it started, in microseconds,
    /// as the operating system accounts it (REQ-6626 item 1 a): asked of
    /// the broker, which started it, never of the helper itself.
    ///
    /// # Errors
    /// When it cannot be read.
    pub fn processor_time_us(&self) -> io::Result<u64> {
        match &self.how {
            How::Brokered { id, broker } => match broker.ask(&Request::ProcessorTime(*id))? {
                (Reply::Spent(microseconds), _) => Ok(microseconds),
                (Reply::Failed(reason), _) => Err(io::Error::other(reason)),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, "an unexpected reply")),
            },
            How::Local(process) => process.processor_time_us(),
        }
    }

    /// Waits for it to end.
    ///
    /// # Errors
    /// When it cannot be waited for.
    pub fn wait(&mut self) -> io::Result<Exit> {
        match &mut self.how {
            How::Brokered { id, broker } => match broker.ask(&Request::Wait(*id))? {
                (Reply::Exited(exit), _) => Ok(exit),
                (Reply::Failed(reason), _) => Err(io::Error::other(reason)),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, "an unexpected reply")),
            },
            How::Local(process) => process.wait().map(Exit::from),
        }
    }
}

impl Broker {
    /// The core's side of the channel its broker started it with: its
    /// standard input (and, on Windows, output).
    ///
    /// # Errors
    /// When this process was not started by a broker.
    pub fn from_parent() -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self { channel: Mutex::new(Channel::from_stdin()?) }))
    }

    /// The core's side of `channel`, whose other end a test's stand-in
    /// broker serves (`stand_in`).
    #[cfg(all(unix, feature = "test-support"))]
    pub(crate) fn over(channel: Channel) -> Arc<Self> {
        Arc::new(Self { channel: Mutex::new(channel) })
    }

    /// Calls `ended`, on a thread of its own, once the broker has ended
    /// (TODO-2006): the broker ended by a signal or a crash leaves the
    /// core with no one to start, end or wait for its helpers, so the core
    /// ends too, and each helper with the pipe the core held to it. On
    /// Linux the core is killed with its broker as well (`end_with_parent`);
    /// on Windows its job, which the broker holds, ends it.
    ///
    /// # Errors
    /// When the channel cannot be watched.
    #[cfg(unix)]
    pub fn when_ended(&self, ended: impl FnOnce() + Send + 'static) -> io::Result<()> {
        let watched = self.channel.lock().unwrap_or_else(PoisonError::into_inner).watched()?;
        std::thread::spawn(move || {
            crate::unix::wait_for_hang_up(&watched);
            ended();
        });
        Ok(())
    }

    /// Sends one request and reads its reply, with the pipes that came
    /// with it.
    fn ask(&self, request: &Request) -> io::Result<(Reply, Vec<File>)> {
        let bytes = encode_request(request)?;
        let mut channel = self.channel.lock().unwrap_or_else(PoisonError::into_inner);
        channel.write(&bytes)?;
        let (kind, payload, pipes) = channel.receive_reply()?;
        Ok((decode_reply(kind, payload)?, pipes))
    }

    /// Starts `program` with `arguments`.
    ///
    /// # Errors
    /// When the broker cannot or will not start it.
    pub fn spawn(self: &Arc<Self>, program: Program, arguments: &[String], local_network: bool) -> io::Result<Helper> {
        match self.ask(&Request::Spawn { program, local_network, arguments: arguments.to_vec() })? {
            (Reply::Spawned { id, report }, pipes) => {
                let mut pipes = pipes.into_iter();
                let (Some(stdin), Some(stdout), None) = (pipes.next(), pipes.next(), pipes.next()) else {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "a helper without its two pipes"));
                };
                Ok(Helper {
                    stdin: Some(stdin),
                    stdout: Some(stdout),
                    report: Report { layers: vec![report] },
                    how: How::Brokered { id, broker: Arc::clone(self) },
                })
            }
            (Reply::Failed(reason), _) => Err(io::Error::other(reason)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "an unexpected reply")),
        }
    }

    /// A file channel's bytes, read by the broker.
    ///
    /// # Errors
    /// Why it could not be read, in words for the log.
    pub fn read_source(&self, path: &Path) -> Result<Vec<u8>, String> {
        let text = path.to_str().ok_or("the path is not text")?;
        match self.ask(&Request::Read(text.to_owned())).map_err(|error| error.to_string())? {
            (Reply::Source(bytes), _) => Ok(bytes),
            (Reply::Failed(reason), _) => Err(reason),
            _ => Err(String::from("an unexpected reply")),
        }
    }

    /// The time and the local time zone's offset, as the broker reads them.
    ///
    /// # Errors
    /// Why they could not be had.
    pub fn local_time(&self) -> Result<LocalTime, String> {
        match self.ask(&Request::LocalTime).map_err(|error| error.to_string())? {
            (Reply::Time(time), _) => Ok(time),
            (Reply::Failed(reason), _) => Err(reason),
            _ => Err(String::from("an unexpected reply")),
        }
    }

    /// The viewer key file's sealing key, from the platform's secret store
    /// (VS-108).
    ///
    /// # Errors
    /// Why it could not be had: no secret store, or one that refused.
    pub fn viewer_keys_key(&self) -> Result<[u8; SEALING_KEY_BYTES], String> {
        match self.ask(&Request::ViewerKeysKey).map_err(|error| error.to_string())? {
            (Reply::Key(key), _) => key.try_into().map_err(|_| String::from("a key of the wrong length")),
            (Reply::Failed(reason), _) => Err(reason),
            _ => Err(String::from("an unexpected reply")),
        }
    }

    /// Says the version is healthy, for the launcher (section 3.2 of the
    /// installation design).
    ///
    /// # Errors
    /// Why it could not be said.
    pub fn healthy(&self) -> Result<(), String> {
        match self.ask(&Request::Healthy).map_err(|error| error.to_string())? {
            (Reply::Done, _) => Ok(()),
            (Reply::Failed(reason), _) => Err(reason),
            _ => Err(String::from("an unexpected reply")),
        }
    }

    /// The launcher's state file's bytes, which the core reads strictly.
    ///
    /// # Errors
    /// When no launcher started the browser, or the state cannot be read.
    pub fn update_state(&self) -> Result<Vec<u8>, String> {
        match self.ask(&Request::UpdateState).map_err(|error| error.to_string())? {
            (Reply::Source(bytes), _) if bytes.len() <= MAX_UPDATE_STATE => Ok(bytes),
            (Reply::Failed(reason), _) => Err(reason),
            _ => Err(String::from("an unexpected reply")),
        }
    }

    /// Asks for the file dialog; the choice arrives on [`Broker::openings`]'
    /// pipe, or nothing does when the user cancels.
    ///
    /// # Errors
    /// Why it was not shown: one is open already, or it cannot be started.
    pub fn choose(&self, kind: ChooseKind, parent: &str, keep: bool) -> Result<(), String> {
        match self.ask(&Request::Choose { kind, parent: parent.to_owned(), keep }).map_err(|error| error.to_string())? {
            (Reply::Done, _) => Ok(()),
            (Reply::Failed(reason), _) => Err(reason),
            _ => Err(String::from("an unexpected reply")),
        }
    }

    /// A picture the user chose in the picture dialog, read by the broker
    /// (`docs/picture-tools.md`).
    ///
    /// # Errors
    /// Why it could not be read, in words for the log.
    pub fn read_picture(&self, path: &Path) -> Result<Vec<u8>, String> {
        let text = path.to_str().ok_or("the path is not text")?;
        match self.ask(&Request::ReadPicture(text.to_owned())).map_err(|error| error.to_string())? {
            (Reply::Source(bytes), _) => Ok(bytes),
            (Reply::Failed(reason), _) => Err(reason),
            _ => Err(String::from("an unexpected reply")),
        }
    }

    /// Has the broker write `bytes` where the user last chose to save a page
    /// of `kind`.
    ///
    /// # Errors
    /// Why it was not written: no such choice, or the system refused.
    pub fn save(&self, kind: ChooseKind, bytes: &[u8]) -> Result<(), String> {
        match self.ask(&Request::Save { kind, bytes: bytes.to_vec() }).map_err(|error| error.to_string())? {
            (Reply::Done, _) => Ok(()),
            (Reply::Failed(reason), _) => Err(reason),
            _ => Err(String::from("an unexpected reply")),
        }
    }

    /// The pipe on which what opens from outside arrives.
    ///
    /// # Errors
    /// When it was given already, or the broker has none.
    pub fn openings(&self) -> io::Result<File> {
        match self.ask(&Request::Openings)? {
            (Reply::Pipe, pipes) => {
                let mut pipes = pipes.into_iter();
                match (pipes.next(), pipes.next()) {
                    (Some(pipe), None) => Ok(pipe),
                    _ => Err(io::Error::new(io::ErrorKind::InvalidData, "a reply without its pipe")),
                }
            }
            (Reply::Failed(reason), _) => Err(io::Error::other(reason)),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "an unexpected reply")),
        }
    }

    /// Hands `address` to the system's browser, through the broker.
    ///
    /// # Errors
    /// Why it was not opened.
    pub fn open_web(&self, address: &str) -> Result<(), String> {
        match self.ask(&Request::OpenWeb(address.to_owned())).map_err(|error| error.to_string())? {
            (Reply::Done, _) => Ok(()),
            (Reply::Failed(reason), _) => Err(reason),
            _ => Err(String::from("an unexpected reply")),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    /// Longer than a hang-up takes to be heard, and than a reply waiting
    /// unread is looked at again.
    const HEARD_WITHIN: Duration = Duration::from_secs(5);
    /// Long enough for a watch that wrongly took a reply for the end to
    /// have said so.
    const QUIET_FOR: Duration = Duration::from_millis(300);

    /// TODO-2006: the core hears its broker end, however it ends, and not
    /// before: a reply waiting unread is not taken for the end, and is left
    /// for the thread that asked.
    #[test]
    fn the_core_hears_its_broker_end_and_nothing_else() {
        let (core_end, mut broker_end) = UnixStream::pair().unwrap();
        let broker = Broker { channel: Mutex::new(Channel::new(core_end)) };
        let (said, heard) = mpsc::channel();
        broker.when_ended(move || said.send(()).unwrap()).unwrap();
        broker_end.write_all(b"a reply").unwrap();
        assert!(heard.recv_timeout(QUIET_FOR).is_err(), "a reply is not the broker's end");
        drop(broker_end);
        assert!(heard.recv_timeout(HEARD_WITHIN).is_ok(), "the broker's end was not heard");
    }
}
