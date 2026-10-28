//! A release's files (`installation-and-updates.md` sections 4, 6 and 10):
//! the version's folder laid out for its platform, its manifest, Linux's
//! detached signatures by the release key, the check the launcher will
//! make, and the compressed targets The Update Framework signs.

use std::path::Path;

use update::files;
use update::manifest::{Entry, Manifest};
use update::platform::{
    APPARMOR_PROFILE, BUNDLE, Family, LINUX_ICON_SIZES, MACOS_ICON, MANIFEST, MANIFEST_SIGNATURE, MAX_FILES, PROGRAMS, Platform, SIGNATURE_SUFFIX,
};
use update::version::Version;

use crate::keys::Source;

/// The bundle's identifier.
const BUNDLE_IDENTIFIER: &str = "com.neoteletext.browser";
/// The oldest macOS a release runs on (requirement 13.16; the binaries'
/// own minimum, measured with `otool -l`, section 2).
const MACOS_MINIMUM: &str = "11.0";

/// Apple's tool that makes an icon file from an iconset and an iconset
/// from an icon file (`iconutil(1)`), at its fixed place, never looked up
/// on the path.
const ICONUTIL: &str = "/usr/bin/iconutil";

/// The bundle's `Info.plist` (Apple's Information Property List Key
/// Reference): its main executable is the launcher (section 4.1), its icon
/// the art's icon file (decision 124).
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
	<key>CFBundleIconFile</key>
	<string>{MACOS_ICON}</string>
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
/// section 8.3; `installation-and-updates.md` section 14), for the default
/// installation's root: the four programs that make namespaces, there
/// alone (decision 95). `neoteletext --install` writes and prints one for
/// the root it installs in.
fn apparmor() -> String {
    update::platform::apparmor_profile(update::platform::APPARMOR_DEFAULT_ROOT)
}

/// Runs `iconutil` with `arguments`.
fn iconutil(arguments: &[&std::ffi::OsStr]) -> Result<(), String> {
    let output = std::process::Command::new(ICONUTIL).args(arguments).output().map_err(|error| format!("{ICONUTIL}: {error}"))?;
    if output.status.success() { Ok(()) } else { Err(format!("{ICONUTIL} failed: {}", String::from_utf8_lossy(&output.stderr).trim())) }
}

/// The macOS bundle's icon file (decision 124): `iconutil` makes it from
/// an iconset of the art at every size, in `work`, a new folder; it is
/// then read back into an iconset and checked to be the art at every size,
/// pixel for pixel, before its bytes are returned.
///
/// # Errors
/// When `work` exists, `iconutil` fails, or what it made is not the art.
fn macos_icon(work: &Path) -> Result<Vec<u8>, String> {
    std::fs::create_dir(work).map_err(|error| format!("{}: {error}", work.display()))?;
    let made = macos_icon_in(work);
    let _ = std::fs::remove_dir_all(work);
    made
}

/// [`macos_icon`]'s work, in the folder `work`.
fn macos_icon_in(work: &Path) -> Result<Vec<u8>, String> {
    let iconset = work.join("NeoTeletext.iconset");
    icon::macos::write_iconset(&iconset)?;
    let file = work.join(MACOS_ICON);
    iconutil(&["-c".as_ref(), "icns".as_ref(), "-o".as_ref(), file.as_os_str(), iconset.as_os_str()])?;
    let back = work.join("check.iconset");
    iconutil(&["-c".as_ref(), "iconset".as_ref(), "-o".as_ref(), back.as_os_str(), file.as_os_str()])?;
    icon::macos::check_iconset(&back)?;
    std::fs::read(&file).map_err(|error| format!("{}: {error}", file.display()))
}

/// Every Linux icon file (decision 124), by its path in a version: the
/// art at each of [`LINUX_ICON_SIZES`], by nearest neighbour.
///
/// # Errors
/// When the art cannot be read.
fn linux_icons() -> Result<Vec<(String, Vec<u8>)>, String> {
    let art = icon::source().map_err(|refused| refused.to_string())?;
    LINUX_ICON_SIZES
        .iter()
        .map(|side| {
            let png = art.scaled(*side).and_then(|image| icon::png::encode(&image)).map_err(|refused| refused.to_string())?;
            Ok((update::platform::linux_icon(*side), png))
        })
        .collect()
}

/// Lays out version `version` for `platform` in `out`, a new folder, from
/// the built programs in `programs`; on macOS the icon file is made in
/// `<out>.icon`, a new folder removed afterwards.
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
            let icon = out.join(BUNDLE).join("Contents/Resources").join(MACOS_ICON);
            if let Some(folder) = icon.parent() {
                std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
            }
            let bytes = macos_icon(Path::new(&format!("{}.icon", out.display())))?;
            files::write_synced(&icon, &bytes, false).map_err(|error| format!("{}: {error}", icon.display()))?;
        }
        Family::Linux => {
            files::write_synced(&out.join(APPARMOR_PROFILE), apparmor().as_bytes(), false).map_err(|error| error.to_string())?;
            for (path, bytes) in linux_icons()? {
                files::write_synced(&out.join(&path), &bytes, false).map_err(|error| format!("{path}: {error}"))?;
            }
        }
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
    refuse_file_key_for_production(folder, platform, key)?;
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

/// Release step 2 on Windows, after `signtool` has signed every program:
/// the manifest written, listing the signed programs, and signed by the
/// release key (`manifest.sig`, decision 92), as Linux's is.
///
/// # Errors
/// When the folder is not a Windows version, or the key cannot sign.
pub fn sign_windows(folder: &Path, version: Version, key: &Source) -> Result<(), String> {
    let platform = Platform::WindowsX86_64;
    refuse_file_key_for_production(folder, platform, key)?;
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

/// The identity the release's launcher in `folder` was built with, by its
/// identity mark (finding F-24).
///
/// # Errors
/// When the launcher cannot be read or carries no one identity mark.
pub fn launcher_identity(folder: &Path, platform: Platform) -> Result<&'static str, String> {
    let launcher = folder.join(platform.program_path(update::platform::LAUNCHER_PROGRAM));
    let bytes = files::read_bounded(&launcher, usize::try_from(update::manifest::MAX_FILE_BYTES).unwrap_or(usize::MAX))
        .map_err(|error| format!("{}: {error}", launcher.display()))?;
    update::mark::find_identity(&bytes).map_err(|refused| format!("{}: {refused}", launcher.display()))
}

/// A production release's keys are hardware keys (`command:`): a key in a
/// file, a test's, is refused for one (finding F-24).
///
/// # Errors
/// When `folder`'s launcher was built with the production identity and
/// `key` is a file.
pub fn refuse_file_key_for_production(folder: &Path, platform: Platform, key: &Source) -> Result<(), String> {
    if matches!(key, Source::File(_)) && launcher_identity(folder, platform).is_ok_and(|identity| identity == update::mark::PRODUCTION) {
        return Err(String::from("a production release is signed with its hardware key (command:), never a key in a file"));
    }
    Ok(())
}

/// Section 10, step 3: the launcher's own acceptance check on the signed
/// version, with the signers this build pins, and the check that the
/// launcher is unchanged against the last launcher release's unless this
/// release is one (section 7).
///
/// # Errors
/// Why the launcher would refuse it.
pub fn check(folder: &Path, platform: Platform, last_launcher: Option<&Path>, launcher_release: bool, test_identity: bool) -> Result<Version, String> {
    if let Some(last) = last_launcher {
        let ours = std::fs::read(folder.join(platform.program_path(update::platform::LAUNCHER_PROGRAM))).map_err(|error| error.to_string())?;
        let theirs = std::fs::read(last).map_err(|error| format!("{}: {error}", last.display()))?;
        if ours != theirs && !launcher_release {
            return Err(String::from("the launcher differs from the last launcher release's; a launcher release says so (--launcher-release)"));
        }
    }
    let identity = launcher_identity(folder, platform)?;
    if identity != update::mark::PRODUCTION && !test_identity {
        return Err(String::from(
            "the release's launcher was built with a test identity, which trusts test keys; build it without NEOTELETEXT_RELEASE_IDENTITY, or say --test-identity for a test",
        ));
    }
    let keys = release_identity::keys();
    let signer: Box<dyn update::accept::Signer> = match platform.family() {
        Family::Linux => Box::new(update::accept::LinuxSigner { key: keys.release }),
        Family::Macos => Box::new(update::accept::MacosSigner { requirement: keys.macos_requirement }),
        #[cfg(windows)]
        Family::Windows => Box::new(os_install::WindowsSigner { key: keys.windows_signer, release: keys.release }),
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

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// Decision 95 (finding F-23): the profile every Linux release carries
    /// lets only the four programs that make namespaces make them, at the
    /// default installation's path, and says so.
    #[test]
    fn the_release_profile_names_only_the_programs_that_make_namespaces() {
        let profile = apparmor();
        assert!(
            profile.contains("\"@{HOME}/.local/share/neoteletext/versions/*/{neoteletext,neoteletext-decoder,neoteletext-interface,neoteletext-sound}\""),
            "{profile}"
        );
        assert!(!profile.contains("neoteletext{,-*}"), "no pattern for every program: {profile}");
        assert_eq!(profile.matches("userns").count(), 2, "one rule, and the sentence saying what it allows: {profile}");
    }

    /// Decision 124: the bundle names its icon file, which the release
    /// lists.
    #[test]
    fn the_bundle_names_its_icon() {
        let plist = info_plist(Version::new(0, 1, 0));
        assert!(plist.contains("\t<key>CFBundleIconFile</key>\n\t<string>NeoTeletext.icns</string>\n"), "{plist}");
        assert!(Platform::MacosArm64.file(&format!("{BUNDLE}/Contents/Resources/{MACOS_ICON}")).is_some_and(|file| file.required));
    }

    /// Decision 124: Linux's icon files are the art at each size, by
    /// nearest neighbour, each a file every Linux version must have.
    #[test]
    fn the_linux_icons_are_the_art_at_each_size() {
        let art = icon::source().unwrap();
        let icons = linux_icons().unwrap();
        assert_eq!(icons.len(), LINUX_ICON_SIZES.len());
        for ((path, png), side) in icons.iter().zip(LINUX_ICON_SIZES) {
            assert_eq!(*path, format!("neoteletext-{side}.png"));
            assert_eq!(icon::png::decode(png).unwrap(), art.scaled(side).unwrap(), "{path}");
            assert!(Platform::LinuxX86_64.file(path).is_some_and(|file| file.required && !file.executable), "{path}");
        }
    }
}
