//! The Update Framework's repository (`installation-and-updates.md`
//! section 9): the root signed by its keys to threshold, a release's
//! targets added and the targets, snapshot and timestamp signed (section
//! 10, step 4), and the timestamp re-signed alone every day (section
//! 9.3). The repository is a folder, `metadata/` and `targets/`, that the
//! update host serves as it is.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use aws_lc_rs::rand::SystemRandom;
use tough::editor::RepositoryEditor;
use tough::editor::signed::{PathExists, SignedRole};
use tough::schema::decoded::{Decoded, Hex};
use tough::schema::{Hashes, KeyHolder, Metafile, RoleKeys, RoleType, Root, Signature, Signed, Target, Timestamp};
use tough::{ExpirationEnforcement, RepositoryLoader, TargetName};

use crate::keys::{self, Source};

/// The specification version the metadata says.
const SPEC_VERSION: &str = "1.0.0";
/// Seconds in a day.
const DAY: i64 = 24 * 60 * 60;
/// Section 9.1's expiries.
pub const ROOT_DAYS: i64 = 365;
/// Targets and snapshot: 3 months.
pub const TARGETS_DAYS: i64 = 91;
/// Timestamp: 7 days (the owner's answer to section 12, item 8).
pub const TIMESTAMP_DAYS: i64 = update::wire::TIMESTAMP_DAYS;
/// The threshold of root keys (requirement 13.9: two of three).
pub const ROOT_THRESHOLD: u64 = 2;

fn now() -> jiff::Timestamp {
    jiff::Timestamp::now()
}

fn days_from_now(days: i64) -> Result<jiff::Timestamp, String> {
    now().checked_add(jiff::SignedDuration::from_secs(days.saturating_mul(DAY))).map_err(|error| error.to_string())
}

/// A version number that only increases: the time, in seconds.
fn version_now() -> NonZeroU64 {
    NonZeroU64::new(u64::try_from(now().as_second()).unwrap_or(1)).unwrap_or(NonZeroU64::MIN)
}

fn key_id(public: &[u8; update::signature::PUBLIC_KEY_BYTES]) -> Result<Decoded<Hex>, String> {
    keys::tuf_key(public).key_id().map_err(|error| error.to_string())
}

/// The keys of each role.
#[derive(Debug)]
pub struct RootKeys {
    /// The root's keys.
    pub root: Vec<Source>,
    /// The targets key.
    pub targets: Source,
    /// The snapshot key.
    pub snapshot: Source,
    /// The timestamp key.
    pub timestamp: Source,
}

/// Writes a root of `version` naming `keys`, signed by the root keys that
/// can sign and by `old`, the previous root's, for a rotation (the
/// specification's section 5.3.4: a new root is signed by a threshold of
/// both the old and the new keys).
///
/// # Errors
/// When a key cannot be read or a signature made.
pub async fn root(out: &Path, keys: &RootKeys, version: u64, old: &[Source]) -> Result<(), String> {
    let mut all_keys = HashMap::new();
    let mut roles = HashMap::new();
    let mut role = |role: RoleType, sources: &[&Source], threshold: u64| -> Result<(), String> {
        let mut ids = Vec::new();
        for source in sources {
            let public = source.public()?;
            let id = key_id(&public)?;
            all_keys.insert(id.clone(), keys::tuf_key(&public));
            ids.push(id);
        }
        roles.insert(role, RoleKeys { keyids: ids, threshold: NonZeroU64::new(threshold).ok_or("a threshold of zero")?, _extra: HashMap::new() });
        Ok(())
    };
    role(RoleType::Root, &keys.root.iter().collect::<Vec<_>>(), ROOT_THRESHOLD)?;
    role(RoleType::Targets, &[&keys.targets], 1)?;
    role(RoleType::Snapshot, &[&keys.snapshot], 1)?;
    role(RoleType::Timestamp, &[&keys.timestamp], 1)?;
    let root = Root {
        spec_version: SPEC_VERSION.to_owned(),
        consistent_snapshot: true,
        version: NonZeroU64::new(version).ok_or("a version of zero")?,
        expires: days_from_now(ROOT_DAYS)?,
        keys: all_keys,
        roles,
        _extra: HashMap::new(),
    };
    let sources: Vec<_> = keys.root.iter().filter(|source| source.signs()).map(Source::key_source).collect();
    let mut signed = SignedRole::new(root.clone(), &KeyHolder::Root(root.clone()), &sources, &SystemRandom::new()).await.map_err(|error| error.to_string())?;
    if !old.is_empty() {
        let canonical = canonical(&root)?;
        let mut signatures = Vec::new();
        for source in old {
            signatures.push(Signature { keyid: key_id(&source.public()?)?, sig: source.sign(&canonical)?.into() });
        }
        signed = signed.add_old_signatures(signatures).map_err(|error| error.to_string())?;
    }
    std::fs::write(out, signed.buffer()).map_err(|error| format!("{}: {error}", out.display()))
}

/// A role's canonical JSON, the bytes its signatures cover, as `tough`
/// computes it.
fn canonical<T: tough::schema::Role>(role: &T) -> Result<Vec<u8>, String> {
    role.canonical_form().map_err(|error| error.to_string())
}

/// Loads the repository in `folder` from disk, expired or not: only to
/// read what it holds.
async fn load(folder: &Path, root: &[u8]) -> Result<tough::Repository, String> {
    let url = |part: &str| url_of(&folder.join(part));
    // The client's record of what it trusts, which this reading of the
    // repository needs and nothing keeps: outside the repository.
    let scratch = std::env::temp_dir().join(format!("neoteletext-release-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|error| format!("{}: {error}", scratch.display()))?;
    let repository = RepositoryLoader::new(&root.to_vec(), url("metadata")?, url("targets")?)
        .datastore(&scratch)
        .expiration_enforcement(ExpirationEnforcement::Unsafe)
        .load()
        .await
        .map_err(|error| error.to_string());
    let _ = std::fs::remove_dir_all(&scratch);
    repository
}

fn url_of(folder: &Path) -> Result<url::Url, String> {
    let absolute = std::fs::canonicalize(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    url::Url::from_directory_path(&absolute).map_err(|()| format!("{}: not a folder", absolute.display()))
}

/// Section 10, step 4: adds every file under `release` (each named by its
/// path under it, `<version>/<platform>/<file>.gz`) to the repository in
/// `folder`, signs the targets, snapshot and timestamp, and writes them;
/// the targets are copied into `targets/` first, then the metadata into
/// `metadata/`, so a host copied in that order never names a file it
/// lacks (step 5).
///
/// # Errors
/// When a file, a key or the repository cannot be read or written.
pub async fn publish(folder: &Path, root_path: &Path, release: &Path, keys: &RootKeys, timestamp_days: i64) -> Result<(), String> {
    let root = std::fs::read(root_path).map_err(|error| format!("{}: {error}", root_path.display()))?;
    let metadata = folder.join("metadata");
    let targets = folder.join("targets");
    std::fs::create_dir_all(&metadata).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&targets).map_err(|error| error.to_string())?;
    let mut editor = if metadata.join("timestamp.json").exists() {
        RepositoryEditor::from_repo(root_path, load(folder, &root).await?).await.map_err(|error| error.to_string())?
    } else {
        RepositoryEditor::new(root_path).await.map_err(|error| error.to_string())?
    };
    let version = version_now();
    editor
        .targets_version(version)
        .map_err(|error| error.to_string())?
        .targets_expires(days_from_now(TARGETS_DAYS)?)
        .map_err(|error| error.to_string())?
        .snapshot_version(version)
        .snapshot_expires(days_from_now(TARGETS_DAYS)?)
        .timestamp_version(version)
        .timestamp_expires(days_from_now(timestamp_days)?);
    let files = release_files(release)?;
    for (name, path) in &files {
        let target = Target::from_path(path).await.map_err(|error| error.to_string())?;
        editor.add_target(name.as_str(), target).map_err(|error| error.to_string())?;
    }
    let sources = [keys.targets.key_source(), keys.snapshot.key_source(), keys.timestamp.key_source()];
    let signed = editor.sign(&sources).await.map_err(|error| error.to_string())?;
    for (name, path) in &files {
        let name = TargetName::new(name.as_str()).map_err(|error| error.to_string())?;
        signed.copy_target(path, &targets, PathExists::Skip, Some(&name)).await.map_err(|error| error.to_string())?;
    }
    signed.write(&metadata).await.map_err(|error| error.to_string())
}

/// Every file under `release`, by its path there.
fn release_files(release: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let names = update::files::list_files(release, usize::MAX).map_err(|error| error.to_string())?;
    Ok(names.into_iter().map(|name| (name.clone(), release.join(name))).collect())
}

/// Section 9.3: a new timestamp, naming the current snapshot, signed by the
/// timestamp key alone; it says "nothing new" and can name no release.
///
/// # Errors
/// When the repository or the key cannot be read, or the timestamp written.
pub async fn timestamp(folder: &Path, root_path: &Path, key: &Source, days: i64) -> Result<(), String> {
    let root_bytes = std::fs::read(root_path).map_err(|error| format!("{}: {error}", root_path.display()))?;
    let repository = load(folder, &root_bytes).await?;
    let snapshot_version = repository.snapshot().signed.version;
    let metadata = folder.join("metadata");
    let snapshot_path = metadata.join(format!("{snapshot_version}.snapshot.json"));
    let snapshot = std::fs::read(&snapshot_path).map_err(|error| format!("{}: {error}", snapshot_path.display()))?;
    let mut stamp = Timestamp::new(SPEC_VERSION.to_owned(), version_now(), days_from_now(days)?);
    stamp.meta.insert(
        String::from("snapshot.json"),
        Metafile {
            length: Some(u64::try_from(snapshot.len()).unwrap_or(0)),
            hashes: Some(Hashes { sha256: update::sha256(&snapshot).to_vec().into(), _extra: HashMap::new() }),
            version: snapshot_version,
            _extra: HashMap::new(),
        },
    );
    let root: Signed<Root> = serde_json::from_slice(&root_bytes).map_err(|error| format!("{}: {error}", root_path.display()))?;
    let signed = SignedRole::new(stamp, &KeyHolder::Root(root.signed), &[key.key_source()], &SystemRandom::new()).await.map_err(|error| error.to_string())?;
    signed.write(&metadata, true).await.map_err(|error| error.to_string())
}
