//! The launcher of one installation, and what a start runs, under the
//! installation's lock (section 3.1).

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use update::accept::Signer;
use update::layout::Layout;
use update::platform::{Family, Platform};
use update::version::Version;

use crate::start;

/// How long a trial version's trusted core has to find it healthy
/// (section 3.2: 30 seconds), with room for the broker and the core to
/// start before the core's own count begins.
pub const HEALTH_DEADLINE: Duration = Duration::from_secs(30 + 15);

/// A launcher of one installation.
pub struct Launcher {
    /// The platform.
    pub platform: Platform,
    /// The installation.
    pub layout: Layout,
    /// The pinned signer's check of a version.
    pub signer: Box<dyn Signer>,
    /// The launcher's own program: a launcher release replaces it when the
    /// version's differs (section 7).
    pub own: PathBuf,
    /// How long a trial has to be found healthy.
    pub health_deadline: Duration,
}

impl std::fmt::Debug for Launcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Launcher").field("platform", &self.platform).field("layout", &self.layout).finish_non_exhaustive()
    }
}

/// What a start runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chosen {
    /// A trial version, its start counted: commit it when healthy.
    Trial(Version),
    /// A committed version, or whatever another start chose.
    Run(Version),
}

impl Chosen {
    /// The version.
    #[must_use]
    pub const fn version(self) -> Version {
        match self {
            Self::Trial(version) | Self::Run(version) => version,
        }
    }
}

/// How a start ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ended {
    /// The trusted core found the version healthy.
    pub healthy: bool,
    /// The broker ended successfully.
    pub success: bool,
    /// Its exit code, where it has one.
    pub code: Option<i32>,
}

/// The pinned signer for this platform, from the release's identity.
#[must_use]
pub fn pinned_signer(platform: Platform) -> Box<dyn Signer> {
    let keys = release_identity::keys();
    match platform.family() {
        Family::Linux => Box::new(update::accept::LinuxSigner { key: keys.linux_release }),
        Family::Macos => Box::new(update::accept::MacosSigner { requirement: keys.macos_requirement }),
        #[cfg(windows)]
        Family::Windows => Box::new(os_install::WindowsSigner { key: keys.windows_signer }),
        #[cfg(not(windows))]
        Family::Windows => Box::new(update::accept::LinuxSigner { key: None }),
    }
}

pub(crate) fn io_text(what: &Path, error: &io::Error) -> String {
    format!("{}: {error}", what.display())
}

impl Launcher {
    /// The launcher this program is, from where it is.
    ///
    /// # Errors
    /// When this platform has no releases, or the program is not where an
    /// installation keeps it.
    pub fn this() -> Result<Self, String> {
        let platform = Platform::this().ok_or("there are no releases for this platform")?;
        let own = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
        let layout = Layout::of_launcher(platform.family(), &own).ok_or("this program is not in an installation")?;
        Ok(Self { platform, layout, signer: pinned_signer(platform), own, health_deadline: HEALTH_DEADLINE })
    }

    /// Runs the browser: chooses a version, starts it, commits it when
    /// healthy, and returns the broker's exit code. A trial that ends
    /// badly is counted and the choice made again, so a broken version
    /// is rolled back within the same start (section 13).
    ///
    /// # Errors
    /// When nothing can be started.
    pub fn run(&self, arguments: &[OsString]) -> Result<Option<i32>, String> {
        loop {
            let lock = self.open_lock()?;
            let chosen = match lock.try_lock() {
                Ok(()) => {
                    let chosen = self.prepare()?;
                    lock.unlock().map_err(|error| io_text(&self.layout.lock(), &error))?;
                    lock.lock_shared().map_err(|error| io_text(&self.layout.lock(), &error))?;
                    chosen
                }
                Err(TryLockError::WouldBlock) => {
                    // Another start changes things; once it runs its version,
                    // this one runs the same, and counts nothing.
                    lock.lock_shared().map_err(|error| io_text(&self.layout.lock(), &error))?;
                    Chosen::Run(self.read_state()?.running())
                }
                Err(TryLockError::Error(error)) => return Err(io_text(&self.layout.lock(), &error)),
            };
            let ended = start::start(self, chosen, arguments)?;
            drop(lock);
            if matches!(chosen, Chosen::Trial(_)) && !ended.healthy && !ended.success {
                eprintln!("neoteletext: version {} did not start; trying again", chosen.version());
                continue;
            }
            return Ok(ended.code);
        }
    }

    pub(crate) fn open_lock(&self) -> Result<File, String> {
        std::fs::create_dir_all(&self.layout.root).map_err(|error| io_text(&self.layout.root, &error))?;
        OpenOptions::new().read(true).write(true).create(true).truncate(false).open(self.layout.lock()).map_err(|error| io_text(&self.layout.lock(), &error))
    }
}
