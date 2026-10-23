//! Linux's installation from its signed tarball (`installation-and-updates.md`
//! section 4.3): the version checked and copied into the user's data
//! folder, a menu entry and a command on the path.

use std::path::{Path, PathBuf};

use update::files;
use update::layout::Layout;
use update::platform::{Family, LAUNCHER_PROGRAM};
use update::version::Version;

use crate::io_text;
use crate::launcher_release::LINUX_LAUNCHER;
use crate::trial::COPYING_SUFFIX;

/// The user's data folder for applications' entries (`XDG_DATA_HOME`).
fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute()).map(|home| home.join(".local/share")))
}

/// The menu entry's file (the freedesktop.org Desktop Entry
/// Specification, its "Installing" section).
pub(super) fn desktop_entry() -> Option<PathBuf> {
    data_home().map(|data| data.join("applications/neoteletext.desktop"))
}

/// The command on the path (`~/.local/bin`, the systemd file-hierarchy
/// specification's user executables).
pub(super) fn path_link() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute()).map(|home| home.join(".local/bin/neoteletext"))
}

/// The menu entry for a launcher at `program` (Desktop Entry
/// Specification version 1.5). There is no icon of the browser's own yet;
/// the theme's web browser icon stands in.
fn desktop_text(program: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.5\nName=NeoTeletext\nGenericName=Teletext browser\nComment=Browse teletext channels on the web\nExec=\"{}\" %U\nIcon=web-browser\nTerminal=false\nCategories=Network;WebBrowser;\n",
        program.display()
    )
}

/// Puts a link to `launcher` at `link`, in place of an older link: the
/// link, once made.
fn link_on_path(launcher: &Path, link: PathBuf) -> Option<PathBuf> {
    if let Some(folder) = link.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    if std::fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        let _ = std::fs::remove_file(&link);
    }
    std::os::unix::fs::symlink(launcher, &link).is_ok().then_some(link)
}

/// The `AppArmor` restriction of Ubuntu from 24.04 (`sandboxing.md`
/// section 8.2): unprivileged user namespaces only for programs a profile
/// allows.
const APPARMOR_RESTRICTION: &str = "/proc/sys/kernel/apparmor_restrict_unprivileged_userns";
/// Where an administrator installs the browser's profile.
pub(super) const APPARMOR_TARGET: &str = "/etc/apparmor.d/neoteletext";

/// The `AppArmor` profile for the installation at `root`, by its real path
/// (`AppArmor` matches a program by the path the kernel resolved, so a
/// root reached through a link, or under `XDG_DATA_HOME`, is named as it
/// is), written to `root`'s `neoteletext.apparmor` for an administrator to
/// install: where it is, and its text (decision 95).
fn profile_for(root: &Path) -> Result<(PathBuf, String), String> {
    let real = root.canonicalize().map_err(|error| io_text(root, &error))?;
    let named = update::platform::apparmor_root(&real).ok_or_else(|| format!("no AppArmor profile can name {}", real.display()))?;
    let text = update::platform::apparmor_profile(&named);
    let file = root.join(update::platform::APPARMOR_PROFILE);
    files::write_atomically(&file, text.as_bytes()).map_err(|error| io_text(&file, &error))?;
    Ok((file, text))
}

/// Linux: `neoteletext --install`, run from the unpacked tarball, whose
/// signature the download page says how to check first: the version
/// beside this program is checked by the pinned release key, copied into
/// the user's data folder with this launcher, and given a menu entry and
/// a command on the path (section 4.3). Nothing needs root; what the
/// machine needs that only an administrator can give is said.
///
/// # Errors
/// When the version is not accepted or cannot be installed.
pub fn install_linux(own: &Path) -> Result<(), String> {
    let platform = update::platform::Platform::LinuxX86_64;
    let source = own.parent().ok_or("this program has no folder")?;
    let layout = Layout::for_user(Family::Linux, None).ok_or("there is no home folder to install in")?;
    let signer = crate::pinned_signer(platform);
    let Some((version, _)) = install_version(source, &layout, signer.as_ref())? else {
        println!("NeoTeletext is already installed in {}; it keeps itself up to date.", layout.root.display());
        return Ok(());
    };
    let launcher = layout.launcher_folder().join(LINUX_LAUNCHER);
    if let Some(entry) = desktop_entry() {
        if let Some(folder) = entry.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        files::write_atomically(&entry, desktop_text(&launcher).as_bytes()).map_err(|error| io_text(&entry, &error))?;
    }
    let on_path = path_link().and_then(|link| link_on_path(&launcher, link));
    println!("NeoTeletext {version} is installed in {}.", layout.root.display());
    match on_path {
        Some(link) => println!("Start it from the applications menu, or with {} .", link.display()),
        None => println!("Start it from the applications menu, or with {} .", launcher.display()),
    }
    if std::fs::read_to_string(APPARMOR_RESTRICTION).is_ok_and(|value| value.trim() == "1") && !Path::new(APPARMOR_TARGET).exists() {
        match profile_for(&layout.root) {
            Ok((profile, text)) => println!(
                "\nThis system lets a program make a user namespace, which the browser's sandbox needs, only when an AppArmor profile allows it.\n\
                 An administrator must install the browser's profile once (it covers every later version). It allows user namespaces,\n\
                 and nothing else, to any program at these four paths, which you can write:\n\n{text}\n    \
                 sudo install -m 644 {} {APPARMOR_TARGET}\n    sudo apparmor_parser -r {APPARMOR_TARGET}\n\n\
                 Until then the browser refuses to start: it never runs with less of its sandbox.",
                profile.display()
            ),
            Err(why) => {
                println!("\nThis system restricts user namespaces, which the browser's sandbox needs, but {why}; until it has a profile it refuses to start.");
            }
        }
    }
    Ok(())
}

/// The one version's folder in `source` installed into `layout` with its
/// launcher and a state naming it: the version and its folder, or `None`
/// when an installation is there already.
fn install_version(source: &Path, layout: &Layout, signer: &dyn update::accept::Signer) -> Result<Option<(Version, PathBuf)>, String> {
    let platform = update::platform::Platform::LinuxX86_64;
    let versions: Vec<(Version, PathBuf)> = std::fs::read_dir(source)
        .map_err(|error| io_text(source, &error))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            Some((entry.file_name().to_str().and_then(Version::parse)?, entry.path()))
        })
        .collect();
    let [(version, folder)] = versions.as_slice() else { return Err(format!("{} must hold exactly one version's folder", source.display())) };
    if layout.state().exists() {
        return Ok(None);
    }
    // Copied first, then the copy accepted, as the launcher accepts an
    // update (decision 87): what is checked is what is installed, whatever
    // changes in the unpacked tarball meanwhile (finding F-69).
    std::fs::create_dir_all(layout.versions()).map_err(|error| io_text(&layout.versions(), &error))?;
    let copying = layout.versions().join(format!("{version}{COPYING_SUFFIX}"));
    files::remove_tree(&copying).map_err(|error| io_text(&copying, &error))?;
    update::accept::copy_release(folder, &copying, platform).map_err(|why| format!("the release is refused: {why}"))?;
    let accepted = update::accept::accept(&copying, platform, signer).map_err(|why| format!("the release is refused: {why}"))?;
    if accepted != *version {
        return Err(format!("the release is refused: {} holds {accepted}", folder.display()));
    }
    let installed = layout.version(*version);
    files::remove_tree(&installed).map_err(|error| io_text(&installed, &error))?;
    std::fs::rename(&copying, &installed).map_err(|error| io_text(&installed, &error))?;
    files::sync_folder(&layout.versions()).map_err(|error| io_text(&layout.versions(), &error))?;
    let bin = layout.launcher_folder();
    std::fs::create_dir_all(&bin).map_err(|error| io_text(&bin, &error))?;
    let launcher = bin.join(LINUX_LAUNCHER);
    let bytes = std::fs::read(installed.join(LAUNCHER_PROGRAM)).map_err(|error| io_text(&installed, &error))?;
    files::write_synced(&launcher, &bytes, true).map_err(|error| io_text(&launcher, &error))?;
    files::sync_folder(&bin).map_err(|error| io_text(&bin, &error))?;
    files::write_atomically(&layout.state(), update::state::State::installed(*version).to_text().as_bytes())
        .map_err(|error| io_text(&layout.state(), &error))?;
    Ok(Some((*version, installed)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use update::accept::{LinuxSigner, Signer};
    use update::manifest::{Entry, Manifest};
    use update::platform::{MANIFEST, MANIFEST_SIGNATURE, Platform, SIGNATURE_SUFFIX};

    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    /// A complete Linux version of `version` in `folder`, signed by `pair`.
    fn signed_version(folder: &Path, version: Version, pair: &Ed25519KeyPair) {
        let platform = Platform::LinuxX86_64;
        let mut entries = Vec::new();
        for file in platform.files().into_iter().filter(|file| !file.path.ends_with(SIGNATURE_SUFFIX)) {
            let bytes = format!("{} of {version}", file.path).into_bytes();
            let path = folder.join(&file.path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            files::write_synced(&path, &bytes, file.executable).unwrap();
            let signature = pair.sign(&bytes);
            let signature_path = format!("{}{SIGNATURE_SUFFIX}", file.path);
            std::fs::write(folder.join(&signature_path), signature.as_ref()).unwrap();
            entries.push(Entry { path: file.path, length: bytes.len() as u64, sha256: update::sha256(&bytes) });
            entries.push(Entry { path: signature_path, length: signature.as_ref().len() as u64, sha256: update::sha256(signature.as_ref()) });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let manifest = Manifest { version, platform, files: entries }.to_text();
        std::fs::write(folder.join(MANIFEST), &manifest).unwrap();
        std::fs::write(folder.join(MANIFEST_SIGNATURE), pair.sign(manifest.as_bytes()).as_ref()).unwrap();
    }

    /// The release key's check, after which the unpacked tarball's launcher
    /// is changed, as another program of the user's could change it while
    /// `--install` runs.
    struct ChangedAfterAcceptance {
        signer: LinuxSigner,
        source: PathBuf,
    }

    impl Signer for ChangedAfterAcceptance {
        fn release_key(&self) -> Option<&[u8; 32]> {
            self.signer.release_key()
        }

        fn check(&self, folder: &Path, manifest: &Manifest) -> Result<(), String> {
            self.signer.check(folder, manifest)?;
            std::fs::write(&self.source, b"changed after acceptance").map_err(|error| error.to_string())
        }
    }

    /// Finding F-69: `--install` copies the version first and accepts the
    /// copy, so a file changed in the unpacked tarball after the check is
    /// not what is installed.
    #[test]
    fn a_file_changed_after_acceptance_is_not_installed() {
        let folder = std::env::temp_dir().join(format!("neoteletext-install-linux-changed-{}", std::process::id()));
        let _ = files::remove_tree(&folder);
        let version = Version::new(0, 1, 0);
        let source = folder.join("unpacked");
        let pair = Ed25519KeyPair::from_pkcs8(Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap().as_ref()).unwrap();
        signed_version(&source.join(version.to_string()), version, &pair);
        let launcher = source.join(version.to_string()).join(LAUNCHER_PROGRAM);
        let genuine = std::fs::read(&launcher).unwrap();
        let signer = ChangedAfterAcceptance { signer: LinuxSigner { key: Some(pair.public_key().as_ref().try_into().unwrap()) }, source: launcher };
        let layout = Layout { family: Family::Linux, root: folder.join("installation"), bundle: None };
        let installed = install_version(&source, &layout, &signer).unwrap().unwrap();
        assert_eq!(installed.0, version);
        assert!(std::fs::read(installed.1.join(LAUNCHER_PROGRAM)).unwrap() == genuine, "the version installed is the one accepted");
        assert!(std::fs::read(layout.launcher_folder().join(LINUX_LAUNCHER)).unwrap() == genuine, "the launcher installed is the one accepted");
        files::remove_tree(&folder).unwrap();
    }

    /// Decision 95 (finding F-23): the profile the launcher writes names
    /// the four programs that make namespaces under the installation's
    /// real root, however it was reached.
    #[test]
    fn the_profile_names_the_real_root() {
        let folder = std::env::temp_dir().join(format!("neoteletext-apparmor-{}", std::process::id()));
        let _ = files::remove_tree(&folder);
        std::fs::create_dir_all(folder.join("real")).unwrap();
        std::os::unix::fs::symlink(folder.join("real"), folder.join("through")).unwrap();
        let (file, text) = profile_for(&folder.join("through")).unwrap();
        let real = folder.join("real").canonicalize().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        assert!(
            text.contains(&format!("\"{}/versions/*/{{neoteletext,neoteletext-decoder,neoteletext-interface,neoteletext-sound}}\"", real.display())),
            "{text}"
        );
        files::remove_tree(&folder).unwrap();
    }

    #[test]
    fn the_menu_entry_starts_the_launcher() {
        let text = desktop_text(Path::new("/home/a/.local/share/neoteletext/bin/neoteletext"));
        assert!(text.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(text.contains("\nExec=\"/home/a/.local/share/neoteletext/bin/neoteletext\" %U\n"));
        assert!(text.ends_with("Categories=Network;WebBrowser;\n"));
    }
}
