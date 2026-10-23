//! The helpers' sandboxes on macOS: `sandbox-exec` profiles that start
//! from "deny everything", take Apple's baseline for any process
//! (`system.sb`: the dynamic loader, system libraries, time zones, random
//! numbers) and add only what the helper is for. The kernel reports what
//! a profile denies only to the system log's administrators; the profiles
//! were made by running each helper under candidates, where a denied
//! operation shows as a crash and the sound services' absence as a silent
//! browser (`security-analysis.md` section 13, 21 October 2026).
//!
//! The trusted core itself is not confined: macOS refuses a nested
//! `sandbox-exec`, so a core under a profile could start no confined
//! helper. That needs the App Sandbox at packaging (requirement 3.8).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Role, Spawn};

/// Every profile's start: nothing, then Apple's baseline, then the
/// program itself and the null device for what is not wanted.
fn preamble(program: &Path) -> String {
    let program = program.display();
    format!(
        "(version 1)
(deny default)
(import \"system.sb\")
(allow process-exec (literal \"{program}\"))
(allow file-read* (literal \"{program}\"))
(allow file-write-data (literal \"/dev/null\"))
"
    )
}

/// The user's home folder, for what lies under it.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute())
}

/// The system's folders and the metadata of any path, which the
/// window system's and Core Audio's libraries read.
const SYSTEM_READS: &str = "(deny network*)
(allow file-read-metadata)
(allow file-read* (subpath \"/System\") (subpath \"/Library\") (subpath \"/usr\") (subpath \"/private/var/db\") (subpath \"/private/var/folders\") (subpath \"/private/etc\") (subpath \"/dev\"))
";

/// The profile for `program` as `role` says; `writable` is the core's
/// settings folder.
fn profile(program: &Path, role: Role, writable: Option<&Path>) -> String {
    let mut profile = preamble(program);
    match role {
        Role::Decoder => profile.push_str("(deny network*)\n"),
        // Names are resolved through mDNSResponder's socket and
        // opendirectoryd's libinfo, with the hosts file.
        Role::Network => profile.push_str(
            "(system-network)
(allow network-outbound (remote tcp \"*:*\") (literal \"/private/var/run/mDNSResponder\"))
(allow mach-lookup (global-name \"com.apple.system.opendirectoryd.libinfo\") (global-name \"com.apple.system.DirectoryService.libinfo_v1\"))
(allow file-read* (literal \"/private/etc/hosts\") (literal \"/private/etc/resolv.conf\") (literal \"/private/var/run/resolv.conf\"))
",
        ),
        // Found by running the interface under candidate profiles (21
        // October 2026): it needs to read the system's folders and the
        // metadata of any path (the root's symbolic links), and the
        // services of the window, fonts, text input, the pasteboard,
        // notifications, preferences and the Dock. Sound is the sound
        // helper's (decision 55). Without the window manager's service
        // (found 23 October 2026, macOS 15.6) the window is made and drawn
        // but never put on the screen.
        Role::Interface => {
            profile.push_str(SYSTEM_READS);
            profile.push_str(
                "(allow mach-lookup
  (global-name \"com.apple.windowserver.active\")
  (global-name \"com.apple.CARenderServer\")
  (global-name \"com.apple.CoreDisplay.master\")
  (global-name \"com.apple.coreservices.launchservicesd\")
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
",
            );
        }
        // The sound output: Core Audio's services, which the interface
        // used before decision 55 (found 21 October 2026: without them it
        // ran silent), and the system's folders for its plug-ins.
        Role::Sound => {
            profile.push_str(SYSTEM_READS);
            profile.push_str(
                "(allow mach-lookup
  (global-name \"com.apple.audio.coreaudiod\")
  (global-name \"com.apple.audio.audiohald\")
  (global-name \"com.apple.audio.AudioComponentRegistrar\")
  (global-name \"com.apple.audio.SandboxHelper\")
  (global-name \"com.apple.cfprefsd.daemon\")
  (global-name \"com.apple.cfprefsd.agent\"))
",
            );
        }
        // The updater: names resolved as a fetcher resolves them, TCP to
        // its one port, and its staging folder (`installation-and-updates.md`
        // section 8.1). `sandbox-exec` names a port, not a host; TLS pins
        // the host.
        Role::Updater { port } => {
            let _ = writeln!(
                profile,
                "(system-network)
(allow network-outbound (remote tcp \"*:{port}\") (literal \"/private/var/run/mDNSResponder\"))
(allow mach-lookup (global-name \"com.apple.system.opendirectoryd.libinfo\") (global-name \"com.apple.system.DirectoryService.libinfo_v1\"))
(allow file-read* (literal \"/private/etc/hosts\") (literal \"/private/etc/resolv.conf\") (literal \"/private/var/run/resolv.conf\"))"
            );
            if let Some(folder) = writable {
                let _ = writeln!(profile, "(allow file-read* file-write* (subpath \"{}\"))", folder.display());
            }
        }
        // The trusted core: its settings folder, read and written, and
        // nothing else (decision 56).
        Role::Core => {
            profile.push_str("(deny network*)\n");
            if let Some(folder) = writable {
                let _ = writeln!(profile, "(allow file-read* file-write* (subpath \"{}\"))", folder.display());
            }
        }
    }
    if let (Role::Interface, Some(home)) = (role, home()) {
        for folder in ["Library/Fonts", "Library/Preferences", "Library/Keyboard Layouts", "Library/Input Methods"] {
            let _ = writeln!(profile, "(allow file-read* (subpath \"{}\"))", home.join(folder).display());
        }
    }
    profile
}

/// The command that runs the helper under its profile.
pub fn command(spawn: &Spawn<'_>) -> Command {
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command.arg("-p").arg(profile(spawn.program, spawn.role, spawn.writable)).arg(spawn.program).args(spawn.arguments);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every profile starts from nothing, names its program, and gives
    /// the network only to the network helpers.
    #[test]
    fn profiles_start_from_nothing_and_give_the_network_only_to_fetchers() {
        let program = Path::new("/Applications/NeoTeletext.app/Contents/MacOS/neoteletext-decoder");
        for role in [Role::Decoder, Role::Network, Role::Interface, Role::Core, Role::Sound] {
            let profile = profile(program, role, None);
            assert!(profile.starts_with("(version 1)\n(deny default)\n(import \"system.sb\")\n"), "{role:?}");
            assert!(profile.contains("(allow process-exec (literal \"/Applications/NeoTeletext.app/Contents/MacOS/neoteletext-decoder\"))"));
            assert_eq!(profile.contains("(deny network*)"), role != Role::Network, "{role:?}");
            assert_eq!(profile.contains("(system-network)"), role == Role::Network, "{role:?}");
            assert!(!profile.contains("(allow default)"));
        }
        assert!(!profile(program, Role::Decoder, None).contains("mach-lookup"), "a decoder looks nothing up");
        assert!(!profile(program, Role::Core, None).contains("mach-lookup"), "nor does the core");
        assert!(!profile(program, Role::Interface, None).contains("audio"), "sound is the sound helper's");
        assert!(
            profile(program, Role::Interface, None).contains("(global-name \"com.apple.windowmanager.server\")"),
            "without it the window never reaches the screen"
        );
        let core = profile(program, Role::Core, Some(Path::new("/Users/someone/Library/Application Support/NeoTeletext")));
        assert!(core.contains("(allow file-read* file-write* (subpath \"/Users/someone/Library/Application Support/NeoTeletext\"))"));
    }

    /// Section 8.1 of the installation design: the updater reaches one
    /// port and writes its staging folder alone.
    #[test]
    fn the_updater_reaches_one_port_and_writes_only_its_staging_folder() {
        let program = Path::new("/Applications/NeoTeletext.app/Contents/MacOS/neoteletext-updater");
        let staging = Path::new("/Users/someone/Library/Application Support/NeoTeletext Updates/staging");
        let updater = profile(program, Role::Updater { port: 443 }, Some(staging));
        assert!(updater.starts_with("(version 1)\n(deny default)\n"));
        assert!(updater.contains("(remote tcp \"*:443\")"));
        assert!(!updater.contains("*:*"), "no other port");
        assert!(!updater.contains("(deny network*)"));
        assert_eq!(updater.matches("file-write*").count(), 1, "one folder written: {updater}");
        assert!(updater.contains(&format!("(allow file-read* file-write* (subpath \"{}\"))", staging.display())));
        assert!(!updater.contains("process-fork") && !updater.contains("(allow process-exec*"), "it starts nothing");
    }
}
