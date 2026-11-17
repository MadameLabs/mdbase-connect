//! Dates and datetimes of the task semantics, as the suite reads them.
//!
//! A date is `YYYY-MM-DD` on the calendar (leap years count). A datetime is a date,
//! `T`, a time `HH:MM` or `HH:MM:SS` with an optional fraction, and an offset (`Z`
//! or `±HH:MM`). Comparisons and "the day of" a value use the date as written;
//! only the UTC reading of an instant (`utc_date`) moves it to another day.
//!
//! Portable: no I/O, no clock.

use crate::TaskError;

/// A calendar date (`YYYY-MM-DD`) as written, after validation.
pub fn calendar_date(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !b
        .iter()
        .enumerate()
        .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    let year = num(&b[0..4]);
    let month = num(&b[5..7]);
    let day = num(&b[8..10]);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(s)
}

/// The year, month and day of a calendar date that exists.
pub(crate) fn ymd(s: &str) -> Option<(i64, i64, i64)> {
    let date = calendar_date(s)?;
    Some((
        num(date[0..4].as_bytes()),
        num(date[5..7].as_bytes()),
        num(date[8..10].as_bytes()),
    ))
}

/// A datetime with an offset, parsed into its parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    /// Year, as written.
    pub year: i64,
    /// Month, 1 to 12.
    pub month: i64,
    /// Day of the month, as written.
    pub day: i64,
    /// Hour, 0 to 23, as written.
    pub hour: i64,
    /// Minute, 0 to 59, as written.
    pub minute: i64,
    /// Second, 0 to 59, as written.
    pub second: i64,
    /// Offset from UTC in minutes (`+05:30` is 330).
    pub offset: i64,
}

/// Parses a datetime with a `T` separator and an offset. Fractions are allowed.
pub fn datetime(s: &str) -> Option<DateTime> {
    let date = calendar_date(s.get(0..10)?)?;
    let rest = s.get(10..)?.strip_prefix('T')?;
    let (time, offset) = split_offset(rest)?;
    let b = time.as_bytes();
    if b.len() < 5
        || b[2] != b':'
        || !b[0..2].iter().all(u8::is_ascii_digit)
        || !b[3..5].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let hour = num(&b[0..2]);
    let minute = num(&b[3..5]);
    let mut second = 0;
    let tail = &time[5..];
    if !tail.is_empty() {
        let seconds = tail.strip_prefix(':')?;
        let (sec, frac) = match seconds.split_once('.') {
            Some((sec, frac)) => (sec, Some(frac)),
            None => (seconds, None),
        };
        if sec.len() != 2 || !sec.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        second = num(sec.as_bytes());
        if second > 59 {
            return None;
        }
        if let Some(frac) = frac {
            if frac.is_empty() || !frac.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
        }
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    let (y, m, d) = (
        num(date[0..4].as_bytes()),
        num(date[5..7].as_bytes()),
        num(date[8..10].as_bytes()),
    );
    Some(DateTime {
        year: y,
        month: m,
        day: d,
        hour,
        minute,
        second,
        offset,
    })
}

/// Splits `HH:MM[:SS[.fff]]` from its offset: `Z`, `+HH:MM` or `-HH:MM`.
fn split_offset(rest: &str) -> Option<(&str, i64)> {
    if let Some(time) = rest.strip_suffix('Z') {
        return Some((time, 0));
    }
    let at = rest.len().checked_sub(6)?;
    let (time, off) = rest.split_at(at);
    let ob = off.as_bytes();
    if !(ob[0] == b'+' || ob[0] == b'-') || ob[3] != b':' {
        return None;
    }
    if !ob[1..3].iter().all(u8::is_ascii_digit) || !ob[4..6].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let (h, m) = (num(&ob[1..3]), num(&ob[4..6]));
    if h > 23 || m > 59 {
        return None;
    }
    let minutes = h * 60 + m;
    Some((time, if ob[0] == b'-' { -minutes } else { minutes }))
}

/// The date of a datetime as written: its first ten characters.
pub fn written_date(dt_or_date: &str) -> Option<&str> {
    if let Some(d) = calendar_date(dt_or_date) {
        return Some(d);
    }
    datetime(dt_or_date)?;
    calendar_date(&dt_or_date[0..10])
}

/// The UTC date of a value: a date is its own day; a datetime is the day of the
/// instant it names.
pub fn utc_date(s: &str) -> Option<String> {
    if let Some(d) = calendar_date(s) {
        return Some(d.to_string());
    }
    let (y, m, d) = civil_from_days(datetime(s)?.unix_seconds().div_euclid(86_400));
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

impl DateTime {
    /// Seconds since 1970-01-01T00:00:00Z.
    pub fn unix_seconds(&self) -> i64 {
        let days = days_from_civil(self.year, self.month, self.day);
        (days * 86_400) + self.hour * 3600 + self.minute * 60 + self.second - self.offset * 60
    }

    /// ISO weekday, Monday 1 to Sunday 7, of the calendar date.
    pub fn iso_weekday(&self) -> i64 {
        iso_weekday(days_from_civil(self.year, self.month, self.day))
    }

    /// Day of the year, from 1.
    pub fn ordinal(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day) - days_from_civil(self.year, 1, 1) + 1
    }

    /// The ISO 8601 week number of the calendar date.
    pub fn iso_week(&self) -> i64 {
        let week = (self.ordinal() - self.iso_weekday() + 10) / 7;
        if week < 1 {
            weeks_in_year(self.year - 1)
        } else if week > weeks_in_year(self.year) {
            1
        } else {
            week
        }
    }
}

fn iso_weekday(days: i64) -> i64 {
    (days + 3).rem_euclid(7) + 1
}

fn weeks_in_year(y: i64) -> i64 {
    let jan1 = iso_weekday(days_from_civil(y, 1, 1));
    if jan1 == 4 || (is_leap(y) && jan1 == 3) {
        53
    } else {
        52
    }
}

pub(crate) fn num(b: &[u8]) -> i64 {
    b.iter().fold(0, |acc, c| acc * 10 + i64::from(c - b'0'))
}

pub(crate) fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub(crate) fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count since 1970-01-01.
pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Whether a value carries a time: `T` then `HH:MM`, anywhere in it.
pub fn has_time(value: &str) -> bool {
    value.as_bytes().windows(6).any(|w| {
        w[0] == b'T'
            && w[1].is_ascii_digit()
            && w[2].is_ascii_digit()
            && w[3] == b':'
            && w[4].is_ascii_digit()
            && w[5].is_ascii_digit()
    })
}

/// The date an operation targets: the explicit date when given (it must exist), else
/// the first existing one of `scheduled` and `due`, else `today`. A value that is not
/// a date counts as absent, except the explicit one.
pub fn operation_target(
    explicit: Option<&str>,
    scheduled: Option<&str>,
    due: Option<&str>,
    today: &str,
) -> Result<String, TaskError> {
    if let Some(explicit) = explicit {
        return written_date(explicit)
            .map(str::to_string)
            .ok_or_else(|| TaskError::new("invalid_date", format!("Invalid date: {explicit:?}")));
    }
    Ok([scheduled, due]
        .into_iter()
        .flatten()
        .find_map(written_date)
        .unwrap_or(today)
        .to_string())
}
