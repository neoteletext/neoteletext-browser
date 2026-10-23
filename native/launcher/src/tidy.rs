//! Step 7 (section 3.1): what no state names is removed.

use update::files;
use update::layout::{UPDATER_METADATA, UPDATER_METADATA_FILES};
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
                if name == UPDATER_METADATA && entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    tidy_metadata(&entry.path());
                    continue;
                }
                let wanted = name.to_str().and_then(Version::parse).is_some_and(|version| version > state.current && !state.is_bad(version));
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
        {
            macos::tidy_accepting(self, state);
            macos::tidy_previous(self, state);
        }
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

/// Keeps in the updater's folder only the files the updater writes there
/// (`UPDATER_METADATA_FILES`), each a plain file with one name, and removes
/// anything else the confined updater left: another name, a folder, a
/// link, or a file also named elsewhere by a hard link (decision 87).
fn tidy_metadata(folder: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let own = entry.file_name().to_str().is_some_and(|name| UPDATER_METADATA_FILES.contains(&name));
        let single = std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_file() && links(&metadata) == 1);
        if !(own && single) {
            let removed = if entry.file_type().is_ok_and(|kind| kind.is_dir()) { files::remove_tree(&path) } else { std::fs::remove_file(&path) };
            if removed.is_ok() {
                eprintln!("neoteletext: {} is not the updater's and is removed", path.display());
            }
        }
    }
}

/// How many names a file has.
#[cfg(unix)]
fn links(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    metadata.nlink()
}

/// How many names a file has: Windows's standard library does not say,
/// and a hard link there cannot reach a version, which is copied in.
#[cfg(not(unix))]
fn links(_metadata: &std::fs::Metadata) -> u64 {
    1
}
