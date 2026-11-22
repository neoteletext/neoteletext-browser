//! A stand-in for the broker, for the trusted core's tests (finding F-98):
//! the core is given a [`Broker`] connected, over the same socket and wire
//! as the real one, to a thread of the test's process instead of the
//! broker's. Started with [`StandIn::start`], it starts no program: each
//! helper the core asks for is two pipes whose far ends the stand-in holds;
//! what the core writes to the helper is kept, for the test to read, and
//! what the helper says is what the test chose. Started with
//! [`StandIn::start_real`], a decoder or fetcher the core asks for is the
//! real program, confined as the broker confines it, its pipes handed to
//! the core as the broker hands them (TODO-1254); every other program is
//! still two pipes. Every request is kept either way, so a test sees which
//! helpers the core started, which files it asked for and what it handed
//! to the system's browser.
//!
//! Only with the `test-support` feature, which only the trusted core's
//! tests turn on (a dev-dependency's feature): no shipped program holds it.

use std::collections::BTreeMap;
use std::io::{self, Read as _, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::wire::{Exit, Reply, Request, decode_request, encode_reply, header};
use super::{Broker, Channel, HEADER_BYTES, MAX_HELPERS, MAX_REPLY, MAX_REQUEST, Program};
use crate::Process;
use crate::unix::send;

/// What the core has written to one helper that is kept: far more than a
/// test's helper is sent; past it, the rest is read and dropped, and the
/// helper's record says so.
pub const MAX_KEPT: usize = 64 * 1024 * 1024;

/// What is read from a helper's input at a time: a pipe's usual size.
const READ_CHUNK: usize = 64 * 1024;

/// How the stand-in confined a helper, as the core reports it.
const REPORT: &str = "a test's stand-in broker";

/// Reads a file channel for the core, as the broker's handler does.
pub type ReadSource = Box<dyn Fn(&Path) -> Result<Vec<u8>, String> + Send>;
/// What a helper says first, once started: `None` ends it at once.
pub type FirstWords = Box<dyn Fn(&Started) -> Option<Vec<u8>> + Send>;

/// A helper the core asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Started {
    /// The number the core names it by.
    pub id: u32,
    /// Which program.
    pub program: Program,
    /// Its arguments.
    pub arguments: Vec<String>,
    /// Whether it may reach the local network.
    pub local_network: bool,
    /// The UDP port its sandbox was to allow, a fetcher's alone.
    pub udp_port: Option<u16>,
    /// How it was confined, as the core is told: the real program's
    /// report, or the stand-in's own words for two pipes.
    pub report: String,
}

/// What the core wrote to one helper, once its end of the pipe closed.
#[derive(Debug)]
pub struct Written {
    /// The helper.
    pub helper: Started,
    /// The bytes, at most [`MAX_KEPT`].
    pub bytes: Vec<u8>,
    /// More than [`MAX_KEPT`] was written.
    pub cut: bool,
}

/// The stand-in's record.
#[derive(Default)]
struct Record {
    /// Every request not yet taken, in the order it came.
    requests: Vec<Request>,
    /// Every helper started.
    started: Vec<Started>,
    /// How many of them are two pipes, whose input is kept.
    piped: usize,
}

/// A stand-in broker serving one core.
pub struct StandIn {
    record: Arc<Mutex<Record>>,
    /// Each helper's input, as its pipe closes.
    written: Receiver<Written>,
    /// The serving thread has ended: the core's side has gone.
    served: Receiver<()>,
}

impl StandIn {
    /// A broker for a core under test, and the stand-in serving it, which
    /// reads file channels with `read_source` and has each helper say
    /// `first_words`.
    ///
    /// # Errors
    /// When the socket between them cannot be made.
    pub fn start(read_source: ReadSource, first_words: FirstWords) -> io::Result<(Arc<Broker>, Self)> {
        Self::serving(read_source, first_words, None)
    }

    /// As [`StandIn::start`], but a decoder or fetcher the core asks for is
    /// the real program in `programs`, the folder the build put it in,
    /// started confined as its program's role says, as the broker starts
    /// it; `first_words` are said by the other programs' pipes.
    ///
    /// # Errors
    /// When the socket between them cannot be made.
    pub fn start_real(read_source: ReadSource, first_words: FirstWords, programs: PathBuf) -> io::Result<(Arc<Broker>, Self)> {
        Self::serving(read_source, first_words, Some(programs))
    }

    fn serving(read_source: ReadSource, first_words: FirstWords, programs: Option<PathBuf>) -> io::Result<(Arc<Broker>, Self)> {
        let (core, ours) = UnixStream::pair()?;
        let record = Arc::new(Mutex::new(Record::default()));
        let (written_sender, written) = mpsc::channel();
        let (served_sender, served) = mpsc::channel();
        let theirs = Arc::clone(&record);
        std::thread::spawn(move || {
            let mut serving = Server {
                stream: ours,
                record: theirs,
                outputs: BTreeMap::new(),
                alive: BTreeMap::new(),
                programs,
                read_source,
                first_words,
                written: written_sender,
                next: 1,
            };
            serving.serve();
            // As the broker does once the core has gone: every real helper
            // still alive is ended.
            for (_, mut helper) in std::mem::take(&mut serving.alive) {
                let _ = helper.kill();
                let _ = helper.wait();
            }
            drop(serving);
            let _ = served_sender.send(());
        });
        Ok((Broker::over(Channel::new(core)), Self { record, written, served }))
    }

    /// Every request made since the last call, in order. A request the
    /// core made on its own thread is here once that call has returned,
    /// since the stand-in keeps it before it answers.
    #[must_use]
    pub fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut self.lock().requests)
    }

    /// Every helper started so far.
    #[must_use]
    pub fn started(&self) -> Vec<Started> {
        self.lock().started.clone()
    }

    /// Once the core and every helper it was given have gone, what it
    /// wrote to each helper of two pipes, in the order their pipes closed;
    /// a real helper read its own input.
    ///
    /// # Errors
    /// When the core's side, or a helper's input, is still open at
    /// `deadline` from now.
    pub fn finish(self, deadline: Duration) -> Result<Vec<Written>, String> {
        let until = Instant::now().checked_add(deadline).ok_or("a deadline too far")?;
        let left = || until.saturating_duration_since(Instant::now());
        self.served.recv_timeout(left()).map_err(|_| String::from("the core's side of the broker is still open"))?;
        let count = self.lock().piped;
        let mut all = Vec::with_capacity(count);
        while all.len() < count {
            let written =
                self.written.recv_timeout(left()).map_err(|_| format!("{} of {count} helpers' inputs are still open", count.saturating_sub(all.len())))?;
            all.push(written);
        }
        Ok(all)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Record> {
        self.record.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The serving thread's state.
struct Server {
    stream: UnixStream,
    record: Arc<Mutex<Record>>,
    /// Each live helper's output, which the core reads: dropped when the
    /// core ends the helper.
    outputs: BTreeMap<u32, io::PipeWriter>,
    /// Each real helper not yet waited for.
    alive: BTreeMap<u32, Process>,
    /// Where the real decoder and fetcher are, when they are started.
    programs: Option<PathBuf>,
    read_source: ReadSource,
    first_words: FirstWords,
    written: Sender<Written>,
    next: u32,
}

impl Server {
    /// Answers requests until the core's side closes or breaks the wire.
    fn serve(&mut self) {
        while let Ok(request) = self.read_request() {
            if self.answer(request).is_err() {
                return;
            }
        }
    }

    fn read_request(&mut self) -> io::Result<Request> {
        let mut head = [0_u8; HEADER_BYTES];
        self.stream.read_exact(&mut head)?;
        let (kind, length) = header(head, MAX_REQUEST)?;
        let mut payload = Vec::with_capacity(length);
        (&self.stream).take(length as u64).read_to_end(&mut payload)?;
        if payload.len() != length {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        decode_request(kind, &payload)
    }

    fn answer(&mut self, request: Request) -> io::Result<()> {
        let reply = match &request {
            Request::Spawn { program, local_network, udp_port, arguments } => {
                let helper = Started {
                    id: self.next,
                    program: *program,
                    arguments: arguments.clone(),
                    local_network: *local_network,
                    udp_port: *udp_port,
                    report: String::from(REPORT),
                };
                if self.outputs.len().saturating_add(self.alive.len()) >= MAX_HELPERS {
                    Reply::Failed(String::from("too many helpers"))
                } else {
                    let real = self
                        .programs
                        .as_ref()
                        .filter(|_| matches!(program, Program::Decoder | Program::Fetcher | Program::ScriptHost))
                        .map(|programs| programs.join(program.file_name()));
                    self.keep(request);
                    return match real {
                        Some(path) => self.start_program(helper, &path),
                        None => self.start(helper),
                    };
                }
            }
            Request::Kill(id) => {
                self.outputs.remove(id);
                if let Some(helper) = self.alive.get_mut(id) {
                    let _ = helper.kill();
                }
                Reply::Done
            }
            Request::ProcessorTime(id) => match self.alive.get(id).map(Process::processor_time_us) {
                Some(Ok(microseconds)) => Reply::Spent(microseconds),
                Some(Err(error)) => Reply::Failed(error.to_string()),
                None => Reply::Failed(String::from("no such helper")),
            },
            Request::Wait(id) => {
                self.outputs.remove(id);
                if let Some(mut helper) = self.alive.remove(id) {
                    match helper.wait() {
                        Ok(status) => Reply::Exited(Exit::from(status)),
                        Err(error) => Reply::Failed(error.to_string()),
                    }
                } else if self.lock().started.iter().any(|helper| helper.id == *id) {
                    Reply::Exited(Exit { success: true, text: String::from(REPORT) })
                } else {
                    Reply::Failed(String::from("no such helper"))
                }
            }
            Request::Read(path) => match (self.read_source)(Path::new(path)) {
                Ok(bytes) if bytes.len() <= MAX_REPLY => Reply::Source(bytes),
                Ok(_) => Reply::Failed(String::from("the channel is too large")),
                Err(reason) => Reply::Failed(reason),
            },
            Request::OpenWeb(_) | Request::Healthy | Request::Choose { .. } => Reply::Done,
            Request::LocalTime => match crate::local_time() {
                Ok(time) => Reply::Time(time),
                Err(error) => Reply::Failed(error.to_string()),
            },
            Request::ViewerKeysKey => Reply::Failed(String::from("a test has no secret store")),
            Request::UpdateState => Reply::Failed(String::from("no launcher started this broker")),
            Request::Openings => Reply::Failed(String::from("a test opens nothing from outside")),
            Request::ReadPicture(_) | Request::Save { .. } => Reply::Failed(String::from("a test has no pictures")),
            Request::ReadFont(_) => Reply::Failed(String::from("a test has no fonts")),
            // A test's reader is asked for the script as the file of its
            // name beside the page file: the real broker's rules for it
            // are the browser's own (`core/src/source.rs`).
            Request::ReadScript { file, name } => match (self.read_source)(&Path::new(file).with_file_name(name)) {
                Ok(bytes) if bytes.len() <= MAX_REPLY => Reply::Source(bytes),
                Ok(_) => Reply::Failed(String::from("the script is too large")),
                Err(reason) => Reply::Failed(reason),
            },
        };
        self.keep(request);
        self.stream.write_all(&encode_reply(&reply)?)
    }

    /// Keeps `request`, before it is answered.
    fn keep(&self, request: Request) {
        self.lock().requests.push(request);
    }

    /// Makes `helper`'s two pipes, sends the core its ends, and keeps what
    /// the core writes to it.
    fn start(&mut self, helper: Started) -> io::Result<()> {
        let (mut from_core, core_writes) = io::pipe()?;
        let (core_reads, mut to_core) = io::pipe()?;
        let reply = encode_reply(&Reply::Spawned { id: helper.id, report: String::from(REPORT) })?;
        send(&self.stream, &reply, &[core_writes.as_raw_fd(), core_reads.as_raw_fd()])?;
        // The core holds its own ends now.
        drop((core_writes, core_reads));
        self.next = self.next.wrapping_add(1).max(1);
        {
            let mut record = self.lock();
            record.started.push(helper.clone());
            record.piped = record.piped.saturating_add(1);
        }
        if let Some(words) = (self.first_words)(&helper) {
            // A helper's first words fit in its pipe; a test's helper that
            // says more holds its output open, unread, as a slow one would.
            to_core.write_all(&words)?;
            self.outputs.insert(helper.id, to_core);
        }
        let written = self.written.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut cut = false;
            let mut buffer = vec![0_u8; READ_CHUNK];
            while let Ok(read) = from_core.read(&mut buffer) {
                let Some(chunk) = buffer.get(..read).filter(|chunk| !chunk.is_empty()) else { break };
                let room = MAX_KEPT.saturating_sub(bytes.len());
                bytes.extend_from_slice(chunk.get(..room.min(chunk.len())).unwrap_or_default());
                cut |= chunk.len() > room;
            }
            let _ = written.send(Written { helper, bytes, cut });
        });
        Ok(())
    }

    /// Starts `helper` as the real program at `path`, confined as the
    /// broker confines it, and sends the core its pipes as the broker does.
    fn start_program(&mut self, mut helper: Started, path: &Path) -> io::Result<()> {
        let role = helper.program.role();
        let started = super::server::start_confined(path, role, (helper.local_network, helper.udp_port), &helper.arguments, &crate::environment(role), None);
        let mut process = match started {
            Ok(process) => process,
            Err(error) => {
                let reply = Reply::Failed(format!("cannot start the {}: {error}", helper.program.name()));
                return self.stream.write_all(&encode_reply(&reply)?);
            }
        };
        let (Some(stdin), Some(stdout)) = (process.stdin.take(), process.stdout.take()) else {
            let _ = process.kill();
            let _ = process.wait();
            return self.stream.write_all(&encode_reply(&Reply::Failed(String::from("a helper without its pipes")))?);
        };
        helper.report = process.report.to_string();
        let reply = encode_reply(&Reply::Spawned { id: helper.id, report: helper.report.clone() })?;
        send(&self.stream, &reply, &[stdin.as_raw_fd(), stdout.as_raw_fd()])?;
        // Our copies close here; the core holds its own.
        drop((stdin, stdout));
        self.alive.insert(helper.id, process);
        self.next = self.next.wrapping_add(1).max(1);
        self.lock().started.push(helper);
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Record> {
        self.record.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
