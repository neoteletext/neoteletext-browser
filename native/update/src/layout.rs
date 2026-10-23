//! Where each desktop keeps an installed browser
//! (`installation-and-updates.md` section 4), per user, with no
//! administrator (requirement 13.15):
//!
//! | | macOS | Windows | Linux |
//! |---|---|---|---|
//! | The programs | `/Applications/NeoTeletext.app` or `~/Applications/…` | `%LOCALAPPDATA%\Programs\NeoTeletext\versions\<version>\` | `~/.local/share/neoteletext/versions/<version>/` |
//! | The launcher | inside the bundle | `…\NeoTeletext.exe`, the stub, and `launcher-a.exe`, `launcher-b.exe` | `~/.local/share/neoteletext/bin/neoteletext` |
//! | `state`, `staging` | `~/Library/Application Support/NeoTeletext Updates/` | `%LOCALAPPDATA%\Programs\NeoTeletext\` | `~/.local/share/neoteletext/` |
//!
//! On macOS the update folder is not the settings folder, which the
//! confined trusted core may write: only the launcher writes the state,
//! and only the updater the staging folder (section 14 of the design).

use std::path::{Path, PathBuf};

use crate::platform::{BUNDLE, Family};
use crate::version::Version;

/// The macOS update folder's name, beside the settings' `NeoTeletext`.
pub const MACOS_UPDATES: &str = "NeoTeletext Updates";
/// The Windows installation's folder under `%LOCALAPPDATA%\Programs`.
pub const WINDOWS_FOLDER: &str = "NeoTeletext";
/// The Linux installation's folder under the user's data folder.
pub const LINUX_FOLDER: &str = "neoteletext";
/// The stub every Windows shortcut starts (section 4.2).
pub const WINDOWS_STUB: &str = "NeoTeletext.exe";
/// The one-byte file naming the Windows launcher copy in use.
pub const WINDOWS_LAUNCHER_CHOICE: &str = "launcher";
/// The variable the launcher sets for the broker it starts: the broker
/// then reports health on its standard output and may run the updater.
pub const LAUNCHED_VARIABLE: &str = "NEOTELETEXT_LAUNCHED";
/// The updater's own folder in the staging folder, kept between runs: The
/// Update Framework's trusted metadata and its last check's time.
pub const UPDATER_METADATA: &str = "metadata";
/// The file in the updater's folder holding the last successful check's
/// time.
pub const UPDATER_CHECKED: &str = "checked";
/// Every name the updater's folder may hold: The Update Framework's
/// client's datastore (`tough` 0.24 writes `root.json`, `timestamp.json`,
/// `snapshot.json`, `targets.json` and `latest_known_time.json`, and
/// nothing else for a repository without delegations) and the updater's
/// own [`UPDATER_CHECKED`]. The launcher removes anything else there, so
/// nothing the confined updater leaves in its folder outlives a start
/// (decision 87).
pub const UPDATER_METADATA_FILES: [&str; 6] = ["root.json", "timestamp.json", "snapshot.json", "targets.json", "latest_known_time.json", UPDATER_CHECKED];
/// What the broker writes to the launcher once the trusted core found
/// the version healthy (section 3.2).
pub const HEALTHY_LINE: &str = "healthy\n";

/// The user's settings folder (requirement 19.1), which the trusted core
/// keeps and the uninstaller offers to remove (requirement 13.20):
/// `~/Library/Application Support/NeoTeletext` on macOS,
/// `%APPDATA%\NeoTeletext` on Windows, `$XDG_CONFIG_HOME/neoteletext` or
/// `~/.config/neoteletext` elsewhere.
#[must_use]
pub fn settings_folder() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return home().map(|home| home.join("Library/Application Support/NeoTeletext"));
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|data| PathBuf::from(data).join("NeoTeletext"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home().map(|home| home.join(".config")))
        .map(|config| config.join("neoteletext"))
}

/// An installation's places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The desktop's family.
    pub family: Family,
    /// The folder of the state, staging and, but on macOS, the versions.
    pub root: PathBuf,
    /// macOS: the installed bundle.
    pub bundle: Option<PathBuf>,
}

/// The user's home folder, when it is an absolute path.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute())
}

impl Layout {
    /// Where a new installation goes for this user. On macOS `bundle` is
    /// the bundle's place, which the installer chooses (section 4.1).
    #[must_use]
    pub fn for_user(family: Family, bundle: Option<PathBuf>) -> Option<Self> {
        let root = match family {
            Family::Macos => home()?.join("Library/Application Support").join(MACOS_UPDATES),
            Family::Windows => PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join(WINDOWS_FOLDER),
            Family::Linux => std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .or_else(|| home().map(|home| home.join(".local/share")))?
                .join(LINUX_FOLDER),
        };
        Some(Self { family, root, bundle })
    }

    /// The installation a running launcher at `launcher` belongs to:
    /// Linux `<root>/bin/neoteletext`, Windows `<root>\launcher-a.exe`,
    /// macOS `<bundle>/Contents/MacOS/neoteletext-launcher`.
    #[must_use]
    pub fn of_launcher(family: Family, launcher: &Path) -> Option<Self> {
        match family {
            Family::Linux => Some(Self { family, root: launcher.parent()?.parent()?.to_path_buf(), bundle: None }),
            Family::Windows => Some(Self { family, root: launcher.parent()?.to_path_buf(), bundle: None }),
            Family::Macos => Self::for_user(family, Some(launcher.parent()?.parent()?.parent()?.to_path_buf())),
        }
    }

    /// The installation a version's program at `program` belongs to:
    /// `<root>/versions/<version>/<program>` on Windows and Linux, the
    /// bundle's `Contents/MacOS/<program>` on macOS.
    #[must_use]
    pub fn of_program(family: Family, program: &Path) -> Option<Self> {
        match family {
            Family::Windows | Family::Linux => Some(Self { family, root: program.parent()?.parent()?.parent()?.to_path_buf(), bundle: None }),
            Family::Macos => Self::of_launcher(family, program),
        }
    }

    /// The state file.
    #[must_use]
    pub fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    /// The lock every launcher holds while it runs a version: exclusively
    /// while it changes anything, shared while a version runs.
    #[must_use]
    pub fn lock(&self) -> PathBuf {
        self.root.join("state.lock")
    }

    /// The updater's folder, the only one it may write.
    #[must_use]
    pub fn staging(&self) -> PathBuf {
        self.root.join("staging")
    }

    /// A staged version's folder.
    #[must_use]
    pub fn staged(&self, version: Version) -> PathBuf {
        self.staging().join(version.to_string())
    }

    /// The folder of the versions side by side (Windows, Linux).
    #[must_use]
    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }

    /// An installed version's folder (Windows, Linux).
    #[must_use]
    pub fn version(&self, version: Version) -> PathBuf {
        self.versions().join(version.to_string())
    }

    /// macOS: the launcher's copies of staged versions, which it accepts
    /// and exchanges in (decision 87); only the launcher writes here, and
    /// it is on the volume of the state, as the installed bundle must be
    /// for the exchange.
    #[must_use]
    pub fn accepting(&self) -> PathBuf {
        self.root.join("accepting")
    }

    /// macOS: the launcher's copy of a staged version.
    #[must_use]
    pub fn accepted(&self, version: Version) -> PathBuf {
        self.accepting().join(version.to_string())
    }

    /// macOS: the folder holding the previous bundle (section 4.1).
    #[must_use]
    pub fn previous(&self) -> PathBuf {
        self.root.join("previous")
    }

    /// macOS: the previous bundle.
    #[must_use]
    pub fn previous_bundle(&self) -> PathBuf {
        self.previous().join(BUNDLE)
    }

    /// The folder of a running version's programs: the bundle's
    /// `Contents/MacOS` on macOS, the version's folder elsewhere.
    #[must_use]
    pub fn programs(&self, version: Version) -> PathBuf {
        match (&self.bundle, self.family) {
            (Some(bundle), Family::Macos) => bundle.join("Contents/MacOS"),
            _ => self.version(version),
        }
    }

    /// Linux: the launcher's folder, exchanged whole by a launcher release
    /// (section 7).
    #[must_use]
    pub fn launcher_folder(&self) -> PathBuf {
        self.root.join("bin")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn each_program_finds_its_installation() {
        let linux = Layout::of_launcher(Family::Linux, Path::new("/home/a/.local/share/neoteletext/bin/neoteletext")).unwrap();
        assert_eq!(linux.root, Path::new("/home/a/.local/share/neoteletext"));
        assert_eq!(Layout::of_program(Family::Linux, Path::new("/home/a/.local/share/neoteletext/versions/0.2.0/neoteletext")).unwrap(), linux);
        assert_eq!(linux.version(Version::new(0, 2, 0)), Path::new("/home/a/.local/share/neoteletext/versions/0.2.0"));
        assert_eq!(linux.staged(Version::new(0, 3, 0)), Path::new("/home/a/.local/share/neoteletext/staging/0.3.0"));
        let windows = Layout::of_launcher(Family::Windows, Path::new("/c/Users/a/AppData/Local/Programs/NeoTeletext/launcher-a.exe")).unwrap();
        assert_eq!(
            Layout::of_program(Family::Windows, Path::new("/c/Users/a/AppData/Local/Programs/NeoTeletext/versions/0.2.0/neoteletext.exe")).unwrap(),
            windows
        );
        let bundle = Path::new("/Applications/NeoTeletext.app");
        let macos = Layout::of_launcher(Family::Macos, &bundle.join("Contents/MacOS/neoteletext-launcher"));
        if let Some(macos) = macos {
            assert_eq!(macos.bundle.as_deref(), Some(bundle));
            assert_eq!(macos.programs(Version::new(0, 1, 0)), bundle.join("Contents/MacOS"));
            assert!(macos.root.ends_with("Library/Application Support/NeoTeletext Updates"));
        }
    }
}
