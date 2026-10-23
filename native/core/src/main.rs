//! The trusted core (design section 5): it keeps the tabs, their history
//! and their channels, reads each channel's data and hands it to that
//! channel's decoder at a fixed pace, forwards the pages that arrive,
//! checks every frame, encodes page images, and decides what the trusted
//! strip shows. It never parses teletext data and never draws; the
//! interface never reaches a decoder.
//!
//! Usage: `neoteletext [--allow-local <ip>]... [address]...`, each address
//! a web channel's folder (`http://host/folder/`), a `file://` address, or
//! an absolute path to a recording, a page file or a folder of page files.
//! A channel on this computer or the local network is fetched only from
//! the addresses given with `--allow-local` or allowed in the settings
//! (the engine's REQ-SEC-006). With no address, the tab opens the home
//! address of the settings, which the trusted core keeps between runs
//! (`settings-home-console-proxy.md`). The decoder and the interface are
//! found beside this program.

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

mod session;
mod source;
mod trusted_core;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use confine::Role;
use confine::broker::{Broker, Handlers};
use neoteletext::settings as stored;

/// The trusted core's own flag: the broker starts this program again
/// with it (decision 56).
const CORE_FLAG: &str = "--core";

/// This release's version, in the program's own bytes, where the launcher
/// reads it from the signed file (`installation-and-updates.md` section 5).
static VERSION_MARK: &str = update::version_mark!(env!("NEOTELETEXT_VERSION"));

/// Folders under the home folder whose files are the user's secrets: the
/// broker never reads a file channel there for the core, however it is
/// named (decision 56).
const SECRET_FOLDERS: &[&str] = &[".ssh", ".gnupg", ".aws", ".kube", ".password-store", ".config/gcloud", ".docker", "Library/Keychains", "Library/Cookies"];

/// The trusted core: confined first (decision 56), then the browser.
fn core() -> Result<(), String> {
    let report = confine::apply(Role::Core, stored::folder().as_deref())?;
    eprintln!("neoteletext: the trusted core is confined: {report}");
    let broker = Broker::from_parent().map_err(|error| error.to_string())?;
    trusted_core::run(&broker, std::env::args().skip(2), &report.to_string())
}

/// What the cookie file's key is kept under in the platform's secret store
/// (`cookies.md` section 7): a name a person reading the store recognises.
const KEY_SERVICE: &str = "NeoTeletext cookie file";
const KEY_ACCOUNT: &str = "cookies.bin";

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

/// A file channel's bytes, read by the broker for the core: never from a
/// folder of the user's secrets, and bounded as `Source::read` bounds it.
fn read_for_core(path: &Path) -> Result<Vec<u8>, String> {
    let canonical = path.canonicalize().map_err(|error| format!("{}: {error}", path.display()))?;
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if let Some(home) = home.and_then(|home| home.canonicalize().ok())
        && SECRET_FOLDERS.iter().any(|secret| canonical.starts_with(home.join(secret)))
    {
        return Err(format!("{}: not a channel", path.display()));
    }
    source::Source::read(&canonical).map(source::Source::into_bytes)
}

/// The broker (decision 56): the program the user starts, and the one
/// process not confined. It starts the trusted core confined, then serves
/// its requests until it ends.
fn broker() -> Result<(), String> {
    let program = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
    let programs = program.parent().ok_or("this program has no directory")?.to_path_buf();
    let settings = stored::folder().ok_or("there is no folder for the settings")?;
    std::fs::create_dir_all(&settings).map_err(|error| format!("{}: {error}", settings.display()))?;
    let mut arguments = vec![OsString::from(CORE_FLAG)];
    arguments.extend(std::env::args_os().skip(1));
    let environment = confine::environment(Role::Core);
    let core = confine::broker::start_core(&program, &arguments, &settings, &environment).map_err(|error| format!("cannot start the trusted core: {error}"))?;
    let version = update::mark::own(VERSION_MARK).map_err(|refused| format!("this program's version: {refused}"))?;
    eprintln!("neoteletext: version {version}; the trusted core started confined: {}", core.report);
    #[cfg(feature = "links")]
    let open_web = |address: &str| web_opener::open(address).map_err(|error| error.to_string());
    // With links compiled out the core never asks; if it does, it is refused.
    #[cfg(not(feature = "links"))]
    let open_web = |_: &str| Err(String::from("links are compiled out"));
    let handlers = Handlers {
        read_source: &read_for_core,
        open_web: &open_web,
        environment: &confine::environment,
        cookie_key: &cookie_key,
        healthy: &trusted_core::tell_launcher_healthy,
        update_state: &trusted_core::launcher_state,
        updater: &trusted_core::updater_grant,
    };
    let exit = confine::broker::serve(core, &programs, &handlers).map_err(|error| format!("cannot wait for the trusted core: {error}"))?;
    if exit.success { Ok(()) } else { Err(format!("the trusted core ended abnormally ({exit})")) }
}

fn main() -> ExitCode {
    let step = if std::env::args().nth(1).as_deref() == Some(CORE_FLAG) { core() } else { broker() };
    match step {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(refused.ends_with("not a channel"), "{refused}");
    }
}
