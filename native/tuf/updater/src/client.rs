//! The Update Framework's client: the repository loaded and verified with
//! `tough`, and the newest version it names for this platform.

use std::path::PathBuf;

use tough::{IntoVec as _, Limits, Repository, RepositoryLoader, TargetName};
use update::files;
use update::manifest::MAX_MANIFEST;
use update::platform::{MANIFEST, Platform};
use update::version::Version;

use crate::{COMPRESSED, Https};

/// The updater's own folder in the staging folder, kept between runs: The
/// Update Framework's trusted metadata (its datastore, which holds the
/// newest root and the versions seen, so the host cannot roll them back)
/// and the time of the last successful check.
pub const METADATA: &str = update::layout::UPDATER_METADATA;
/// The file in it holding the last successful check's time; the launcher
/// keeps only the names `update::layout` lists there.
pub(super) const CHECKED: &str = update::layout::UPDATER_CHECKED;
/// The longest that file is: a time in decimal seconds.
const MAX_CHECKED: usize = 24;
/// The file in it holding the highest root version reached (decision 91).
const ROOT_VERSION: &str = update::layout::UPDATER_ROOT_VERSION;
/// The longest that file is: a root version, at most `u64::MAX`'s 20
/// decimal digits, and its line's end.
const MAX_ROOT_VERSION: usize = 21;
/// How many times a release file that fails its checks is fetched before
/// the download waits for the next check (section 3.1, step 2).
pub const MAX_ATTEMPTS: u32 = 3;
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
    pub(super) fn metadata(&self) -> PathBuf {
        self.staging.join(METADATA)
    }

    /// The last successful check's time, if one was made.
    #[must_use]
    pub fn last_success(&self) -> Option<i64> {
        let bytes = files::read_bounded(&self.metadata().join(CHECKED), MAX_CHECKED).ok()?;
        std::str::from_utf8(&bytes).ok()?.trim_end().parse().ok()
    }

    /// The highest root version reached, if one was recorded.
    fn root_floor(&self) -> Option<u64> {
        let bytes = files::read_bounded(&self.metadata().join(ROOT_VERSION), MAX_ROOT_VERSION).ok()?;
        std::str::from_utf8(&bytes).ok()?.strip_suffix('\n')?.parse().ok()
    }

    /// Loads and verifies the repository: The Update Framework's client
    /// workflow (its specification's section 5: root rotation, then the
    /// timestamp, snapshot and targets, each signed by its role's keys to
    /// threshold, unexpired, and never older than the versions kept). The
    /// chain of roots is verified from the built-in root every time, and a
    /// root older than the highest one reached before is refused: a host
    /// that stops serving the newer roots cannot take the updater back to
    /// keys a rotation retired (decision 91).
    pub(super) async fn load(&self) -> Result<Repository, String> {
        std::fs::create_dir_all(self.metadata()).map_err(|error| format!("{}: {error}", self.metadata().display()))?;
        let base = |folder: &str| url::Url::parse(&format!("https://{}:{}/{folder}/", self.host, self.port)).map_err(|error| error.to_string());
        let repository = RepositoryLoader::new(&self.root, base("metadata")?, base("targets")?)
            .transport(self.https.clone())
            .datastore(self.metadata())
            .limits(limits())
            .load()
            .await
            .map_err(|error| format!("the update repository is refused: {}", chain(&error)))?;
        let reached = repository.root().signed.version.get();
        match self.root_floor() {
            Some(floor) if reached < floor => {
                return Err(format!("the update repository is refused: the update host offered an older root (version {reached}, after {floor})"));
            }
            Some(floor) if reached == floor => {}
            _ => files::write_atomically(&self.metadata().join(ROOT_VERSION), format!("{reached}\n").as_bytes())
                .map_err(|error| format!("{ROOT_VERSION}: {error}"))?,
        }
        Ok(repository)
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
    pub(super) async fn fetch(&self, repository: &Repository, version: Version, path: &str, length: Option<u64>) -> Result<Vec<u8>, String> {
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
