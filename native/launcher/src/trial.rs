//! Steps 1 to 5 of a start: a trial rolled back, a staged version
//! accepted as the trial, its starts counted, and a healthy trial
//! committed.

use std::path::Path;

use update::fault;
use update::files;
use update::platform::Family;
use update::state::{FAILED_STARTS_TO_ROLL_BACK, State, Trial};
use update::version::Version;

#[cfg(target_os = "macos")]
use crate::macos;
use crate::{Chosen, Launcher, io_text, launcher_release};

/// What a version is called under `versions/` while it is copied in: no
/// version's name, so `tidy` removes one left by a launcher stopped part
/// way.
pub(crate) const COPYING_SUFFIX: &str = ".copying";

impl Launcher {
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

    /// Step 3: accepts `version` and makes it the trial. What is accepted
    /// is the launcher's own copy of what the updater staged, made where
    /// only the launcher writes and checked there, never the staged files
    /// themselves: the confined updater could otherwise change a version
    /// after its check, through a hard link, a descriptor it kept open, or
    /// a staged folder that is a link (decision 87).
    fn accept(&self, version: Version, state: &mut State) -> Result<(), String> {
        let staged = self.layout.staged(version);
        let staged_there = match std::fs::symlink_metadata(&staged) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Err(String::from("the staged version is a link")),
            Ok(_) => true,
            Err(_) => false,
        };
        match self.platform.family() {
            Family::Macos => {
                let accepted = self.layout.accepted(version);
                std::fs::create_dir_all(self.layout.accepting()).map_err(|error| io_text(&self.layout.accepting(), &error))?;
                self.copy_and_accept(&staged, &accepted, version)?;
                state.trial = Some(Trial { version, failed: 0 });
                self.write_state(state)?;
                fault::point("accept-written");
                // The staged copy goes now; one left behind is tidied.
                let _ = files::remove_tree(&staged);
                #[cfg(target_os = "macos")]
                macos::reconcile(self, state)?;
                Ok(())
            }
            Family::Windows | Family::Linux => {
                let installed = self.layout.version(version);
                if staged_there {
                    std::fs::create_dir_all(self.layout.versions()).map_err(|error| io_text(&self.layout.versions(), &error))?;
                    files::remove_tree(&installed).map_err(|error| io_text(&installed, &error))?;
                    let copying = self.layout.versions().join(format!("{version}{COPYING_SUFFIX}"));
                    self.copy_and_accept(&staged, &copying, version)?;
                    std::fs::rename(&copying, &installed).map_err(|error| io_text(&copying, &error))?;
                    files::sync_folder(&self.layout.versions()).map_err(|error| io_text(&self.layout.versions(), &error))?;
                    // The staged copy goes now; one left behind is tidied.
                    let _ = files::remove_tree(&staged);
                    fault::point("accept-moved");
                } else {
                    // Copied and renamed in by a start that ended before it
                    // wrote the state: the launcher's own copy, checked again.
                    let accepted = update::accept::accept(&installed, self.platform, self.signer.as_ref())?;
                    if accepted != version {
                        return Err(format!("{} says {accepted}", installed.display()));
                    }
                }
                state.trial = Some(Trial { version, failed: 0 });
                self.write_state(state)?;
                fault::point("accept-written");
                Ok(())
            }
        }
    }

    /// The staged `version` copied by the launcher into `copy`, a folder
    /// only the launcher writes, and accepted there. On Windows a copy is
    /// also what makes a version the launcher's: what the updater wrote
    /// carries its container's full control and the low integrity label of
    /// every file an `AppContainer` writes, and the mitigation that refuses
    /// low integrity programs would refuse its helpers (measured end to
    /// end: "Access is denied"). A copy cut short is never what the state
    /// names, and is made again or tidied.
    fn copy_and_accept(&self, staged: &Path, copy: &Path, version: Version) -> Result<(), String> {
        files::remove_tree(copy).map_err(|error| io_text(copy, &error))?;
        update::accept::copy_release(staged, copy, self.platform)?;
        fault::point("accept-copied");
        let copied = update::accept::accept(copy, self.platform, self.signer.as_ref())?;
        if copied != version {
            return Err(format!("the copy of {version} says {copied}"));
        }
        Ok(())
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
            if let Err(why) = launcher_release::launcher_release(self, version) {
                eprintln!("neoteletext: the launcher is not replaced: {why}");
            }
        }
        Ok(())
    }
}
