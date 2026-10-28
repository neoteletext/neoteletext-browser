//! The helpers' sandboxes on macOS: `sandbox-exec` profiles that start
//! from "deny everything", take Apple's baseline for any process
//! (`system.sb`: the dynamic loader, system libraries, time zones, random
//! numbers) and add only what the helper is for. The kernel reports what
//! a profile denies only to the system log's administrators; the profiles
//! were made by running each helper under candidates, where a denied
//! operation shows as a crash and the sound services' absence as a silent
//! browser (`security-analysis.md` section 13, 21 October 2026).
//!
//! The trusted core is confined too, behind a broker (decision 56): macOS
//! refuses a nested `sandbox-exec`, so a core under a profile could start
//! no confined helper, and the broker, the one unconfined process, starts
//! the core under its profile and every helper under its own.
//!
//! Every path a profile names is passed to `sandbox-exec` as a `-D`
//! parameter and named in the profile as `(param "NAME")`, never written
//! into the profile's text (finding F-51, decision 97): a path is the
//! user's and may hold any character, and a `"` or `)` in it would
//! otherwise end the string and could open the sandbox.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Report, Role, Spawn};

/// The profile's parameters: `sandbox-exec -D NAME=value` for each, named
/// in the profile as `(param "NAME")`. A path goes here, never into the
/// profile's text (finding F-51).
#[derive(Default, Debug)]
struct Parameters {
    values: Vec<(String, PathBuf)>,
}

impl Parameters {
    /// Registers `path` under `name` and returns the SBPL token that reads
    /// it, `(param "NAME")`. A name is used once, so two paths never
    /// collide.
    fn path(&mut self, name: &str, path: &Path) -> String {
        self.values.push((name.to_owned(), path.to_path_buf()));
        format!("(param \"{name}\")")
    }
}

/// The user's home folder, for what lies under it.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute())
}

/// The user's per-process cache and temporary folders under
/// `/private/var/folders`, where the font registry's and other caches the
/// window system reads live, read from `getconf` (finding F-52): the whole
/// of `/private/var/folders` held every application's per-user caches, far
/// more than this browser needs. A folder that cannot be read is an error,
/// since the profile would otherwise be built wrong (decision 58).
fn darwin_user_folders() -> Result<[PathBuf; 2], String> {
    let read = |name: &str| -> Result<PathBuf, String> {
        let output = Command::new("/usr/bin/getconf").arg(name).output().map_err(|error| format!("getconf {name}: {error}"))?;
        if !output.status.success() {
            return Err(format!("getconf {name} failed"));
        }
        let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if path.is_absolute() { Ok(path) } else { Err(format!("getconf {name} gave no path")) }
    };
    Ok([read("DARWIN_USER_CACHE_DIR")?, read("DARWIN_USER_TEMP_DIR")?])
}

/// The system's folders and the metadata of any path, which the window
/// system's and Core Audio's libraries read, with the two user folders
/// named as parameters (finding F-52).
fn system_reads(parameters: &mut Parameters) -> Result<String, String> {
    let [cache, temp] = darwin_user_folders()?;
    let cache = parameters.path("USER_CACHE", &cache);
    let temp = parameters.path("USER_TEMP", &temp);
    Ok(format!(
        "(deny network*)
(allow file-read-metadata)
(allow file-read* (subpath \"/System\") (subpath \"/Library\") (subpath \"/usr\") (subpath \"/private/var/db\") (subpath {cache}) (subpath {temp}) (subpath \"/private/etc\") (subpath \"/dev\"))
"
    ))
}

/// The listings of the folders the interface's program lies in, and in
/// `NeoTeletext.app` the bundle's `Info.plist`, where the version is.
/// `AppKit` finds the main bundle by listing them; without them it has no
/// path, and the standard About panel stops the interface on an assertion
/// in `NSImage` (found 25 October 2026, macOS 15.6). A folder's listing
/// gives its names, not what its files hold.
fn own_bundle_reads(parameters: &mut Parameters, program: &Path) -> String {
    let Some(folder) = program.parent() else { return String::new() };
    let contents = folder.parent().filter(|contents| folder.ends_with("MacOS") && contents.ends_with("Contents"));
    let bundle = contents.and_then(Path::parent).filter(|bundle| bundle.extension().is_some_and(|extension| extension == "app"));
    match (contents, bundle) {
        (Some(contents), Some(bundle)) => {
            let folder = parameters.path("BUNDLE_PROGRAMS", folder);
            let information = parameters.path("BUNDLE_INFORMATION", &contents.join("Info.plist"));
            let contents = parameters.path("BUNDLE_CONTENTS", contents);
            let bundle = parameters.path("BUNDLE", bundle);
            format!("(allow file-read* (literal {bundle}) (literal {contents}) (literal {folder}) (literal {information}))\n")
        }
        _ => format!("(allow file-read* (literal {}))\n", parameters.path("PROGRAM_FOLDER", folder)),
    }
}

/// Every profile's start: nothing, then Apple's baseline, then the
/// program itself, named as the parameter `PROGRAM`, and the null device
/// for what is not wanted.
fn preamble(program: &str) -> String {
    format!(
        "(version 1)
(deny default)
(import \"system.sb\")
(allow process-exec (literal {program}))
(allow file-read* (literal {program}))
(allow file-write-data (literal \"/dev/null\"))
"
    )
}

/// The profile for `program` as `role` says, with the parameters its paths
/// are passed as; `writable` is the core's settings folder, and
/// `local_network` whether a fetcher may reach this computer.
fn profile(program: &Path, role: Role, writable: Option<&Path>, local_network: bool) -> Result<(String, Parameters), String> {
    let mut parameters = Parameters::default();
    let mut profile = preamble(&parameters.path("PROGRAM", program));
    match role {
        Role::Decoder => profile.push_str("(deny network*)\n"),
        // Names are resolved through mDNSResponder's socket and
        // opendirectoryd's libinfo, with the hosts file.
        Role::Network => {
            profile.push_str(RESOLVER);
            // A fetcher's refusal of this computer's addresses, when the
            // user has not allowed the local network (decision 89,
            // requirement 5.10): after the allowing of TCP, and the later
            // rule wins. The profile language names `localhost` but no
            // private ranges, so the local network beyond this computer is
            // left to the fetcher's own check.
            if !local_network {
                profile.push_str("(deny network-outbound (remote ip \"localhost:*\"))\n");
            }
        }
        // Found by running the interface under candidate profiles (21
        // October 2026): it needs to read the system's folders and the
        // metadata of any path (the root's symbolic links), and the
        // services of the window, fonts, text input, the pasteboard,
        // notifications, preferences and the Dock. Sound is the sound
        // helper's (decision 55). Without the window manager's service
        // (found 23 October 2026, macOS 15.6) the window is made and drawn
        // but never put on the screen.
        Role::Interface => {
            profile.push_str(&system_reads(&mut parameters)?);
            profile.push_str(&own_bundle_reads(&mut parameters, program));
            profile.push_str(INTERFACE_SERVICES);
            if let Some(home) = home() {
                for (name, folder) in INTERFACE_HOME {
                    let token = parameters.path(name, &home.join(folder));
                    let _ = writeln!(profile, "(allow file-read* (subpath {token}))");
                }
            }
        }
        // The sound output: Core Audio's services, which the interface
        // used before decision 55 (found 21 October 2026: without them it
        // ran silent), and the system's folders for its plug-ins.
        Role::Sound => {
            profile.push_str(&system_reads(&mut parameters)?);
            profile.push_str(SOUND_SERVICES);
        }
        // The updater: names resolved as a fetcher resolves them, TCP to
        // its one port, and its staging folder (`installation-and-updates.md`
        // section 8.1). `sandbox-exec` names a port, not a host; TLS pins
        // the host.
        Role::Updater { port } => {
            profile.push_str("(system-network)\n");
            let _ = writeln!(profile, "(allow network-outbound (remote tcp \"*:{port}\") (literal \"/private/var/run/mDNSResponder\"))");
            profile.push_str(RESOLVER_LOOKUP);
            writable_folder(&mut profile, &mut parameters, writable);
        }
        // The trusted core: its settings folder, read and written, and
        // nothing else (decision 56).
        Role::Core => {
            profile.push_str("(deny network*)\n");
            writable_folder(&mut profile, &mut parameters, writable);
        }
    }
    Ok((profile, parameters))
}

/// Allows read and write under `folder`, named as a parameter.
fn writable_folder(profile: &mut String, parameters: &mut Parameters, folder: Option<&Path>) {
    if let Some(folder) = folder {
        let token = parameters.path("WRITABLE", folder);
        let _ = writeln!(profile, "(allow file-read* file-write* (subpath {token}))");
    }
}

/// A network helper's name resolution: outbound TCP, mDNSResponder,
/// opendirectoryd's libinfo and the resolver's files.
const RESOLVER: &str = "(system-network)
(allow network-outbound (remote tcp \"*:*\") (literal \"/private/var/run/mDNSResponder\"))
(allow mach-lookup (global-name \"com.apple.system.opendirectoryd.libinfo\") (global-name \"com.apple.system.DirectoryService.libinfo_v1\"))
(allow file-read* (literal \"/private/etc/hosts\") (literal \"/private/etc/resolv.conf\") (literal \"/private/var/run/resolv.conf\"))
";
/// The resolver's lookups and files, without the broad TCP of [`RESOLVER`]
/// (the updater names its own one port).
const RESOLVER_LOOKUP: &str =
    "(allow mach-lookup (global-name \"com.apple.system.opendirectoryd.libinfo\") (global-name \"com.apple.system.DirectoryService.libinfo_v1\"))
(allow file-read* (literal \"/private/etc/hosts\") (literal \"/private/etc/resolv.conf\") (literal \"/private/var/run/resolv.conf\"))
";
/// The window system's services the interface reaches.
const INTERFACE_SERVICES: &str = "(allow mach-lookup
  (global-name \"com.apple.windowserver.active\")
  (global-name \"com.apple.CARenderServer\")
  (global-name \"com.apple.CoreDisplay.master\")
  (global-name \"com.apple.coreservices.launchservicesd\")
  (global-name \"com.apple.coreservices.appleevents\")
  (global-name \"com.apple.lsd.mapdb\")
  (global-name \"com.apple.FontObjectsServer\")
  (global-name \"com.apple.fonts\")
  (global-name \"com.apple.FontServer\")
  (global-name \"com.apple.tsm.uiserver\")
  (global-name \"com.apple.tsm.portname\")
  (global-name \"com.apple.inputmethodkit.launcher\")
  (global-name \"com.apple.TextInput\")
  (global-name \"com.apple.pasteboard.1\")
  (global-name \"com.apple.distributed_notifications@Uv3\")
  (global-name \"com.apple.system.notification_center\")
  (global-name \"com.apple.cfprefsd.daemon\")
  (global-name \"com.apple.cfprefsd.agent\")
  (global-name \"com.apple.dock.server\")
  (global-name \"com.apple.dock.fullscreen\")
  (global-name \"com.apple.windowmanager.server\"))
";
/// The folders under the user's home the interface may read, each passed
/// as the named parameter.
const INTERFACE_HOME: [(&str, &str); 4] = [
    ("FONTS", "Library/Fonts"),
    ("PREFERENCES", "Library/Preferences"),
    ("KEYBOARD_LAYOUTS", "Library/Keyboard Layouts"),
    ("INPUT_METHODS", "Library/Input Methods"),
];
/// Core Audio's services the sound helper reaches.
const SOUND_SERVICES: &str = "(allow mach-lookup
  (global-name \"com.apple.audio.coreaudiod\")
  (global-name \"com.apple.audio.audiohald\")
  (global-name \"com.apple.audio.AudioComponentRegistrar\")
  (global-name \"com.apple.audio.SandboxHelper\")
  (global-name \"com.apple.cfprefsd.daemon\")
  (global-name \"com.apple.cfprefsd.agent\"))
";

/// A process's own side: the broker confined it at its start.
pub(crate) fn apply() -> Report {
    Report { layers: vec![String::from("sandbox-exec, by its parent")] }
}

/// The command that runs the helper under its profile, each path a `-D`
/// parameter (finding F-51).
pub(crate) fn command(spawn: &Spawn<'_>) -> Result<Command, String> {
    let (profile, parameters) = profile(spawn.program, spawn.role, spawn.writable, spawn.local_network)?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command.arg("-p").arg(profile);
    for (name, value) in parameters.values {
        let mut argument = std::ffi::OsString::from(format!("{name}="));
        argument.push(value);
        command.arg("-D").arg(argument);
    }
    command.arg(spawn.program).args(spawn.arguments);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every profile starts from nothing, names its program as a
    /// parameter, and gives the network only to the network helpers.
    #[test]
    fn profiles_start_from_nothing_and_give_the_network_only_to_fetchers() {
        let program = Path::new("/Applications/NeoTeletext.app/Contents/MacOS/neoteletext-decoder");
        for role in [Role::Decoder, Role::Network, Role::Interface, Role::Core, Role::Sound] {
            let (profile, parameters) = profile(program, role, None, false).unwrap();
            assert!(profile.starts_with("(version 1)\n(deny default)\n(import \"system.sb\")\n"), "{role:?}");
            assert!(profile.contains("(allow process-exec (literal (param \"PROGRAM\")))"), "{role:?}");
            assert_eq!(parameters.values.first().map(|(name, path)| (name.as_str(), path.as_path())), Some(("PROGRAM", program)), "{role:?}");
            assert_eq!(profile.contains("(deny network*)"), role != Role::Network, "{role:?}");
            assert_eq!(profile.contains("(system-network)"), role == Role::Network, "{role:?}");
            assert!(!profile.contains("(allow default)"));
        }
        assert!(!profile(program, Role::Decoder, None, false).unwrap().0.contains("mach-lookup"), "a decoder looks nothing up");
        assert!(!profile(program, Role::Core, None, false).unwrap().0.contains("mach-lookup"), "nor does the core");
        assert!(!profile(program, Role::Interface, None, false).unwrap().0.contains("audio"), "sound is the sound helper's");
        assert!(
            profile(program, Role::Interface, None, false).unwrap().0.contains("(global-name \"com.apple.windowmanager.server\")"),
            "without it the window never reaches the screen"
        );
        let (core, parameters) = profile(program, Role::Core, Some(Path::new("/Users/someone/Library/Application Support/NeoTeletext")), false).unwrap();
        assert!(core.contains("(allow file-read* file-write* (subpath (param \"WRITABLE\")))"));
        assert_eq!(
            parameters.values.iter().find(|(name, _)| name == "WRITABLE").map(|(_, path)| path.as_path()),
            Some(Path::new("/Users/someone/Library/Application Support/NeoTeletext"))
        );
    }

    /// Finding F-51: a path with a `"` or `)` in it, which would end a
    /// string literal and could open the sandbox, is a parameter, so it
    /// never reaches the profile's text; the profile reads it by name.
    #[test]
    fn a_crafted_path_stays_a_parameter() {
        let program = Path::new("/tmp/x\")(allow default)(\"/neoteletext-decoder");
        let (profile, parameters) = profile(program, Role::Decoder, None, false).unwrap();
        assert!(!profile.contains("allow default"), "the path reached the profile: {profile}");
        assert!(profile.contains("(literal (param \"PROGRAM\"))"));
        assert_eq!(parameters.values.first().map(|(_, path)| path.as_path()), Some(program));
    }

    /// Finding F-52: the interface and the sound helper read only this
    /// program's own cache and temporary folders under
    /// `/private/var/folders`, named as parameters, not all of it, where
    /// every application's per-user caches live.
    #[test]
    fn the_system_reads_name_only_this_user_s_folders() {
        let program = Path::new("/Applications/NeoTeletext.app/Contents/MacOS/neoteletext-interface");
        for role in [Role::Interface, Role::Sound] {
            let (profile, parameters) = profile(program, role, None, false).unwrap();
            assert!(!profile.contains("(subpath \"/private/var/folders\")"), "{role:?} still reads all of /private/var/folders: {profile}");
            assert!(profile.contains("(subpath (param \"USER_CACHE\")) (subpath (param \"USER_TEMP\"))"), "{role:?}: {profile}");
            for name in ["USER_CACHE", "USER_TEMP"] {
                let path = parameters.values.iter().find(|(parameter, _)| parameter == name).map(|(_, path)| path);
                assert!(path.is_some_and(|path| path.is_absolute()), "{role:?} {name}: {parameters:?}");
            }
        }
    }

    /// The interface lists the folders it lies in, and reads its bundle's
    /// `Info.plist`, and nothing else of them: without the listings the
    /// About panel stopped it on an assertion. No other role has them.
    #[test]
    fn the_interface_lists_its_own_folders_for_its_bundle() {
        let read = |parameters: &Parameters, name: &str| parameters.values.iter().find(|(parameter, _)| parameter == name).map(|(_, path)| path.clone());
        let bundled = Path::new("/Applications/NeoTeletext.app/Contents/MacOS/neoteletext-interface");
        let (rules, parameters) = profile(bundled, Role::Interface, None, false).unwrap();
        assert!(
            rules.contains(
                "(allow file-read* (literal (param \"BUNDLE\")) (literal (param \"BUNDLE_CONTENTS\")) (literal (param \"BUNDLE_PROGRAMS\")) (literal (param \"BUNDLE_INFORMATION\")))"
            ),
            "{rules}"
        );
        assert_eq!(read(&parameters, "BUNDLE"), Some(PathBuf::from("/Applications/NeoTeletext.app")));
        assert_eq!(read(&parameters, "BUNDLE_CONTENTS"), Some(PathBuf::from("/Applications/NeoTeletext.app/Contents")));
        assert_eq!(read(&parameters, "BUNDLE_PROGRAMS"), Some(PathBuf::from("/Applications/NeoTeletext.app/Contents/MacOS")));
        assert_eq!(read(&parameters, "BUNDLE_INFORMATION"), Some(PathBuf::from("/Applications/NeoTeletext.app/Contents/Info.plist")));
        assert!(!rules.contains("(subpath (param \"BUNDLE"), "listings, not the bundle's files");
        let built = Path::new("/Users/someone/neoteletext-browser/native/target/release/neoteletext-interface");
        let (rules, parameters) = profile(built, Role::Interface, None, false).unwrap();
        assert!(rules.contains("(allow file-read* (literal (param \"PROGRAM_FOLDER\")))"), "{rules}");
        assert_eq!(read(&parameters, "PROGRAM_FOLDER"), Some(PathBuf::from("/Users/someone/neoteletext-browser/native/target/release")));
        assert!(!rules.contains("BUNDLE"), "not a bundle: {rules}");
        for role in [Role::Decoder, Role::Network, Role::Core, Role::Sound] {
            let (rules, _) = profile(bundled, role, None, false).unwrap();
            assert!(!rules.contains("BUNDLE") && !rules.contains("PROGRAM_FOLDER"), "{role:?}");
        }
    }

    /// Section 8.1 of the installation design: the updater reaches one
    /// port and writes its staging folder alone.
    #[test]
    fn the_updater_reaches_one_port_and_writes_only_its_staging_folder() {
        let program = Path::new("/Applications/NeoTeletext.app/Contents/MacOS/neoteletext-updater");
        let staging = Path::new("/Users/someone/Library/Application Support/NeoTeletext Updates/staging");
        let (updater, parameters) = profile(program, Role::Updater { port: 443 }, Some(staging), false).unwrap();
        assert!(updater.starts_with("(version 1)\n(deny default)\n"));
        assert!(updater.contains("(remote tcp \"*:443\")"));
        assert!(!updater.contains("*:*"), "no other port");
        assert!(!updater.contains("(deny network*)"));
        assert_eq!(updater.matches("file-write*").count(), 1, "one folder written: {updater}");
        assert!(updater.contains("(allow file-read* file-write* (subpath (param \"WRITABLE\")))"));
        assert_eq!(parameters.values.iter().find(|(name, _)| name == "WRITABLE").map(|(_, path)| path.as_path()), Some(staging));
        assert!(!updater.contains("process-fork") && !updater.contains("(allow process-exec*"), "it starts nothing");
    }

    /// Finding F-89 of the plan is elsewhere; here, the fetcher's loopback
    /// deny is present only when the local network is not allowed
    /// (decision 89).
    #[test]
    fn the_fetcher_denies_loopback_only_when_the_local_network_is_not_allowed() {
        let program = Path::new("/Applications/NeoTeletext.app/Contents/MacOS/neoteletext-fetcher");
        assert!(profile(program, Role::Network, None, false).unwrap().0.contains("(deny network-outbound (remote ip \"localhost:*\"))"));
        assert!(!profile(program, Role::Network, None, true).unwrap().0.contains("localhost"));
    }
}
