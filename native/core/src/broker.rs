//! The broker (decision 56): the program the user starts, and the one
//! process not confined. It starts the trusted core confined and serves
//! its requests: it reads file channels for it, only teletext and never
//! from the user's dot-folders, applications' data or the browser's own
//! folders (decision 88); opens web pages; holds the viewer key file's sealing key; tells the
//! launcher it started (decision 93) and that the version is healthy, reads the launcher's state for the
//! core, and gives the updater its staging folder and its one port
//! (`installation-and-updates.md` sections 3 and 5).

use std::ffi::OsString;
use std::fs::File;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
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

/// This program's version, read from its mark as the launcher reads it,
/// and the same as the one the trusted core reports to the interface.
fn version() -> Result<update::version::Version, String> {
    let marked = update::mark::own(VERSION_MARK).map_err(|refused| format!("this program's version: {refused}"))?;
    if update::version::Version::parse(crate::VERSION) != Some(marked) {
        return Err(format!("this program's mark says version {marked} and its report {}", crate::VERSION));
    }
    Ok(marked)
}

/// `--version`: the version on standard output, and nothing started.
pub(crate) fn print_version() -> Result<(), String> {
    let version = version()?;
    writeln!(std::io::stdout(), "neoteletext {version}").map_err(|error| format!("cannot write the version: {error}"))
}

/// The single-instance socket's name in the broker's folder.
const INSTANCE: &str = "instance";
/// The longest path a local-domain socket takes on macOS: `sun_path`'s 104
/// bytes, less the zero that ends it.
const MOST_SOCKET_PATH: usize = 103;
// Every address, and every link from outside that wraps one, passes a request.
const _: () = assert!(address::OUTSIDE_SCHEME.len() + protocol::MAX_ADDRESS <= MAX_TEXT, "an address fits a request");
/// What the viewer key file's sealing key is kept under in the platform's
/// secret store (VS-108): a name a person reading the store recognises.
const KEY_SERVICE: &str = "NeoTeletext viewer keys";
const KEY_ACCOUNT: &str = "viewer-keys.bin";
/// The cookie file's key's entry, deleted once (VS-603): nothing reads it.
const OLD_COOKIE_SERVICE: &str = "NeoTeletext cookie file";
const OLD_COOKIE_ACCOUNT: &str = "cookies.bin";

/// The exit status of a broker that ended because its launcher had gone,
/// which nothing reads: the launcher that would is gone. Not success, so a
/// log of it is not taken for a quit.
const LAUNCHER_GONE_EXIT: i32 = 1;

/// The broker (decision 56): the program the user starts, and the one
/// process not confined. It starts the trusted core confined, then serves
/// its requests until it ends.
pub(crate) fn broker() -> Result<(), String> {
    let given: Vec<OsString> = std::env::args_os().skip(1).collect();
    // First of all: a registered link or file allows one argument, and no
    // switch takes effect with it (REQ-1673). A launch refused opens nothing.
    let launched = launch::read(&given).map_err(|refused| format!("this start was refused: {refused}"))?;
    // A launcher killed outright ends the browser too (TODO-2006): the
    // trusted core hears this broker end and ends, and each helper with
    // the pipe the core held to it.
    update::layout::when_launcher_gone(|| {
        eprintln!("neoteletext: the launcher has gone, and the browser ends with it");
        std::process::exit(LAUNCHER_GONE_EXIT);
    });
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
    // The core and the helpers inherit it too: a short slice, so that a
    // thread a key, a sound message or a frame wakes runs at once rather
    // than after whatever else holds its processor (RULE-64, RULE-65;
    // DEC-6753). Without it the browser still works, only later.
    #[cfg(target_os = "linux")]
    if let Err(error) = confine::scheduling::ask_for_short_slices() {
        eprintln!("neoteletext: the short scheduling slice was refused: {error}");
    }
    // The core and the helpers inherit it: room for two pipes a helper
    // where the system starts a program with less (macOS gives 256).
    #[cfg(unix)]
    if let Err(error) = confine::raise_descriptor_limit() {
        eprintln!("neoteletext: the limit on open files could not be raised: {error}");
    }
    let mut arguments = vec![OsString::from(CORE_FLAG)];
    arguments.extend(given);
    let environment = confine::environment(Role::Core);
    let core = confine::broker::start_core(&program, &arguments, &settings, &environment).map_err(|error| format!("cannot start the trusted core: {error}"))?;
    let version = version()?;
    eprintln!("neoteletext: version {version}; the trusted core started confined: {}", core.report);
    #[cfg(feature = "links")]
    let last_opened = std::cell::Cell::new(None);
    #[cfg(feature = "links")]
    let open_web = |address: &str| paced(&last_opened, Instant::now(), || web_opener::open(address).map_err(|error| error.to_string()));
    // With links compiled out the core never asks; if it does, it is refused.
    #[cfg(not(feature = "links"))]
    let open_web = |_: &str| Err(String::from("links are compiled out"));
    let chooser = Chooser {
        program: chooser_program(&programs),
        access: Arc::clone(&access),
        openings: openings_writer,
        open: Arc::new(AtomicBool::new(false)),
        picture: Arc::new(Mutex::new(None)),
        save: Arc::new(Mutex::new(None)),
        font: Arc::new(Mutex::new(None)),
    };
    let read_source = |path: &Path| read_given(path, &access, &places());
    let (chosen_picture, chosen_save) = (Arc::clone(&chooser.picture), Arc::clone(&chooser.save));
    let read_picture = move |path: &Path| read_chosen_picture(path, &chosen_picture, &places());
    let chosen_font = Arc::clone(&chooser.font);
    let read_font = move |path: &Path| read_chosen_font(path, &chosen_font, &places());
    let save = move |kind: ChooseKind, bytes: &[u8]| save_chosen(kind, bytes, &chosen_save);
    let openings_reader = std::cell::Cell::new(Some(file_of(openings_reader)));
    let declared_access = Arc::clone(&access);
    let declared = move |declaration: Declaration| lock(&declared_access).declare(&declaration, home().as_deref(), Instant::now());
    // Asked now, on a thread, so that the answer is in by the time the core
    // asks for the interface (REQ-754).
    #[cfg(target_os = "linux")]
    let motion = ReducedMotion::ask(&chooser.program);
    #[cfg(target_os = "linux")]
    let environment = |role: Role| with_reduced_motion(role, confine::environment(role), motion.reduced());
    #[cfg(not(target_os = "linux"))]
    let environment = confine::environment;
    let handlers = Handlers {
        read_source: &read_source,
        open_web: &open_web,
        environment: &environment,
        viewer_keys_key: &viewer_keys_key,
        healthy: &tell_launcher_healthy,
        update_state: &launcher_state,
        updater: &updater_grant,
        choose: &|kind, parent, keep| chooser.start(kind, parent, keep),
        openings: &|| openings_reader.take(),
        read_picture: &read_picture,
        read_font: &read_font,
        save: &save,
        declared: Arc::new(declared),
    };
    let exit = confine::broker::serve(core, &programs, &handlers).map_err(|error| format!("cannot wait for the trusted core: {error}"))?;
    if exit.success { Ok(()) } else { Err(format!("the trusted core ended abnormally ({exit})")) }
}

/// The viewer key file's sealing key, read by the broker from the
/// platform's secret store, or made there the first time from the system's
/// random source (VS-108). The trusted core cannot reach the store. The
/// cookie file's key is deleted, if it is there: cookies are gone (VS-603).
fn viewer_keys_key() -> Result<Vec<u8>, String> {
    if let Ok(old) = keyring::Entry::new(OLD_COOKIE_SERVICE, OLD_COOKIE_ACCOUNT) {
        // Gone already, or a store that refuses: nothing reads it either way.
        let _ = old.delete_credential();
    }
    let entry = keyring::Entry::new(KEY_SERVICE, KEY_ACCOUNT).map_err(|error| format!("no secret store: {error}"))?;
    match entry.get_secret() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => {
            use ring::rand::SecureRandom as _;
            let mut key = vec![0_u8; confine::broker::SEALING_KEY_BYTES];
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
    short_place(own_folder, std::env::var_os("TMPDIR").map(PathBuf::from).as_deref())
}

/// `instance` in `own_folder`, or, under a home too deep for a socket's
/// path, a name made from that folder in the user's own temporary folder,
/// which no other user can enter.
fn short_place(own_folder: &Path, temporary: Option<&Path>) -> Option<PathBuf> {
    let place = own_folder.join(INSTANCE);
    if place.as_os_str().len() <= MOST_SOCKET_PATH {
        return Some(place);
    }
    let mut name = DefaultHasher::new();
    own_folder.hash(&mut name);
    Some(temporary.filter(|folder| folder.is_absolute())?.join(format!("neoteletext-{:016x}", name.finish())))
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
    /// The picture last chosen to convert: the one file a picture read
    /// may name (`docs/picture-tools.md`).
    picture: Arc<Mutex<Option<PathBuf>>>,
    /// Where the user last chose to save a converted page, and as what:
    /// written once, then forgotten.
    save: Arc<Mutex<Option<(ChooseKind, PathBuf)>>>,
    /// The font for pictures last chosen, and for which language: the one
    /// file a font read may name (renderer parity PAR-35).
    font: Arc<Mutex<Option<(ChooseKind, PathBuf)>>>,
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
        let (picture, save, font) = (Arc::clone(&self.picture), Arc::clone(&self.save), Arc::clone(&self.font));
        std::thread::spawn(move || {
            if let Some(path) = ask_chooser(command) {
                if keep {
                    let folder = if kind == ChooseKind::Folder { Some(path.as_path()) } else { path.parent() };
                    lock(&access).set_last_folder(folder);
                }
                // A picture is read only as a picture, and a place to save
                // is written once: neither is a grant to read as a channel.
                match kind {
                    ChooseKind::Picture => *picture.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.clone()),
                    // A font likewise, read only as a font of its kind.
                    ChooseKind::RegisFont | ChooseKind::NaplpsFont => *font.lock().unwrap_or_else(PoisonError::into_inner) = Some((kind, path.clone())),
                    ChooseKind::SaveTti | ChooseKind::SaveT42 | ChooseKind::SaveRegis | ChooseKind::SaveNaplps => {
                        *save.lock().unwrap_or_else(PoisonError::into_inner) = Some((kind, path.clone()));
                    }
                    ChooseKind::File | ChooseKind::Folder | ChooseKind::Bookmarks => lock(&access).name(&path, keep),
                }
                if let Some(path) = path.to_str().map(str::to_owned) {
                    send_opening(&openings, &Opening::Chosen { kind, path });
                }
            }
            open.store(false, Ordering::SeqCst);
        });
        Ok(())
    }
}

/// The picture the user last chose to convert, when `path` names it, read
/// by the rules of a file channel (decision 88) but only as one of the five
/// picture formats; every refusal in one sentence.
fn read_chosen_picture(path: &Path, chosen: &Mutex<Option<PathBuf>>, places: &source::Places) -> Result<Vec<u8>, String> {
    let refusal = || String::from(source::REFUSAL);
    let canonical = path.canonicalize().map_err(|_| refusal())?;
    let named = chosen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if named.and_then(|named| named.canonicalize().ok()).as_deref() != Some(canonical.as_path()) {
        return Err(refusal());
    }
    source::read_picture(&canonical, places).map_err(|_| refusal())
}

/// Writes `bytes` where the user last chose to save a page of `kind`, once:
/// the choice is forgotten whatever happens. The path is the system's save
/// dialog's, which asked before replacing a file; a link in its place is
/// refused rather than followed.
fn save_chosen(kind: ChooseKind, bytes: &[u8], chosen: &Mutex<Option<(ChooseKind, PathBuf)>>) -> Result<(), String> {
    let taken = chosen.lock().unwrap_or_else(PoisonError::into_inner).take();
    let Some((chosen_kind, path)) = taken.filter(|(chosen_kind, _)| *chosen_kind == kind) else {
        return Err(String::from("no place to save was chosen"));
    };
    debug_assert_eq!(chosen_kind, kind);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(bytes).and_then(|()| file.flush()).map_err(|error| format!("{}: {error}", path.display()))
}

/// The font for pictures the user last chose (renderer parity PAR-35), when
/// `path` names it, read by the rules of a file channel (decision 88) within
/// its language's bound, which is checked before the file is read whole:
/// the decoders parse it, never the core or the broker.
fn read_chosen_font(path: &Path, chosen: &Mutex<Option<(ChooseKind, PathBuf)>>, places: &source::Places) -> Result<Vec<u8>, String> {
    let refusal = || String::from(source::REFUSAL);
    let canonical = path.canonicalize().map_err(|_| refusal())?;
    let named = chosen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let Some((kind, named)) = named.and_then(|(kind, named)| Some((kind, named.canonicalize().ok()?))) else { return Err(refusal()) };
    if named != canonical {
        return Err(refusal());
    }
    let most = if kind == ChooseKind::NaplpsFont { protocol::regis::MAX_NAPLPS_FONT_BYTES } else { protocol::regis::MAX_REGIS_FONT_BYTES };
    source::read_font(&canonical, places, most).map_err(str::to_owned)
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

/// How long the interface's start waits for the desktop's reduced-motion
/// preference, at most: a portal already running answers in milliseconds,
/// one the session bus must start first can take longer, and a window
/// should not wait for a preference about its animations. Unanswered by
/// then, the interface moves as it always did.
#[cfg(target_os = "linux")]
const MOTION_DEADLINE: std::time::Duration = std::time::Duration::from_secs(1);

/// The desktop's reduced-motion preference on Linux (REQ-754), read for
/// the interface, which reaches no session bus (decision 86), by the
/// chooser, the browser's one holder of the desktop portal, which the broker
/// starts unconfined beside itself. Asked once, at the broker's start; a
/// change is followed at the browser's next start (DEC-2424).
#[cfg(target_os = "linux")]
struct ReducedMotion {
    answer: Mutex<Option<std::sync::mpsc::Receiver<bool>>>,
    known: std::sync::OnceLock<bool>,
}

#[cfg(target_os = "linux")]
impl ReducedMotion {
    /// Starts the chooser asking, on a thread of its own.
    fn ask(chooser: &Path) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut command = Command::new(chooser);
        command.arg(confine::broker::REDUCED_MOTION_ARGUMENT).stdin(Stdio::null()).stdout(Stdio::piped());
        std::thread::spawn(move || {
            let _ = sender.send(ask_reduced_motion(command));
        });
        Self { answer: Mutex::new(Some(receiver)), known: std::sync::OnceLock::new() }
    }

    /// The answer, waited for at most `MOTION_DEADLINE` the first time and
    /// kept; no preference if none came.
    fn reduced(&self) -> bool {
        *self.known.get_or_init(|| {
            let receiver = self.answer.lock().unwrap_or_else(PoisonError::into_inner).take();
            receiver.and_then(|receiver| receiver.recv_timeout(MOTION_DEADLINE).ok()).unwrap_or(false)
        })
    }
}

/// What the chooser printed, believed only as the one answer that means
/// reduced motion, and only when it ended well; anything else, or a chooser
/// that cannot start, is no preference. Its test is Unix's.
#[cfg(any(target_os = "linux", all(test, unix)))]
fn ask_reduced_motion(mut command: Command) -> bool {
    use confine::broker::REDUCED_MOTION_ANSWER;
    let Ok(mut child) = command.spawn().map_err(|error| eprintln!("neoteletext: cannot ask the desktop about reduced motion: {error}")) else {
        return false;
    };
    let mut printed = Vec::new();
    let most = u64::try_from(REDUCED_MOTION_ANSWER.len()).unwrap_or(u64::MAX).saturating_add(1);
    let read = child.stdout.take().map(|output| output.take(most).read_to_end(&mut printed));
    let ended_well = child.wait().is_ok_and(|status| status.success());
    ended_well && read.is_some_and(|read| read.is_ok()) && printed == REDUCED_MOTION_ANSWER
}

/// A helper's environment, with the interface's told that motion is
/// reduced when it is: the broker's own word, never a variable passed on
/// from its own environment, which `confine::environment` does not pass.
#[cfg(any(target_os = "linux", test))]
fn with_reduced_motion(role: Role, mut environment: Vec<(OsString, OsString)>, reduced: bool) -> Vec<(OsString, OsString)> {
    if role == Role::Interface && reduced {
        environment.push((OsString::from(confine::broker::REDUCED_MOTION_VARIABLE), OsString::from(confine::broker::REDUCED_MOTION_VALUE)));
    }
    environment
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

    /// REQ-1703 item 8: a home too deep for a socket's path still has a
    /// channel, in the user's temporary folder, the same at every start; an
    /// ordinary one stays in the broker's own folder. Without the short
    /// place a second start ran a second browser.
    #[cfg(unix)]
    #[test]
    fn a_deep_home_still_has_a_single_instance_channel() {
        let shallow = Path::new("/Users/a/Library/Application Support/NeoTeletext-broker");
        assert_eq!(short_place(shallow, Some(Path::new("/tmp"))).unwrap(), shallow.join(INSTANCE));
        let deep = Path::new("/private/tmp").join("a".repeat(MOST_SOCKET_PATH));
        let temporary = std::env::temp_dir();
        let place = short_place(&deep, Some(&temporary)).unwrap();
        assert!(place.as_os_str().len() <= MOST_SOCKET_PATH, "{}", place.display());
        assert!(place.starts_with(&temporary));
        assert_eq!(short_place(&deep, Some(&temporary)), Some(place.clone()), "the same each start");
        assert_ne!(short_place(&deep.join("x"), Some(&temporary)), Some(place.clone()), "each home its own");
        assert_eq!(short_place(&deep, None), None);
        let instance = Instance::bind(&place).unwrap();
        assert!(instance::connect(&place).is_ok(), "it binds and answers");
        drop(instance);
        let _ = std::fs::remove_file(&place);
    }

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
        let marked = update::mark::own(VERSION_MARK).unwrap();
        assert_eq!(Some(marked), update::version::Version::parse(crate::VERSION));
        assert_eq!(version(), Ok(marked));
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

    /// REQ-754: the chooser is believed that motion is reduced only when it
    /// prints the one answer and ends well.
    #[cfg(unix)]
    #[test]
    fn the_chooser_is_believed_only_for_its_one_answer() {
        let printing = |text: &str, status: u8| {
            let mut command = Command::new("/bin/sh");
            command.arg("-c").arg(format!("printf '%s' '{text}'; exit {status}")).stdin(Stdio::null()).stdout(Stdio::piped());
            ask_reduced_motion(command)
        };
        let answer = std::str::from_utf8(confine::broker::REDUCED_MOTION_ANSWER).unwrap();
        assert!(printing(answer, 0));
        assert!(!printing(answer, 1), "a chooser that failed");
        assert!(!printing("", 0), "no preference");
        for other in ["reduced\n", "reducedmotion", "1", "Reduced"] {
            assert!(!printing(other, 0), "{other}");
        }
        assert!(!ask_reduced_motion(Command::new("/nonexistent/neoteletext-chooser")), "no chooser");
    }

    /// REQ-754: only the interface is told, and only when motion is
    /// reduced.
    #[test]
    fn only_the_interface_is_told_motion_is_reduced() {
        let told = (OsString::from(confine::broker::REDUCED_MOTION_VARIABLE), OsString::from(confine::broker::REDUCED_MOTION_VALUE));
        let base = vec![(OsString::from("HOME"), OsString::from("/home/a"))];
        assert_eq!(with_reduced_motion(Role::Interface, base.clone(), true), vec![base[0].clone(), told]);
        assert_eq!(with_reduced_motion(Role::Interface, base.clone(), false), base);
        for role in [Role::Core, Role::Decoder, Role::Network, Role::Sound] {
            assert_eq!(with_reduced_motion(role, base.clone(), true), base, "{role:?}");
        }
    }

    /// A home folder of its own for one test, with the browser's settings
    /// folder in it, as the broker's places name them.
    /// In the build's own folder, on the disk, not under `TMPDIR`: whether
    /// a file made where one was removed takes its inode number depends on
    /// the file system (TODO-6629), so the test of a grant's file must not
    /// depend on where `TMPDIR` points.
    fn home(name: &str) -> PathBuf {
        let home = std::env::current_exe().unwrap().parent().unwrap().join(format!("neoteletext-broker-{name}-{}", std::process::id()));
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

    /// The picture converter (`docs/picture-tools.md`): only the picture
    /// last chosen in its dialog is read, and only when it starts as one of
    /// the five formats.
    #[test]
    fn only_the_chosen_picture_is_read_and_only_as_a_picture() {
        let home = home("picture");
        let places = Places::new(Some(home.clone()), vec![home.join("settings")]);
        let (picture, other, text) = (home.join("photo.png"), home.join("other.png"), home.join("notes.png"));
        let png = b"\x89PNG\r\n\x1a\n and the rest".to_vec();
        write(&picture, &png);
        write(&other, &png);
        write(&text, b"not a picture at all");
        let chosen = Mutex::new(None);
        assert!(read_chosen_picture(&picture, &chosen, &places).is_err(), "nothing chosen yet");
        *chosen.lock().unwrap() = Some(picture.clone());
        assert_eq!(read_chosen_picture(&picture, &chosen, &places).unwrap(), png);
        assert!(read_chosen_picture(&other, &chosen, &places).is_err(), "another file");
        *chosen.lock().unwrap() = Some(text.clone());
        assert!(read_chosen_picture(&text, &chosen, &places).is_err(), "not a picture");
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// A save is written once, where the user chose and as the kind chosen;
    /// a link in that place is not followed.
    #[test]
    fn a_page_is_saved_once_where_chosen() {
        let home = home("save");
        let place = home.join("page.tti");
        let chosen = Mutex::new(Some((ChooseKind::SaveTti, place.clone())));
        assert!(save_chosen(ChooseKind::SaveT42, b"x", &chosen).is_err(), "another kind");
        *chosen.lock().unwrap() = Some((ChooseKind::SaveTti, place.clone()));
        save_chosen(ChooseKind::SaveTti, b"PN,10000\n", &chosen).unwrap();
        assert_eq!(std::fs::read(&place).unwrap(), b"PN,10000\n");
        assert!(save_chosen(ChooseKind::SaveTti, b"again", &chosen).is_err(), "once only");
        #[cfg(unix)]
        {
            let target = home.join("target.txt");
            write(&target, b"kept");
            let link = home.join("link.tti");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            *chosen.lock().unwrap() = Some((ChooseKind::SaveTti, link));
            assert!(save_chosen(ChooseKind::SaveTti, b"over", &chosen).is_err());
            assert_eq!(std::fs::read(&target).unwrap(), b"kept");
        }
        std::fs::remove_dir_all(&home).unwrap();
    }
}
