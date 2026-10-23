//! Step 7 (section 3.1): what no state names is removed.

use update::files;
use update::platform::Family;
use update::state::State;
use update::version::Version;

use crate::Launcher;
#[cfg(target_os = "macos")]
use crate::macos;

impl Launcher {
    /// Step 7 (section 3.1): removes what no state names, leaving for next
    /// time what cannot be removed now (a program still running on Windows).
    pub(crate) fn tidy(&self, state: &State) {
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

    /// After a commit, the versions older than the previous one go.
    pub(crate) fn tidy_versions(&self, state: &State) {
        let kept = state.kept();
        let Ok(entries) = std::fs::read_dir(self.layout.versions()) else { return };
        for entry in entries.filter_map(Result::ok) {
            if entry.file_name().to_str().and_then(Version::parse).is_some_and(|version| !kept.contains(&version)) {
                let _ = files::remove_tree(&entry.path());
            }
        }
    }
}
