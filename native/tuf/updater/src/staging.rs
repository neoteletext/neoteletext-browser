//! Staging a version: each file downloaded, checked against the
//! manifest, written and flushed in `staging/<version>.partial`, which is
//! then renamed whole.

use std::path::Path;

use update::fault;
use update::files;
use update::manifest::Manifest;
use update::platform::{MANIFEST, MANIFEST_SIGNATURE};
use update::signature::SIGNATURE_BYTES;
use update::version::Version;

use crate::client::CHECKED;
use crate::{Config, MAX_ATTEMPTS, now};

/// The suffix of a version being staged.
pub const PARTIAL: &str = ".partial";
/// The release files' compression's suffix (section 6).
pub const COMPRESSED: &str = ".gz";

impl Config {
    pub(super) fn record_success(&self, at: i64) {
        let _ = files::write_atomically(&self.metadata().join(CHECKED), format!("{at}\n").as_bytes());
    }

    /// Downloads and stages `version` (section 3.1, steps 2 and 3).
    ///
    /// # Errors
    /// When the repository, a file or the disk fails; what was written is
    /// left in the partial folder, whose files that check are kept for the
    /// next attempt.
    pub async fn download(&self, version: Version) -> Result<(), String> {
        let staged = self.staging.join(version.to_string());
        if update::accept::complete(&staged, self.platform).is_ok() {
            return Ok(());
        }
        let repository = self.load().await?;
        self.record_success(now());
        let manifest_bytes = self.fetch(&repository, version, MANIFEST, None).await?;
        let manifest = Manifest::parse(&manifest_bytes).map_err(|refused| format!("the manifest: {refused}"))?;
        if manifest.version != version || manifest.platform != self.platform {
            return Err(format!("the manifest is of {} for {}", manifest.version, manifest.platform));
        }
        let partial = files::with_suffix(&staged, PARTIAL);
        std::fs::create_dir_all(&partial).map_err(|error| format!("{}: {error}", partial.display()))?;
        files::write_synced(&partial.join(MANIFEST), &manifest_bytes, false).map_err(|error| format!("{MANIFEST}: {error}"))?;
        if self.platform.signs_manifest() {
            let signature = self.fetch(&repository, version, MANIFEST_SIGNATURE, Some(u64::try_from(SIGNATURE_BYTES).unwrap_or(0))).await?;
            files::write_synced(&partial.join(MANIFEST_SIGNATURE), &signature, false).map_err(|error| format!("{MANIFEST_SIGNATURE}: {error}"))?;
        }
        for (index, entry) in manifest.files.iter().enumerate() {
            let target = partial.join(&entry.path);
            if files::read_bounded(&target, usize::try_from(entry.length).unwrap_or(0))
                .is_ok_and(|bytes| bytes.len() as u64 == entry.length && update::sha256(&bytes) == entry.sha256)
            {
                continue;
            }
            let _ = std::fs::remove_file(&target);
            let mut attempt: u32 = 0;
            let bytes = loop {
                attempt = attempt.saturating_add(1);
                match self.fetch(&repository, version, &entry.path, Some(entry.length)).await {
                    Ok(bytes) if update::sha256(&bytes) == entry.sha256 => break bytes,
                    Ok(_) if attempt < MAX_ATTEMPTS => {}
                    Err(why) if attempt >= MAX_ATTEMPTS => return Err(why),
                    Err(_) => {}
                    Ok(_) => return Err(format!("{} is not the file the manifest names", entry.path)),
                }
            };
            if let Some(folder) = target.parent() {
                std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
            }
            let executable = self.platform.file(&entry.path).is_some_and(|file| file.executable);
            if fault::armed("download-truncated") {
                // Section 13, "writes cut short": half the file, then the end.
                let _ = files::write_synced(&target, bytes.get(..bytes.len() / 2).unwrap_or_default(), executable);
                fault::point("download-truncated");
            }
            files::write_synced(&target, &bytes, executable).map_err(|error| format!("{}: {error}", entry.path))?;
            fault::point_at("download-file-written", index);
        }
        sync_tree(&partial)?;
        update::accept::complete(&partial, self.platform).map_err(|why| format!("the staged files: {why}"))?;
        fault::point("stage-before-rename");
        files::remove_tree(&staged).map_err(|error| format!("{}: {error}", staged.display()))?;
        std::fs::rename(&partial, &staged).map_err(|error| format!("{}: {error}", staged.display()))?;
        files::sync_folder(&self.staging).map_err(|error| format!("{}: {error}", self.staging.display()))?;
        fault::point("stage-renamed");
        Ok(())
    }
}

/// Every folder under `folder` flushed, so the files' names are on the disk.
fn sync_tree(folder: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(folder).map_err(|error| error.to_string())?.filter_map(Result::ok) {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            sync_tree(&entry.path())?;
        }
    }
    files::sync_folder(folder).map_err(|error| format!("{}: {error}", folder.display()))
}
