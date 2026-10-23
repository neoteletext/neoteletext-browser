//! The environment each role is given: only what it needs (requirement
//! 3.7).

use std::ffi::OsString;

use crate::Role;

/// What a process needs of the environment to start at all on this
/// platform; on Windows an `AppContainer` also needs the user's profile
/// folders, or it cannot be created.
#[cfg(windows)]
const ENVIRONMENT_NEEDED: &[&str] = &[
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "PROGRAMDATA",
    "ALLUSERSPROFILE",
    "USERNAME",
    "HOMEDRIVE",
    "HOMEPATH",
];
#[cfg(not(windows))]
const ENVIRONMENT_NEEDED: &[&str] = &[];
/// What the interface needs besides: the window system, the user's
/// folders for fonts and preferences, the locale, and its own
/// diagnostics. Not the session bus's address: nothing in the interface
/// uses D-Bus (decision 86).
const ENVIRONMENT_WINDOW: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "TMPDIR",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XAUTHORITY",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_DATA_DIRS",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "NEOTELETEXT_FRAME_TRACE",
];
/// What the sound helper needs: the sound server and the user's sound
/// configuration, and its own diagnostics.
const ENVIRONMENT_SOUND: &[&str] =
    &["HOME", "XDG_RUNTIME_DIR", "XDG_CONFIG_HOME", "PULSE_SERVER", "PULSE_RUNTIME_PATH", "PIPEWIRE_RUNTIME_DIR", "NEOTELETEXT_AUDIO_TRACE"];
/// What the trusted core needs: where its settings are, the phone
/// keyboard's relay for tests, and the update host for tests.
const ENVIRONMENT_CORE: &[&str] = &["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME", "NEOTELETEXT_QRKEYBOARD_RELAY"];

/// What the updater needs: the home folder, under which macOS keeps its
/// staging folder (`installation-and-updates.md` section 4.1), and in a
/// debug build the fault point a test asks for (section 13); a release
/// build reads none.
#[cfg(debug_assertions)]
const ENVIRONMENT_UPDATER: &[&str] = &["HOME", "NEOTELETEXT_FAULT"];
#[cfg(not(debug_assertions))]
const ENVIRONMENT_UPDATER: &[&str] = &["HOME"];

/// The environment a process of `role` is given, taken from this
/// process's: only what it needs, nothing else.
#[must_use]
pub fn environment(role: Role) -> Vec<(OsString, OsString)> {
    let extra: &[&str] = match role {
        Role::Interface => ENVIRONMENT_WINDOW,
        Role::Sound => ENVIRONMENT_SOUND,
        Role::Core => ENVIRONMENT_CORE,
        Role::Updater { .. } => ENVIRONMENT_UPDATER,
        Role::Decoder | Role::Network => &[],
    };
    let mut names: Vec<&str> = ENVIRONMENT_NEEDED.iter().chain(extra).copied().collect();
    names.sort_unstable();
    names.dedup();
    names.into_iter().filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value))).collect()
}
