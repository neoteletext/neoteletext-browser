//! The broker (decision 56): the program the user starts, and the one
//! process not confined. It starts the trusted core confined and serves
//! its requests: it reads file channels for it, only teletext and never
//! from the user's dot-folders, applications' data or the browser's own
//! folders (decision 88); opens web pages; holds the cookie file's key; tells the
//! launcher the version is healthy, reads the launcher's state for the
//! core, and gives the updater its staging folder and its one port
//! (`installation-and-updates.md` sections 3 and 5).

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use confine::Role;
use confine::broker::{Handlers, UpdaterGrant};
use neoteletext::settings as stored;
use update::layout::{HEALTHY_LINE, LAUNCHED_VARIABLE, Layout};
use update::platform::Platform;
use update::state::MAX_STATE;

use crate::source;

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
    let program = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
    let programs = program.parent().ok_or("this program has no directory")?.to_path_buf();
    let settings = stored::folder().ok_or("there is no folder for the settings")?;
    neoteletext::make_folder(&settings).map_err(|error| format!("{}: {error}", settings.display()))?;
    let mut arguments = vec![OsString::from(CORE_FLAG)];
    arguments.extend(std::env::args_os().skip(1));
    let environment = confine::environment(Role::Core);
    let core = confine::broker::start_core(&program, &arguments, &settings, &environment).map_err(|error| format!("cannot start the trusted core: {error}"))?;
    let version = update::mark::own(VERSION_MARK).map_err(|refused| format!("this program's version: {refused}"))?;
    eprintln!("neoteletext: version {version}; the trusted core started confined: {}", core.report);
    #[cfg(feature = "links")]
    let last_opened = std::cell::Cell::new(None);
    #[cfg(feature = "links")]
    let open_web = |address: &str| paced(&last_opened, std::time::Instant::now(), || web_opener::open(address).map_err(|error| error.to_string()));
    // With links compiled out the core never asks; if it does, it is refused.
    #[cfg(not(feature = "links"))]
    let open_web = |_: &str| Err(String::from("links are compiled out"));
    let handlers = Handlers {
        read_source: &read_for_core,
        open_web: &open_web,
        environment: &confine::environment,
        cookie_key: &cookie_key,
        healthy: &tell_launcher_healthy,
        update_state: &launcher_state,
        updater: &updater_grant,
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

/// A file channel's bytes, read by the broker for the core by the rules of
/// decision 88 (`Source::read`), in this user's places.
fn read_for_core(path: &Path) -> Result<Vec<u8>, String> {
    read_within(path, &places())
}

/// A file channel's bytes, read in `places`; every refusal in one sentence.
fn read_within(path: &Path, places: &source::Places) -> Result<Vec<u8>, String> {
    source::Source::read(path, places).map(source::Source::into_bytes).map_err(String::from)
}

/// The places no file channel is read from (decision 88), taken afresh at
/// each read: beneath the home folder, every entry beginning with a dot;
/// the platform's folders of applications' data; and the browser's own
/// settings and update folders, the only ones the trusted core writes.
fn places() -> source::Places {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).filter(|home| home.is_absolute());
    let mut refused: Vec<PathBuf> = stored::folder().into_iter().collect();
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
fn paced(last: &std::cell::Cell<Option<std::time::Instant>>, now: std::time::Instant, open: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
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
        let start = std::time::Instant::now();
        assert_eq!(paced(&last, start, open), Ok(()));
        let soon = start + std::time::Duration::from_secs(1);
        assert!(paced(&last, soon, open).is_err(), "a second request a second later is refused");
        let later = start + std::time::Duration::from_millis(neoteletext::links::HANDOFF_INTERVAL_MILLISECONDS);
        assert_eq!(paced(&last, later, open), Ok(()), "one an interval later is opened");
        assert_eq!(opened.get(), 2);
    }
    use confine::broker::Program;

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
        let refused = read_for_core(&ssh.join("..").join(".ssh")).unwrap_err();
        assert_eq!(refused, REFUSAL);
    }

    use crate::source::{Places, REFUSAL};

    /// The places of a test's home folder: its dot-entries, and its
    /// `settings` folder as the browser's own.
    fn places(home: &Path) -> Places {
        Places::new(Some(home.to_path_buf()), vec![home.join("settings")])
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
