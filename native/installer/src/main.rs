//! The Windows installer, `NeoTeletext-Setup.exe`
//! (`installation-and-updates.md` section 4.2): per user, no
//! administrator, no service, no Windows Installer. It checks the release
//! it carries by the signer the launcher pins, then copies the stub, a
//! launcher and the first version into `%LOCALAPPDATA%\Programs\NeoTeletext`,
//! grants `ALL APPLICATION PACKAGES` read on the programs so the
//! containers can start them, writes the launcher's state, the Start menu
//! shortcut and the uninstall entry Settings › Apps removes it by, and
//! starts the browser.

// A desktop program: no console window opens with it.
#![cfg_attr(windows, windows_subsystem = "windows")]
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

mod shortcut;

#[allow(missing_docs, reason = "the generated list carries its own documentation")]
mod payload {
    include!(concat!(env!("OUT_DIR"), "/payload.rs"));
}

use std::process::ExitCode;

/// The stub's and the launcher's names in the payload.
const PAYLOAD_STUB: &str = "NeoTeletext.exe";
const PAYLOAD_LAUNCHER: &str = "launcher.exe";

fn main() -> ExitCode {
    #[cfg(windows)]
    os_install::restrict_dll_search();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report(&error);
            ExitCode::FAILURE
        }
    }
}

fn report(error: &str) {
    #[cfg(windows)]
    os_install::tell("NeoTeletext", &format!("NeoTeletext was not installed: {error}"));
    eprintln!("neoteletext-installer: {error}");
}

#[cfg(not(windows))]
fn run() -> Result<(), String> {
    let _ = (payload::FILES, PAYLOAD_STUB, PAYLOAD_LAUNCHER, shortcut::shortcut);
    Err(String::from("NeoTeletext-Setup.exe installs on Windows; on macOS drag NeoTeletext to Applications, on Linux run neoteletext --install"))
}

#[cfg(windows)]
fn run() -> Result<(), String> {
    use std::path::Path;
    use update::files;
    use update::layout::{Layout, WINDOWS_LAUNCHER_CHOICE, WINDOWS_STUB};
    use update::platform::{Family, Platform};
    use update::state::State;
    use update::version::Version;

    let platform = Platform::WindowsX86_64;
    if payload::FILES.is_empty() {
        return Err(String::from("this installer was built without a release"));
    }
    let layout = Layout::for_user(Family::Windows, None).ok_or("there is no user's application folder (LOCALAPPDATA)")?;
    let stub = layout.root.join(WINDOWS_STUB);
    if layout.state().exists() {
        // Installed already: updates are the updater's; the browser starts.
        std::process::Command::new(&stub).spawn().map_err(|error| format!("{}: {error}", stub.display()))?;
        return Ok(());
    }
    let io = |path: &Path, error: std::io::Error| format!("{}: {error}", path.display());
    let mut version: Option<Version> = None;
    let partial_name = |path: &str| -> Option<(Version, String)> {
        let (folder, rest) = path.split_once('/')?;
        Some((Version::parse(folder)?, rest.to_owned()))
    };
    std::fs::create_dir_all(layout.versions()).map_err(|error| io(&layout.versions(), error))?;
    let staging = layout.root.join("installing");
    files::remove_tree(&staging).map_err(|error| io(&staging, error))?;
    for &(name, bytes) in payload::FILES {
        let (target, executable) = match name {
            PAYLOAD_STUB | PAYLOAD_LAUNCHER => (staging.join(name), true),
            other => {
                let (found, rest) = partial_name(other).ok_or_else(|| format!("{other}: not a file of one version"))?;
                if version.is_some_and(|known| known != found) {
                    return Err(String::from("the payload holds more than one version"));
                }
                version = Some(found);
                (
                    staging.join(found.to_string()).join(rest.replace('/', "\\")),
                    Path::new(&rest).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("exe")),
                )
            }
        };
        if let Some(folder) = target.parent() {
            std::fs::create_dir_all(folder).map_err(|error| io(folder, error))?;
        }
        files::write_synced(&target, bytes, executable).map_err(|error| io(&target, error))?;
    }
    let version = version.ok_or("the payload holds no version")?;
    // The launcher's own check, with the signer it pins (section 5).
    let signer = os_install::WindowsSigner { key: release_identity::WINDOWS_SIGNER, release: release_identity::RELEASE_KEY };
    let staged = staging.join(version.to_string());
    update::accept::accept(&staged, platform, &signer).map_err(|why| format!("the release is refused: {why}"))?;
    for program in [PAYLOAD_STUB, PAYLOAD_LAUNCHER] {
        signer.check_program(&staging.join(program))?;
    }
    let installed = layout.version(version);
    files::remove_tree(&installed).map_err(|error| io(&installed, error))?;
    std::fs::rename(&staged, &installed).map_err(|error| io(&installed, error))?;
    let launcher = launcher::windows::launcher_copy(&layout.root, b'a');
    std::fs::rename(staging.join(PAYLOAD_LAUNCHER), &launcher).map_err(|error| io(&launcher, error))?;
    std::fs::rename(staging.join(PAYLOAD_STUB), &stub).map_err(|error| io(&stub, error))?;
    files::remove_tree(&staging).map_err(|error| io(&staging, error))?;
    files::write_atomically(&layout.root.join(WINDOWS_LAUNCHER_CHOICE), b"a").map_err(|error| io(&layout.root, error))?;
    // The containers start the programs only from a folder that grants
    // them read, as Program Files does (`sandboxing.md` section 8.2).
    confine::grant_packages_read(&layout.versions()).map_err(|error| io(&layout.versions(), error))?;
    files::write_atomically(&layout.state(), State::installed(version).to_text().as_bytes()).map_err(|error| io(&layout.state(), error))?;
    if let Some(link) = launcher::windows::shortcut() {
        if let Some(folder) = link.parent() {
            std::fs::create_dir_all(folder).map_err(|error| io(folder, error))?;
        }
        files::write_atomically(&link, &shortcut::shortcut(&stub, &layout.root, "NeoTeletext")).map_err(|error| io(&link, error))?;
    }
    let size = folder_size(&layout.root) / BYTES_PER_KIB;
    let uninstall = format!("\"{}\" --uninstall", stub.display());
    os_install::uninstall_entry(&os_install::UninstallEntry {
        name: "NeoTeletext",
        version: &version.to_string(),
        publisher: "NeoTeletext",
        location: &layout.root,
        uninstall: &uninstall,
        icon: &stub,
        size_kib: u32::try_from(size).unwrap_or(u32::MAX),
    })?;
    std::process::Command::new(&stub).spawn().map_err(|error| io(&stub, error))?;
    Ok(())
}

/// Bytes in a kibibyte, the uninstall entry's unit.
#[cfg(windows)]
const BYTES_PER_KIB: u64 = 1024;

/// The bytes of every file under `folder`.
#[cfg(windows)]
fn folder_size(folder: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(folder) else { return 0 };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => folder_size(&entry.path()),
            _ => entry.metadata().map_or(0, |metadata| metadata.len()),
        })
        .fold(0_u64, u64::saturating_add)
}
