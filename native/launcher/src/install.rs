//! Installing, removing, and a launcher release
//! (`installation-and-updates.md` sections 4 and 7, requirements 13.15
//! and 13.20). Linux is installed by this program from its signed
//! tarball (`neoteletext --install`), macOS by dragging the bundle to
//! Applications, Windows by `NeoTeletext-Setup.exe` (the `installer`
//! crate). Each is removed by `--uninstall`, which asks before removing
//! the user's settings and bookmarks.

use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};

use update::fault;
use update::files;
use update::layout::{Layout, settings_folder};
use update::platform::{Family, LAUNCHER_PROGRAM};
use update::version::Version;

use crate::{Launcher, io_text};

/// Linux: the folder kept beside `bin/` for the launcher being replaced,
/// and for the launcher replaced (section 7).
const NEW_LAUNCHER: &str = "bin.new";
const OLD_LAUNCHER: &str = "bin.old";
/// The launcher's name in its folder on Linux.
const LINUX_LAUNCHER: &str = "neoteletext";

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
fn linux_launcher_release(launcher: &Launcher, bytes: &[u8]) -> Result<(), String> {
    let root = &launcher.layout.root;
    let (bin, new, old) = (launcher.layout.launcher_folder(), root.join(NEW_LAUNCHER), root.join(OLD_LAUNCHER));
    files::remove_tree(&new).map_err(|error| io_text(&new, &error))?;
    std::fs::create_dir(&new).map_err(|error| io_text(&new, &error))?;
    files::write_synced(&new.join(LINUX_LAUNCHER), bytes, true).map_err(|error| io_text(&new, &error))?;
    files::sync_folder(&new).map_err(|error| io_text(&new, &error))?;
    fault::point("launcher-written");
    #[cfg(unix)]
    {
        if let Err(error) = files::exchange(&new, &bin) {
            eprintln!("neoteletext: this file system cannot exchange folders ({error}); the new launcher waits");
            return Ok(());
        }
    }
    fault::point("launcher-exchanged");
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

/// The user's data folder for applications' entries (`XDG_DATA_HOME`).
fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute()).map(|home| home.join(".local/share")))
}

/// The menu entry's file (the freedesktop.org Desktop Entry
/// Specification, its "Installing" section).
fn desktop_entry() -> Option<PathBuf> {
    data_home().map(|data| data.join("applications/neoteletext.desktop"))
}

/// The command on the path (`~/.local/bin`, the systemd file-hierarchy
/// specification's user executables).
fn path_link() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute()).map(|home| home.join(".local/bin/neoteletext"))
}

/// The menu entry for a launcher at `program` (Desktop Entry
/// Specification version 1.5). There is no icon of the browser's own yet;
/// the theme's web browser icon stands in.
fn desktop_text(program: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.5\nName=NeoTeletext\nGenericName=Teletext browser\nComment=Browse teletext channels on the web\nExec=\"{}\" %U\nIcon=web-browser\nTerminal=false\nCategories=Network;WebBrowser;\n",
        program.display()
    )
}

/// The `AppArmor` restriction of Ubuntu from 24.04 (`sandboxing.md`
/// section 8.2): unprivileged user namespaces only for programs a profile
/// allows.
const APPARMOR_RESTRICTION: &str = "/proc/sys/kernel/apparmor_restrict_unprivileged_userns";
/// Where an administrator installs the browser's profile.
const APPARMOR_TARGET: &str = "/etc/apparmor.d/neoteletext";

/// Linux: `neoteletext --install`, run from the unpacked tarball, whose
/// signature the download page says how to check first: the version
/// beside this program is checked by the pinned release key, copied into
/// the user's data folder with this launcher, and given a menu entry and
/// a command on the path (section 4.3). Nothing needs root; what the
/// machine needs that only an administrator can give is said.
///
/// # Errors
/// When the version is not accepted or cannot be installed.
pub fn install_linux(own: &Path) -> Result<(), String> {
    let platform = update::platform::Platform::LinuxX86_64;
    let source = own.parent().ok_or("this program has no folder")?;
    let versions: Vec<(Version, PathBuf)> = std::fs::read_dir(source)
        .map_err(|error| io_text(source, &error))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            Some((entry.file_name().to_str().and_then(Version::parse)?, entry.path()))
        })
        .collect();
    let [(version, folder)] = versions.as_slice() else { return Err(format!("{} must hold exactly one version's folder", source.display())) };
    let signer = crate::pinned_signer(platform);
    update::accept::accept(folder, platform, signer.as_ref()).map_err(|why| format!("the release is refused: {why}"))?;
    let layout = Layout::for_user(Family::Linux, None).ok_or("there is no home folder to install in")?;
    if layout.state().exists() {
        println!("NeoTeletext is already installed in {}; it keeps itself up to date.", layout.root.display());
        return Ok(());
    }
    std::fs::create_dir_all(layout.versions()).map_err(|error| io_text(&layout.versions(), &error))?;
    let installed = layout.version(*version);
    files::remove_tree(&installed).map_err(|error| io_text(&installed, &error))?;
    files::copy_tree(folder, &installed).map_err(|error| io_text(&installed, &error))?;
    let bin = layout.launcher_folder();
    std::fs::create_dir_all(&bin).map_err(|error| io_text(&bin, &error))?;
    let launcher = bin.join(LINUX_LAUNCHER);
    let bytes = std::fs::read(installed.join(LAUNCHER_PROGRAM)).map_err(|error| io_text(&installed, &error))?;
    files::write_synced(&launcher, &bytes, true).map_err(|error| io_text(&launcher, &error))?;
    files::sync_folder(&bin).map_err(|error| io_text(&bin, &error))?;
    files::write_atomically(&layout.state(), update::state::State::installed(*version).to_text().as_bytes())
        .map_err(|error| io_text(&layout.state(), &error))?;
    if let Some(entry) = desktop_entry() {
        if let Some(folder) = entry.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        files::write_atomically(&entry, desktop_text(&launcher).as_bytes()).map_err(|error| io_text(&entry, &error))?;
    }
    let mut on_path = None;
    if let Some(link) = path_link() {
        if let Some(folder) = link.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        if std::fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            let _ = std::fs::remove_file(&link);
        }
        #[cfg(unix)]
        if std::os::unix::fs::symlink(&launcher, &link).is_ok() {
            on_path = Some(link);
        }
    }
    println!("NeoTeletext {version} is installed in {}.", layout.root.display());
    match on_path {
        Some(link) => println!("Start it from the applications menu, or with {} .", link.display()),
        None => println!("Start it from the applications menu, or with {} .", launcher.display()),
    }
    if std::fs::read_to_string(APPARMOR_RESTRICTION).is_ok_and(|value| value.trim() == "1") && !Path::new(APPARMOR_TARGET).exists() {
        let profile = installed.join(update::platform::APPARMOR_PROFILE);
        println!(
            "\nThis system lets a program make a user namespace, which the browser's sandbox needs, only when an AppArmor profile allows it.\n\
             An administrator must install the browser's profile once (it covers every later version):\n\n    \
             sudo install -m 644 {} {APPARMOR_TARGET}\n    sudo apparmor_parser -r {APPARMOR_TARGET}\n\n\
             Until then the browser refuses to start: it never runs with less of its sandbox.",
            profile.display()
        );
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_entry_starts_the_launcher() {
        let text = desktop_text(Path::new("/home/a/.local/share/neoteletext/bin/neoteletext"));
        assert!(text.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(text.contains("\nExec=\"/home/a/.local/share/neoteletext/bin/neoteletext\" %U\n"));
        assert!(text.ends_with("Categories=Network;WebBrowser;\n"));
    }
}
