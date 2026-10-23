//! The interface's and the sound helper's own `/run` and `/tmp` (decision
//! 86, finding F-01): both may open local sockets, for the display and the
//! sound server, and Landlock up to version 6 does not stop a `connect` by
//! path, so without this either could reach the session bus, where systemd
//! starts any program unconfined, the system bus or an agent's socket. In
//! the mount namespace the helper made with its user namespace, `/run`,
//! `/tmp` and the runtime folder are covered by empty private folders, and
//! only the sockets the role needs are mounted back into them.

#![expect(unsafe_code, reason = "mount, open_tree and move_mount, which the standard library does not offer (requirement 10.12, decision 86)")]

use std::ffi::CString;
use std::fs::{self, DirBuilder, File};
use std::io;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

use crate::Role;

/// The folders covered, besides the runtime folder: where the system's and
/// the session's sockets live (the session bus, the system bus, agents'
/// sockets, the X server's).
const COVERED: [&str; 2] = ["/run", "/tmp"];
/// Each private folder's size: it holds only the folders and empty files
/// the sockets are mounted on, and the helper writes nothing there.
const PRIVATE_BYTES: usize = 64 * 1024;
/// Each private folder's most entries: a few sockets and their folders.
const PRIVATE_ENTRIES: usize = 64;
/// Only its owner may look in a private folder or a folder made in it.
const PRIVATE_MODE: u32 = 0o700;
/// The Wayland display when `WAYLAND_DISPLAY` is not set, as
/// `wl_display_connect` defaults to it (libwayland's `wayland-client.c`).
const WAYLAND_DEFAULT: &str = "wayland-0";
/// Where the X server's local sockets are, `X` and the display's number
/// (Xlib's and xcb's `/tmp/.X11-unix/X<n>`).
const X11_SOCKETS: &str = "/tmp/.X11-unix";
/// `PipeWire`'s socket in its runtime folder (`PW_DEFAULT_REMOTE`).
const PIPEWIRE_SOCKET: &str = "pipewire-0";
/// `PulseAudio`'s socket in the runtime folder, and in `PULSE_RUNTIME_PATH`.
const PULSE_SOCKET: &str = "native";
/// `PulseAudio`'s folder in the runtime folder.
const PULSE_FOLDER: &str = "pulse";
/// The prefix of a local socket in `PULSE_SERVER` (`pulse-client.conf(5)`).
const PULSE_LOCAL: &str = "unix:";
/// The longest X display number read from `DISPLAY`.
const MAX_DISPLAY_DIGITS: usize = 5;

/// An absolute path from the environment variable `name`.
fn absolute(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|path| path.is_absolute())
}

/// The X server's local socket for `DISPLAY`, `:n` or `unix:n`, with or
/// without a screen (`X(7)`); a display on another host has none.
fn x11_socket(display: &str) -> Option<PathBuf> {
    let (host, rest) = display.rsplit_once(':')?;
    if !(host.is_empty() || host == "unix") {
        return None;
    }
    let number = rest.split('.').next()?;
    if number.is_empty() || number.len() > MAX_DISPLAY_DIGITS || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(Path::new(X11_SOCKETS).join(format!("X{number}")))
}

/// The sockets and files `role` needs from the covered folders.
fn needed(role: Role) -> Vec<PathBuf> {
    let runtime = absolute("XDG_RUNTIME_DIR");
    let mut paths = Vec::new();
    match role {
        Role::Interface => {
            let wayland = std::env::var_os("WAYLAND_DISPLAY").filter(|name| !name.is_empty()).map_or_else(|| PathBuf::from(WAYLAND_DEFAULT), PathBuf::from);
            if wayland.is_absolute() {
                paths.push(wayland);
            } else if let Some(runtime) = &runtime {
                paths.push(runtime.join(wayland));
            }
            paths.extend(std::env::var("DISPLAY").ok().and_then(|display| x11_socket(&display)));
            // GNOME's Xwayland keeps its authority file in the runtime folder.
            paths.extend(absolute("XAUTHORITY"));
        }
        Role::Sound => {
            if let Some(folder) = absolute("PIPEWIRE_RUNTIME_DIR").or_else(|| runtime.clone()) {
                paths.push(folder.join(PIPEWIRE_SOCKET));
            }
            if let Some(folder) = absolute("PULSE_RUNTIME_PATH").or_else(|| runtime.as_ref().map(|runtime| runtime.join(PULSE_FOLDER))) {
                paths.push(folder.join(PULSE_SOCKET));
            }
            paths.extend(
                std::env::var("PULSE_SERVER").ok().and_then(|server| server.strip_prefix(PULSE_LOCAL).map(PathBuf::from)).filter(|path| path.is_absolute()),
            );
        }
        Role::Decoder | Role::Network | Role::Core | Role::Updater { .. } => {}
    }
    paths
}

/// The folders to cover: `/run`, `/tmp` and the runtime folder where it is
/// elsewhere, each as it is reached, links resolved.
fn covered() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = COVERED.iter().filter_map(|folder| fs::canonicalize(folder).ok()).collect();
    if let Some(runtime) = absolute("XDG_RUNTIME_DIR").and_then(|runtime| fs::canonicalize(runtime).ok())
        && runtime.is_dir()
        && !folders.iter().any(|folder| runtime.starts_with(folder))
    {
        folders.push(runtime);
    }
    folders
}

/// `path` as the C string the system calls take.
fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// The result of a system call that returns -1 and sets `errno`.
fn checked(result: libc::c_long) -> io::Result<libc::c_long> {
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result) }
}

/// `mount(2)` with no source data but `data`.
fn mount(source: &str, target: &Path, kind: &str, flags: libc::c_ulong, data: Option<&str>) -> io::Result<()> {
    let source = CString::new(source).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let target = c_path(target)?;
    let kind = CString::new(kind).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let data = data.map(CString::new).transpose().map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let data_pointer = data.as_ref().map_or(std::ptr::null(), |data| data.as_ptr().cast());
    // SAFETY: every pointer is to a NUL-terminated string alive for the
    // call, or null where mount(2) allows it (the data).
    let result = unsafe { libc::mount(source.as_ptr(), target.as_ptr(), kind.as_ptr(), flags, data_pointer) };
    checked(libc::c_long::from(result)).map(drop)
}

/// A detached copy of the mount of `path` alone (`open_tree(2)` with
/// `OPEN_TREE_CLONE`), to be put back after its folder is covered.
fn capture(path: &Path) -> io::Result<OwnedFd> {
    let name = c_path(path)?;
    let flags = libc::OPEN_TREE_CLONE | libc::OPEN_TREE_CLOEXEC;
    // SAFETY: open_tree takes a descriptor, a NUL-terminated path alive for
    // the call, and flags.
    let result = checked(unsafe { libc::syscall(libc::SYS_open_tree, libc::AT_FDCWD, name.as_ptr(), flags) })?;
    let descriptor = RawFd::try_from(result).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    // SAFETY: the kernel just returned this descriptor, which nothing else
    // holds.
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
}

/// Mounts the captured `tree` on `target` (`move_mount(2)`).
fn place(tree: &OwnedFd, target: &Path) -> io::Result<()> {
    let target = c_path(target)?;
    let empty = c"";
    // SAFETY: move_mount takes descriptors, NUL-terminated paths alive for
    // the call (the source's empty, as MOVE_MOUNT_F_EMPTY_PATH asks), and
    // flags.
    let result =
        unsafe { libc::syscall(libc::SYS_move_mount, tree.as_raw_fd(), empty.as_ptr(), libc::AT_FDCWD, target.as_ptr(), libc::MOVE_MOUNT_F_EMPTY_PATH) };
    checked(result).map(drop)
}

/// Covers `/run`, `/tmp` and the runtime folder with empty private
/// folders, and mounts back the sockets `role` needs, each where it was.
/// A path that does not exist is not needed; any mount that fails is the
/// error, and the helper does not run (decision 58).
pub(super) fn private_runtime(role: Role) -> Result<(), String> {
    let covered = covered();
    // The helper's mounts stay its own, and none of the system's later
    // ones reach it.
    mount("none", Path::new("/"), "", libc::MS_REC | libc::MS_PRIVATE, None).map_err(|error| format!("cannot make its mounts private: {error}"))?;
    let mut captured: Vec<(PathBuf, OwnedFd)> = Vec::new();
    for path in needed(role) {
        // Only a socket or a file is put back: a folder would bring back
        // whatever it holds.
        let Ok(path) = fs::canonicalize(&path) else { continue };
        if path.is_dir() {
            continue;
        }
        if !covered.iter().any(|folder| path.starts_with(folder) && path != *folder) || captured.iter().any(|(seen, _)| *seen == path) {
            continue;
        }
        let tree = capture(&path).map_err(|error| format!("cannot keep {}: {error}", path.display()))?;
        captured.push((path, tree));
    }
    let options = format!("size={PRIVATE_BYTES},nr_inodes={PRIVATE_ENTRIES},mode={PRIVATE_MODE:o}");
    for folder in &covered {
        mount("tmpfs", folder, "tmpfs", libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC, Some(&options))
            .map_err(|error| format!("cannot cover {}: {error}", folder.display()))?;
    }
    for (path, tree) in &captured {
        if let Some(parent) = path.parent() {
            DirBuilder::new().recursive(true).mode(PRIVATE_MODE).create(parent).map_err(|error| format!("cannot make {}: {error}", parent.display()))?;
        }
        File::create(path).map_err(|error| format!("cannot make {}: {error}", path.display()))?;
        place(tree, path).map_err(|error| format!("cannot put back {}: {error}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// `X(7)`: a local display's socket, with or without its screen; none
    /// for a display on another host or a malformed one.
    #[test]
    fn a_local_display_names_its_socket() {
        assert_eq!(x11_socket(":0").unwrap(), Path::new("/tmp/.X11-unix/X0"));
        assert_eq!(x11_socket(":99.0").unwrap(), Path::new("/tmp/.X11-unix/X99"));
        assert_eq!(x11_socket("unix:1").unwrap(), Path::new("/tmp/.X11-unix/X1"));
        for elsewhere in ["host:0", "", ":", ":x", ":123456", ":0/../../etc"] {
            assert_eq!(x11_socket(elsewhere), None, "{elsewhere}");
        }
    }
}
