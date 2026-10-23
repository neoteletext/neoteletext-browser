//! A helper's self-confinement on Linux, in the order that lets each
//! layer be set up before the next forbids setting things up:
//!
//! 1. No new privileges, so no program it might start could gain any.
//! 2. For a helper with no network, an empty network namespace: nothing to
//!    connect to at all; and for the interface and the sound helper, which
//!    open local sockets, their own `/run` and `/tmp` holding only the
//!    sockets they need (decision 86).
//! 3. Landlock, all of what [`landlock::LANDLOCK_ABI`] can say: the directories the
//!    helper may read (none for a decoder) and no TCP for one with no
//!    network.
//! 4. A seccomp system-call filter: no starting programs, no tracing, no
//!    mounting, no kernel interfaces a helper has no use for, and sockets
//!    only of the families its role needs. A denylist rather than an
//!    allowlist, so a new C library or kernel release cannot stop a helper
//!    from running.
//!
//! Every layer is required (decision 58: security is never weakened or
//! downgraded): a machine that cannot give one, such as a kernel without
//! that Landlock, a distribution that forbids unprivileged user
//! namespaces, or an architecture with no filter, gets an error and no
//! process, never a process with less.

use std::path::Path;

use crate::{Report, Role};

mod landlock;
mod namespace;
mod paths;
pub mod probe;
mod runtime;
mod seccomp;

/// Confines the calling process as `role` says, every layer or none: the
/// first that cannot be applied is the error, and the process must end
/// (decision 58).
pub(crate) fn apply(role: Role, writable: Option<&Path>) -> Result<Report, String> {
    let mut report = Report::default();
    namespace::no_new_privileges().map_err(|error| format!("cannot refuse new privileges: {error}"))?;
    if !matches!(role, Role::Network | Role::Updater { .. }) {
        namespace::network_namespace().map_err(|error| format!("cannot make an empty network namespace: {error}"))?;
        report.layers.push(String::from("network namespace"));
    }
    if matches!(role, Role::Interface | Role::Sound) {
        runtime::private_runtime(role).map_err(|error| format!("cannot make its own /run and /tmp: {error}"))?;
        report.layers.push(String::from("private /run and /tmp"));
    }
    landlock::landlock(role, writable).map_err(|error| format!("cannot apply Landlock: {error}"))?;
    report.layers.push(String::from("Landlock"));
    let arch = seccomp::architecture().ok_or("no system-call filter exists for this architecture")?;
    seccomp::seccomp(role, arch).map_err(|error| format!("cannot apply the system-call filter: {error}"))?;
    report.layers.push(String::from("seccomp"));
    Ok(report)
}
