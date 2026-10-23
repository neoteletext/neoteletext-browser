//! The desktops a release is built for, and the files a version has on
//! each (`installation-and-updates.md` section 6): a release names only
//! these paths, so no path read from a release can escape the version's
//! folder or name anything the browser does not ship.

use std::fmt;

/// The browser's programs, as each version has them: the broker and
/// trusted core, its helpers, the updater, and the launcher of this
/// release (installed only by a launcher release, section 7).
pub const PROGRAMS: [&str; 8] = [
    "neoteletext",
    "neoteletext-decoder",
    "neoteletext-fetcher",
    "neoteletext-interface",
    "neoteletext-keyboard",
    "neoteletext-sound",
    "neoteletext-updater",
    "neoteletext-launcher",
];
/// The program whose bytes carry the version (`mark`): the broker, which
/// every start runs.
pub const MARKED_PROGRAM: &str = "neoteletext";
/// The launcher's program in a version.
pub const LAUNCHER_PROGRAM: &str = "neoteletext-launcher";
/// The bundle every macOS version is.
pub const BUNDLE: &str = "NeoTeletext.app";
/// A Linux file's detached signature is beside it, with this suffix
/// (section 4.3).
pub const SIGNATURE_SUFFIX: &str = ".sig";
/// The Linux `AppArmor` profile a version carries (`sandboxing.md` section
/// 8.3, decision 77).
pub const APPARMOR_PROFILE: &str = "neoteletext.apparmor";
/// The programs whose sandbox makes a user namespace on Linux (`confine`'s
/// roles that make a namespace: the broker and trusted core, the decoder,
/// the interface and the sound helper), the only ones the `AppArmor`
/// profile names (decision 95). The fetcher, the keyboard and the updater
/// keep the network and make none.
pub const NAMESPACE_PROGRAMS: [&str; 4] = ["neoteletext", "neoteletext-decoder", "neoteletext-interface", "neoteletext-sound"];
/// The installation's root a release's own profile names: the default,
/// `~/.local/share/neoteletext`, in `AppArmor`'s variables.
pub const APPARMOR_DEFAULT_ROOT: &str = "@{HOME}/.local/share/neoteletext";

/// The `AppArmor` profile for an installation whose root is `root`, an
/// absolute path or [`APPARMOR_DEFAULT_ROOT`], already checked by
/// [`apparmor_root`] (`sandboxing.md` section 8.3, decision 95): on a
/// system that restricts unprivileged user namespaces, it lets the
/// [`NAMESPACE_PROGRAMS`] of every version there make one, and changes
/// nothing else.
#[must_use]
pub fn apparmor_profile(root: &str) -> String {
    format!(
        "# NeoTeletext: four of its programs may make a user namespace, for the
# namespaces of their sandbox (docs/sandboxing.md section 8).
# It allows userns, and nothing else, to any program at these four paths,
# which the user who installed NeoTeletext can write.
# Installed by an administrator once; it covers every later version.
abi <abi/4.0>,
include <tunables/global>

profile neoteletext \"{root}/versions/*/{{{}}}\" flags=(unconfined) {{
  userns,

  include if exists <local/neoteletext>
}}
",
        NAMESPACE_PROGRAMS.join(",")
    )
}

/// `path` as the installation's root in an `AppArmor` profile, if it is
/// absolute text holding none of the characters `AppArmor` treats as a
/// pattern, a quote or an escape (its `apparmor.d` manual's globbing
/// section: `* ? [ ] { } ^ , \\ "`), nor a control character or a line's
/// end, so the profile names exactly that folder.
#[must_use]
pub fn apparmor_root(path: &std::path::Path) -> Option<String> {
    let text = path.to_str()?;
    let plain = text.starts_with('/')
        && !text.ends_with('/')
        && text.chars().all(|c| !c.is_control() && !matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '^' | ',' | '\\' | '"' | '@'));
    plain.then(|| text.to_owned())
}
/// The release's own list of its files, in every version's folder.
pub const MANIFEST: &str = "manifest";
/// The release key's signature of the manifest, on Linux and Windows
/// (decision 92).
pub const MANIFEST_SIGNATURE: &str = "manifest.sig";
/// The most files a version has on any platform.
pub const MAX_FILES: usize = 32;

/// A desktop and its processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    /// macOS on Apple silicon.
    MacosArm64,
    /// macOS on Intel.
    MacosX86_64,
    /// Windows on x86-64.
    WindowsX86_64,
    /// Linux on x86-64.
    LinuxX86_64,
}

/// The desktop's family, where the platforms differ by more than the
/// processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// One bundle, swapped whole (section 4.1).
    Macos,
    /// A fixed stub, two launchers and versioned folders (section 4.2).
    Windows,
    /// Versioned folders in the user's data folder (section 4.3).
    Linux,
}

/// A file a version has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseFile {
    /// Its path inside the version's folder, with `/` between parts.
    pub path: String,
    /// It is a program, to be marked executable where that is a mode.
    pub executable: bool,
    /// Every version has it; a version without it is incomplete.
    pub required: bool,
}

impl Platform {
    /// Every platform, for reading a name.
    pub const ALL: [Self; 4] = [Self::MacosArm64, Self::MacosX86_64, Self::WindowsX86_64, Self::LinuxX86_64];

    /// The platform this program was built for, if releases are made for it.
    #[must_use]
    pub const fn this() -> Option<Self> {
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some(Self::MacosArm64)
        } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            Some(Self::MacosX86_64)
        } else if cfg!(all(windows, target_arch = "x86_64")) {
            Some(Self::WindowsX86_64)
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Some(Self::LinuxX86_64)
        } else {
            None
        }
    }

    /// Its name in the release's paths: `<version>/<name>/<file>.gz`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::MacosArm64 => "macos-arm64",
            Self::MacosX86_64 => "macos-x86_64",
            Self::WindowsX86_64 => "windows-x86_64",
            Self::LinuxX86_64 => "linux-x86_64",
        }
    }

    /// The platform of `name`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|platform| platform.name() == name)
    }

    /// Its family.
    #[must_use]
    pub const fn family(self) -> Family {
        match self {
            Self::MacosArm64 | Self::MacosX86_64 => Family::Macos,
            Self::WindowsX86_64 => Family::Windows,
            Self::LinuxX86_64 => Family::Linux,
        }
    }

    /// Whether a version's manifest is signed by the release key
    /// (`manifest.sig`): on Linux, which has no platform signature, and on
    /// Windows, whose Authenticode signs each program but not the set of
    /// them (decision 92); not on macOS, whose bundle's seal covers every
    /// file.
    #[must_use]
    pub const fn signs_manifest(self) -> bool {
        matches!(self.family(), Family::Linux | Family::Windows)
    }

    /// A program's file name on this platform.
    #[must_use]
    pub fn program_file(self, program: &str) -> String {
        match self.family() {
            Family::Windows => format!("{program}.exe"),
            Family::Macos | Family::Linux => program.to_owned(),
        }
    }

    /// A program's path inside a version's folder.
    #[must_use]
    pub fn program_path(self, program: &str) -> String {
        match self.family() {
            Family::Macos => format!("{BUNDLE}/Contents/MacOS/{program}"),
            Family::Windows | Family::Linux => self.program_file(program),
        }
    }

    /// Every file a version may have on this platform, the only paths a
    /// release may name; the manifest and its signature are the release's
    /// own and not among them.
    #[must_use]
    pub fn files(self) -> Vec<ReleaseFile> {
        let program = |program: &str| ReleaseFile { path: self.program_path(program), executable: true, required: true };
        let data = |path: &str, required: bool| ReleaseFile { path: path.to_owned(), executable: false, required };
        let mut files: Vec<ReleaseFile> = PROGRAMS.iter().map(|name| program(name)).collect();
        match self.family() {
            Family::Macos => {
                files.push(data(&format!("{BUNDLE}/Contents/Info.plist"), true));
                files.push(data(&format!("{BUNDLE}/Contents/_CodeSignature/CodeResources"), true));
                // The notarisation ticket, stapled (section 4.1); a test
                // build signed without Apple has none.
                files.push(data(&format!("{BUNDLE}/Contents/CodeResources"), false));
            }
            Family::Windows => {}
            Family::Linux => {
                files.push(data(APPARMOR_PROFILE, true));
                let signed: Vec<String> = files.iter().map(|file| format!("{}{SIGNATURE_SUFFIX}", file.path)).collect();
                files.extend(signed.iter().map(|path| data(path, true)));
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files
    }

    /// The file of the release at `path`, if a version may have it.
    #[must_use]
    pub fn file(self, path: &str) -> Option<ReleaseFile> {
        self.files().into_iter().find(|file| file.path == path)
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 95: the profile names the four programs that make
    /// namespaces under the root given, and a root that would be a
    /// pattern is refused.
    #[test]
    fn the_apparmor_profile_names_four_programs_under_the_real_root() {
        let profile = apparmor_profile("/home/a b/.local/share/neoteletext");
        assert!(
            profile.contains("\nprofile neoteletext \"/home/a b/.local/share/neoteletext/versions/*/{neoteletext,neoteletext-decoder,neoteletext-interface,neoteletext-sound}\" flags=(unconfined) {\n  userns,\n"),
            "{profile}"
        );
        for program in NAMESPACE_PROGRAMS {
            assert!(PROGRAMS.contains(&program), "{program}");
        }
        assert_eq!(apparmor_root(std::path::Path::new("/home/a b/.local/share/neoteletext")).as_deref(), Some("/home/a b/.local/share/neoteletext"));
        for refused in ["relative", "/home/*", "/home/{a,b}", "/home/a\"", "/home/a\\b", "/home/a\nb", "/home/@{HOME}", "/home/a/"] {
            assert_eq!(apparmor_root(std::path::Path::new(refused)), None, "{refused:?}");
        }
    }

    #[test]
    fn every_platform_has_its_programs_and_no_path_escapes() {
        for platform in Platform::ALL {
            assert_eq!(Platform::from_name(platform.name()), Some(platform));
            let files = platform.files();
            assert!(files.len() <= MAX_FILES, "{platform}");
            for name in PROGRAMS {
                assert!(files.iter().any(|file| file.path == platform.program_path(name) && file.executable), "{platform} {name}");
            }
            for file in &files {
                assert!(!file.path.starts_with('/') && !file.path.contains('\\'), "{}", file.path);
                assert!(file.path.split('/').all(|part| !part.is_empty() && part != "." && part != ".."), "{}", file.path);
                assert_ne!(file.path, MANIFEST);
                assert_ne!(file.path, MANIFEST_SIGNATURE);
            }
            let mut paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
            paths.dedup();
            assert_eq!(paths.len(), files.len(), "{platform}: each path once");
        }
        assert_eq!(Platform::WindowsX86_64.program_path("neoteletext"), "neoteletext.exe");
        assert_eq!(Platform::MacosArm64.program_path("neoteletext"), "NeoTeletext.app/Contents/MacOS/neoteletext");
        assert!(Platform::LinuxX86_64.file("neoteletext.sig").is_some_and(|file| file.required && !file.executable));
        assert!(Platform::LinuxX86_64.file("../neoteletext").is_none());
        assert!(Platform::MacosArm64.file("NeoTeletext.app/Contents/CodeResources").is_some_and(|file| !file.required));
    }
}
