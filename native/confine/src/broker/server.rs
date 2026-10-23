//! The broker's side: it starts the trusted core confined, then serves
//! its requests until it ends.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use super::wire::{Exit, Reply, Request, decode_request, encode_reply};
use super::{COOKIE_KEY_BYTES, Channel, MAX_HELPERS, MAX_REPLY, MAX_REQUEST, MAX_UPDATE_STATE, Program, UpdaterGrant};
#[cfg(windows)]
use super::{HEADER_BYTES, KIND_SPAWNED, wire::frame};
use crate::{Process, Report, Role, Spawn};

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
        let process = Process::new(None, None, child, report.clone());
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
        let into = core.process.raw_process();
        let values = [crate::windows::duplicate_into(stdin, into)?, crate::windows::duplicate_into(stdout, into)?];
        let mut payload = bytes.get(HEADER_BYTES..).unwrap_or_default().to_vec();
        for value in values {
            payload.extend_from_slice(&value.to_be_bytes());
        }
        core.channel.write(&frame(KIND_SPAWNED, &payload)?)
    }
}
