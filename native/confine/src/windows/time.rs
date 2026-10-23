//! The local time zone's offset, by Windows's own rules (decision 57).

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
