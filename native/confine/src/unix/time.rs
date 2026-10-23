//! The local time zone's offset, which the standard library does not
//! offer, from the C library's own `localtime_r` (decision 57).

#![expect(unsafe_code, reason = "tzset and localtime_r, which the standard library does not offer (decision 57)")]

use std::io;

// POSIX `<time.h>` (IEEE Std 1003.1-2017): sets the C library's time zone
// from `TZ`, or the system's default. The `libc` crate declares it only for
// Windows.
unsafe extern "C" {
    fn tzset();
}

/// The local time zone's offset from UTC at `utc` seconds since 1970, in
/// seconds, as the C library's `localtime_r` gives it (`tm_gmtoff`). The C
/// library reads the zone itself; `localtime_r` need not read it again
/// (POSIX), and glibc's does not, so `tzset` asks it to first: glibc then
/// reads `TZ`, or `/etc/localtime` again when it changed, and a change of
/// zone while the browser runs is seen at once.
#[allow(clippy::useless_conversion, reason = "time_t is 64 bits on the platforms built for, and 32 on some others")]
pub fn local_offset(utc: i64) -> io::Result<i32> {
    let time: libc::time_t = utc.try_into().map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: `tzset` takes no arguments and only sets the C library's own
    // zone state, which `localtime_r` then reads.
    unsafe { tzset() };
    // SAFETY: an all-zero `tm` is a valid value of a plain C struct.
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to live values of the types the call takes,
    // and the result is checked before `local` is read.
    let result = unsafe { libc::localtime_r(&raw const time, &raw mut local) };
    if result.is_null() {
        return Err(io::Error::last_os_error());
    }
    i32::try_from(local.tm_gmtoff).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;

    /// The local offset is one a zone can have, and the broker answers
    /// it (`history-and-bookmarks.md` section 7).
    #[test]
    fn the_local_offset_is_a_zones() {
        let time = crate::local_time().unwrap();
        assert!(time.is_plausible(), "{time:?}");
        assert!(time.utc > 1_700_000_000, "a clock after 2023: {time:?}");
    }

    /// A child of this test's process, with its own environment: set to run
    /// the body of `a_change_of_zone_is_seen_at_once`.
    const ZONE_CHILD: &str = "NEOTELETEXT_ZONE_CHILD";

    /// `history-and-bookmarks.md` section 14, item 1: a change of zone while
    /// the browser runs is seen at the next reading, not at the next start.
    /// The zone is changed in a child process that runs this test alone, so
    /// no other thread reads the environment while it changes.
    #[test]
    fn a_change_of_zone_is_seen_at_once() {
        // 1 October 2026: India keeps no summer time.
        const UTC: i64 = 1_790_812_800;
        const INDIA: i32 = 5 * 3600 + 30 * 60;
        if std::env::var_os(ZONE_CHILD).is_some() {
            assert_eq!(local_offset(UTC).unwrap(), 0, "TZ=UTC0");
            // SAFETY: this process runs this one test on one thread, and
            // nothing else of it reads the environment while it changes.
            unsafe { std::env::set_var("TZ", "IST-5:30") };
            assert_eq!(local_offset(UTC).unwrap(), INDIA, "the new zone, read at once");
            return;
        }
        let test = std::env::current_exe().unwrap();
        let status = std::process::Command::new(test)
            .args(["unix::time::tests::a_change_of_zone_is_seen_at_once", "--exact", "--test-threads=1"])
            .env(ZONE_CHILD, "1")
            .env("TZ", "UTC0")
            .status()
            .unwrap();
        assert!(status.success(), "the child saw the zone change: {status}");
    }
}
