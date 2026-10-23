//! The launcher of one installation, and what a start runs, under the
//! installation's lock (section 3.1).

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use update::accept::Signer;
use update::layout::{HEALTH_DEADLINE, Layout, START_DEADLINE};
use update::platform::{Family, Platform};
use update::version::Version;

use crate::start;

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
    /// How long a trial's broker has to say it started.
    pub start_deadline: Duration,
    /// How long a trial has, once its broker said it started, to be found
    /// healthy.
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

/// The deadline that ended a trial's start (decision 93).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deadline {
    /// Its broker did not say it started in time.
    Start,
    /// It said it started, and was not found healthy in time.
    Health,
}

/// How a start ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ended {
    /// The broker said it started (or that it was healthy, which says it
    /// too).
    pub started: bool,
    /// The trusted core found the version healthy.
    pub healthy: bool,
    /// The deadline that ended it, if one did.
    pub deadline: Option<Deadline>,
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
        Ok(Self { platform, layout, signer: pinned_signer(platform), own, start_deadline: START_DEADLINE, health_deadline: HEALTH_DEADLINE })
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
            if let Chosen::Trial(version) = chosen
                && !ended.healthy
            {
                if ended.deadline == Some(Deadline::Start) {
                    return self.run_current(version, arguments);
                }
                if !ended.success {
                    eprintln!("neoteletext: version {version} did not start; trying again");
                    continue;
                }
            }
            return Ok(ended.code);
        }
    }

    /// After a trial missed its start deadline, the current version runs
    /// for this start, and the trial is tried again at the next (decision
    /// 93). On macOS the installed bundle is the trial's until a rollback
    /// exchanges it back, so there this start ends instead.
    fn run_current(&self, trial: Version, arguments: &[OsString]) -> Result<Option<i32>, String> {
        if self.platform.family() == Family::Macos {
            return Err(format!("version {trial} did not start in time; start NeoTeletext again"));
        }
        let lock = self.open_lock()?;
        lock.lock_shared().map_err(|error| io_text(&self.layout.lock(), &error))?;
        let current = self.read_state()?.current;
        eprintln!("neoteletext: running version {current} this time; {trial} is tried again at the next start");
        Ok(start::start(self, Chosen::Run(current), arguments)?.code)
    }

    pub(crate) fn open_lock(&self) -> Result<File, String> {
        std::fs::create_dir_all(&self.layout.root).map_err(|error| io_text(&self.layout.root, &error))?;
        OpenOptions::new().read(true).write(true).create(true).truncate(false).open(self.layout.lock()).map_err(|error| io_text(&self.layout.lock(), &error))
    }
}
