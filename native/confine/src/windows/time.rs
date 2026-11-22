//! The local time zone's offset, by Windows's own rules (decision 57), and
//! this process's own processor time.

#![expect(unsafe_code, reason = "the Win32 calls that create a confined process (requirement 10.12, decision 52)")]

use std::io;

use windows_sys::Win32::System::SystemServices::{TIME_ZONE_ID_DAYLIGHT, TIME_ZONE_ID_STANDARD};
use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_ID_INVALID, TIME_ZONE_INFORMATION};

use super::handles::last_error;

/// Seconds in a minute: Windows gives a zone's bias in minutes.
const SECONDS_PER_MINUTE: i32 = 60;

/// The local time zone's offset from UTC now, in seconds: the negated bias
/// `GetTimeZoneInformation` gives, with the daylight or standard bias in
/// force now added to it.
pub(crate) fn local_offset() -> io::Result<i32> {
    // SAFETY: an all-zero TIME_ZONE_INFORMATION is a valid value of a plain
    // C struct, and the call writes into the one it is given.
    let mut information: TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the call writes into the live local it is given.
    let zone = unsafe { GetTimeZoneInformation(&raw mut information) };
    let bias = match zone {
        TIME_ZONE_ID_INVALID => return Err(last_error()),
        TIME_ZONE_ID_DAYLIGHT => information.Bias.saturating_add(information.DaylightBias),
        TIME_ZONE_ID_STANDARD => information.Bias.saturating_add(information.StandardBias),
        _ => information.Bias,
    };
    Ok(bias.saturating_mul(SECONDS_PER_MINUTE).saturating_neg())
}

/// The processor time this process has used since it started, kernel and
/// user together (`GetProcessTimes`, each a count of 100 nanoseconds);
/// none when Windows does not say.
pub(crate) fn processor_time() -> Option<std::time::Duration> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    /// Nanoseconds in one count of a `FILETIME`.
    const NANOSECONDS_PER_COUNT: u64 = 100;
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handle of this process needs no closing, and the
    // call writes the four live locals it is given.
    let read = unsafe { GetProcessTimes(GetCurrentProcess(), &raw mut created, &raw mut exited, &raw mut kernel, &raw mut user) };
    if read == 0 {
        return None;
    }
    let counts = |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Some(std::time::Duration::from_nanos(counts(kernel).saturating_add(counts(user)).saturating_mul(NANOSECONDS_PER_COUNT)))
}
