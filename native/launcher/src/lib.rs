//! The launcher (`installation-and-updates.md` section 3): what the user
//! starts, on every desktop. It parses no network data: its only inputs
//! are its own state file and a version's signed files.
//!
//! At each start, holding the installation's lock alone:
//!
//! 1. It reads the state (section 3.1), and on macOS makes the installed
//!    bundle the one the state names (section 4.1).
//! 2. A trial version that failed to start twice is rolled back: marked
//!    bad, and the current version, never touched, runs (step 6).
//! 3. With no trial, the newest staged version newer than the current one
//!    and not bad is accepted: complete, signed by the pinned signer, of
//!    the version its files say (section 5); it becomes the trial.
//! 4. A trial's failed starts are counted before it starts (step 5).
//!
//! It then starts the broker of the version chosen, sharing the lock, so
//! a second start runs the same version and changes nothing. When the
//! trusted core of a trial version finds it healthy (section 3.2), the
//! broker says so on its standard output and the launcher commits it. A
//! trial that ends badly, or says nothing in time, is ended with every
//! process it started and counted; the next start rolls it back.

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

pub mod install;
#[cfg(target_os = "macos")]
mod macos;
mod start;
#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub use macos::{must_move, offer_move};

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use update::accept::Signer;
use update::fault;
use update::files;
use update::layout::Layout;
use update::platform::{Family, Platform};
use update::state::{FAILED_STARTS_TO_ROLL_BACK, MAX_STATE, State, Trial};
use update::version::Version;

/// How long a trial version's trusted core has to find it healthy
/// (section 3.2: 30 seconds), with room for the broker and the core to
/// start before the core's own count begins.
pub const HEALTH_DEADLINE: Duration = Duration::from_secs(30 + 15);

/// What a Windows version is called under `versions\` while it is copied
/// in: no version's name, so `tidy` removes one left by a launcher
/// stopped part way.
const COPYING_SUFFIX: &str = ".copying";

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

fn io_text(what: &Path, error: &io::Error) -> String {
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

    fn open_lock(&self) -> Result<File, String> {
        std::fs::create_dir_all(&self.layout.root).map_err(|error| io_text(&self.layout.root, &error))?;
        OpenOptions::new().read(true).write(true).create(true).truncate(false).open(self.layout.lock()).map_err(|error| io_text(&self.layout.lock(), &error))
    }

    /// The state, or one rebuilt from what is installed when the file is
    /// missing or unreadable (section 14 of the design).
    ///
    /// # Errors
    /// When there is no state and nothing installed to rebuild it from.
    pub fn read_state(&self) -> Result<State, String> {
        let path = self.layout.state();
        match files::read_bounded(&path, MAX_STATE) {
            Ok(bytes) => match State::parse(&bytes) {
                Ok(state) => return Ok(state),
                Err(refused) => eprintln!("neoteletext: the state file is not one ({refused}); rebuilding it from what is installed"),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_text(&path, &error)),
        }
        self.recovered()
    }

    /// A state naming the newest complete version installed.
    fn recovered(&self) -> Result<State, String> {
        if self.platform.family() == Family::Macos {
            #[cfg(target_os = "macos")]
            return macos::installed(self).map(State::installed);
        }
        let mut found: Vec<Version> = std::fs::read_dir(self.layout.versions())
            .map_err(|error| io_text(&self.layout.versions(), &error))?
            .filter_map(|entry| entry.ok()?.file_name().to_str().and_then(Version::parse))
            .collect();
        found.sort_unstable();
        found
            .into_iter()
            .rev()
            .find(|version| update::accept::complete(&self.layout.version(*version), self.platform).is_ok())
            .map(State::installed)
            .ok_or_else(|| String::from("no complete version is installed"))
    }

    /// Writes the state whole (requirement 13.4).
    ///
    /// # Errors
    /// When it cannot be written; the old state then stands.
    pub fn write_state(&self, state: &State) -> Result<(), String> {
        files::write_atomically(&self.layout.state(), state.to_text().as_bytes()).map_err(|error| io_text(&self.layout.state(), &error))
    }

    /// Steps 1 to 4, holding the lock alone: what this start runs.
    ///
    /// # Errors
    /// When the state cannot be read or written.
    pub fn prepare(&self) -> Result<Chosen, String> {
        let mut state = self.read_state()?;
        // A state rebuilt from what is installed is written; a rollback is
        // said once, in the start that made it.
        let written = std::fs::read(self.layout.state()).ok();
        let mut changed = written.as_deref() != Some(state.to_text().as_bytes());
        changed |= state.rolled_back.take().is_some();
        #[cfg(target_os = "macos")]
        {
            changed |= macos::reconcile(self, &mut state)?;
        }
        if let Some(trial) = state.trial
            && trial.failed >= FAILED_STARTS_TO_ROLL_BACK
        {
            eprintln!("neoteletext: version {} failed to start {} times and is rolled back to {}", trial.version, trial.failed, state.current);
            state.trial = None;
            state.mark_bad(trial.version);
            state.rolled_back = Some(trial.version);
            self.write_state(&state)?;
            changed = false;
            fault::point("rollback-written");
            #[cfg(target_os = "macos")]
            {
                changed |= macos::reconcile(self, &mut state)?;
            }
        }
        if state.trial.is_none() {
            for candidate in self.candidates(&state) {
                match self.accept(candidate, &mut state) {
                    Ok(()) => {
                        changed = false;
                        break;
                    }
                    Err(why) => {
                        eprintln!("neoteletext: version {candidate} is refused: {why}");
                        state.mark_bad(candidate);
                        let _ = files::remove_tree(&self.layout.staged(candidate));
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.write_state(&state)?;
        }
        self.tidy(&state);
        if let Some(trial) = &mut state.trial {
            trial.failed = trial.failed.saturating_add(1);
            let version = trial.version;
            self.write_state(&state)?;
            fault::point("trial-counted");
            return Ok(Chosen::Trial(version));
        }
        Ok(Chosen::Run(state.current))
    }

    /// Versions that might be accepted, newest first: staged ones, and on
    /// Windows and Linux ones already moved beside the others whose state
    /// was not yet written when the power went.
    fn candidates(&self, state: &State) -> Vec<Version> {
        let mut found: Vec<Version> = Vec::new();
        let mut folders = vec![self.layout.staging()];
        if self.platform.family() != Family::Macos {
            folders.push(self.layout.versions());
        }
        for folder in folders {
            let Ok(entries) = std::fs::read_dir(&folder) else { continue };
            found.extend(entries.filter_map(|entry| entry.ok()?.file_name().to_str().and_then(Version::parse)));
        }
        found.retain(|version| *version > state.current && !state.is_bad(*version));
        found.sort_unstable();
        found.dedup();
        found.reverse();
        found
    }

    /// Step 3: accepts `version` and makes it the trial.
    fn accept(&self, version: Version, state: &mut State) -> Result<(), String> {
        let staged = self.layout.staged(version);
        match self.platform.family() {
            Family::Macos => {
                let accepted = update::accept::accept(&staged, self.platform, self.signer.as_ref())?;
                debug_assert_eq!(accepted, version, "accept checks the folder's name");
                state.trial = Some(Trial { version, failed: 0 });
                self.write_state(state)?;
                fault::point("accept-written");
                #[cfg(target_os = "macos")]
                macos::reconcile(self, state)?;
                Ok(())
            }
            Family::Windows | Family::Linux => {
                let installed = self.layout.version(version);
                let from = if staged.is_dir() { staged } else { installed.clone() };
                update::accept::accept(&from, self.platform, self.signer.as_ref())?;
                if from != installed {
                    std::fs::create_dir_all(self.layout.versions()).map_err(|error| io_text(&self.layout.versions(), &error))?;
                    files::remove_tree(&installed).map_err(|error| io_text(&installed, &error))?;
                    if self.platform.family() == Family::Windows {
                        self.copy_in(&from, &installed, version)?;
                    } else {
                        std::fs::rename(&from, &installed).map_err(|error| io_text(&from, &error))?;
                    }
                    files::sync_folder(&self.layout.versions()).map_err(|error| io_text(&self.layout.versions(), &error))?;
                    fault::point("accept-moved");
                }
                state.trial = Some(Trial { version, failed: 0 });
                self.write_state(state)?;
                fault::point("accept-written");
                Ok(())
            }
        }
    }

    /// Windows: the staged `version` copied into `installed` by the
    /// launcher, not renamed, and checked again there. What the updater
    /// wrote carries its container's full control and the low integrity
    /// label of every file an `AppContainer` writes; renamed, a version
    /// would keep both, so the updater could change it after it was
    /// checked, and the mitigation that refuses low integrity programs
    /// would refuse its helpers (measured end to end: "Access is denied").
    /// A copy is the launcher's, under `versions\`'s access list. A copy
    /// cut short is under a name that is no version, which `tidy` removes.
    fn copy_in(&self, staged: &Path, installed: &Path, version: Version) -> Result<(), String> {
        let copying = self.layout.versions().join(format!("{version}{COPYING_SUFFIX}"));
        files::remove_tree(&copying).map_err(|error| io_text(&copying, &error))?;
        files::copy_tree(staged, &copying).map_err(|error| io_text(&copying, &error))?;
        fault::point("accept-copied");
        let copied = update::accept::accept(&copying, self.platform, self.signer.as_ref())?;
        if copied != version {
            return Err(format!("the copy of {version} says {copied}"));
        }
        std::fs::rename(&copying, installed).map_err(|error| io_text(&copying, &error))?;
        // The staged copy goes now; one left behind is tidied.
        let _ = files::remove_tree(staged);
        Ok(())
    }

    /// Step 7 (section 3.1): removes what no state names, leaving for next
    /// time what cannot be removed now (a program still running on Windows).
    fn tidy(&self, state: &State) {
        let kept = state.kept();
        if let Ok(entries) = std::fs::read_dir(self.layout.staging()) {
            for entry in entries.filter_map(Result::ok) {
                let name = entry.file_name();
                // The updater's trusted metadata, which keeps the host from
                // rolling the repository back (requirement 13.8).
                if name == update::layout::UPDATER_METADATA {
                    continue;
                }
                let wanted = name.to_str().and_then(Version::parse).is_some_and(|version| version > state.current && !state.is_bad(version));
                #[cfg(target_os = "macos")]
                let wanted = wanted || macos::holds_old_current(self, state, &entry.path());
                if !wanted {
                    let _ = files::remove_tree(&entry.path());
                }
            }
        }
        if self.platform.family() != Family::Macos
            && let Ok(entries) = std::fs::read_dir(self.layout.versions())
        {
            for entry in entries.filter_map(Result::ok) {
                let version = entry.file_name().to_str().and_then(Version::parse);
                if !version.is_some_and(|version| kept.contains(&version)) {
                    let _ = files::remove_tree(&entry.path());
                }
            }
        }
        #[cfg(target_os = "macos")]
        macos::tidy_previous(self, state);
    }

    /// Step 5: the trial `version` was found healthy; it becomes current,
    /// and the version it replaces is kept as the previous one.
    ///
    /// # Errors
    /// When the state cannot be read or written.
    pub fn commit(&self, version: Version) -> Result<(), String> {
        let mut state = self.read_state()?;
        if state.trial.map(|trial| trial.version) != Some(version) {
            return Ok(());
        }
        fault::point("commit-before-write");
        state.previous = Some(state.current);
        state.current = version;
        state.trial = None;
        self.write_state(&state)?;
        fault::point("commit-written");
        eprintln!("neoteletext: version {version} is healthy and committed");
        if self.platform.family() != Family::Macos {
            self.tidy_versions(&state);
            // A launcher that cannot be replaced now is replaced at the
            // next commit; the version runs either way.
            if let Err(why) = install::launcher_release(self, version) {
                eprintln!("neoteletext: the launcher is not replaced: {why}");
            }
        }
        Ok(())
    }

    /// After a commit, the versions older than the previous one go.
    fn tidy_versions(&self, state: &State) {
        let kept = state.kept();
        let Ok(entries) = std::fs::read_dir(self.layout.versions()) else { return };
        for entry in entries.filter_map(Result::ok) {
            if entry.file_name().to_str().and_then(Version::parse).is_some_and(|version| !kept.contains(&version)) {
                let _ = files::remove_tree(&entry.path());
            }
        }
    }
}
