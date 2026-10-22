//! Days and times as history shows them (`history-and-bookmarks.md`
//! section 7): the local day of a moment, a day written as `2026-10-22`
//! and read back strictly, and a day and a time in words. The civil
//! calendar's arithmetic is Howard Hinnant's `days_from_civil` and
//! `civil_from_days` (proleptic Gregorian), in checked integers.

/// Seconds in a day.
pub const SECONDS_PER_DAY: i64 = 86_400;

/// Seconds since 1970, UTC, now; 0 if the clock is before 1970.
#[must_use]
pub fn utc_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().and_then(|since| i64::try_from(since.as_secs()).ok()).unwrap_or(0)
}
const SECONDS_PER_HOUR: i64 = 3_600;
const SECONDS_PER_MINUTE: i64 = 60;
/// Days in a 400-year era of the Gregorian calendar.
const DAYS_PER_ERA: i64 = 146_097;
/// Days from 0000-03-01 to 1970-01-01.
const EPOCH_SHIFT: i64 = 719_468;
/// 1970-01-01 was a Thursday: its index in [`WEEKDAYS`].
const EPOCH_WEEKDAY: i64 = 3;
/// The years a day may be in: from the epoch to the last four-digit year.
const FIRST_YEAR: i64 = 1970;
const LAST_YEAR: i64 = 9999;

const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// The local day, in days since 1970-01-01, of `utc` seconds since 1970
/// in a zone `offset` seconds ahead of UTC.
#[must_use]
pub fn local_day(utc: i64, offset: i32) -> i64 {
    utc.saturating_add(i64::from(offset)).div_euclid(SECONDS_PER_DAY)
}

/// The day `year-month-day` as days since 1970-01-01, if it is a real day
/// of a year from 1970 to 9999.
#[must_use]
pub fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(FIRST_YEAR..=LAST_YEAR).contains(&year) || !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let (month, day) = (i64::from(month), i64::from(day));
    let year = if month <= 2 { year.checked_sub(1)? } else { year };
    let era = year.checked_div(400)?;
    let year_of_era = year.checked_sub(era.checked_mul(400)?)?;
    let shifted_month = if month > 2 { month.checked_sub(3)? } else { month.checked_add(9)? };
    let day_of_year = shifted_month.checked_mul(153)?.checked_add(2)?.checked_div(5)?.checked_add(day)?.checked_sub(1)?;
    let day_of_era =
        year_of_era.checked_mul(365)?.checked_add(year_of_era.checked_div(4)?)?.checked_sub(year_of_era.checked_div(100)?)?.checked_add(day_of_year)?;
    era.checked_mul(DAYS_PER_ERA)?.checked_add(day_of_era)?.checked_sub(EPOCH_SHIFT)
}

/// The year, month and day of `days` since 1970-01-01.
#[must_use]
pub fn civil_from_days(days: i64) -> Option<(i64, u32, u32)> {
    let z = days.checked_add(EPOCH_SHIFT)?;
    let era = z.div_euclid(DAYS_PER_ERA);
    let day_of_era = z.rem_euclid(DAYS_PER_ERA);
    let year_of_era = day_of_era
        .checked_sub(day_of_era.checked_div(1460)?)?
        .checked_add(day_of_era.checked_div(36524)?)?
        .checked_sub(day_of_era.checked_div(DAYS_PER_ERA.checked_sub(1)?)?)?
        .checked_div(365)?;
    let day_of_year =
        day_of_era.checked_sub(year_of_era.checked_mul(365)?.checked_add(year_of_era.checked_div(4)?)?.checked_sub(year_of_era.checked_div(100)?)?)?;
    let shifted_month = day_of_year.checked_mul(5)?.checked_add(2)?.checked_div(153)?;
    let day = day_of_year.checked_sub(shifted_month.checked_mul(153)?.checked_add(2)?.checked_div(5)?)?.checked_add(1)?;
    let month = if shifted_month < 10 { shifted_month.checked_add(3)? } else { shifted_month.checked_sub(9)? };
    let year = year_of_era.checked_add(era.checked_mul(400)?)?.checked_add(i64::from(month <= 2))?;
    Some((year, u32::try_from(month).ok()?, u32::try_from(day).ok()?))
}

fn days_in_month(year: i64, month: u32) -> u32 {
    let leap = year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A day written as `2026-10-22`.
#[must_use]
pub fn day_text(days: i64) -> String {
    civil_from_days(days).map_or_else(String::new, |(year, month, day)| format!("{year:04}-{month:02}-{day:02}"))
}

/// A day read from `2026-10-22`, strictly: four digits, two, two, each a
/// real date from 1970 to 9999.
#[must_use]
pub fn read_day(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = text.get(range)?;
        if part.bytes().all(|byte| byte.is_ascii_digit()) { part.parse().ok() } else { None }
    };
    days_from_civil(i64::from(number(0..4)?), number(5..7)?, number(8..10)?)
}

/// A day in words, as `Wednesday 22 October 2026`.
#[must_use]
pub fn day_words(days: i64) -> String {
    let Some((year, month, day)) = civil_from_days(days) else { return String::new() };
    let weekday = usize::try_from(days.checked_add(EPOCH_WEEKDAY).unwrap_or(0).rem_euclid(7)).ok().and_then(|index| WEEKDAYS.get(index)).copied().unwrap_or("");
    let month = usize::try_from(month).ok().and_then(|month| month.checked_sub(1)).and_then(|index| MONTHS.get(index)).copied().unwrap_or("");
    format!("{weekday} {day} {month} {year}")
}

/// The local time of day of `utc` in a zone `offset` seconds ahead, as
/// `14:05`.
#[must_use]
pub fn time_words(utc: i64, offset: i32) -> String {
    let seconds = utc.saturating_add(i64::from(offset)).rem_euclid(SECONDS_PER_DAY);
    let hours = seconds.checked_div(SECONDS_PER_HOUR).unwrap_or(0);
    let minutes = seconds.rem_euclid(SECONDS_PER_HOUR).checked_div(SECONDS_PER_MINUTE).unwrap_or(0);
    format!("{hours:02}:{minutes:02}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn days_and_dates_agree_both_ways() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        assert_eq!(days_from_civil(2026, 10, 22), Some(20_748));
        assert_eq!(civil_from_days(20_748), Some((2026, 10, 22)));
        assert_eq!(days_from_civil(2000, 2, 29).and_then(civil_from_days), Some((2000, 2, 29)));
        for days in (0..2_932_896).step_by(997) {
            let (year, month, day) = civil_from_days(days).unwrap();
            assert_eq!(days_from_civil(year, month, day), Some(days), "{year}-{month}-{day}");
        }
    }

    #[test]
    fn a_day_is_read_strictly() {
        assert_eq!(read_day("2026-10-22"), Some(20_748));
        assert_eq!(day_text(20_748), "2026-10-22");
        for bad in ["2026-02-29", "1900-02-28", "2026-13-01", "2026-1-22", "2026-10-22 ", "+026-10-22", "2026-10-00", "10000-01-01", ""] {
            assert_eq!(read_day(bad), None, "{bad}");
        }
        assert_eq!(read_day("2024-02-29"), Some(19_782), "a leap day");
    }

    #[test]
    fn days_and_times_read_as_words() {
        assert_eq!(day_words(20_748), "Thursday 22 October 2026");
        assert_eq!(day_words(0), "Thursday 1 January 1970");
        // 2026-10-22 03:05 UTC is 14:05 in Sydney, eleven hours ahead.
        let utc = 20_748 * SECONDS_PER_DAY + 3 * SECONDS_PER_HOUR + 5 * SECONDS_PER_MINUTE;
        assert_eq!(time_words(utc, 11 * 3_600), "14:05");
        assert_eq!(local_day(utc - 4 * SECONDS_PER_HOUR, 11 * 3_600), 20_748, "Sydney's day began before UTC's");
        assert_eq!(local_day(utc - 4 * SECONDS_PER_HOUR, 0), 20_747);
    }
}
