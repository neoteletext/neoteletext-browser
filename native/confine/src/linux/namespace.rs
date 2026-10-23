//! The first two layers: no new privileges, and an empty network
//! namespace, with a mount namespace of the helper's own.

#![expect(unsafe_code, reason = "the C library calls for the namespaces and no-new-privileges (requirement 10.12)")]

/// No new privileges: nothing started from here on can gain any, and
/// Landlock and seccomp require it.
pub(super) fn no_new_privileges() -> Result<(), String> {
    // SAFETY: prctl with PR_SET_NO_NEW_PRIVS takes integer arguments only
    // and touches no memory of ours.
    let result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().to_string()) }
}

/// An empty network namespace, if the kernel lets an unprivileged process
/// make one: the process is then alone on a network with nothing on it.
/// A mount namespace of its own comes with it, in which the interface and
/// the sound helper cover the system's sockets (decision 86, `runtime`).
/// The user namespace that makes that possible maps the user to
/// themselves, so the sound server and the display still see who is
/// connecting; unmapped, the process would be "nobody" to them, and
/// `PipeWire` drops such a client.
pub(super) fn network_namespace() -> Result<(), String> {
    // SAFETY: these take no pointers and read the process's own ids.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    // SAFETY: unshare takes flags only; the process has one thread here,
    // which CLONE_NEWUSER requires.
    let result = unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET | libc::CLONE_NEWNS) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // user_namespaces(7): setgroups must be denied before an unprivileged
    // process may write its own group map.
    std::fs::write("/proc/self/setgroups", "deny").map_err(|error| format!("setgroups: {error}"))?;
    std::fs::write("/proc/self/uid_map", format!("{uid} {uid} 1")).map_err(|error| format!("uid_map: {error}"))?;
    std::fs::write("/proc/self/gid_map", format!("{gid} {gid} 1")).map_err(|error| format!("gid_map: {error}"))?;
    Ok(())
}
