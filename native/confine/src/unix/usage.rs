//! The processor time another process has used, on macOS
//! (`proc_pid_rusage(3)`, `RUSAGE_INFO_V2`): its user and system time,
//! which the kernel keeps in units of `mach_absolute_time`, converted to
//! microseconds by the machine's time base (`mach_timebase_info(3)`). On
//! Apple silicon a unit is 125/3 nanoseconds; on Intel, one (REQ-6626
//! item 1 a; DEC-6659 item 18).

#![expect(unsafe_code, reason = "proc_pid_rusage and mach_timebase_info, which the standard library does not offer (REQ-6626)")]

use std::io;

/// Nanoseconds in a microsecond.
const NANOSECONDS_PER_MICROSECOND: u128 = 1_000;

/// The microseconds the process `pid` has run on a processor, user and
/// system time together.
///
/// # Errors
/// When the kernel refuses: the process has ended, or is not this user's.
pub(crate) fn processor_time_us(pid: u32) -> io::Result<u64> {
    let pid = libc::c_int::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: the buffer is a live rusage_info_v2, the structure the
    // flavour RUSAGE_INFO_V2 asks the kernel to write.
    let result = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, usage.as_mut_ptr().cast()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel wrote the whole structure, and it was zeroed before.
    let usage = unsafe { usage.assume_init() };
    let (numer, denom) = time_base()?;
    let units = u128::from(usage.ri_user_time).saturating_add(u128::from(usage.ri_system_time));
    let nanoseconds = units.saturating_mul(u128::from(numer)).checked_div(u128::from(denom)).unwrap_or(0);
    Ok(u64::try_from(nanoseconds.checked_div(NANOSECONDS_PER_MICROSECOND).unwrap_or(0)).unwrap_or(u64::MAX))
}

/// The machine's time base, the nanoseconds of `numer / denom` in one unit
/// of `mach_absolute_time`.
#[expect(
    deprecated,
    reason = "libc marks its mach bindings deprecated in favour of the mach2 crate; the browser takes no crate for one call the C library it links already has"
)]
fn time_base() -> io::Result<(u32, u32)> {
    let mut base = libc::mach_timebase_info { numer: 0, denom: 0 };
    // SAFETY: the time base is written to a live local.
    if unsafe { libc::mach_timebase_info(&raw mut base) } != 0 || base.denom == 0 {
        return Err(io::Error::other("no time base"));
    }
    Ok((base.numer, base.denom))
}
