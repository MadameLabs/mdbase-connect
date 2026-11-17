//! The person's timer on a task, and the correction of a closed time entry.
//!
//! A time entry is `{startTime, endTime?, description?, duration?}`. The person
//! running it is stored in `description` as `pessoa: <person>` (`PERSON_PREFIX`),
//! because the entry schema allows no other field. An entry without that prefix has
//! no owner: a stop by any person closes it only when that person has no open entry
//! of their own.
//!
//! Times are ISO 8601 strings. Callers pass `now`; nothing here reads a clock.
//! Entries keep the strings they were written with, except a retimed entry, whose
//! times are written back in UTC with milliseconds, as the adapter writes them.

use serde_json::{Map, Value, json};

use crate::plan::{check_status, list_of, status_fields, update};
use crate::{Binding, TaskError, TaskView};

/// Prefix that stores the person of an entry in its `description`.
pub const PERSON_PREFIX: &str = "pessoa: ";

const IN_PROGRESS: &str = "em-andamento";

/// An entry still running. `person` is empty when the entry has no owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenEntry {
    /// The person of the entry, without the prefix; empty for an ownerless entry.
    pub person: String,
    /// The entry's start, as stored.
    pub start: String,
}

/// The entries of the task that have no end yet.
pub fn open_entries(b: &Binding, t: &TaskView) -> Vec<OpenEntry> {
    entries_of(b, t)
        .iter()
        .filter(|e| is_open(e))
        .map(|e| OpenEntry {
            person: owner(e).unwrap_or_default().to_string(),
            start: text(e, "startTime").unwrap_or_default().to_string(),
        })
        .collect()
}

/// Plans the start of `person`'s timer: a new open entry, and the status
/// `em-andamento`, in one update. When the person already has an open entry on this
/// task, nothing is planned.
pub fn start(b: &Binding, t: &TaskView, person: &str, now: &str) -> Result<Vec<Value>, TaskError> {
    check_person(person)?;
    check_instant(now, "invalid_now")?;
    check_status(b, IN_PROGRESS)?;

    let mut entries = entries_of(b, t);
    if entries
        .iter()
        .any(|e| is_open(e) && owner(e) == Some(person))
    {
        return Ok(Vec::new());
    }
    entries.push(json!({"startTime": now, "description": format!("{PERSON_PREFIX}{person}")}));

    let mut fields = Map::new();
    fields.insert(b.field("status").to_string(), json!(IN_PROGRESS));
    fields.insert(b.field("timeEntries").to_string(), Value::Array(entries));
    Ok(vec![update(t, fields)])
}

/// Plans the stop of `person`'s timer with the given status, in one update.
///
/// The person's own open entries are closed at `now`. When the person has none, the
/// open ownerless entries are closed instead. The status goes through the same
/// closing rules as `set_status`, so a closed status records `completedDate`. The
/// entries are written only when one was closed.
pub fn stop(
    b: &Binding,
    t: &TaskView,
    person: &str,
    status: &str,
    now: &str,
    today: &str,
) -> Result<Vec<Value>, TaskError> {
    check_person(person)?;
    check_instant(now, "invalid_now")?;
    if status.is_empty() || !b.statuses.iter().any(|s| s == status) {
        return Err(TaskError::new(
            "missing_or_invalid_status",
            format!("stop needs a status of the task type, not {status:?}"),
        ));
    }
    let mut fields = status_fields(b, status, None, today)?;

    let mut entries = entries_of(b, t);
    let own: Vec<usize> = (0..entries.len())
        .filter(|&i| is_open(&entries[i]) && owner(&entries[i]) == Some(person))
        .collect();
    let closing: Vec<usize> = if own.is_empty() {
        (0..entries.len())
            .filter(|&i| is_open(&entries[i]) && owner(&entries[i]).is_none())
            .collect()
    } else {
        own
    };
    if closing.is_empty() {
        return Ok(vec![update(t, fields)]);
    }

    for &i in &closing {
        check_after_start(&entries[i], now)?;
        if let Some(entry) = entries[i].as_object_mut() {
            entry.insert("endTime".to_string(), json!(now));
        }
    }
    fields.insert(b.field("timeEntries").to_string(), Value::Array(entries));
    Ok(vec![update(t, fields)])
}

/// Plans the correction of a closed entry, found by its stored start. A new start
/// or end replaces the old one; the reason is required but is not stored in the
/// entry, which has no field for it (the caller's audit log keeps it).
///
/// The corrected entry must end after it starts, must not end after `now`, and must
/// not overlap another entry of the task (an open entry counts as running until
/// `now`). An entry still open is corrected by stopping it, not by retiming it.
pub fn retime(
    b: &Binding,
    t: &TaskView,
    entry_start: &str,
    new_start: Option<&str>,
    new_end: Option<&str>,
    reason: &str,
    now: &str,
) -> Result<Value, TaskError> {
    if entry_start.is_empty() {
        return Err(TaskError::new(
            "missing_entry_start_time",
            "retime needs the start time of the entry",
        ));
    }
    if reason.trim().is_empty() {
        return Err(TaskError::new("missing_reason", "retime needs a reason"));
    }
    if new_start.is_none() && new_end.is_none() {
        return Err(TaskError::new(
            "missing_new_time",
            "retime needs a new start or a new end",
        ));
    }
    check_instant(now, "invalid_now")?;

    let mut entries = entries_of(b, t);
    let index = entries
        .iter()
        .position(|e| text(e, "startTime") == Some(entry_start))
        .ok_or_else(|| {
            TaskError::new(
                "time_entry_not_found",
                format!("no time entry starts at {entry_start}"),
            )
        })?;
    if is_open(&entries[index]) {
        return Err(TaskError::new(
            "entry_still_open",
            "an open entry is stopped, not retimed",
        ));
    }
    let mut updated = entries[index]
        .as_object()
        .cloned()
        .ok_or_else(|| TaskError::new("time_entry_not_found", "the time entry is not an object"))?;

    let start_text = match new_start {
        Some(value) => instant_text(value, "invalid_new_start")?,
        None => text(&entries[index], "startTime")
            .unwrap_or_default()
            .to_string(),
    };
    let end_text = match new_end {
        Some(value) => instant_text(value, "invalid_new_end")?,
        None => text(&entries[index], "endTime")
            .unwrap_or_default()
            .to_string(),
    };
    updated.insert("startTime".to_string(), json!(start_text));
    updated.insert("endTime".to_string(), json!(end_text));
    // A duration written by the plugin would no longer match the new interval.
    updated.remove("duration");

    let start = parse_instant(&start_text).ok_or_else(invalid_stored_time)?;
    let end = parse_instant(&end_text).ok_or_else(invalid_stored_time)?;
    let present = parse_instant(now).ok_or_else(|| invalid_instant("invalid_now"))?;
    if end <= start {
        return Err(TaskError::new(
            "end_not_after_start",
            "the entry must end after it starts",
        ));
    }
    if end > present {
        return Err(TaskError::new(
            "end_in_future",
            "the entry cannot end in the future",
        ));
    }
    for (other, entry) in entries.iter().enumerate() {
        if other == index {
            continue;
        }
        let Some(other_start) = text(entry, "startTime").and_then(parse_instant) else {
            continue;
        };
        let other_end = if is_open(entry) {
            Some(present)
        } else {
            text(entry, "endTime").and_then(parse_instant)
        };
        if other_end.is_some_and(|e| other_start < end && e > start) {
            return Err(TaskError::new(
                "time_entry_overlap",
                "the corrected entry would overlap another entry of the task",
            ));
        }
    }

    entries[index] = Value::Object(updated);
    let mut fields = Map::new();
    fields.insert(b.field("timeEntries").to_string(), Value::Array(entries));
    Ok(update(t, fields))
}

fn entries_of(b: &Binding, t: &TaskView) -> Vec<Value> {
    list_of(t, b.field("timeEntries"))
}

/// An entry is open when it has no end: no `endTime`, a null one, or an empty one.
fn is_open(entry: &Value) -> bool {
    match entry.as_object() {
        Some(o) => match o.get("endTime") {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.is_empty(),
            Some(_) => false,
        },
        None => false,
    }
}

/// The person of an entry, read from its `description`; `None` when it has no owner.
fn owner(entry: &Value) -> Option<&str> {
    text(entry, "description").and_then(|d| d.strip_prefix(PERSON_PREFIX))
}

fn text<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry.get(key).and_then(Value::as_str)
}

fn check_person(person: &str) -> Result<(), TaskError> {
    if person.trim().is_empty() {
        return Err(TaskError::new("missing_person", "the timer needs a person"));
    }
    Ok(())
}

fn check_instant(value: &str, code: &'static str) -> Result<(), TaskError> {
    parse_instant(value)
        .map(|_| ())
        .ok_or_else(|| invalid_instant(code))
}

/// Checks that a closed or stopped entry does not end before it starts.
fn check_after_start(entry: &Value, now: &str) -> Result<(), TaskError> {
    let start = text(entry, "startTime").and_then(parse_instant);
    let end = parse_instant(now);
    match (start, end) {
        (Some(start), Some(end)) if end <= start => Err(TaskError::new(
            "end_not_after_start",
            "the entry cannot end before or when it starts",
        )),
        _ => Ok(()),
    }
}

/// A new instant, normalized to UTC with milliseconds as `toISOString` writes it.
fn instant_text(value: &str, code: &'static str) -> Result<String, TaskError> {
    parse_instant(value)
        .map(format_instant)
        .ok_or_else(|| invalid_instant(code))
}

fn invalid_instant(code: &'static str) -> TaskError {
    TaskError::new(code, "the time is not an ISO 8601 date-time")
}

fn invalid_stored_time() -> TaskError {
    TaskError::new(
        "invalid_entry_time",
        "the time entry holds a time that is not an ISO 8601 date-time",
    )
}

/// Parses `YYYY-MM-DDTHH:MM[:SS[.fff]][Z|±HH:MM]` into milliseconds since the Unix
/// epoch. Without an offset the time is taken as UTC.
fn parse_instant(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let mut parts = date.split('-');
    let year = digits(parts.next()?, 4)?;
    let month = digits(parts.next()?, 2)?;
    let day = digits(parts.next()?, 2)?;
    if parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if day < 1 || day > days_in_month(year, month) {
        return None;
    }

    let (clock, offset_minutes) = if let Some(body) = rest.strip_suffix('Z') {
        (body, 0)
    } else if let Some(pos) = rest.rfind(['+', '-']) {
        let (body, zone) = rest.split_at(pos);
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let (zone_hours, zone_minutes) = zone[1..].split_once(':')?;
        let zone_hours = digits(zone_hours, 2)?;
        let zone_minutes = digits(zone_minutes, 2)?;
        if zone_hours > 23 || zone_minutes > 59 {
            return None;
        }
        (body, sign * (zone_hours * 60 + zone_minutes))
    } else {
        (rest, 0)
    };

    let (hms, fraction) = match clock.split_once('.') {
        Some((hms, fraction)) => (hms, Some(fraction)),
        None => (clock, None),
    };
    let mut fields = hms.split(':');
    let hour = digits(fields.next()?, 2)?;
    let minute = digits(fields.next()?, 2)?;
    let second = match fields.next() {
        Some(part) => digits(part, 2)?,
        None if fraction.is_none() => 0,
        None => return None,
    };
    if fields.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let millis = match fraction {
        None => 0,
        Some(f) if !f.is_empty() && f.bytes().all(|c| c.is_ascii_digit()) => {
            f.bytes().take(3).enumerate().fold(0, |acc, (i, c)| {
                acc + i64::from(c - b'0') * 10_i64.pow(2 - i as u32)
            })
        }
        Some(_) => return None,
    };

    let local = days_from_civil(year, month, day) * 86_400_000
        + (hour * 3600 + minute * 60 + second) * 1000
        + millis;
    Some(local - offset_minutes * 60_000)
}

/// Formats milliseconds since the epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
fn format_instant(ms: i64) -> String {
    let (year, month, day) = civil_from_days(ms.div_euclid(86_400_000));
    let rem = ms.rem_euclid(86_400_000);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// Exactly `len` ASCII digits, as a number.
fn digits(s: &str, len: usize) -> Option<i64> {
    if s.len() == len && s.bytes().all(|c| c.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
