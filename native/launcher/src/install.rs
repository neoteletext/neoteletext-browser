//! A launcher release (`installation-and-updates.md` section 7): the
//! launcher of a version that committed replaces the running one when
//! they differ, by exchanging Linux's launcher folder or switching
//! Windows's two copies; the old one is kept until the new one has started
//! a healthy version.

use update::files;
use update::platform::{Family, LAUNCHER_PROGRAM};
use update::version::Version;

use crate::{Launcher, io_text};

/// Linux: the folder kept beside `bin/` for the launcher being replaced,
/// and for the launcher replaced (section 7).
const NEW_LAUNCHER: &str = "bin.new";
const OLD_LAUNCHER: &str = "bin.old";
/// The launcher's name in its folder on Linux.
#[cfg(unix)]
pub(crate) const LINUX_LAUNCHER: &str = "neoteletext";

/// Section 7: after `version` committed, its launcher replaces this one
/// when they differ, and this one is kept until the new one has started a
/// version that committed.
///
/// # Errors
/// When the new launcher cannot be read.
pub fn launcher_release(launcher: &Launcher, version: Version) -> Result<(), String> {
    let new = launcher.layout.version(version).join(launcher.platform.program_file(LAUNCHER_PROGRAM));
    let bytes = std::fs::read(&new).map_err(|error| io_text(&new, &error))?;
    if std::fs::read(&launcher.own).is_ok_and(|own| own == bytes) {
        return Ok(());
    }
    match launcher.platform.family() {
        #[cfg(unix)]
        Family::Linux => linux_launcher_release(launcher, &bytes),
        #[cfg(windows)]
        Family::Windows => crate::windows::launcher_release(launcher, &bytes),
        _ => Ok(()),
    }
}

/// Linux: the new launcher is written beside `bin/`, then the two
/// folders are exchanged (`renameat2`, `RENAME_EXCHANGE`); on a file
/// system that refuses the exchange, the old launcher stays and the
/// release waits.
#[cfg(unix)]
fn linux_launcher_release(launcher: &Launcher, bytes: &[u8]) -> Result<(), String> {
    let root = &launcher.layout.root;
    let (bin, new, old) = (launcher.layout.launcher_folder(), root.join(NEW_LAUNCHER), root.join(OLD_LAUNCHER));
    files::remove_tree(&new).map_err(|error| io_text(&new, &error))?;
    std::fs::create_dir(&new).map_err(|error| io_text(&new, &error))?;
    files::write_synced(&new.join(LINUX_LAUNCHER), bytes, true).map_err(|error| io_text(&new, &error))?;
    files::sync_folder(&new).map_err(|error| io_text(&new, &error))?;
    update::fault::point("launcher-written");
    if let Err(error) = files::exchange(&new, &bin) {
        eprintln!("neoteletext: this file system cannot exchange folders ({error}); the new launcher waits");
        return Ok(());
    }
    update::fault::point("launcher-exchanged");
    files::remove_tree(&old).map_err(|error| io_text(&old, &error))?;
    std::fs::rename(&new, &old).map_err(|error| io_text(&new, &error))?;
    files::sync_folder(root).map_err(|error| io_text(root, &error))?;
    eprintln!("neoteletext: the launcher is replaced; the old one is kept until the new one has started a healthy version");
    Ok(())
}

/// Section 7: once a version this launcher started is healthy, the
/// launcher it replaced is no longer kept.
pub fn forget_old_launcher(launcher: &Launcher) {
    if launcher.platform.family() == Family::Linux {
        // After an exchange cut short, `bin.new` holds the old one.
        let _ = files::remove_tree(&launcher.layout.root.join(OLD_LAUNCHER));
        let _ = files::remove_tree(&launcher.layout.root.join(NEW_LAUNCHER));
    }
}
