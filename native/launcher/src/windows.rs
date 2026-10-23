//! Windows: a launcher release by the fixed stub's two copies, and the
//! uninstaller (`installation-and-updates.md` sections 4.2 and 7,
//! requirement 13.20).

use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use update::fault;
use update::files;
use update::layout::{Layout, WINDOWS_LAUNCHER_CHOICE, settings_folder};
use update::platform::Family;

use crate::{Launcher, io_text};

/// The launcher copy a choice names.
#[must_use]
pub fn launcher_copy(root: &Path, choice: u8) -> PathBuf {
    root.join(format!("launcher-{}.exe", char::from(choice)))
}

/// The two copies' letters.
pub const CHOICES: [u8; 2] = *b"ab";

/// The Start menu shortcut, in the user's own Start menu.
#[must_use]
pub fn shortcut() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|data| PathBuf::from(data).join(r"Microsoft\Windows\Start Menu\Programs\NeoTeletext.lnk"))
}

/// Section 7: the running launcher writes the other copy, flushes it,
/// checks its signature, and switches the one-byte choice; the copy it
/// replaces stays until the next launcher release.
///
/// # Errors
/// When the new copy cannot be written, is not signed by the pinned
/// signer, or the choice cannot be switched; the old launcher stays.
pub fn launcher_release(launcher: &Launcher, bytes: &[u8]) -> Result<(), String> {
    let root = &launcher.layout.root;
    let choice_path = root.join(WINDOWS_LAUNCHER_CHOICE);
    let current = std::fs::read(&choice_path).ok().and_then(|bytes| bytes.first().copied()).filter(|choice| CHOICES.contains(choice)).unwrap_or(b'a');
    let other = if current == b'a' { b'b' } else { b'a' };
    let target = launcher_copy(root, other);
    let new = files::with_suffix(&target, files::NEW_SUFFIX);
    files::write_synced(&new, bytes, true).map_err(|error| io_text(&new, &error))?;
    fault::point("launcher-written");
    let signer = os_install::WindowsSigner { key: release_identity::WINDOWS_SIGNER, release: release_identity::RELEASE_KEY };
    if let Err(why) = signer.check_program(&new) {
        let _ = std::fs::remove_file(&new);
        return Err(format!("the new launcher is refused: {why}"));
    }
    std::fs::rename(&new, &target).map_err(|error| io_text(&target, &error))?;
    fault::point("launcher-renamed");
    files::write_atomically(&choice_path, &[other]).map_err(|error| io_text(&choice_path, &error))?;
    fault::point("launcher-switched");
    eprintln!("neoteletext: the launcher is replaced by {}", target.display());
    Ok(())
}

/// The flag the uninstaller's copy is started with.
pub const UNINSTALL_FROM: &str = "--uninstall-from";
/// And the one that says to remove the settings too.
pub const AND_SETTINGS: &str = "--and-settings";

/// `--uninstall`, from Settings › Apps: asks, then hands the removal to a
/// copy of this program in the temporary folder, since a running program
/// cannot remove itself on Windows, and ends.
///
/// # Errors
/// When the browser is running or the copy cannot start.
pub fn uninstall(launcher: &Launcher) -> Result<(), String> {
    let root = &launcher.layout.root;
    if !os_install::ask("Remove NeoTeletext", &format!("Remove NeoTeletext from {}?", root.display()), "Remove", "Keep") {
        return Ok(());
    }
    let lock = launcher.open_lock()?;
    if lock.try_lock().is_err() {
        os_install::tell("NeoTeletext", "NeoTeletext is running. Close it, then remove it again.");
        return Err(String::from("NeoTeletext is running"));
    }
    drop(lock);
    let settings = settings_folder().filter(|folder| folder.exists()).is_some_and(|folder| {
        os_install::ask("Remove NeoTeletext", &format!("Also remove your settings, bookmarks, history and cookies in {}?", folder.display()), "Remove", "Keep")
    });
    let copy = std::env::temp_dir().join(format!("neoteletext-uninstall-{}.exe", std::process::id()));
    std::fs::copy(&launcher.own, &copy).map_err(|error| io_text(&copy, &error))?;
    let mut arguments: Vec<OsString> = vec![OsString::from(UNINSTALL_FROM), root.as_os_str().to_owned()];
    if settings {
        arguments.push(OsString::from(AND_SETTINGS));
    }
    // Its input is ours: it ends when this program does.
    let child = Command::new(&copy).args(&arguments).stdin(Stdio::piped()).spawn().map_err(|error| io_text(&copy, &error))?;
    std::mem::forget(child);
    Ok(())
}

/// How many times the stub's file is tried after this launcher ended, and
/// how long apart: the stub ends at once after the launcher it waits for,
/// but has no pipe of its own to say so.
const STUB_TRIES: u32 = 50;
const STUB_TRY_INTERVAL: Duration = Duration::from_millis(100);

/// The uninstaller's copy: once the launcher that started it has ended
/// (its standard input closes), removes the installation, the Start menu
/// shortcut and the uninstall entry, and the settings if asked.
///
/// # Errors
/// When something cannot be removed.
pub fn uninstall_from(root: &Path, and_settings: bool) -> Result<(), String> {
    let layout = Layout { family: Family::Windows, root: root.to_path_buf(), bundle: None };
    if !layout.state().exists() && !root.join(update::layout::WINDOWS_STUB).exists() {
        return Err(format!("{} is not an installation", root.display()));
    }
    let mut ignored = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut ignored);
    let mut removed = files::remove_tree(root);
    for _ in 0..STUB_TRIES {
        if removed.is_ok() {
            break;
        }
        std::thread::sleep(STUB_TRY_INTERVAL);
        removed = files::remove_tree(root);
    }
    removed.map_err(|error| io_text(root, &error))?;
    if let Some(shortcut) = shortcut() {
        let _ = std::fs::remove_file(shortcut);
    }
    os_install::remove_uninstall_entry()?;
    confine::remove_containers();
    if and_settings && let Some(settings) = settings_folder() {
        files::remove_tree(&settings).map_err(|error| io_text(&settings, &error))?;
    }
    os_install::tell("NeoTeletext", "NeoTeletext is removed.");
    Ok(())
}
