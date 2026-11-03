//! The first two layers: no new privileges, and an empty network
//! namespace, with a mount namespace of the helper's own; and the helper's
//! end with its parent's.

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

/// The helper is killed when its parent, the broker, ends, however it
/// ends (finding F-34): otherwise a helper of a broker that crashed lives
/// on, reparented, with its sandbox but with no one to end it. The signal
/// is sent when the thread that started the helper ends; the broker
/// starts every helper from its one serving thread. A parent that ended
/// before the signal was set would leave the helper reparented already,
/// so the parent is read before and after: a change means it ended. A
/// parent that ended even before the first read is seen by the number its
/// parent gave it at its start ([`crate::broker::PARENT_VARIABLE`]), which
/// is then not its parent's (TODO-1278 item 2).
pub(super) fn end_with_parent() -> Result<(), String> {
    // SAFETY: getppid takes no arguments.
    let parent = unsafe { libc::getppid() };
    // SAFETY: prctl with PR_SET_PDEATHSIG takes integer arguments only.
    let result = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: as above.
    let now = unsafe { libc::getppid() };
    let given = std::env::var(crate::broker::PARENT_VARIABLE).ok().and_then(|number| number.parse::<libc::pid_t>().ok());
    parent_unchanged(parent, now, given)
}

/// Whether the parent read before the signal was set, `before`, is still
/// the parent, `now`, and is the one its parent named, `given`, if any.
fn parent_unchanged(before: libc::pid_t, now: libc::pid_t, given: Option<libc::pid_t>) -> Result<(), String> {
    if now != before || given.is_some_and(|given| given != now) { Err(String::from("its parent ended before it was confined")) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::parent_unchanged;

    /// TODO-1278 item 2, finding F-34: a helper whose parent ended before
    /// it first read its parent is reparented already, so its parent is
    /// not the one that started it and named itself; it refuses to run.
    /// On the code before, only a change between the two reads was seen.
    #[test]
    fn a_parent_that_ended_before_the_first_look_is_seen() {
        const BROKER: libc::pid_t = 4_242;
        const ADOPTER: libc::pid_t = 1;
        assert!(parent_unchanged(BROKER, BROKER, Some(BROKER)).is_ok());
        assert!(parent_unchanged(BROKER, BROKER, None).is_ok(), "a helper started by no broker, as a test starts one");
        assert!(parent_unchanged(BROKER, ADOPTER, Some(BROKER)).is_err(), "it ended between the two reads");
        assert!(parent_unchanged(ADOPTER, ADOPTER, Some(BROKER)).is_err(), "it ended before the first read");
    }
}
