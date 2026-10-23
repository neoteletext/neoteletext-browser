//! The updater (`installation-and-updates.md` sections 3 and 8): a helper
//! of the trusted core, confined to the update host and the staging
//! folder (requirement 13.18). It walks The Update Framework's client
//! workflow with `tough` (requirement 13.8), says which version is the
//! newest the repository names for this platform, and, when the user has
//! accepted one, downloads each of its files, checks it against the
//! targets metadata (by `tough`) and against the release's manifest (its
//! length and SHA-256 once decompressed), and stages the version whole:
//! the files are written and flushed in `staging/<version>.partial`, which
//! is then renamed to `staging/<version>`. It switches nothing: the
//! launcher accepts a staged version at the next start, by its own check.
//!
//! The repository's layout on the host: `/metadata/` holds the four roles'
//! metadata, `/targets/` the release files, each
//! `<version>/<platform>/<path>.gz`, the manifest among them.

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

pub mod transport;

use std::path::{Path, PathBuf};

use tough::{IntoVec as _, Limits, Repository, RepositoryLoader, TargetName};
use update::fault;
use update::files;
use update::manifest::{MAX_MANIFEST, Manifest};
use update::platform::{Family, MANIFEST, MANIFEST_SIGNATURE, Platform};
use update::signature::SIGNATURE_BYTES;
use update::version::Version;

pub use transport::Https;

/// The updater's own folder in the staging folder, kept between runs: The
/// Update Framework's trusted metadata (its datastore, which holds the
/// newest root and the versions seen, so the host cannot roll them back)
/// and the time of the last successful check.
pub const METADATA: &str = update::layout::UPDATER_METADATA;
/// The file in it holding the last successful check's time.
const CHECKED: &str = "checked";
/// The longest that file is: a time in decimal seconds.
const MAX_CHECKED: usize = 24;
/// How many times a release file that fails its checks is fetched before
/// the download waits for the next check (section 3.1, step 2).
pub const MAX_ATTEMPTS: u32 = 3;
/// The suffix of a version being staged.
pub const PARTIAL: &str = ".partial";
/// The release files' compression's suffix (section 6).
pub const COMPRESSED: &str = ".gz";
/// The limits on metadata: `tough`'s own, which bound each role's file
/// to 1 MiB (root, timestamp, snapshot) and 10 MiB (targets), and the
/// root rotations read at a time.
fn limits() -> Limits {
    Limits::default()
}

/// What the updater works with.
#[derive(Debug)]
pub struct Config {
    /// The update host's transport.
    pub https: Https,
    /// The host and port, for the repository's addresses.
    pub host: String,
    /// The port.
    pub port: u16,
    /// The first trusted root metadata, built in.
    pub root: Vec<u8>,
    /// The staging folder, the one place it writes.
    pub staging: PathBuf,
    /// This platform.
    pub platform: Platform,
}

/// The target name of a release file.
#[must_use]
pub fn target_name(version: Version, platform: Platform, path: &str) -> String {
    format!("{version}/{platform}/{path}{COMPRESSED}")
}

/// Seconds since 1970, now.
#[must_use]
pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|since| i64::try_from(since.as_secs()).ok()).unwrap_or(0)
}

impl Config {
    fn metadata(&self) -> PathBuf {
        self.staging.join(METADATA)
    }

    /// The last successful check's time, if one was made.
    #[must_use]
    pub fn last_success(&self) -> Option<i64> {
        let bytes = files::read_bounded(&self.metadata().join(CHECKED), MAX_CHECKED).ok()?;
        std::str::from_utf8(&bytes).ok()?.trim_end().parse().ok()
    }

    fn record_success(&self, at: i64) {
        let _ = files::write_atomically(&self.metadata().join(CHECKED), format!("{at}\n").as_bytes());
    }

    /// Loads and verifies the repository: The Update Framework's client
    /// workflow (its specification's section 5: root rotation, then the
    /// timestamp, snapshot and targets, each signed by its role's keys to
    /// threshold, unexpired, and never older than the versions kept).
    async fn load(&self) -> Result<Repository, String> {
        std::fs::create_dir_all(self.metadata()).map_err(|error| format!("{}: {error}", self.metadata().display()))?;
        let base = |folder: &str| url::Url::parse(&format!("https://{}:{}/{folder}/", self.host, self.port)).map_err(|error| error.to_string());
        RepositoryLoader::new(&self.root, base("metadata")?, base("targets")?)
            .transport(self.https.clone())
            .datastore(self.metadata())
            .limits(limits())
            .load()
            .await
            .map_err(|error| format!("the update repository is refused: {}", chain(&error)))
    }

    /// The newest version the repository names for this platform.
    ///
    /// # Errors
    /// When the repository cannot be loaded or verified.
    pub async fn check(&self) -> Result<Option<Version>, String> {
        let repository = self.load().await?;
        let latest = newest(&repository, self.platform);
        self.record_success(now());
        Ok(latest)
    }

    /// One release file, verified by The Update Framework (its length and
    /// hashes in the targets metadata) and decompressed to exactly
    /// `length` bytes, or, for the manifest, which says the others'
    /// lengths, to at most its bound.
    async fn fetch(&self, repository: &Repository, version: Version, path: &str, length: Option<u64>) -> Result<Vec<u8>, String> {
        let name = TargetName::new(target_name(version, self.platform, path)).map_err(|error| error.to_string())?;
        let stream = repository.read_target(&name).await.map_err(|error| chain(&error))?.ok_or_else(|| format!("the repository names no {}", name.raw()))?;
        let compressed = stream.into_vec().await.map_err(|error| chain(&error))?;
        match length {
            Some(length) => {
                let length = usize::try_from(length).map_err(|_| String::from("too long"))?;
                update::gzip::decompress(&compressed, length)
            }
            None => update::gzip::decompress_at_most(&compressed, MAX_MANIFEST),
        }
        .map_err(|refused| format!("{path}: {refused}"))
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
        if self.platform.family() == Family::Linux {
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

/// The newest version whose manifest the repository names for `platform`.
#[must_use]
pub fn newest(repository: &Repository, platform: Platform) -> Option<Version> {
    let suffix = format!("/{platform}/{MANIFEST}{COMPRESSED}");
    repository.targets().signed.targets.keys().filter_map(|name| name.raw().strip_suffix(&suffix).and_then(Version::parse)).max()
}

/// An error and its causes, in words.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
