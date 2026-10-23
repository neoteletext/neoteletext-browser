//! Installing and removing on the terminal, macOS and Linux
//! (`installation-and-updates.md` sections 4.1 and 4.3, requirements 13.15
//! and 13.20): Linux is installed by this program from its signed tarball
//! (`neoteletext --install`), macOS by dragging the bundle to Applications;
//! each is removed by `--uninstall`, which asks before removing the user's
//! settings and bookmarks.

use std::io::{BufRead as _, Write as _};
use std::path::Path;

use update::files;
use update::layout::settings_folder;
use update::platform::Family;

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
    let layout = &launcher.layout;
    let place = layout.bundle.clone().unwrap_or_else(|| layout.root.clone());
    if !confirm(&format!("Remove NeoTeletext from {}?", place.display())) {
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
        && confirm(&format!("Also remove your settings, bookmarks, history and cookies in {}?", settings.display()))
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
