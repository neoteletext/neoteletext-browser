//! The number of files a process may hold open (`getrlimit(2)`,
//! `RLIMIT_NOFILE`). macOS starts a program with a soft limit of 256, and
//! the trusted core holds two pipes for every helper it keeps: a decoder and
//! a fetcher for each of its 64 tabs (`protocol::MAX_TABS`) is 256 alone, so
//! the core ran out at about 60 tabs, and a helper that could not start left
//! its tab without pages.

#![expect(unsafe_code, reason = "getrlimit and setrlimit, which the standard library does not offer")]

use std::io;

use crate::broker::MAX_HELPERS;

/// A helper's two pipes, as the core holds them (`descriptors::MAX_DESCRIPTORS`).
const DESCRIPTORS_PER_HELPER: libc::rlim_t = super::descriptors::MAX_DESCRIPTORS as libc::rlim_t;
/// The core's own beside its helpers: its standard streams, the broker's
/// channel and the files it keeps open. Five at rest in the hostile
/// harness on 3 November 2026; the rest is room.
const OWN_DESCRIPTORS: libc::rlim_t = 64;
/// What the trusted core needs for the most helpers the broker keeps alive.
pub const DESCRIPTORS_NEEDED: libc::rlim_t = MAX_HELPERS as libc::rlim_t * DESCRIPTORS_PER_HELPER + OWN_DESCRIPTORS;

/// Raises this process's soft limit on open files to [`DESCRIPTORS_NEEDED`],
/// or to its hard limit where that is lower, so that it and every process it
/// starts, which inherit it, have room for the most helpers; a limit already
/// higher is left as it is. Returns the soft limit now.
///
/// # Errors
/// When the limits cannot be read or set.
pub fn raise_descriptor_limit() -> io::Result<libc::rlim_t> {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: `getrlimit` writes the one `rlimit` it is given, which lives
    // for the call.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let wanted = DESCRIPTORS_NEEDED.min(limit.rlim_max);
    if limit.rlim_cur >= wanted {
        return Ok(limit.rlim_cur);
    }
    limit.rlim_cur = wanted;
    // SAFETY: `setrlimit` reads the one `rlimit` it is given; a soft limit
    // at or below the hard limit needs no privilege.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    // REQ-683: after raising, the soft limit holds the most helpers' pipes, or
    // is the hard limit where that is lower. On macOS, where a program starts
    // with 256, this is what changed.
    #[test]
    fn the_limit_on_open_files_holds_the_most_helpers() {
        let raised = raise_descriptor_limit().unwrap();
        let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: as in `raise_descriptor_limit`.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) }, 0);
        assert_eq!(limit.rlim_cur, raised);
        assert!(raised >= DESCRIPTORS_NEEDED.min(limit.rlim_max), "{raised} of {DESCRIPTORS_NEEDED}, hard {}", limit.rlim_max);
    }
}
