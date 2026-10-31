//! The broker (decision 56): the program the user starts, and the one
//! process not confined. It starts the trusted core confined and serves
//! its requests: it reads file channels for it, only teletext and never
//! from the user's dot-folders, applications' data or the browser's own
//! folders (decision 88); opens web pages; holds the cookie file's key; tells the
//! launcher it started (decision 93) and that the version is healthy, reads the launcher's state for the
//! core, and gives the updater its staging folder and its one port
//! (`installation-and-updates.md` sections 3 and 5).

use std::ffi::OsString;
use std::fs::File;
use std::io::{PipeWriter, Read as _, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use confine::Role;
use confine::broker::{ChooseKind, Declaration, Handlers, MAX_TEXT, Opening, UpdaterGrant, read_opening, write_opening};
use confine::instance::{self, Instance};
use neoteletext::address::{self, Address};
use neoteletext::grants;
use neoteletext::launch::{self, Launch};
use neoteletext::settings as stored;
use update::layout::{HEALTHY_LINE, LAUNCHED_VARIABLE, Layout, STARTED_LINE};
use update::platform::Platform;
use update::state::MAX_STATE;

use crate::source;
use crate::trusted_core::given_addresses;

mod access;

use access::{Access, Admission};

/// The trusted core's own flag: the broker starts this program again
/// with it (decision 56).
pub(crate) const CORE_FLAG: &str = "--core";

/// This release's version, in the program's own bytes, where the launcher
/// reads it from the signed file (`installation-and-updates.md` section 5).
static VERSION_MARK: &str = update::version_mark!(env!("NEOTELETEXT_VERSION"));

/// What the cookie file's key is kept under in the platform's secret store
/// (`cookies.md` section 7): a name a person reading the store recognises.
const KEY_SERVICE: &str = "NeoTeletext cookie file";
const KEY_ACCOUNT: &str = "cookies.bin";

/// The broker (decision 56): the program the user starts, and the one
/// process not confined. It starts the trusted core confined, then serves
/// its requests until it ends.
pub(crate) fn broker() -> Result<(), String> {
    let given: Vec<OsString> = std::env::args_os().skip(1).collect();
    // First of all: a registered link or file allows one argument, and no
    // switch takes effect with it (REQ-1673). A launch refused opens nothing.
    let launched = launch::read(&given).map_err(|refused| format!("this start was refused: {refused}"))?;
    tell_launcher_started();
    let program = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
    let programs = program.parent().ok_or("this program has no directory")?.to_path_buf();
    let settings = stored::folder().ok_or("there is no folder for the settings")?;
    neoteletext::make_folder(&settings).map_err(|error| format!("{}: {error}", settings.display()))?;
    let own_folder = grants::folder(&settings).ok_or("there is no folder for the broker's files")?;
    neoteletext::make_folder(&own_folder).map_err(|error| format!("{}: {error}", own_folder.display()))?;
    let (links, files) = what_it_names(&launched, &given);
    let place = instance_place(&own_folder);
    // Something to open, and a browser running: it opens there (DEC-1551
    // item 7). A start that names nothing runs as it always has.
    if let Some(place) = &place
        && !(links.is_empty() && files.is_empty())
        && hand_over(place, &links, &files)
    {
        return Ok(());
    }
    let access = Arc::new(Mutex::new(Access::load(Some(own_folder.clone()))));
    for file in &files {
        lock(&access).name(Path::new(file), true);
    }
    let (openings_reader, openings_writer) = std::io::pipe().map_err(|error| format!("cannot make a pipe: {error}"))?;
    let openings_writer = Arc::new(Mutex::new(openings_writer));
    if let Some(place) = &place {
        listen(place, &access, &openings_writer);
    }
    let mut arguments = vec![OsString::from(CORE_FLAG)];
    arguments.extend(given);
    let environment = confine::environment(Role::Core);
    let core = confine::broker::start_core(&program, &arguments, &settings, &environment).map_err(|error| format!("cannot start the trusted core: {error}"))?;
    let version = update::mark::own(VERSION_MARK).map_err(|refused| format!("this program's version: {refused}"))?;
    eprintln!("neoteletext: version {version}; the trusted core started confined: {}", core.report);
    #[cfg(feature = "links")]
    let last_opened = std::cell::Cell::new(None);
    #[cfg(feature = "links")]
    let open_web = |address: &str| paced(&last_opened, Instant::now(), || web_opener::open(address).map_err(|error| error.to_string()));
    // With links compiled out the core never asks; if it does, it is refused.
    #[cfg(not(feature = "links"))]
    let open_web = |_: &str| Err(String::from("links are compiled out"));
    let chooser =
        Chooser { program: chooser_program(&programs), access: Arc::clone(&access), openings: openings_writer, open: Arc::new(AtomicBool::new(false)) };
    let read_source = |path: &Path| read_given(path, &access, &places());
    let openings_reader = std::cell::Cell::new(Some(file_of(openings_reader)));
    let declared_access = Arc::clone(&access);
    let declared = move |declaration: Declaration| lock(&declared_access).declare(&declaration, home().as_deref(), Instant::now());
    let handlers = Handlers {
        read_source: &read_source,
        open_web: &open_web,
        environment: &confine::environment,
        cookie_key: &cookie_key,
        healthy: &tell_launcher_healthy,
        update_state: &launcher_state,
        updater: &updater_grant,
        choose: &|kind, parent, keep| chooser.start(kind, parent, keep),
        openings: &|| openings_reader.take(),
        declared: Arc::new(declared),
    };
    let exit = confine::broker::serve(core, &programs, &handlers).map_err(|error| format!("cannot wait for the trusted core: {error}"))?;
    if exit.success { Ok(()) } else { Err(format!("the trusted core ended abnormally ({exit})")) }
}

/// The cookie file's key, read by the broker from the platform's secret
/// store, or made there the first time from the system's random source
/// (requirement 21.4). The trusted core cannot reach the store.
fn cookie_key() -> Result<Vec<u8>, String> {
    let entry = keyring::Entry::new(KEY_SERVICE, KEY_ACCOUNT).map_err(|error| format!("no secret store: {error}"))?;
    match entry.get_secret() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => {
            use ring::rand::SecureRandom as _;
            let mut key = vec![0_u8; confine::broker::COOKIE_KEY_BYTES];
            ring::rand::SystemRandom::new().fill(&mut key).map_err(|_| String::from("no random source"))?;
            entry.set_secret(&key).map_err(|error| format!("the secret store refused the key: {error}"))?;
            Ok(key)
        }
        Err(error) => Err(format!("the secret store refused: {error}")),
    }
}

fn lock(access: &Mutex<Access>) -> std::sync::MutexGuard<'_, Access> {
    access.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A file's bytes, read for the core only when `access` says it was given
/// (REQ-1698), then by the rules of decision 88 in `places`; every refusal in
/// one sentence. The table is not held while the file is read.
fn read_given(path: &Path, access: &Mutex<Access>, places: &source::Places) -> Result<Vec<u8>, String> {
    let canonical = path.canonicalize().map_err(|_| String::from(source::REFUSAL))?;
    let admission = lock(access).admit(&canonical, Instant::now());
    let expected = match admission {
        Admission::Refused => return Err(source::REFUSAL.to_owned()),
        Admission::Granted(identity) => Some(identity),
        Admission::Fresh { .. } => None,
    };
    let (read, identity) = source::Source::read_as(&canonical, places, expected).map_err(String::from)?;
    if let Admission::Fresh { keep } = admission {
        lock(access).read(&canonical, identity, keep);
    }
    Ok(read.into_bytes())
}

/// The home folder, as `places` takes it.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).filter(|home| home.is_absolute())
}

/// The links and the files of this start: what a marked launch names, or the
/// addresses on an ordinary command line that are files.
fn what_it_names(launched: &Launch, given: &[OsString]) -> (Vec<String>, Vec<String>) {
    match launched {
        Launch::Link(link) => (vec![link.clone()], Vec::new()),
        Launch::File(file) => (Vec::new(), address::parse(file).ok().and_then(file_of_address).into_iter().collect()),
        Launch::Plain => {
            let text = given.iter().filter_map(|argument| argument.to_str()).map(str::to_owned);
            let files = given_addresses(text).iter().filter_map(|given| address::parse(given).ok()).filter_map(file_of_address).collect();
            (Vec::new(), files)
        }
    }
}

fn file_of_address(address: Address) -> Option<String> {
    address.file.and_then(|path| path.to_str().map(str::to_owned))
}

/// Where the single-instance channel is: a pipe's name on Windows, for this
/// user; on Linux the runtime folder, which neither of the roles allowed
/// local sockets can see (`confine/src/linux/runtime.rs`), and with none no
/// channel; on macOS the broker's own folder, as every profile denies the
/// network's operations, these among them.
fn instance_place(own_folder: &Path) -> Option<PathBuf> {
    if cfg!(windows) {
        let user = std::env::var_os("USERNAME")?;
        let mut name = OsString::from(r"\\.\pipe\NeoTeletext-");
        name.push(user);
        return Some(PathBuf::from(name));
    }
    if cfg!(target_os = "linux") {
        return std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|folder| folder.is_absolute()).map(|folder| folder.join("neoteletext-instance"));
    }
    Some(own_folder.join("instance"))
}

/// Hands `links` and `files` to the browser running at `place`: false when
/// none answers. At most what one opening may name goes.
fn hand_over(place: &Path, links: &[String], files: &[String]) -> bool {
    let Ok(mut stream) = instance::connect(place) else { return false };
    write_opening(&mut stream, &Opening::Outside { links: links.to_vec(), files: files.to_vec() }).is_ok()
}

/// Hears second starts, on a thread of their own. What arrives may come from
/// any program that reaches the channel, so it is read strictly, a file is
/// allowed only as if it were on this broker's command line, and a link goes
/// to the core, which checks it again (REQ-1672 item 10).
fn listen(place: &Path, access: &Arc<Mutex<Access>>, openings: &Arc<Mutex<PipeWriter>>) {
    let instance = match Instance::bind(place) {
        Ok(instance) => instance,
        Err(error) => {
            eprintln!("neoteletext: another start cannot hand this browser anything: {error}");
            return;
        }
    };
    let (access, openings) = (Arc::clone(access), Arc::clone(openings));
    std::thread::spawn(move || {
        while let Ok(mut stream) = instance.next() {
            match read_opening(&mut stream) {
                Ok(opening @ Opening::Outside { .. }) => {
                    let Opening::Outside { files, .. } = &opening else { continue };
                    for file in files {
                        if let Some(path) = address::parse(file).ok().and_then(|address| address.file) {
                            lock(&access).name(&path, true);
                        }
                    }
                    send_opening(&openings, &opening);
                }
                _ => eprintln!("neoteletext: another start sent something that is not an opening, which is refused"),
            }
        }
    });
}

fn send_opening(openings: &Mutex<PipeWriter>, opening: &Opening) {
    let mut pipe = openings.lock().unwrap_or_else(PoisonError::into_inner);
    if let Err(error) = write_opening(&mut *pipe, opening) {
        eprintln!("neoteletext: the trusted core was not told of an opening: {error}");
    }
}

/// The chooser (DEC-1551 item 1): its program, and what its answer changes.
struct Chooser {
    program: PathBuf,
    access: Arc<Mutex<Access>>,
    openings: Arc<Mutex<PipeWriter>>,
    /// A dialog is open: one at a time.
    open: Arc<AtomicBool>,
}

impl Chooser {
    /// Shows the dialog on a thread of its own, so that the core goes on
    /// being served, and answers now. The choice arrives on the pipe.
    fn start(&self, kind: ChooseKind, parent: &str, keep: bool) -> Result<(), String> {
        if self.open.swap(true, Ordering::SeqCst) {
            return Err(String::from("a file dialog is already open"));
        }
        let start_in = lock(&self.access).last_folder();
        let mut command = Command::new(&self.program);
        command.arg(kind.argument()).arg(parent).arg(start_in.as_deref().unwrap_or(Path::new(""))).stdin(Stdio::null()).stdout(Stdio::piped());
        let (access, openings, open) = (Arc::clone(&self.access), Arc::clone(&self.openings), Arc::clone(&self.open));
        std::thread::spawn(move || {
            if let Some(path) = ask_chooser(command) {
                if keep {
                    let folder = if kind == ChooseKind::Folder { Some(path.as_path()) } else { path.parent() };
                    lock(&access).set_last_folder(folder);
                }
                lock(&access).name(&path, keep);
                if let Some(path) = path.to_str().map(str::to_owned) {
                    send_opening(&openings, &Opening::Chosen { kind, path });
                }
            }
            open.store(false, Ordering::SeqCst);
        });
        Ok(())
    }
}

/// What the chooser printed, bounded, read as one absolute path; nothing when
/// the user cancelled or it failed.
fn ask_chooser(mut command: Command) -> Option<PathBuf> {
    let mut child = command.spawn().map_err(|error| eprintln!("neoteletext: cannot start the file dialog: {error}")).ok()?;
    let mut printed = Vec::new();
    let read = child.stdout.take().map(|output| output.take((MAX_TEXT as u64).saturating_add(1)).read_to_end(&mut printed));
    let status = child.wait().ok()?;
    read?.ok()?;
    if !status.success() || printed.len() > MAX_TEXT {
        return None;
    }
    let path = PathBuf::from(String::from_utf8(printed).ok()?);
    path.is_absolute().then_some(path)
}

/// The chooser's program: in a macOS bundle, its helper bundle, so that it
/// has no Dock icon of its own; beside the broker otherwise.
fn chooser_program(programs: &Path) -> PathBuf {
    let name = format!("neoteletext-chooser{}", std::env::consts::EXE_SUFFIX);
    let helper = programs.parent().map(|contents| contents.join("Helpers/NeoTeletextChooser.app/Contents/MacOS").join(&name));
    helper.filter(|helper| helper.is_file()).unwrap_or_else(|| programs.join(name))
}

/// The pipe's reading end as a file, to pass on.
#[cfg(unix)]
fn file_of(pipe: std::io::PipeReader) -> File {
    File::from(std::os::fd::OwnedFd::from(pipe))
}

/// The pipe's reading end as a file, to pass on.
#[cfg(windows)]
fn file_of(pipe: std::io::PipeReader) -> File {
    File::from(std::os::windows::io::OwnedHandle::from(pipe))
}

/// The places no file channel is read from (decision 88), taken afresh at
/// each read: beneath the home folder, every entry beginning with a dot;
/// the platform's folders of applications' data; and the browser's own
/// settings and update folders, the only ones the trusted core writes.
fn places() -> source::Places {
    let home = home();
    let mut refused: Vec<PathBuf> = stored::folder().into_iter().collect();
    // The broker's own files, which are no channel.
    refused.extend(stored::folder().as_deref().and_then(grants::folder));
    refused.extend(Platform::this().and_then(|platform| Layout::for_user(platform.family(), None)).map(|layout| layout.root));
    refused.extend(installation().map(|layout| layout.root));
    if cfg!(target_os = "macos") {
        refused.extend(home.as_ref().map(|home| home.join("Library")));
    }
    // Windows' folders of applications' data, and on Linux those that the
    // XDG variables move out of the home folder's dot-entries.
    let data = if cfg!(windows) {
        ["APPDATA", "LOCALAPPDATA"].as_slice()
    } else {
        ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"].as_slice()
    };
    refused.extend(data.iter().filter_map(std::env::var_os).map(PathBuf::from).filter(|folder| folder.is_absolute()));
    source::Places::new(home, refused)
}

/// The installation this broker belongs to, when a launcher started it.
fn installation() -> Option<Layout> {
    std::env::var_os(LAUNCHED_VARIABLE)?;
    let program = std::env::current_exe().ok()?;
    Layout::of_program(Platform::this()?.family(), &program)
}

/// Tells the launcher, on the broker's standard output, that the version
/// is healthy (section 3.2).
///
/// # Errors
/// When no launcher started the browser, or the pipe fails.
fn tell_launcher_healthy() -> Result<(), String> {
    installation().ok_or("no launcher started this browser")?;
    let mut output = std::io::stdout().lock();
    output.write_all(HEALTHY_LINE.as_bytes()).and_then(|()| output.flush()).map_err(|error| format!("cannot tell the launcher: {error}"))
}

/// Tells the launcher, on the broker's standard output, that the program
/// has started, before the trusted core is (decision 93): its health
/// deadline begins. Only when a launcher started the browser; a failed
/// write is ignored, and the launcher's start deadline then ends the
/// trial.
fn tell_launcher_started() {
    if installation().is_some() {
        let _ = write_started(&mut std::io::stdout().lock());
    }
}

/// Writes the started line to `output`, as the healthy line is written.
fn write_started(output: &mut impl Write) -> std::io::Result<()> {
    output.write_all(STARTED_LINE.as_bytes()).and_then(|()| output.flush())
}

/// The launcher's state file's bytes, for the core.
///
/// # Errors
/// When no launcher started the browser, or the file cannot be read.
fn launcher_state() -> Result<Vec<u8>, String> {
    let layout = installation().ok_or("no launcher started this browser")?;
    update::files::read_bounded(&layout.state(), MAX_STATE).map_err(|error| format!("the launcher's state: {error}"))
}

/// The updater's staging folder and port, by the broker's own rules:
/// where this installation keeps it, and the update host's port or the
/// port of the proxy the core names (`--proxy URL`).
///
/// # Errors
/// When no launcher installed the browser, or the proxy is not one.
fn updater_grant(arguments: &[String]) -> Result<UpdaterGrant, String> {
    let layout = installation().ok_or("no launcher installed this browser, so it does not update itself")?;
    let staging = layout.staging();
    std::fs::create_dir_all(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let port = match arguments {
        [] => release_identity::PORT,
        [flag, proxy] if flag == "--proxy" => neoteletext_web::proxy::parse_proxy(proxy).ok_or("the proxy is not one")?.port,
        _ => return Err(String::from("the updater takes a proxy and nothing else")),
    };
    Ok(UpdaterGrant { staging, port })
}

/// The least time between two web pages the broker opens for the core:
/// the interval the core keeps between hand-offs (requirement 17.9), one
/// constant for both (decision 88, finding F-61).
#[cfg(feature = "links")]
const OPEN_WEB_INTERVAL: std::time::Duration = std::time::Duration::from_millis(neoteletext::links::HANDOFF_INTERVAL_MILLISECONDS);

/// Opens a web page with `open` unless one was opened, at `last`, within
/// [`OPEN_WEB_INTERVAL`] of `now`: a compromised core cannot use the
/// system's web browser as a channel out faster than a link could.
#[cfg(feature = "links")]
fn paced(last: &std::cell::Cell<Option<Instant>>, now: Instant, open: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    if last.get().is_some_and(|last| now.saturating_duration_since(last) < OPEN_WEB_INTERVAL) {
        return Err(String::from("web pages are opened at most once every two seconds"));
    }
    last.set(Some(now));
    open()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finding F-61: the broker opens a web page at most once in the
    /// interval the core keeps between hand-offs, whatever the core asks.
    #[cfg(feature = "links")]
    #[test]
    fn the_broker_opens_the_web_at_most_once_an_interval() {
        let last = std::cell::Cell::new(None);
        let opened = std::cell::Cell::new(0);
        let open = || {
            opened.set(opened.get() + 1);
            Ok(())
        };
        let start = Instant::now();
        assert_eq!(paced(&last, start, open), Ok(()));
        let soon = start + std::time::Duration::from_secs(1);
        assert!(paced(&last, soon, open).is_err(), "a second request a second later is refused");
        let later = start + std::time::Duration::from_millis(neoteletext::links::HANDOFF_INTERVAL_MILLISECONDS);
        assert_eq!(paced(&last, later, open), Ok(()), "one an interval later is opened");
        assert_eq!(opened.get(), 2);
    }
    use confine::broker::Program;

    /// Decision 93: the broker tells the launcher it started with the line
    /// the launcher waits for, whole and flushed.
    #[test]
    fn the_broker_says_started_with_the_launchers_line() {
        let mut output = std::io::BufWriter::new(Vec::new());
        write_started(&mut output).unwrap();
        assert_eq!(output.buffer(), b"", "flushed");
        assert_eq!(output.get_ref().as_slice(), STARTED_LINE.as_bytes());
        assert_eq!(STARTED_LINE, "started\n");
    }

    /// `installation-and-updates.md` section 5: the broker's bytes carry
    /// its version, which the launcher finds there.
    #[test]
    fn the_broker_carries_its_version() {
        let version = update::mark::own(VERSION_MARK).unwrap();
        assert_eq!(Some(version), update::version::Version::parse(env!("NEOTELETEXT_VERSION")));
    }

    #[test]
    fn a_helper_program_has_this_platforms_file_name() {
        let expected = if cfg!(windows) { "neoteletext-interface.exe" } else { "neoteletext-interface" };
        assert_eq!(Program::Interface.file_name(), expected);
    }

    /// Decision 56: the broker reads no file channel in a folder of the
    /// user's secrets for the core, however the path is written.
    #[cfg(unix)]
    #[test]
    fn the_broker_reads_no_secret_for_the_core() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
        let ssh = home.join(".ssh");
        if !ssh.is_dir() {
            return;
        }
        let refused = read_within(&ssh.join("..").join(".ssh"), &super::places()).unwrap_err();
        assert_eq!(refused, REFUSAL);
    }

    use crate::source::{Places, REFUSAL};

    /// A file's bytes as the broker reads one the user gave.
    fn read_within(path: &Path, places: &Places) -> Result<Vec<u8>, String> {
        let access = Mutex::new(Access::default());
        lock(&access).name(path, false);
        read_given(path, &access, places)
    }

    /// REQ-1698, finding F-03: the core's word alone reads nothing, even a
    /// recording that DEC-981's checks would pass; the same file is read once
    /// the interface has declared it, and after that through its grant.
    #[test]
    fn the_broker_reads_nothing_nobody_gave_it() {
        let home = home("given");
        write(&home.join("recordings/news.t42"), &recording());
        let path = home.join("recordings/news.t42");
        let places = places(&home);
        let access = Mutex::new(Access::default());
        assert_eq!(read_given(&path, &access, &places), Err(REFUSAL.to_owned()));
        let declaration = Declaration { paths: vec![path.display().to_string()], keep: false };
        lock(&access).declare(&declaration, None, Instant::now());
        assert_eq!(read_given(&path, &access, &places).unwrap(), recording());
        assert_eq!(read_given(&path, &access, &places).unwrap(), recording(), "through its grant");
        std::fs::remove_file(&path).unwrap();
        write(&path, &recording());
        assert_eq!(read_given(&path, &access, &places), Err(REFUSAL.to_owned()), "another file at its path");
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// The places of a test's home folder: its dot-entries, and its
    /// `settings` folder as the browser's own.
    fn places(home: &Path) -> Places {
        Places::new(Some(home.to_path_buf()), vec![home.join("settings")])
    }

    /// DEC-1551 item 1: the chooser's answer is one absolute path, whatever
    /// it prints; nothing printed is a cancel.
    #[cfg(unix)]
    #[test]
    fn the_chooser_is_believed_only_for_one_path() {
        let printing = |text: &str| {
            let mut command = Command::new("/bin/sh");
            command.arg("-c").arg(format!("printf '%s' '{text}'")).stdin(Stdio::null()).stdout(Stdio::piped());
            ask_chooser(command)
        };
        assert_eq!(printing("/home/a/news.t42"), Some(PathBuf::from("/home/a/news.t42")));
        assert_eq!(printing(""), None, "cancelled");
        assert_eq!(printing("relative/news.t42"), None);
        assert_eq!(printing(&"/a".repeat(MAX_TEXT)), None, "too long");
    }

    /// A home folder of its own for one test, with the browser's settings
    /// folder in it, as the broker's places name them.
    fn home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("neoteletext-broker-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("settings")).unwrap();
        home.canonicalize().unwrap()
    }

    /// Twenty packets of magazine 1 with good addresses: a T42 recording
    /// the engine's `detect` recognises.
    fn recording() -> Vec<u8> {
        use neoteletext_core::ecc::hamming84_encode;
        let mut bytes = Vec::new();
        for row in 0..20_u8 {
            bytes.extend([hamming84_encode(1 | ((row & 1) << 3)), hamming84_encode(row >> 1)]);
            bytes.extend([b' '; 40]);
        }
        bytes
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Decision 88 (finding F-03): a link the core makes in the one folder
    /// it writes is not followed, to a secret or to a recording, and a
    /// hard link there is refused with the folder.
    #[cfg(unix)]
    #[test]
    fn a_link_the_core_made_in_its_folder_is_not_followed() {
        let home = home("link");
        write(&home.join(".ssh/id_test"), b"-----BEGIN OPENSSH PRIVATE KEY-----\n");
        write(&home.join("recordings/news.t42"), &recording());
        let folder = home.join("settings/channel");
        std::fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(home.join(".ssh/id_test"), folder.join("100.tti")).unwrap();
        std::os::unix::fs::symlink(home.join(".ssh/id_test"), home.join("settings/key.t42")).unwrap();
        std::fs::hard_link(home.join("recordings/news.t42"), home.join("settings/news.t42")).unwrap();
        let places = places(&home);
        assert_eq!(read_within(&folder, &places), Err(REFUSAL.to_owned()), "a folder holding a link to a secret");
        assert_eq!(read_within(&home.join("settings/key.t42"), &places), Err(REFUSAL.to_owned()), "a link to a secret");
        assert_eq!(read_within(&home.join("settings/news.t42"), &places), Err(REFUSAL.to_owned()), "a hard link in the core's folder");
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// Decision 88: nothing under a dot-entry of the home folder is read,
    /// even bytes that look like teletext.
    #[test]
    fn a_dot_entry_under_home_is_refused() {
        let home = home("dot");
        write(&home.join(".netrc"), &recording());
        write(&home.join(".mozilla/profile/pages/100.tti"), b"PN,10000\nOL,1,Hello\n");
        let places = places(&home);
        assert_eq!(read_within(&home.join(".netrc"), &places), Err(REFUSAL.to_owned()));
        assert_eq!(read_within(&home.join(".mozilla/profile/pages"), &places), Err(REFUSAL.to_owned()));
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// Decision 88: a file the engine does not recognise is refused, in
    /// the same words as a missing one, so no answer tells which.
    #[test]
    fn a_file_that_is_not_teletext_is_refused() {
        let home = home("text");
        write(&home.join("Documents/notes.txt"), b"my bank's PIN is 1234\n");
        write(&home.join("Documents/letters/a.txt"), b"Dear Sir\n");
        let places = places(&home);
        assert_eq!(read_within(&home.join("Documents/notes.txt"), &places), Err(REFUSAL.to_owned()));
        assert_eq!(read_within(&home.join("Documents/letters"), &places), Err(REFUSAL.to_owned()), "a folder with no page file");
        assert_eq!(read_within(&home.join("Documents/missing.t42"), &places), Err(REFUSAL.to_owned()));
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// Decision 88: recordings, folders of page files and bookmarks files
    /// are read as before; a folder's other files are left out.
    #[test]
    fn teletext_and_bookmarks_files_are_read() {
        let home = home("read");
        write(&home.join("recordings/news.t42"), &recording());
        write(&home.join("recordings/pages/100.tti"), b"PN,10000\nOL,1,Hello\n");
        write(&home.join("recordings/pages/101.tti"), b"PN,10100\nOL,1,World\n");
        write(&home.join("recordings/pages/notes.txt"), b"not a page\n");
        write(&home.join("Documents/bookmarks.html"), b"<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<DL><p></DL>\n");
        write(&home.join("Documents/bookmarks-export.txt"), b"neoteletext-bookmarks 1\n");
        let places = places(&home);
        assert_eq!(read_within(&home.join("recordings/news.t42"), &places).unwrap(), recording());
        let pages = read_within(&home.join("recordings/pages"), &places).unwrap();
        assert_eq!(pages, b"PN,10000\nOL,1,Hello\n\nPN,10100\nOL,1,World\n\n");
        assert!(read_within(&home.join("Documents/bookmarks.html"), &places).is_ok());
        assert!(read_within(&home.join("Documents/bookmarks-export.txt"), &places).is_ok());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
