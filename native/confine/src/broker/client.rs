//! The core's side of the broker: its connection, and the helpers the
//! broker started for it.

use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use super::wire::{Exit, Reply, Request, decode_reply, encode_request};
use super::{COOKIE_KEY_BYTES, Channel, MAX_UPDATE_STATE, Program};
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

    /// The cookie file's key, from the platform's secret store.
    ///
    /// # Errors
    /// Why it could not be had: no secret store, or one that refused.
    pub fn cookie_key(&self) -> Result<[u8; COOKIE_KEY_BYTES], String> {
        match self.ask(&Request::CookieKey).map_err(|error| error.to_string())? {
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
