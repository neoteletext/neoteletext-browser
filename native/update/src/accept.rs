//! The launcher's acceptance of a version (`installation-and-updates.md`
//! section 5): before a version is first started, its folder must hold
//! exactly the files its manifest names, each of the length and SHA-256
//! the manifest gives, and the platform's own check, by a signer pinned
//! in the launcher, must pass:
//!
//! - **macOS:** the bundle's code signature is valid and satisfies the
//!   pinned designated requirement (`codesign --verify --deep --strict`).
//! - **Windows:** the manifest's detached Ed25519 signature verifies
//!   against the pinned release key, checked first (decision 92); then
//!   every program's Authenticode signature is valid and its signer's
//!   public key is the pinned one (`os-install`).
//! - **Linux:** the manifest's and every file's detached Ed25519 signature
//!   verifies against the pinned release key.
//!
//! The version accepted is read from inside what was signed: on Linux the
//! signed manifest, elsewhere the mark in the broker's bytes, which on
//! Windows must also be the signed manifest's.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::files;
use crate::manifest::{MAX_FILE_BYTES, MAX_MANIFEST, Manifest};
use crate::mark;
use crate::platform::{BUNDLE, Family, MANIFEST, MANIFEST_SIGNATURE, MARKED_PROGRAM, MAX_FILES, Platform, SIGNATURE_SUFFIX};
use crate::signature::{self, PUBLIC_KEY_BYTES, SIGNATURE_BYTES};
use crate::version::Version;

/// The signers a launcher pins, built into it from the release's identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    /// The release key (section 4.3's Linux release key): it signs every
    /// Linux file and the Linux and Windows manifests (decision 92).
    pub release: Option<[u8; PUBLIC_KEY_BYTES]>,
    /// macOS: the designated requirement every bundle must satisfy, such as
    /// `anchor apple generic and certificate leaf[subject.OU] = "<team>"`.
    pub macos_requirement: Option<String>,
    /// Windows: SHA-256 of the signer certificate's public key.
    pub windows_signer: Option<[u8; crate::SHA256_BYTES]>,
}

/// The platform's own check of a version's folder, by its pinned signer.
pub trait Signer {
    /// The pinned release key, which signs the manifest on a platform whose
    /// manifest is signed (`Platform::signs_manifest`), if this build
    /// carries one.
    fn release_key(&self) -> Option<&[u8; PUBLIC_KEY_BYTES]>;

    /// Whether the platform's signature of every file in `folder`, which
    /// `manifest` describes, is the pinned signer's.
    ///
    /// # Errors
    /// Why not, in words.
    fn check(&self, folder: &Path, manifest: &Manifest) -> Result<(), String>;
}

/// The longest file read: the manifest's bound on any file.
fn max_file() -> usize {
    usize::try_from(MAX_FILE_BYTES).unwrap_or(usize::MAX)
}

/// The files a complete version's folder holds beyond its release files.
fn own_files(platform: Platform) -> Vec<String> {
    let mut own = vec![MANIFEST.to_owned()];
    if platform.signs_manifest() {
        own.push(MANIFEST_SIGNATURE.to_owned());
    }
    own
}

/// Whether `path` in `folder` carries the release key's detached signature
/// beside it, `<path>.sig` or, for the manifest, `manifest.sig`.
fn signed_by(folder: &Path, path: &str, signature_path: &str, key: &[u8; PUBLIC_KEY_BYTES]) -> Result<(), String> {
    let bytes = files::read_bounded(&folder.join(path), max_file()).map_err(|error| format!("{path}: {error}"))?;
    let signature = files::read_bounded(&folder.join(signature_path), SIGNATURE_BYTES).map_err(|error| format!("{signature_path}: {error}"))?;
    if signature::verify(key, &bytes, &signature) { Ok(()) } else { Err(format!("{path} is not signed by the release key")) }
}

/// The manifest in `folder`, if the folder holds exactly the files it
/// names for `platform`, each of its length and hash.
///
/// # Errors
/// What is missing, extra or different.
pub fn complete(folder: &Path, platform: Platform) -> Result<Manifest, String> {
    let manifest = Manifest::parse(&files::read_bounded(&folder.join(MANIFEST), MAX_MANIFEST).map_err(|error| format!("the manifest: {error}"))?)
        .map_err(|refused| format!("the manifest: {refused}"))?;
    if manifest.platform != platform {
        return Err(format!("a release for {}, not {platform}", manifest.platform));
    }
    let found = files::list_files(folder, MAX_FILES.saturating_add(2)).map_err(|error| error.to_string())?;
    let mut expected: Vec<String> = manifest.files.iter().map(|entry| entry.path.clone()).chain(own_files(platform)).collect();
    expected.sort();
    if found != expected {
        let extra: Vec<&String> = found.iter().filter(|path| !expected.contains(path)).collect();
        let missing: Vec<&String> = expected.iter().filter(|path| !found.contains(path)).collect();
        return Err(format!("the folder does not hold the release: missing {missing:?}, extra {extra:?}"));
    }
    for entry in &manifest.files {
        let bytes = files::read_bounded(&folder.join(&entry.path), max_file()).map_err(|error| format!("{}: {error}", entry.path))?;
        if u64::try_from(bytes.len()).ok() != Some(entry.length) || crate::sha256(&bytes) != entry.sha256 {
            return Err(format!("{} is not the file the release names", entry.path));
        }
    }
    Ok(manifest)
}

/// Copies the release in `from`, a folder another process wrote, to the
/// new folder `to`, as the launcher copies a version before it accepts it
/// (decision 87): the manifest, read strictly and for `platform`; the
/// manifest's signature where the platform has one; and each file the
/// manifest names, read to at most its length, each program made
/// executable by the platform's list, not by its mode in `from`. Nothing
/// else in `from` is read, so whatever else a staged folder holds, the copy
/// is no larger than the release (finding F-50). The copy is then for
/// [`accept`] to check.
///
/// # Errors
/// When the manifest is refused, a file cannot be read or is longer than
/// the manifest says, or the copy cannot be written.
pub fn copy_release(from: &Path, to: &Path, platform: Platform) -> Result<(), String> {
    files::refuse_link(from).map_err(|error| error.to_string())?;
    let manifest_bytes = files::read_regular(&from.join(MANIFEST), max_manifest()).map_err(|error| format!("the manifest: {error}"))?;
    let manifest = Manifest::parse(&manifest_bytes).map_err(|refused| format!("the manifest: {refused}"))?;
    if manifest.platform != platform {
        return Err(format!("a release for {}, not {platform}", manifest.platform));
    }
    let written = |path: &str, bytes: &[u8], executable: bool| -> Result<(), String> {
        let target = to.join(path);
        if let Some(folder) = target.parent() {
            std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        }
        files::write_synced(&target, bytes, executable).map_err(|error| format!("{}: {error}", target.display()))
    };
    std::fs::create_dir(to).map_err(|error| format!("{}: {error}", to.display()))?;
    written(MANIFEST, &manifest_bytes, false)?;
    if platform.signs_manifest() {
        let signature =
            files::read_regular(&from.join(MANIFEST_SIGNATURE), SIGNATURE_BYTES as u64).map_err(|error| format!("{MANIFEST_SIGNATURE}: {error}"))?;
        written(MANIFEST_SIGNATURE, &signature, false)?;
    }
    for entry in &manifest.files {
        let bytes = files::read_regular(&from.join(&entry.path), entry.length).map_err(|error| format!("{}: {error}", entry.path))?;
        written(&entry.path, &bytes, platform.file(&entry.path).is_some_and(|file| file.executable))?;
    }
    sync_folders(to)
}

/// Every folder under `folder`, and `folder`, flushed, so the copy's
/// names are on the disk before it is renamed into place.
fn sync_folders(folder: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(folder).map_err(|error| format!("{}: {error}", folder.display()))?.filter_map(Result::ok) {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            sync_folders(&entry.path())?;
        }
    }
    files::sync_folder(folder).map_err(|error| format!("{}: {error}", folder.display()))
}

/// The manifest's bound, as a file length.
fn max_manifest() -> u64 {
    u64::try_from(MAX_MANIFEST).unwrap_or(u64::MAX)
}

/// Accepts the version in `folder`, staged or being installed, for
/// `platform`: complete, signed by the pinned signer, and of the version
/// its folder's name says.
///
/// # Errors
/// Why it is refused.
pub fn accept(folder: &Path, platform: Platform, signer: &dyn Signer) -> Result<Version, String> {
    let manifest = complete(folder, platform)?;
    if platform.signs_manifest() {
        let key = signer.release_key().ok_or("this build carries no release key, so it accepts no version")?;
        signed_by(folder, MANIFEST, MANIFEST_SIGNATURE, key)?;
    }
    signer.check(folder, &manifest)?;
    let version = match platform.family() {
        Family::Linux => manifest.version,
        // On Windows the manifest is signed too, and the mark must say what
        // it says.
        Family::Macos | Family::Windows => {
            let program = folder.join(platform.program_path(MARKED_PROGRAM));
            let bytes = files::read_bounded(&program, max_file()).map_err(|error| format!("{MARKED_PROGRAM}: {error}"))?;
            let marked = mark::find(&bytes).map_err(|refused| format!("{MARKED_PROGRAM}: {refused}"))?;
            if marked != manifest.version {
                return Err(format!("the program says {marked}, the manifest {}", manifest.version));
            }
            marked
        }
    };
    let named = folder.file_name().and_then(|name| name.to_str()).and_then(Version::parse);
    if named.is_some_and(|named| named != version) {
        return Err(format!("the folder's name is not its version {version}"));
    }
    Ok(version)
}

/// Linux's check: the release key's detached signatures of every file
/// (section 4.3); [`accept`] checks the manifest's first, with the same
/// key.
#[derive(Debug)]
pub struct LinuxSigner {
    /// The pinned release key.
    pub key: Option<[u8; PUBLIC_KEY_BYTES]>,
}

impl Signer for LinuxSigner {
    fn release_key(&self) -> Option<&[u8; PUBLIC_KEY_BYTES]> {
        self.key.as_ref()
    }

    fn check(&self, folder: &Path, manifest: &Manifest) -> Result<(), String> {
        let key = self.key.as_ref().ok_or("this build carries no release key, so it accepts no version")?;
        for entry in manifest.files.iter().filter(|entry| !entry.path.ends_with(SIGNATURE_SUFFIX)) {
            signed_by(folder, &entry.path, &format!("{}{SIGNATURE_SUFFIX}", entry.path), key)?;
        }
        Ok(())
    }
}

/// macOS's check: the bundle's code signature, valid, strict, and
/// satisfying the pinned requirement, as Apple's own `codesign` verifies
/// it (section 5).
#[derive(Debug)]
pub struct MacosSigner {
    /// The pinned designated requirement.
    pub requirement: Option<String>,
}

/// Apple's code-signing tool, part of every macOS.
const CODESIGN: &str = "/usr/bin/codesign";

impl Signer for MacosSigner {
    fn release_key(&self) -> Option<&[u8; PUBLIC_KEY_BYTES]> {
        None
    }

    fn check(&self, folder: &Path, _manifest: &Manifest) -> Result<(), String> {
        let requirement = self.requirement.as_deref().ok_or("this build carries no signer requirement, so it accepts no version")?;
        let bundle = folder.join(BUNDLE);
        let output = Command::new(CODESIGN)
            .args(["--verify", "--deep", "--strict"])
            .arg(format!("-R={requirement}"))
            .arg(&bundle)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("{CODESIGN}: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            let said = String::from_utf8_lossy(&output.stderr);
            Err(format!("the bundle's signature is not the pinned signer's: {}", said.trim()))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
pub(crate) mod tests {
    use super::*;
    use crate::manifest::Entry;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};
    use std::fs;
    use std::path::PathBuf;

    /// A complete Linux version of `version` in `folder`, signed by `pair`.
    pub(crate) fn linux_version(folder: &Path, version: Version, pair: &Ed25519KeyPair) {
        let platform = Platform::LinuxX86_64;
        fs::create_dir_all(folder).unwrap();
        let mut entries = Vec::new();
        for file in platform.files().into_iter().filter(|file| !file.path.ends_with(SIGNATURE_SUFFIX)) {
            let bytes = format!("{} of {version}", file.path).into_bytes();
            files::write_synced(&folder.join(&file.path), &bytes, file.executable).unwrap();
            let signature = pair.sign(&bytes);
            let signature_path = format!("{}{SIGNATURE_SUFFIX}", file.path);
            fs::write(folder.join(&signature_path), signature.as_ref()).unwrap();
            entries.push(Entry { path: file.path, length: u64::try_from(bytes.len()).unwrap(), sha256: crate::sha256(&bytes) });
            entries.push(Entry { path: signature_path, length: u64::try_from(SIGNATURE_BYTES).unwrap(), sha256: crate::sha256(signature.as_ref()) });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let manifest = Manifest { version, platform, files: entries }.to_text();
        fs::write(folder.join(MANIFEST), &manifest).unwrap();
        fs::write(folder.join(MANIFEST_SIGNATURE), pair.sign(manifest.as_bytes()).as_ref()).unwrap();
    }

    /// The test's Windows signer, its release key `pair`'s.
    fn windows_signer(pair: &Ed25519KeyPair) -> Authenticode {
        Authenticode { release: pair.public_key().as_ref().try_into().unwrap() }
    }

    pub(crate) fn key_pair() -> Ed25519KeyPair {
        Ed25519KeyPair::from_pkcs8(Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap().as_ref()).unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("neoteletext-accept-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn a_complete_signed_version_is_accepted_and_every_fault_refused() {
        let root = scratch("linux");
        let pair = key_pair();
        let key: [u8; PUBLIC_KEY_BYTES] = pair.public_key().as_ref().try_into().unwrap();
        let signer = LinuxSigner { key: Some(key) };
        let version = Version::new(0, 3, 0);
        let good = root.join("0.3.0");
        linux_version(&good, version, &pair);
        assert_eq!(accept(&good, Platform::LinuxX86_64, &signer), Ok(version));
        assert!(accept(&good, Platform::LinuxX86_64, &LinuxSigner { key: None }).is_err(), "no key built in");
        let other: [u8; PUBLIC_KEY_BYTES] = key_pair().public_key().as_ref().try_into().unwrap();
        assert!(accept(&good, Platform::LinuxX86_64, &LinuxSigner { key: Some(other) }).is_err(), "the wrong signer");

        let fault = |name: &str, change: &dyn Fn(&Path)| {
            let folder = root.join(name).join("0.3.0");
            linux_version(&folder, version, &pair);
            change(&folder);
            let refused = accept(&folder, Platform::LinuxX86_64, &signer);
            assert!(refused.is_err(), "{name}: {refused:?}");
        };
        fault("truncated", &|folder| fs::write(folder.join("neoteletext"), b"neo").unwrap());
        fault("corrupt", &|folder| {
            let mut bytes = fs::read(folder.join("neoteletext-decoder")).unwrap();
            bytes[0] ^= 1;
            fs::write(folder.join("neoteletext-decoder"), bytes).unwrap();
        });
        fault("missing", &|folder| fs::remove_file(folder.join("neoteletext-sound")).unwrap());
        fault("extra", &|folder| fs::write(folder.join("evil"), b"#!/bin/sh").unwrap());
        fault("unsigned manifest", &|folder| fs::remove_file(folder.join(MANIFEST_SIGNATURE)).unwrap());
        fault("a file signed by another key", &|folder| {
            let bytes = fs::read(folder.join("neoteletext")).unwrap();
            let other = key_pair().sign(&bytes);
            fs::write(folder.join("neoteletext.sig"), other.as_ref()).unwrap();
        });
        fault("the manifest changed after signing", &|folder| {
            let text = fs::read_to_string(folder.join(MANIFEST)).unwrap().replace("version 0.3.0", "version 0.3.1");
            fs::write(folder.join(MANIFEST), text).unwrap();
        });
        let misnamed = root.join("misnamed").join("0.4.0");
        linux_version(&misnamed, version, &pair);
        assert!(accept(&misnamed, Platform::LinuxX86_64, &signer).unwrap_err().contains("name"));
        assert!(accept(&good, Platform::WindowsX86_64, &signer).is_err(), "another platform's release");
        fs::remove_dir_all(&root).unwrap();
    }

    /// Windows's programs, each "signed" for the release it came from, and
    /// the release key.
    struct Authenticode {
        release: [u8; PUBLIC_KEY_BYTES],
    }

    impl Signer for Authenticode {
        fn release_key(&self) -> Option<&[u8; PUBLIC_KEY_BYTES]> {
            Some(&self.release)
        }

        fn check(&self, _folder: &Path, _manifest: &Manifest) -> Result<(), String> {
            Ok(())
        }
    }

    /// A Windows version in `folder` whose manifest says `version`, its
    /// broker marked `version` and every other program taken from
    /// `programs_of`; its manifest signed by `pair` when one is given.
    fn windows_version(folder: &Path, version: Version, programs_of: Version, pair: Option<&Ed25519KeyPair>) {
        let platform = Platform::WindowsX86_64;
        fs::create_dir_all(folder).unwrap();
        let mut entries = Vec::new();
        for file in platform.files() {
            let bytes = if file.path == platform.program_path(MARKED_PROGRAM) {
                format!("the broker {}", crate::version_mark!("VERSION").replace("VERSION", &version.to_string())).into_bytes()
            } else {
                format!("{} of {programs_of}", file.path).into_bytes()
            };
            fs::write(folder.join(&file.path), &bytes).unwrap();
            entries.push(Entry { path: file.path, length: u64::try_from(bytes.len()).unwrap(), sha256: crate::sha256(&bytes) });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let manifest = Manifest { version, platform, files: entries }.to_text();
        fs::write(folder.join(MANIFEST), &manifest).unwrap();
        if let Some(pair) = pair {
            fs::write(folder.join(MANIFEST_SIGNATURE), pair.sign(manifest.as_bytes()).as_ref()).unwrap();
        }
    }

    /// Decision 92 (finding F-07): the Windows manifest is signed by the
    /// release key and checked before Authenticode, so programs each
    /// genuinely signed, but for older releases, cannot be put together
    /// under a new version's number.
    #[test]
    fn a_windows_manifest_must_be_signed_by_the_release_key() {
        let root = scratch("windows");
        let pair = key_pair();
        let signer = windows_signer(&pair);
        let (old, older, new) = (Version::new(0, 2, 0), Version::new(0, 3, 0), Version::new(0, 4, 0));
        let unsigned = root.join("unsigned").join("0.4.0");
        windows_version(&unsigned, new, old, None);
        assert!(accept(&unsigned, Platform::WindowsX86_64, &signer).is_err(), "no manifest.sig");
        let mixed = root.join("mixed").join("0.4.0");
        windows_version(&mixed, new, older, Some(&key_pair()));
        assert!(accept(&mixed, Platform::WindowsX86_64, &signer).is_err(), "signed by another key");
        let genuine = root.join("genuine").join("0.4.0");
        windows_version(&genuine, new, new, Some(&pair));
        assert_eq!(accept(&genuine, Platform::WindowsX86_64, &signer), Ok(new));
        fs::remove_dir_all(&root).unwrap();
    }

    /// Finding F-50: a release is copied by its manifest alone, each file to
    /// at most its length, so what else a staged folder holds is never read.
    #[test]
    fn a_release_is_copied_by_its_manifest_alone() {
        /// Far larger than any release file, and sparse, so it costs nothing.
        const HUGE: u64 = 100 * 1024 * 1024;
        let root = scratch("copy");
        let pair = key_pair();
        let signer = LinuxSigner { key: Some(pair.public_key().as_ref().try_into().unwrap()) };
        let version = Version::new(0, 3, 0);
        let staged = root.join("staged").join("0.3.0");
        linux_version(&staged, version, &pair);
        fs::File::create(staged.join("extra")).unwrap().set_len(HUGE).unwrap();
        fs::create_dir_all(staged.join("more")).unwrap();
        fs::write(staged.join("more/extra"), b"not the release's").unwrap();
        let copy = root.join("copy").join("0.3.0");
        fs::create_dir_all(copy.parent().unwrap()).unwrap();
        copy_release(&staged, &copy, Platform::LinuxX86_64).unwrap();
        assert!(!copy.join("extra").exists() && !copy.join("more").exists(), "nothing but the release is copied");
        assert_eq!(accept(&copy, Platform::LinuxX86_64, &signer), Ok(version), "the copy is the release");
        assert!(files::is_executable(&fs::metadata(copy.join("neoteletext")).unwrap()) || cfg!(not(unix)), "a program by the platform's list");

        let refused = |name: &str, change: &dyn Fn(&Path)| {
            let staged = root.join(name).join("0.3.0");
            linux_version(&staged, version, &pair);
            change(&staged);
            let copy = root.join(name).join("copy");
            let result = copy_release(&staged, &copy, Platform::LinuxX86_64);
            assert!(result.is_err(), "{name}: {result:?}");
            result.unwrap_err()
        };
        let longer = refused("longer", &|folder| {
            fs::OpenOptions::new().write(true).open(folder.join("neoteletext-decoder")).unwrap().set_len(HUGE).unwrap();
        });
        assert!(longer.contains("longer"), "{longer}");
        refused("another platform", &|folder| {
            let text = fs::read_to_string(folder.join(MANIFEST)).unwrap().replace("linux-x86_64", "macos-arm64");
            fs::write(folder.join(MANIFEST), text).unwrap();
        });
        #[cfg(unix)]
        {
            let link = refused("link", &|folder| {
                let file = folder.join("neoteletext-decoder");
                fs::rename(&file, folder.join("decoder-elsewhere")).unwrap();
                std::os::unix::fs::symlink(folder.join("decoder-elsewhere"), &file).unwrap();
            });
            assert!(link.contains("neoteletext-decoder"), "{link}");
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
