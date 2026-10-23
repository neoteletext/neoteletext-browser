//! The broker (decision 56): the program the user starts, and the one
//! process not confined. It starts the trusted core confined and serves
//! its requests: it reads file channels for it, never from a folder of the
//! user's secrets; opens web pages; holds the cookie file's key; tells the
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

/// Folders under the home folder whose files are the user's secrets: the
/// broker never reads a file channel there for the core, however it is
/// named (decision 56).
const SECRET_FOLDERS: &[&str] = &[".ssh", ".gnupg", ".aws", ".kube", ".password-store", ".config/gcloud", ".docker", "Library/Keychains", "Library/Cookies"];

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
