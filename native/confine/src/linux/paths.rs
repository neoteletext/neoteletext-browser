//! What each role may read, and the socket families it may open.

use std::path::PathBuf;

use crate::Role;

/// Where the resolver's files and the C library's name services live;
/// a fetcher may read them (as the engine's `neottx` lists them).
const RESOLVER_PATHS: &[&str] = &["/etc", "/usr/lib", "/lib", "/usr/lib64", "/lib64", "/run/systemd/resolve", "/run/resolvconf"];
/// What the window system, fonts, the input method and the sound server
/// keep that the interface reads; `/run` and `/tmp` are its own, holding
/// only its display's sockets (decision 86).
const INTERFACE_PATHS: &[&str] = &["/usr", "/etc", "/lib", "/lib64", "/opt", "/var/lib", "/var/cache/fontconfig", "/run", "/tmp", "/dev", "/proc", "/sys"];
/// What the interface may read under the user's home: fonts and their
/// configuration, and the display's authority file.
const INTERFACE_HOME_PATHS: &[&str] = &[".fonts", ".local/share/fonts", ".config/fontconfig", ".cache/fontconfig", ".Xauthority", ".config/gtk-3.0", ".icons"];
/// The sound devices, for ALSA's direct output.
pub(super) const SOUND_DEVICES: &str = "/dev/snd";
/// What the sound helper reads: the sound libraries' plug-ins and
/// configuration, and the sound server's sockets, alone in its own `/run`
/// (decision 86).
const SOUND_PATHS: &[&str] = &["/usr", "/etc", "/lib", "/lib64", "/opt", "/var/lib", "/run", "/proc", "/sys", "/dev"];
/// What the sound helper may read under the user's home: ALSA's and the
/// sound server's configuration.
const SOUND_HOME_PATHS: &[&str] = &[".asoundrc", ".config/pulse", ".config/pipewire"];

/// The socket families a role may open.
pub(super) fn families(role: Role) -> &'static [u64] {
    match role {
        Role::Decoder | Role::Core => &[],
        // The resolver uses routing netlink to ask which address families
        // the host has (seccomp refuses every other netlink protocol). No
        // local-domain socket (decision 86): a path socket reaches the
        // session bus, where systemd starts any program unconfined. glibc's
        // name service modules that use one (nscd, nss-resolve, mymachines)
        // then fail with EPERM, which glibc takes as "unavailable", and
        // names are resolved by its `dns` module over UDP.
        Role::Network | Role::Updater { .. } => &[libc::AF_INET as u64, libc::AF_INET6 as u64, libc::AF_NETLINK as u64],
        // The display, the sound server and the input method are
        // local-domain sockets, reached in the helper's own `/run` and
        // `/tmp`, which hold only them (`runtime`).
        Role::Interface | Role::Sound => &[libc::AF_UNIX as u64],
    }
}

/// The paths a role may read, of those that exist.
pub(super) fn readable(role: Role) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = match role {
        Role::Decoder | Role::Core => Vec::new(),
        Role::Network | Role::Updater { .. } => RESOLVER_PATHS.iter().map(PathBuf::from).collect(),
        Role::Sound => {
            let mut paths: Vec<PathBuf> = SOUND_PATHS.iter().map(PathBuf::from).collect();
            if let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute()) {
                paths.extend(SOUND_HOME_PATHS.iter().map(|path| home.join(path)));
            }
            if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|path| path.is_absolute()) {
                paths.push(runtime);
            }
            paths
        }
        Role::Interface => {
            let mut paths: Vec<PathBuf> = INTERFACE_PATHS.iter().map(PathBuf::from).collect();
            if let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute()) {
                paths.extend(INTERFACE_HOME_PATHS.iter().map(|path| home.join(path)));
            }
            if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|path| path.is_absolute()) {
                paths.push(runtime);
            }
            if let Some(authority) = std::env::var_os("XAUTHORITY").map(PathBuf::from).filter(|path| path.is_absolute()) {
                paths.push(authority);
            }
            paths
        }
    };
    paths.retain(|path| path.exists());
    paths
}
