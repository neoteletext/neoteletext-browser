//! Installing and removing on the terminal, macOS and Linux
//! (`installation-and-updates.md` sections 4.1 and 4.3, requirements 13.15
//! and 13.20): Linux is installed by this program from its signed tarball
//! (`neoteletext --install`), macOS by dragging the bundle to Applications;
//! each is removed by `--uninstall`, which asks before removing the user's
//! settings and bookmarks.

use std::io::{BufRead as _, Write as _};
use std::path::Path;

use update::files;
use update::layout::{LINUX_FOLDER, settings_folder};
use update::platform::{BUNDLE, Family};

use crate::{Launcher, io_text};

mod linux;

pub use linux::install_linux;
use linux::{APPARMOR_TARGET, desktop_entry, path_link};

/// Asks on the terminal; whether the answer was yes.
fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().take_line(&mut answer);
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

/// A line from the terminal, bounded.
trait TakeLine {
    fn take_line(&mut self, into: &mut String) -> std::io::Result<usize>;
}

impl<R: std::io::BufRead> TakeLine for R {
    fn take_line(&mut self, into: &mut String) -> std::io::Result<usize> {
        /// The longest answer read.
        const MAX_ANSWER: u64 = 16;
        std::io::Read::take(self, MAX_ANSWER).read_line(into)
    }
}

/// macOS and Linux: `--uninstall` on the terminal removes the program, its
/// menu entry and its update files, and asks before removing the settings,
/// bookmarks, history and cookies (requirement 13.20).
///
/// # Errors
/// When the browser is running, or something cannot be removed.
pub fn uninstall(launcher: &Launcher) -> Result<(), String> {
    remove(launcher, &confirm)
}

/// Whether this launcher is in an installation, which `--uninstall` may
/// remove: its places are found from where the program is, so a copy of
/// it anywhere else would otherwise name a folder of the user's (finding
/// F-48). On Linux the folder two above the program must be the one an
/// installation is (`neoteletext`), holding the state or its versions; on
/// macOS the bundle three above it must be `NeoTeletext.app` with its
/// `Info.plist`.
fn is_installation(launcher: &Launcher) -> Result<(), String> {
    let layout = &launcher.layout;
    let installed = match launcher.platform.family() {
        Family::Macos => {
            layout.bundle.as_ref().is_some_and(|bundle| bundle.file_name().is_some_and(|name| name == BUNDLE) && bundle.join("Contents/Info.plist").is_file())
        }
        Family::Linux | Family::Windows => {
            layout.root.file_name().is_some_and(|name| name == LINUX_FOLDER) && (layout.state().is_file() || layout.versions().is_dir())
        }
    };
    if installed {
        Ok(())
    } else {
        let place = layout.bundle.as_ref().unwrap_or(&layout.root);
        Err(format!("{} is not an installation of NeoTeletext, so nothing is removed", place.display()))
    }
}

/// `--uninstall`, asking each question of `ask`.
fn remove(launcher: &Launcher, ask: &dyn Fn(&str) -> bool) -> Result<(), String> {
    is_installation(launcher)?;
    let layout = &launcher.layout;
    let place = layout.bundle.clone().unwrap_or_else(|| layout.root.clone());
    if !ask(&format!("Remove NeoTeletext from {}?", place.display())) {
        return Ok(());
    }
    let lock = launcher.open_lock()?;
    if lock.try_lock().is_err() {
        return Err(String::from("NeoTeletext is running; quit it first"));
    }
    if launcher.platform.family() == Family::Linux {
        if let Some(link) = path_link()
            && std::fs::read_link(&link).is_ok_and(|target| target.starts_with(&layout.root))
        {
            let _ = std::fs::remove_file(&link);
        }
        if let Some(entry) = desktop_entry() {
            let _ = std::fs::remove_file(entry);
        }
    }
    if let Some(bundle) = &layout.bundle {
        files::remove_tree(bundle).map_err(|error| io_text(bundle, &error))?;
    }
    drop(lock);
    files::remove_tree(&layout.root).map_err(|error| io_text(&layout.root, &error))?;
    if let Some(settings) = settings_folder().filter(|folder| folder.exists())
        && ask(&format!("Also remove your settings, bookmarks, history and cookies in {}?", settings.display()))
    {
        files::remove_tree(&settings).map_err(|error| io_text(&settings, &error))?;
    }
    println!("NeoTeletext is removed.");
    if Path::new(APPARMOR_TARGET).exists() {
        println!(
            "Its AppArmor profile was installed by an administrator, who may remove it with:\n    sudo apparmor_parser -R {APPARMOR_TARGET} && sudo rm {APPARMOR_TARGET}"
        );
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use update::layout::Layout;
    use update::platform::Platform;

    /// Finding F-48: `--uninstall` from a launcher copied anywhere else
    /// removes nothing: the folder two levels above it, or the bundle
    /// three above on macOS, is not an installation. The test answers yes
    /// to removing the program and no to the settings.
    #[test]
    fn uninstalling_from_a_copied_launcher_removes_nothing() {
        let folder = std::env::temp_dir().join(format!("neoteletext-uninstall-copied-{}", std::process::id()));
        let _ = files::remove_tree(&folder);
        std::fs::create_dir_all(folder.join("bin")).unwrap();
        std::fs::write(folder.join("bin/neoteletext"), b"a copy of the launcher").unwrap();
        std::fs::write(folder.join("the user's file"), b"kept").unwrap();
        for platform in [Platform::LinuxX86_64, Platform::MacosArm64] {
            let family = platform.family();
            let bundle = (family == Family::Macos).then(|| folder.clone());
            let layout = Layout { family, root: folder.clone(), bundle };
            let own = folder.join("bin/neoteletext");
            let launcher = Launcher {
                platform,
                signer: crate::pinned_signer(platform),
                own,
                layout,
                start_deadline: crate::START_DEADLINE,
                health_deadline: crate::HEALTH_DEADLINE,
            };
            let answer = |question: &str| question.starts_with("Remove NeoTeletext");
            assert!(remove(&launcher, &answer).is_err(), "{platform}: refused");
            assert!(folder.join("the user's file").is_file(), "{platform}: nothing removed");
        }
        files::remove_tree(&folder).unwrap();
    }
}
