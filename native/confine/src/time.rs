//! The time and the local time zone's offset, which the broker reads for
//! the confined core (decision 57).

use std::io;

/// The most a time zone is ahead of or behind UTC, in seconds: 18 hours,
/// the bound of ISO 8601's offsets and of every zone in use.
pub const MAX_OFFSET: i32 = 18 * 60 * 60;

/// The time, and the local time zone's offset from UTC then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// Seconds since 1970, UTC.
    pub utc: i64,
    /// Seconds the local time is ahead of UTC.
    pub offset: i32,
}

impl LocalTime {
    /// Whether the offset is one a zone can have.
    #[must_use]
    pub fn is_plausible(&self) -> bool {
        (-MAX_OFFSET..=MAX_OFFSET).contains(&self.offset)
    }
}

/// The time now and the local offset, by the operating system's own rules
/// for the user's time zone. The broker reads them for the confined core,
/// which may not read the system's time-zone files (decision 56).
///
/// # Errors
/// When the clock or the zone cannot be read.
pub fn local_time() -> io::Result<LocalTime> {
    let utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)
        .and_then(|since| i64::try_from(since.as_secs()).map_err(io::Error::other))?;
    #[cfg(unix)]
    let offset = crate::unix::local_offset(utc)?;
    #[cfg(windows)]
    let offset = crate::windows::local_offset()?;
    #[cfg(not(any(unix, windows)))]
    let offset = 0;
    let time = LocalTime { utc, offset };
    if time.is_plausible() { Ok(time) } else { Err(io::Error::other("an offset no zone has")) }
}
