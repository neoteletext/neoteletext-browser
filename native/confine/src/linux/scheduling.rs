//! How Linux schedules the browser's threads: a short slice, so that a
//! thread woken by a key, a sound message or a frame runs at once rather
//! than after whatever else holds its processor (neoteletext-common's
//! `docs/page-rules.md` RULE-64 and RULE-65; DEC-6753).
//!
//! Linux's fair scheduler (EEVDF, since Linux 6.6) gives each thread a
//! slice, 2.8 milliseconds on the timed test's machine (`se.slice` in
//! `/proc/PID/sched`), and lets the thread on a processor run to the end
//! of it before a thread just woken there may run. The timed test measured
//! that as the browser's hand-offs under load: each wake of the thread a
//! key or a sound message was written to waited up to 2 to 4 milliseconds,
//! runnable and not running. Since Linux 6.12 a thread may ask for a
//! shorter slice of its own (`sched_setattr(2)`, `sched_runtime` with
//! `SCHED_OTHER`, kept between 0.1 and 100 milliseconds by the kernel),
//! and one woken with a shorter slice than the running thread's displaces
//! it (the scheduler's `PREEMPT_SHORT`) at the next scheduling point, at
//! most a timer tick away. It needs no privilege and changes no share of
//! the processor: the slice says how soon a thread runs, not how much.

#![expect(unsafe_code, reason = "sched_setattr, getpriority and getrusage, which neither the standard library nor libc's safe items offer")]

use std::io;

/// The slice asked for, in nanoseconds: shorter than any one wake's work
/// on the way of a key (at most 0.3 milliseconds of processor time per hop
/// in the timed test of 19 November 2026), so a woken thread's deadline
/// comes before a busy one's, and above the kernel's least, 0.1
/// milliseconds (`kernel/sched/syscalls.c`).
pub const SLICE_NANOSECONDS: u64 = 200_000;

/// The structure's size as the kernel's first version of it has it
/// (`SCHED_ATTR_SIZE_VER0`), which is libc's `sched_attr`.
const SCHED_ATTR_SIZE: u32 = 48;
const _: () = assert!(size_of::<libc::sched_attr>() == SCHED_ATTR_SIZE as usize, "libc's sched_attr is the kernel's first version");

/// Asks for [`SLICE_NANOSECONDS`] for the calling thread, keeping its
/// policy, `SCHED_OTHER`, and its nice value, so that the threads and
/// processes it starts afterwards inherit both (`sched(7)`: a child
/// inherits its parent's scheduling attributes through `fork` and
/// `execve`).
///
/// # Errors
/// When the kernel refuses: one older than 6.12 ignores the slice and
/// says nothing, and one that refuses `sched_setattr` gives its error.
pub fn ask_for_short_slices() -> io::Result<()> {
    // getpriority returns the nice value, which may be -1, so the error is
    // told apart by errno (getpriority(2)).
    clear_errno();
    // SAFETY: no pointer is passed; PRIO_PROCESS with 0 is the caller.
    let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
    if nice == -1 && io::Error::last_os_error().raw_os_error().is_some_and(|code| code != 0) {
        return Err(io::Error::last_os_error());
    }
    let policy = u32::try_from(libc::SCHED_OTHER).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let attributes = libc::sched_attr {
        size: SCHED_ATTR_SIZE,
        sched_policy: policy,
        sched_flags: 0,
        sched_nice: nice,
        sched_priority: 0,
        sched_runtime: SLICE_NANOSECONDS,
        sched_deadline: 0,
        sched_period: 0,
    };
    // SAFETY: the attributes are a live sched_attr of the size given in
    // it; pid 0 is the calling thread, and the flags argument is 0.
    let result = unsafe { libc::syscall(libc::SYS_sched_setattr, 0, &raw const attributes, 0) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Sets errno to 0, for a call whose result cannot tell an error.
fn clear_errno() {
    // SAFETY: __errno_location gives the calling thread's own errno.
    unsafe { *libc::__errno_location() = 0 };
}

/// Microseconds in a second, for a `timeval`.
#[cfg(feature = "timed-test")]
const MICROSECONDS_PER_SECOND: u64 = 1_000_000;

/// The calling thread's number, its processor time in microseconds, and
/// its voluntary and involuntary context switches (`gettid(2)`,
/// `getrusage(2)` with `RUSAGE_THREAD`): for the timed test's marks
/// (RULE-66), which tell a hop's time apart as work, a wait to run or a
/// wait for input, in every process, confined or not.
#[cfg(feature = "timed-test")]
#[must_use]
pub fn thread_usage() -> Option<[u64; 4]> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: the buffer is a live rusage, which the call writes whole.
    if unsafe { libc::getrusage(libc::RUSAGE_THREAD, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the kernel wrote it, and it was zeroed before.
    let usage = unsafe { usage.assume_init() };
    // SAFETY: gettid takes nothing and cannot fail.
    let thread = u64::try_from(unsafe { libc::gettid() }).ok()?;
    let microseconds =
        |time: libc::timeval| u64::try_from(time.tv_sec).ok()?.checked_mul(MICROSECONDS_PER_SECOND)?.checked_add(u64::try_from(time.tv_usec).ok()?);
    let ran = microseconds(usage.ru_utime)?.checked_add(microseconds(usage.ru_stime)?)?;
    Some([thread, ran, u64::try_from(usage.ru_nvcsw).ok()?, u64::try_from(usage.ru_nivcsw).ok()?])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// DEC-6753: an unprivileged thread is given the short slice, and a
    /// thread it starts inherits it (the kernel's `se.slice`, in
    /// nanoseconds, in `/proc/thread-self/sched`).
    #[test]
    fn the_short_slice_is_given_and_inherited() {
        let slice = || {
            let sched = std::fs::read_to_string("/proc/thread-self/sched").unwrap();
            sched.lines().find(|line| line.starts_with("se.slice")).and_then(|line| line.split(':').nth(1)).unwrap().trim().parse::<u64>().unwrap()
        };
        std::thread::spawn(move || {
            ask_for_short_slices().unwrap();
            assert_eq!(slice(), SLICE_NANOSECONDS);
            assert_eq!(std::thread::spawn(slice).join().unwrap(), SLICE_NANOSECONDS, "inherited");
        })
        .join()
        .unwrap();
    }
}
