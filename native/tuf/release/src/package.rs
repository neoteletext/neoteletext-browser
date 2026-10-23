//! A release's files (`installation-and-updates.md` sections 4, 6 and 10):
//! the version's folder laid out for its platform, its manifest, Linux's
//! detached signatures by the release key, the check the launcher will
//! make, and the compressed targets The Update Framework signs.

use std::path::Path;

use update::files;
use update::manifest::{Entry, Manifest};
use update::platform::{APPARMOR_PROFILE, BUNDLE, Family, MANIFEST, MANIFEST_SIGNATURE, MAX_FILES, PROGRAMS, Platform, SIGNATURE_SUFFIX};
use update::version::Version;

use crate::keys::Source;

/// The bundle's identifier.
const BUNDLE_IDENTIFIER: &str = "com.neoteletext.browser";
/// The oldest macOS a release runs on (requirement 13.16; the binaries'
/// own minimum, measured with `otool -l`, section 2).
const MACOS_MINIMUM: &str = "11.0";

/// The bundle's `Info.plist` (Apple's Information Property List Key
/// Reference): its main executable is the launcher (section 4.1).
fn info_plist(version: Version) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>NeoTeletext</string>
	<key>CFBundleExecutable</key>
	<string>neoteletext-launcher</string>
	<key>CFBundleIdentifier</key>
	<string>{BUNDLE_IDENTIFIER}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>NeoTeletext</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.news</string>
	<key>LSMinimumSystemVersion</key>
	<string>{MACOS_MINIMUM}</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
"#
    )
}

/// The `AppArmor` profile every Linux version carries (`sandboxing.md`
/// section 8.3; `installation-and-updates.md` section 14): on a system
/// that restricts unprivileged user namespaces, it lets the browser's own
/// programs, where a per-user installation keeps them, make the empty
/// network namespace their sandbox needs, and changes nothing else.
pub const APPARMOR: &str = "# NeoTeletext: its programs may make a user namespace, for the empty
# network namespace of their sandbox (docs/sandboxing.md section 8).
# Installed by an administrator once; it covers every later version.
abi <abi/4.0>,
include <tunables/global>

profile neoteletext @{HOME}/.local/share/neoteletext/versions/*/neoteletext{,-*} flags=(unconfined) {
  userns,

  include if exists <local/neoteletext>
}
";

/// Lays out version `version` for `platform` in `out`, a new folder, from
/// the built programs in `programs`.
///
/// # Errors
/// When a program is missing or a file cannot be written.
pub fn layout(platform: Platform, version: Version, programs: &Path, out: &Path) -> Result<(), String> {
    if out.exists() {
        return Err(format!("{} exists", out.display()));
    }
    for program in PROGRAMS {
        let from = programs.join(platform.program_file(program));
        let bytes = std::fs::read(&from).map_err(|error| format!("{}: {error}", from.display()))?;
        let to = out.join(platform.program_path(program));
        if let Some(folder) = to.parent() {
            std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        }
        files::write_synced(&to, &bytes, true).map_err(|error| format!("{}: {error}", to.display()))?;
    }
    match platform.family() {
        Family::Macos => {
            let plist = out.join(BUNDLE).join("Contents/Info.plist");
            files::write_synced(&plist, info_plist(version).as_bytes(), false).map_err(|error| error.to_string())?;
        }
        Family::Linux => files::write_synced(&out.join(APPARMOR_PROFILE), APPARMOR.as_bytes(), false).map_err(|error| error.to_string())?,
        Family::Windows => {}
    }
    Ok(())
}

/// Writes the manifest of the version in `folder`: every release file
/// present, with its length and hash.
///
/// # Errors
/// When the folder holds a file no release has, or lacks a required one.
pub fn manifest(folder: &Path, platform: Platform, version: Version) -> Result<Manifest, String> {
    let found = files::list_files(folder, MAX_FILES.saturating_add(2)).map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for path in found.iter().filter(|path| *path != MANIFEST && *path != MANIFEST_SIGNATURE) {
        if platform.file(path).is_none() {
            return Err(format!("{path} is not a file this platform's releases have"));
        }
        let bytes = std::fs::read(folder.join(path)).map_err(|error| error.to_string())?;
        entries.push(Entry { path: path.clone(), length: u64::try_from(bytes.len()).unwrap_or(u64::MAX), sha256: update::sha256(&bytes) });
    }
    let manifest = Manifest { version, platform, files: entries };
    let text = manifest.to_text();
    Manifest::parse(text.as_bytes()).map_err(|refused| format!("the manifest would be refused: {refused}"))?;
    files::write_synced(&folder.join(MANIFEST), text.as_bytes(), false).map_err(|error| error.to_string())?;
    Ok(manifest)
}

/// Section 10, step 2 on Linux: the release key signs every file, then the
/// manifest, which lists the signatures too.
///
/// # Errors
/// When a file cannot be read or the key cannot sign.
pub fn sign_linux(folder: &Path, version: Version, key: &Source) -> Result<(), String> {
    let platform = Platform::LinuxX86_64;
    for file in platform.files().into_iter().filter(|file| !file.path.ends_with(SIGNATURE_SUFFIX)) {
        let path = folder.join(&file.path);
        let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let signature = key.sign(&bytes)?;
        files::write_synced(&folder.join(format!("{}{SIGNATURE_SUFFIX}", file.path)), &signature, false).map_err(|error| error.to_string())?;
    }
    let _ = std::fs::remove_file(folder.join(MANIFEST_SIGNATURE));
    manifest(folder, platform, version)?;
    let text = std::fs::read(folder.join(MANIFEST)).map_err(|error| error.to_string())?;
    files::write_synced(&folder.join(MANIFEST_SIGNATURE), &key.sign(&text)?, false).map_err(|error| error.to_string())
}

/// A file's detached signature beside it, as `<file>.sig`: the Linux
/// tarball's, which the download page says how to check with OpenSSL.
///
/// # Errors
/// When the file cannot be read or the key cannot sign.
pub fn sign_file(path: &Path, key: &Source) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let signature = key.sign(&bytes)?;
    std::fs::write(files::with_suffix(path, SIGNATURE_SUFFIX), signature).map_err(|error| error.to_string())
}

/// Section 10, step 3: the launcher's own acceptance check on the signed
/// version, with the signers this build pins, and the check that the
/// launcher is unchanged against the last launcher release's unless this
/// release is one (section 7).
///
/// # Errors
/// Why the launcher would refuse it.
pub fn check(folder: &Path, platform: Platform, last_launcher: Option<&Path>, launcher_release: bool) -> Result<Version, String> {
    if let Some(last) = last_launcher {
        let ours = std::fs::read(folder.join(platform.program_path(update::platform::LAUNCHER_PROGRAM))).map_err(|error| error.to_string())?;
        let theirs = std::fs::read(last).map_err(|error| format!("{}: {error}", last.display()))?;
        if ours != theirs && !launcher_release {
            return Err(String::from("the launcher differs from the last launcher release's; a launcher release says so (--launcher-release)"));
        }
    }
    let keys = release_identity::keys();
    let signer: Box<dyn update::accept::Signer> = match platform.family() {
        Family::Linux => Box::new(update::accept::LinuxSigner { key: keys.linux_release }),
        Family::Macos => Box::new(update::accept::MacosSigner { requirement: keys.macos_requirement }),
        #[cfg(windows)]
        Family::Windows => Box::new(os_install::WindowsSigner { key: keys.windows_signer }),
        #[cfg(not(windows))]
        Family::Windows => return Err(String::from("a Windows release is checked on Windows")),
    };
    update::accept::accept(folder, platform, signer.as_ref())
}

/// Section 6: each file of the version, the manifest and its signature
/// among them, compressed into `out/<version>/<platform>/<path>.gz`, the
/// targets The Update Framework signs.
///
/// # Errors
/// When a file cannot be read or written.
pub fn targets(folder: &Path, platform: Platform, out: &Path) -> Result<(), String> {
    let manifest = update::accept::complete(folder, platform)?;
    let base = out.join(manifest.version.to_string()).join(platform.name());
    for path in files::list_files(folder, MAX_FILES.saturating_add(2)).map_err(|error| error.to_string())? {
        let bytes = std::fs::read(folder.join(&path)).map_err(|error| error.to_string())?;
        let target = base.join(format!("{path}.gz"));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&target, update::gzip::compress(&bytes)).map_err(|error| format!("{}: {error}", target.display()))?;
    }
    Ok(())
}
