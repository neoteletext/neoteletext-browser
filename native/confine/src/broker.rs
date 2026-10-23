//! The broker (decision 56): the one unconfined process, which the user
//! starts. It starts the trusted core confined, and then, on the core's
//! request, every helper, each confined as its program's role says, and
//! passes the core the helper's two pipes. It also reads a file channel's
//! bytes for the core and hands a web address to the system's browser for
//! it, so the core itself needs no files but its settings, no network and
//! no programs.
//!
//! A compromised core gains nothing from the broker: it can name only one
//! of the browser's own helpers, never a program or a role; the broker
//! confines each one by its program; a file is read through the broker's
//! own rules and bounds; a web address is checked by `web-opener` as
//! before. The broker parses only the core's requests, each bounded.
//!
//! The channel is a local-domain socket on macOS and Linux, which carries
//! the pipes as descriptors (`unix`), and two pipes on Windows, where the
//! broker duplicates the helper's pipe handles into the core's process
//! and sends their values.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use crate::{LocalTime, Process, Report, Role, Spawn};

/// The most helpers alive at once: far more than tabs (`protocol::MAX_TABS`
/// is 64, each with a decoder and a fetcher), and few enough that a
/// compromised core cannot exhaust the machine.
pub const MAX_HELPERS: usize = 512;
/// The most arguments a helper is started with, and the longest.
pub const MAX_ARGUMENTS: usize = 64;
/// The longest argument, path or address in a request.
pub const MAX_TEXT: usize = 4096;
/// The largest reply: a file channel's bytes, at most 64 MB with a line
/// end for each of at most 4,096 files (`source.rs`), and room to spare.
pub const MAX_REPLY: usize = 64 * 1024 * 1024 + 64 * 1024;
/// A request is never more than its arguments.
const MAX_REQUEST: usize = MAX_ARGUMENTS * (MAX_TEXT + 2) + 16;
/// A frame's kind byte and big-endian length.
const HEADER_BYTES: usize = 5;

const KIND_SPAWN: u8 = 1;
const KIND_KILL: u8 = 2;
const KIND_WAIT: u8 = 3;
const KIND_READ: u8 = 4;
const KIND_OPEN_WEB: u8 = 5;
const KIND_LOCAL_TIME: u8 = 6;
const KIND_COOKIE_KEY: u8 = 7;
const KIND_HEALTHY: u8 = 8;
const KIND_UPDATE_STATE: u8 = 9;
const KIND_SPAWNED: u8 = 11;
const KIND_DONE: u8 = 12;
const KIND_EXITED: u8 = 13;
const KIND_SOURCE: u8 = 14;
const KIND_FAILED: u8 = 15;
const KIND_TIME: u8 = 16;
const KIND_KEY: u8 = 17;
/// The cookie file's key: 256 bits (`cookies.md` section 7).
pub const COOKIE_KEY_BYTES: usize = 32;
/// The port of TLS, the update host's (`installation-and-updates.md`
/// section 8.1).
const HTTPS_PORT: u16 = 443;
/// The longest launcher state the broker hands the core: the state file's
/// own bound (`update::state::MAX_STATE`), with room.
pub const MAX_UPDATE_STATE: usize = 4 * 1024;

/// The browser's own helpers: the only programs the broker starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// A channel's decoder.
    Decoder,
    /// A web channel's fetcher.
    Fetcher,
    /// The window.
    Interface,
    /// The phone keyboard.
    Keyboard,
    /// The sound output (decision 55).
    Sound,
    /// The updater (`installation-and-updates.md` section 3).
    Updater,
}

/// What the broker's own rules give the updater: the folder it writes and
/// the one port it reaches (`installation-and-updates.md` section 8.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdaterGrant {
    /// Its staging folder.
    pub staging: PathBuf,
    /// The update host's port, or the proxy's.
    pub port: u16,
}

impl Program {
    const ALL: [Self; 6] = [Self::Decoder, Self::Fetcher, Self::Interface, Self::Keyboard, Self::Sound, Self::Updater];

    /// The role it is confined as, decided here and never by the core; the
    /// updater's port is the broker's handler's (`Handlers::updater`), and
    /// this its role on the port of TLS.
    #[must_use]
    pub fn role(self) -> Role {
        match self {
            Self::Decoder => Role::Decoder,
            Self::Fetcher | Self::Keyboard => Role::Network,
            Self::Interface => Role::Interface,
            Self::Sound => Role::Sound,
            Self::Updater => Role::Updater { port: HTTPS_PORT },
        }
    }

    /// What it is called in what the browser reports.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Decoder => "decoder",
            Self::Fetcher => "fetcher",
            Self::Interface => "interface",
            Self::Keyboard => "phone keyboard",
            Self::Sound => "sound helper",
            Self::Updater => "updater",
        }
    }

    /// Its program's name, beside the broker's own.
    #[must_use]
    pub fn file_name(self) -> String {
        let name = match self {
            Self::Decoder => "neoteletext-decoder",
            Self::Fetcher => "neoteletext-fetcher",
            Self::Interface => "neoteletext-interface",
            Self::Keyboard => "neoteletext-keyboard",
            Self::Sound => "neoteletext-sound",
            Self::Updater => "neoteletext-updater",
        };
        format!("{name}{}", std::env::consts::EXE_SUFFIX)
    }

    fn code(self) -> u8 {
        match self {
            Self::Decoder => 1,
            Self::Fetcher => 2,
            Self::Interface => 3,
            Self::Keyboard => 4,
            Self::Sound => 5,
            Self::Updater => 6,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|program| program.code() == code)
    }
}

/// How a helper ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    /// It ended successfully.
    pub success: bool,
    /// How, in the system's words.
    pub text: String,
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl From<std::process::ExitStatus> for Exit {
    fn from(status: std::process::ExitStatus) -> Self {
        Self { success: status.success(), text: status.to_string() }
    }
}

/// A request from the core.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// Start a helper.
    Spawn {
        /// Which.
        program: Program,
        /// May a network helper reach the local network.
        local_network: bool,
        /// Its arguments.
        arguments: Vec<String>,
    },
    /// End a helper.
    Kill(u32),
    /// Wait for a helper to end.
    Wait(u32),
    /// Read a file channel.
    Read(String),
    /// Hand a web address to the system's browser.
    OpenWeb(String),
    /// The time, and the local time zone's offset from it, which the core
    /// cannot read from the system's files (`history-and-bookmarks.md`
    /// section 7).
    LocalTime,
    /// The cookie file's key, from the platform's secret store, which the
    /// core cannot reach (`cookies.md` section 10).
    CookieKey,
    /// The version is healthy (`installation-and-updates.md` section 3.2):
    /// the broker tells the launcher, which commits a trial.
    Healthy,
    /// The launcher's state file, which the core may not read itself:
    /// answered with its bytes, or refused when no launcher started the
    /// broker.
    UpdateState,
}

/// The broker's answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    /// Started: its number and how it was confined; its pipes travel with
    /// the reply.
    Spawned {
        /// The number the core names it by.
        id: u32,
        /// How it was confined.
        report: String,
    },
    /// Done.
    Done,
    /// It ended.
    Exited(Exit),
    /// A file channel's bytes.
    Source(Vec<u8>),
    /// Refused, and why.
    Failed(String),
    /// The time and the local offset.
    Time(LocalTime),
    /// The cookie file's key.
    Key(Vec<u8>),
}

fn push_text(payload: &mut Vec<u8>, text: &str) -> io::Result<()> {
    let length = u16::try_from(text.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    if text.len() > MAX_TEXT {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "text too long"));
    }
    payload.extend_from_slice(&length.to_be_bytes());
    payload.extend_from_slice(text.as_bytes());
    Ok(())
}

/// Reads fields, refusing anything short, long or not UTF-8.
struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < count {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "message ends early"));
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?.first().copied().unwrap_or(0))
    }

    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| io::ErrorKind::InvalidData)?))
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| io::ErrorKind::InvalidData)?))
    }

    fn text(&mut self) -> io::Result<String> {
        let length = usize::from(self.u16()?);
        if length > MAX_TEXT {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "text too long"));
        }
        String::from_utf8(self.take(length)?.to_vec()).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "text not UTF-8"))
    }

    fn end(&self) -> io::Result<()> {
        if self.0.is_empty() { Ok(()) } else { Err(io::Error::new(io::ErrorKind::InvalidData, "message longer than its fields")) }
    }
}

fn frame(kind: u8, payload: &[u8]) -> io::Result<Vec<u8>> {
    let length = u32::try_from(payload.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut bytes = Vec::with_capacity(HEADER_BYTES.saturating_add(payload.len()));
    bytes.push(kind);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// A request as bytes.
///
/// # Errors
/// When it breaks a bound.
pub fn encode_request(request: &Request) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match request {
        Request::Spawn { program, local_network, arguments } => {
            if arguments.len() > MAX_ARGUMENTS {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "too many arguments"));
            }
            payload.push(program.code());
            payload.push(u8::from(*local_network));
            payload.extend_from_slice(&u16::try_from(arguments.len()).map_err(|_| io::ErrorKind::InvalidInput)?.to_be_bytes());
            for argument in arguments {
                push_text(&mut payload, argument)?;
            }
            KIND_SPAWN
        }
        Request::Kill(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_KILL
        }
        Request::Wait(id) => {
            payload.extend_from_slice(&id.to_be_bytes());
            KIND_WAIT
        }
        Request::Read(path) => {
            push_text(&mut payload, path)?;
            KIND_READ
        }
        Request::OpenWeb(address) => {
            push_text(&mut payload, address)?;
            KIND_OPEN_WEB
        }
        Request::LocalTime => KIND_LOCAL_TIME,
        Request::CookieKey => KIND_COOKIE_KEY,
        Request::Healthy => KIND_HEALTHY,
        Request::UpdateState => KIND_UPDATE_STATE,
    };
    frame(kind, &payload)
}

/// A request read from its kind and payload, strictly: the broker's one
/// input from another process.
///
/// # Errors
/// On any malformed or out-of-bounds request.
pub fn decode_request(kind: u8, payload: &[u8]) -> io::Result<Request> {
    let mut fields = Fields(payload);
    let request = match kind {
        KIND_SPAWN => {
            let program = Program::from_code(fields.u8()?).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no such program"))?;
            let local_network = match fields.u8()? {
                0 => false,
                1 => true,
                _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "a flag that is not one")),
            };
            let count = usize::from(fields.u16()?);
            if count > MAX_ARGUMENTS {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "too many arguments"));
            }
            let arguments = (0..count).map(|_| fields.text()).collect::<io::Result<Vec<_>>>()?;
            Request::Spawn { program, local_network, arguments }
        }
        KIND_KILL => Request::Kill(fields.u32()?),
        KIND_WAIT => Request::Wait(fields.u32()?),
        KIND_READ => Request::Read(fields.text()?),
        KIND_OPEN_WEB => Request::OpenWeb(fields.text()?),
        KIND_LOCAL_TIME => Request::LocalTime,
        KIND_COOKIE_KEY => Request::CookieKey,
        KIND_HEALTHY => Request::Healthy,
        KIND_UPDATE_STATE => Request::UpdateState,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "no such request")),
    };
    fields.end()?;
    Ok(request)
}

/// A reply as bytes.
///
/// # Errors
/// When it breaks a bound.
pub fn encode_reply(reply: &Reply) -> io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match reply {
        Reply::Spawned { id, report } => {
            payload.extend_from_slice(&id.to_be_bytes());
            push_text(&mut payload, report)?;
            KIND_SPAWNED
        }
        Reply::Done => KIND_DONE,
        Reply::Exited(exit) => {
            payload.push(u8::from(exit.success));
            push_text(&mut payload, &exit.text)?;
            KIND_EXITED
        }
        Reply::Source(bytes) => {
            if bytes.len() > MAX_REPLY {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "source too large"));
            }
            payload.extend_from_slice(bytes);
            KIND_SOURCE
        }
        Reply::Failed(reason) => {
            push_text(&mut payload, reason)?;
            KIND_FAILED
        }
        Reply::Time(time) => {
            if !time.is_plausible() {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "an offset no zone has"));
            }
            payload.extend_from_slice(&time.utc.to_be_bytes());
            payload.extend_from_slice(&time.offset.to_be_bytes());
            KIND_TIME
        }
        Reply::Key(key) => {
            if key.len() != COOKIE_KEY_BYTES {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "a key of the wrong length"));
            }
            payload.extend_from_slice(key);
            KIND_KEY
        }
    };
    frame(kind, &payload)
}

/// A reply read from its kind and payload, strictly.
///
/// # Errors
/// On any malformed reply.
pub fn decode_reply(kind: u8, payload: Vec<u8>) -> io::Result<Reply> {
    if kind == KIND_SOURCE {
        return Ok(Reply::Source(payload));
    }
    let mut fields = Fields(&payload);
    let reply = match kind {
        KIND_SPAWNED => Reply::Spawned { id: fields.u32()?, report: fields.text()? },
        KIND_DONE => Reply::Done,
        KIND_EXITED => {
            let success = fields.u8()? == 1;
            Reply::Exited(Exit { success, text: fields.text()? })
        }
        KIND_FAILED => Reply::Failed(fields.text()?),
        KIND_TIME => {
            let utc = i64::from_be_bytes(fields.take(8)?.try_into().map_err(|_| io::ErrorKind::InvalidData)?);
            let offset = i32::from_be_bytes(fields.take(4)?.try_into().map_err(|_| io::ErrorKind::InvalidData)?);
            let time = LocalTime { utc, offset };
            if !time.is_plausible() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "an offset no zone has"));
            }
            Reply::Time(time)
        }
        KIND_KEY => Reply::Key(fields.take(COOKIE_KEY_BYTES)?.to_vec()),
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "no such reply")),
    };
    fields.end()?;
    Ok(reply)
}

/// A frame's header: its kind, and its length if within `max`.
fn header(bytes: [u8; HEADER_BYTES], max: usize) -> io::Result<(u8, usize)> {
    let [kind, a, b, c, d] = bytes;
    let length = u32::from_be_bytes([a, b, c, d]) as usize;
    if length > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message too long"));
    }
    Ok((kind, length))
}

/// One end of the broker's channel.
struct Channel {
    #[cfg(unix)]
    stream: std::os::unix::net::UnixStream,
    #[cfg(windows)]
    reader: File,
    #[cfg(windows)]
    writer: File,
}

impl Channel {
    fn read_frame(&mut self, max: usize) -> io::Result<(u8, Vec<u8>)> {
        let mut bytes = [0_u8; HEADER_BYTES];
        #[cfg(unix)]
        let reader = &mut self.stream;
        #[cfg(windows)]
        let reader = &mut self.reader;
        reader.read_exact(&mut bytes)?;
        let (kind, length) = header(bytes, max)?;
        let mut payload = Vec::with_capacity(length);
        reader.take(length as u64).read_to_end(&mut payload)?;
        if payload.len() != length {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok((kind, payload))
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        #[cfg(unix)]
        let writer = &mut self.stream;
        #[cfg(windows)]
        let writer = &mut self.writer;
        writer.write_all(bytes)?;
        writer.flush()
    }
}

// ---- The core's side ----

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
        #[cfg(unix)]
        let (stdin, stdout) = (
            process.stdin.take().map(|pipe| File::from(std::os::fd::OwnedFd::from(pipe))),
            process.stdout.take().map(|pipe| File::from(std::os::fd::OwnedFd::from(pipe))),
        );
        #[cfg(windows)]
        let (stdin, stdout) = (process.stdin.take(), process.stdout.take());
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
        #[cfg(unix)]
        {
            use std::os::fd::AsFd as _;
            let descriptor = io::stdin().as_fd().try_clone_to_owned()?;
            let stream = std::os::unix::net::UnixStream::from(descriptor);
            // Standard input that is not a socket was not set up by a broker.
            stream.peer_addr().map_err(|_| io::Error::other("this program is started by the browser's broker, not directly"))?;
            Ok(Arc::new(Self { channel: Mutex::new(Channel { stream }) }))
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsHandle as _;
            let reader = File::from(io::stdin().as_handle().try_clone_to_owned()?);
            let writer = File::from(io::stdout().as_handle().try_clone_to_owned()?);
            Ok(Arc::new(Self { channel: Mutex::new(Channel { reader, writer }) }))
        }
    }

    /// Sends one request and reads its reply, with the pipes that came
    /// with it.
    fn ask(&self, request: &Request) -> io::Result<(Reply, Vec<File>)> {
        let bytes = encode_request(request)?;
        let mut channel = self.channel.lock().unwrap_or_else(PoisonError::into_inner);
        channel.write(&bytes)?;
        #[cfg(unix)]
        {
            let mut head = [0_u8; HEADER_BYTES];
            let descriptors = crate::unix::receive(&channel.stream, &mut head)?;
            let (kind, length) = header(head, MAX_REPLY)?;
            let mut payload = Vec::with_capacity(length);
            (&channel.stream).take(length as u64).read_to_end(&mut payload)?;
            if payload.len() != length {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let reply = decode_reply(kind, payload)?;
            Ok((reply, descriptors.into_iter().map(File::from).collect()))
        }
        #[cfg(windows)]
        {
            let (kind, mut payload) = channel.read_frame(MAX_REPLY)?;
            let mut pipes = Vec::new();
            if kind == KIND_SPAWNED {
                // Two handle values the broker duplicated into this process
                // follow the reply's own fields.
                let tail = payload.len().saturating_sub(16);
                let values: Vec<u8> = payload.split_off(tail);
                for chunk in values.as_chunks::<8>().0 {
                    pipes.push(crate::windows::file_from_handle_value(u64::from_be_bytes(*chunk))?);
                }
            }
            Ok((decode_reply(kind, payload)?, pipes))
        }
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

// ---- The broker's side ----

/// What the broker does for the core that is not starting helpers.
pub struct Handlers<'a> {
    /// Reads a file channel, by its rules and bounds.
    pub read_source: &'a dyn Fn(&Path) -> Result<Vec<u8>, String>,
    /// Hands a web address to the system's browser.
    pub open_web: &'a dyn Fn(&str) -> Result<(), String>,
    /// The environment a helper of a role is given.
    pub environment: &'a dyn Fn(Role) -> Vec<(OsString, OsString)>,
    /// The cookie file's key, read from the platform's secret store or
    /// made there.
    pub cookie_key: &'a dyn Fn() -> Result<Vec<u8>, String>,
    /// Tells the launcher the version is healthy.
    pub healthy: &'a dyn Fn() -> Result<(), String>,
    /// The launcher's state file's bytes.
    pub update_state: &'a dyn Fn() -> Result<Vec<u8>, String>,
    /// What the updater, started with these arguments, may write and
    /// reach; refused where no launcher installed the browser.
    pub updater: &'a dyn Fn(&[String]) -> Result<UpdaterGrant, String>,
}

/// The core, started confined, and the broker's end of its channel.
pub struct Core {
    process: Process,
    channel: Channel,
    /// How it was confined.
    pub report: Report,
}

/// Starts the trusted core: `program` with `arguments`, confined, its
/// settings folder `writable` the one place it may write, with the
/// environment `environment`.
///
/// # Errors
/// When it cannot be started confined.
pub fn start_core(program: &Path, arguments: &[OsString], writable: &Path, environment: &[(OsString, OsString)]) -> io::Result<Core> {
    #[cfg(unix)]
    {
        use std::process::Stdio;
        let (ours, theirs) = std::os::unix::net::UnixStream::pair()?;
        #[cfg(target_os = "macos")]
        let mut command = crate::macos::command(&Spawn { program, arguments, role: Role::Core, environment, local_network: false, writable: Some(writable) });
        #[cfg(not(target_os = "macos"))]
        let mut command = {
            let _ = writable;
            let mut command = std::process::Command::new(program);
            command.args(arguments);
            command
        };
        command.env_clear();
        for (name, value) in environment {
            command.env(name, value);
        }
        let child = command.stdin(Stdio::from(std::os::fd::OwnedFd::from(theirs))).stdout(Stdio::null()).stderr(Stdio::inherit()).spawn()?;
        #[cfg(target_os = "macos")]
        let report = Report { layers: vec![String::from("sandbox-exec")] };
        #[cfg(not(target_os = "macos"))]
        let report = Report { layers: vec![String::from("confines itself at its start")] };
        let process = Process { stdin: None, stdout: None, inner: child, report: report.clone() };
        Ok(Core { process, channel: Channel { stream: ours }, report })
    }
    #[cfg(windows)]
    {
        let mut process = crate::windows::spawn(&Spawn { program, arguments, role: Role::Core, environment, local_network: false, writable: Some(writable) })?;
        let writer = process.stdin.take().ok_or_else(|| io::Error::other("the core has no input"))?;
        let reader = process.stdout.take().ok_or_else(|| io::Error::other("the core has no output"))?;
        let report = process.report.clone();
        Ok(Core { process, channel: Channel { reader, writer }, report })
    }
}

/// The helpers the broker started, by number.
struct Helpers {
    alive: BTreeMap<u32, Process>,
    next: u32,
}

/// Serves the core until it ends or breaks the channel; then ends every
/// helper still alive, and returns how the core ended.
///
/// # Errors
/// When the core cannot be waited for.
pub fn serve(mut core: Core, programs: &Path, handlers: &Handlers<'_>) -> io::Result<Exit> {
    let mut helpers = Helpers { alive: BTreeMap::new(), next: 1 };
    while let Ok((kind, payload)) = core.channel.read_frame(MAX_REQUEST) {
        let request = match decode_request(kind, &payload) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("neoteletext: the trusted core broke the broker's protocol and is ended: {error}");
                break;
            }
        };
        if answer(&mut core, &mut helpers, programs, handlers, request).is_err() {
            break;
        }
    }
    for (_, mut helper) in std::mem::take(&mut helpers.alive) {
        let _ = helper.kill();
        let _ = helper.wait();
    }
    drop(core.channel);
    core.process.wait().map(Exit::from)
}

/// Carries out one request and writes its reply.
fn answer(core: &mut Core, helpers: &mut Helpers, programs: &Path, handlers: &Handlers<'_>, request: Request) -> io::Result<()> {
    let reply = match request {
        Request::Spawn { program, local_network, arguments } => {
            if helpers.alive.len() >= MAX_HELPERS {
                Reply::Failed(String::from("too many helpers"))
            } else {
                match start_helper(programs, program, local_network, &arguments, handlers) {
                    Ok(mut process) => {
                        let id = helpers.next;
                        helpers.next = helpers.next.wrapping_add(1).max(1);
                        let report = process.report.to_string();
                        let (Some(stdin), Some(stdout)) = (process.stdin.take(), process.stdout.take()) else {
                            return core.channel.write(&encode_reply(&Reply::Failed(String::from("a helper without its pipes")))?);
                        };
                        helpers.alive.insert(id, process);
                        return send_spawned(core, id, &report, &stdin, &stdout);
                    }
                    Err(error) => Reply::Failed(format!("cannot start the {}: {error}", program.name())),
                }
            }
        }
        Request::Kill(id) => match helpers.alive.get_mut(&id) {
            Some(helper) => {
                let _ = helper.kill();
                Reply::Done
            }
            None => Reply::Done,
        },
        Request::Wait(id) => match helpers.alive.remove(&id) {
            Some(mut helper) => match helper.wait() {
                Ok(status) => Reply::Exited(Exit::from(status)),
                Err(error) => Reply::Failed(error.to_string()),
            },
            None => Reply::Failed(String::from("no such helper")),
        },
        Request::Read(path) => match (handlers.read_source)(Path::new(&path)) {
            Ok(bytes) if bytes.len() <= MAX_REPLY => Reply::Source(bytes),
            Ok(_) => Reply::Failed(String::from("the channel is too large")),
            Err(reason) => Reply::Failed(reason),
        },
        Request::OpenWeb(address) => match (handlers.open_web)(&address) {
            Ok(()) => Reply::Done,
            Err(reason) => Reply::Failed(reason),
        },
        Request::LocalTime => match crate::local_time() {
            Ok(time) => Reply::Time(time),
            Err(error) => Reply::Failed(format!("the local time cannot be read: {error}")),
        },
        Request::CookieKey => match (handlers.cookie_key)() {
            Ok(key) if key.len() == COOKIE_KEY_BYTES => Reply::Key(key),
            Ok(_) => Reply::Failed(String::from("the secret store holds a key of the wrong length")),
            Err(reason) => Reply::Failed(reason),
        },
        Request::Healthy => match (handlers.healthy)() {
            Ok(()) => Reply::Done,
            Err(reason) => Reply::Failed(reason),
        },
        Request::UpdateState => match (handlers.update_state)() {
            Ok(bytes) if bytes.len() <= MAX_UPDATE_STATE => Reply::Source(bytes),
            Ok(_) => Reply::Failed(String::from("the launcher's state is too long")),
            Err(reason) => Reply::Failed(reason),
        },
    };
    core.channel.write(&encode_reply(&reply)?)
}

/// Starts one of the browser's own helpers, beside the broker.
fn start_helper(programs: &Path, program: Program, local_network: bool, arguments: &[String], handlers: &Handlers<'_>) -> io::Result<Process> {
    let path: PathBuf = programs.join(program.file_name());
    // The updater's folder and port are the broker's rules', from where it
    // is installed and the proxy the core names, never the core's word.
    let grant = if program == Program::Updater { Some((handlers.updater)(arguments).map_err(io::Error::other)?) } else { None };
    let role = grant.as_ref().map_or(program.role(), |grant| Role::Updater { port: grant.port });
    let writable = grant.as_ref().map(|grant| grant.staging.as_path());
    let arguments: Vec<OsString> = arguments.iter().map(OsString::from).collect();
    let environment = (handlers.environment)(role);
    crate::spawn(&Spawn { program: &path, arguments: &arguments, role, environment: &environment, local_network, writable })
}

/// Sends the core a started helper's number, with its two pipes.
fn send_spawned(core: &mut Core, id: u32, report: &str, stdin: &crate::Input, stdout: &crate::Output) -> io::Result<()> {
    let bytes = encode_reply(&Reply::Spawned { id, report: report.to_owned() })?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd as _;
        crate::unix::send(&core.channel.stream, &bytes, &[stdin.as_raw_fd(), stdout.as_raw_fd()])
        // Our copies close here; the core holds its own.
    }
    #[cfg(windows)]
    {
        // The handles are duplicated into the core and their values sent
        // after the reply's fields; the frame's length counts them.
        let into = core.process.inner.raw_process();
        let values = [crate::windows::duplicate_into(stdin, into)?, crate::windows::duplicate_into(stdout, into)?];
        let mut payload = bytes.get(HEADER_BYTES..).unwrap_or_default().to_vec();
        for value in values {
            payload.extend_from_slice(&value.to_be_bytes());
        }
        core.channel.write(&frame(KIND_SPAWNED, &payload)?)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;
    use crate::MAX_OFFSET;

    fn round_trip(request: &Request) -> Request {
        let bytes = encode_request(request).unwrap();
        let (kind, length) = header(bytes[..HEADER_BYTES].try_into().unwrap(), MAX_REQUEST).unwrap();
        assert_eq!(length, bytes.len() - HEADER_BYTES);
        decode_request(kind, &bytes[HEADER_BYTES..]).unwrap()
    }

    #[test]
    fn every_request_and_reply_round_trips() {
        for request in [
            Request::Spawn { program: Program::Fetcher, local_network: true, arguments: vec!["http://a.example/".into(), "--proxy".into()] },
            Request::Kill(7),
            Request::Wait(9),
            Request::Read("/tmp/x.t42".into()),
            Request::OpenWeb("https://a.example/".into()),
            Request::LocalTime,
            Request::CookieKey,
            Request::Healthy,
            Request::UpdateState,
            Request::Spawn { program: Program::Updater, local_network: false, arguments: vec!["--proxy".into(), "http://proxy.example:3128".into()] },
        ] {
            assert_eq!(round_trip(&request), request);
        }
        for reply in [
            Reply::Spawned { id: 3, report: "sandbox-exec".into() },
            Reply::Done,
            Reply::Exited(Exit { success: false, text: "signal: 9".into() }),
            Reply::Source(vec![1, 2, 3]),
            Reply::Failed("no".into()),
            Reply::Time(LocalTime { utc: 1_792_656_300, offset: 39_600 }),
            Reply::Key(vec![9; COOKIE_KEY_BYTES]),
        ] {
            let bytes = encode_reply(&reply).unwrap();
            let (kind, _) = header(bytes[..HEADER_BYTES].try_into().unwrap(), MAX_REPLY).unwrap();
            assert_eq!(decode_reply(kind, bytes[HEADER_BYTES..].to_vec()).unwrap(), reply);
        }
    }

    /// A compromised core names only the browser's own programs, and every
    /// bound is held.
    #[test]
    fn a_request_out_of_bounds_is_refused() {
        assert!(decode_request(KIND_SPAWN, &[99, 0, 0, 0]).is_err(), "no such program");
        assert!(decode_request(KIND_SPAWN, &[1, 2, 0, 0]).is_err(), "a flag that is not one");
        let too_many = u16::try_from(MAX_ARGUMENTS + 1).unwrap().to_be_bytes();
        assert!(decode_request(KIND_SPAWN, &[1, 0, too_many[0], too_many[1]]).is_err());
        assert!(decode_request(KIND_KILL, &[0, 0, 0]).is_err(), "short");
        assert!(decode_request(KIND_KILL, &[0, 0, 0, 1, 0]).is_err(), "long");
        assert!(decode_request(KIND_READ, &[0, 2, 0xFF, 0xFE]).is_err(), "not UTF-8");
        assert!(decode_request(77, &[]).is_err());
        assert!(decode_request(KIND_LOCAL_TIME, &[0]).is_err(), "long");
        let mut beyond = 1_i64.to_be_bytes().to_vec();
        beyond.extend_from_slice(&(MAX_OFFSET + 1).to_be_bytes());
        assert!(decode_reply(KIND_TIME, beyond).is_err(), "an offset no zone has");
        assert!(encode_reply(&Reply::Time(LocalTime { utc: 0, offset: -MAX_OFFSET - 1 })).is_err());
        assert!(header([KIND_SPAWN, 0xFF, 0xFF, 0xFF, 0xFF], MAX_REQUEST).is_err(), "too long");
        // The role comes from the program, never from the core.
        assert_eq!(Program::Keyboard.role(), Role::Network);
        assert_eq!(Program::Sound.role(), Role::Sound);
        assert_eq!(Program::Updater.role(), Role::Updater { port: HTTPS_PORT }, "its port is the broker's rules', never the core's");
    }
}
