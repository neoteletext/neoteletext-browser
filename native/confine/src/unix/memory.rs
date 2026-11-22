//! The operating system's limit on a process's memory (`setrlimit(2)`),
//! which a script host sets on itself before it reads anything
//! (`script-api.md` sections 10.6 and 11.1, DEC-7266): the WebAssembly
//! container bounds what a script's own memory may grow to, and this
//! bounds the whole process around it, whatever a fault in the
//! interpreter or the host lets a script make it hold.
//!
//! - **Linux:** `RLIMIT_DATA`, which since Linux 4.7 counts the heap and
//!   every private writable mapping, so every byte the process can write
//!   that is its own. Address space merely reserved is not counted, which
//!   is why it is this limit and not `RLIMIT_AS`: the C library reserves
//!   64 MiB of addresses for each thread's allocator, written or not, and
//!   a limit on addresses would count the watchdog thread's reservation
//!   as memory held.
//! - **macOS:** `RLIMIT_DATA` and `RLIMIT_AS` both. Whether its kernel
//!   holds mappings to either is not verified (TODO-7292).
//!
//! Both the soft and the hard limit are set, and a hard limit is raised
//! only with a privilege no helper has, in its own user namespace or out
//! of it (`capable(CAP_SYS_RESOURCE)` is asked of the first namespace).

#![expect(unsafe_code, reason = "setrlimit, which the standard library does not offer")]

use std::io;

/// The limits that hold a process's own memory on this platform.
#[cfg(target_os = "linux")]
const MEMORY_LIMITS: &[libc::__rlimit_resource_t] = &[libc::RLIMIT_DATA];
#[cfg(not(target_os = "linux"))]
const MEMORY_LIMITS: &[libc::c_int] = &[libc::RLIMIT_DATA, libc::RLIMIT_AS];

/// Holds this process's memory to `bytes` from now on, for good.
///
/// # Errors
/// When a limit cannot be set: the process then holds no such limit and
/// must end (decision 58).
pub(crate) fn limit_memory(bytes: usize) -> io::Result<()> {
    let bytes = libc::rlim_t::try_from(bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let limit = libc::rlimit { rlim_cur: bytes, rlim_max: bytes };
    for &resource in MEMORY_LIMITS {
        // SAFETY: `setrlimit` reads the one `rlimit` it is given, which
        // lives for the call; lowering a limit needs no privilege.
        if unsafe { libc::setrlimit(resource, &raw const limit) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Section 10.6: the figure is the specification's 134,217,728.
    #[test]
    fn the_script_hosts_memory_is_the_specifications_figure() {
        assert_eq!(crate::SCRIPT_MEMORY_BYTES, 134_217_728);
    }
}
