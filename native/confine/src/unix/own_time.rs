//! The processor time this process has used, from the kernel's own
//! account of it (`clock_gettime(2)`, `CLOCK_PROCESS_CPUTIME_ID`): every
//! thread's time on a processor, and none of the time it was ready to
//! run and had no processor. A script host counts its scripts' time
//! limits against it, so that a busy machine does not stop a script for
//! time it never had (DEC-7357 as revised, TODO-7409).

#![expect(unsafe_code, reason = "clock_gettime for the process's own processor time, which the standard library does not offer")]

use std::time::Duration;

/// The processor time this process has used since it started; none when
/// the kernel does not say.
pub(crate) fn processor_time() -> Option<Duration> {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `clock_gettime` writes the one `timespec` it is given, which
    // lives for the call.
    if unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &raw mut time) } != 0 {
        return None;
    }
    Some(Duration::new(u64::try_from(time.tv_sec).ok()?, u32::try_from(time.tv_nsec).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The account is of work done and not of time passed: it grows by
    /// about the time a thread spins, and by next to nothing while every
    /// thread sleeps.
    #[test]
    fn the_process_time_counts_work_and_not_sleep() {
        const SPUN: Duration = Duration::from_millis(60);
        const SLEPT: Duration = Duration::from_millis(200);
        let before = processor_time().unwrap();
        std::thread::sleep(SLEPT);
        let asleep = processor_time().unwrap().saturating_sub(before);
        let began = std::time::Instant::now();
        let before = processor_time().unwrap();
        while processor_time().unwrap().saturating_sub(before) < SPUN {
            std::hint::spin_loop();
        }
        // Other tests of this program run beside it and are counted too,
        // so only what cannot be otherwise is held: the sleep itself did
        // not count as work, and the work took at least its time.
        assert!(asleep < SLEPT, "{asleep:?} counted over a sleep of {SLEPT:?}");
        assert!(began.elapsed() * 64 >= SPUN, "{SPUN:?} of work in {:?}", began.elapsed());
    }
}
