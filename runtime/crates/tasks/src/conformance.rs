//! The operations of the tasknotes-spec conformance suite that the task crate
//! implements, in the shapes the adapter contract asks (`ADAPTER_CONTRACT.md`).
//!
//! The claim is what the adapter declares to the suite: the profiles and
//! capabilities whose fixtures this implementation runs. An operation outside the
//! claim, or not implemented here, answers `unsupported_operation`.
//!
//! Portable: no I/O and no clock; the caller passes `today`.

use serde_json::{Map, Value, json};

use crate::temporal::{self, DateTime};

/// The name the adapter reports for this implementation.
pub const IMPLEMENTATION: &str = "mdbase-tasks (MadameLabs)";
/// The `tasknotes-spec` version the claim targets (the clone's `package.json`).
pub const SPEC_VERSION: &str = "0.3.0";
/// The profiles claimed.
pub const PROFILES: &[&str] = &["core-lite"];
/// The capabilities claimed.
pub const CAPABILITIES: &[&str] = &["dependencies", "time-tracking", "archive"];
/// The validation modes claimed.
pub const VALIDATION_MODES: &[&str] = &["strict"];

/// A failed operation: a stable code, a human message, and the field when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpError {
    /// Stable code, e.g. `unsupported_operation`.
    pub code: &'static str,
    /// Human-readable detail.
    pub message: String,
    /// The input field at fault, when there is one.
    pub field: Option<String>,
}

impl OpError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field: None,
        }
    }
}

/// The claim the adapter reports, as the `meta.claim` result.
pub fn claim() -> Value {
    json!({
        "implementation": IMPLEMENTATION,
        "version": env!("CARGO_PKG_VERSION"),
        "spec_version": SPEC_VERSION,
        "validation_modes": VALIDATION_MODES,
        "profiles": PROFILES,
        "capabilities": CAPABILITIES,
    })
}

/// Runs one operation over its input object. `today` is the local date (`YYYY-MM-DD`)
/// for the operations that default to it. `Ok` is the result; `Err` is the failure
/// the envelope reports.
pub fn execute(operation: &str, input: &Value, today: &str) -> Result<Value, OpError> {
    let Some(input) = input.as_object() else {
        return Err(OpError::new(
            "invalid_input",
            "the operation input must be a JSON object",
        ));
    };
    match operation {
        "meta.claim" => Ok(claim()),
        "meta.has_capability" => {
            let cap = text(input, "capability")?;
            Ok(json!({"value": CAPABILITIES.contains(&cap)}))
        }
        "meta.has_profile" => {
            let profile = text(input, "profile")?;
            Ok(json!({"value": PROFILES.contains(&profile)}))
        }
        "date.validate" => {
            let value = text(input, "value")?;
            temporal::calendar_date(value)
                .map(|d| json!({"value": d}))
                .ok_or_else(|| invalid_date(value))
        }
        "date.parse_utc" => {
            let value = text(input, "value")?;
            temporal::utc_date(value)
                .map(|d| json!({"date": d}))
                .ok_or_else(|| invalid_date(value))
        }
        "date.parse_local" => {
            let value = text(input, "value")?;
            if temporal::calendar_date(value).is_some() {
                return Ok(json!({"localDate": value}));
            }
            temporal::utc_date(value)
                .map(|d| json!({"isoDate": d}))
                .ok_or_else(|| invalid_date(value))
        }
        "date.get_part" => {
            let value = text(input, "value")?;
            temporal::written_date(value)
                .map(|d| json!({"value": d}))
                .ok_or_else(|| invalid_date(value))
        }
        "date.has_time" => {
            let value = text(input, "value")?;
            Ok(json!({"value": has_time(value)}))
        }
        "date.is_same" => {
            let (a, b) = (text(input, "a")?, text(input, "b")?);
            Ok(json!({"value": written(a).is_some() && written(a) == written(b)}))
        }
        "date.is_before" => {
            let (a, b) = (text(input, "a")?, text(input, "b")?);
            Ok(json!({"value": match (written(a), written(b)) {
                (Some(a), Some(b)) => a < b,
                _ => false,
            }}))
        }
        "date.resolve_operation_target" => resolve_target(input, today),
        "create_compat.create" => create_compat(input),
        "op.update_patch" => update_patch(input),
        "op.mutate_with_validation" => mutate_with_validation(input),
        "op.idempotency_check" => idempotency_check(input, today),
        "op.complete_nonrecurring" => complete_nonrecurring(input, today),
        "op.uncomplete_nonrecurring" => uncomplete_nonrecurring(input),
        "op.error_shape" => Ok(json!({
            "operation": text(input, "operation")?,
            "code": text(input, "code")?,
            "message": text(input, "message")?,
        })),
        "op.atomic_write" => atomic_write(input),
        "delete.remove" => delete_remove(input),
        _ => Err(OpError::new(
            "unsupported_operation",
            format!("operation {operation} is not implemented"),
        )),
    }
}

/// The envelope for `execute`'s outcome, as the adapter contract shapes it.
pub fn envelope(operation: &str, outcome: Result<Value, OpError>) -> Value {
    match outcome {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(e) => {
            let mut details = Map::new();
            details.insert("operation".into(), json!(operation));
            details.insert("code".into(), json!(e.code));
            details.insert("message".into(), json!(e.message));
            if let Some(field) = e.field {
                details.insert("field".into(), json!(field));
            }
            json!({
                "ok": false,
                "error": e.message,
                "error_details": Value::Object(details),
            })
        }
    }
}

fn text<'a>(input: &'a Map<String, Value>, key: &str) -> Result<&'a str, OpError> {
    match input.get(key) {
        Some(Value::String(s)) => Ok(s),
        _ => Err(OpError {
            code: "invalid_input",
            message: format!("{key} must be a string"),
            field: Some(key.to_string()),
        }),
    }
}

fn invalid_date(value: &str) -> OpError {
    OpError::new("invalid_date", format!("Invalid date: {value:?}"))
}

/// The date of a value as written, when it is a date or a datetime.
fn written(value: &str) -> Option<&str> {
    temporal::written_date(value)
}

/// `T` then `HH:MM`, anywhere in the value.
fn has_time(value: &str) -> bool {
    let b = value.as_bytes();
    b.windows(6).any(|w| {
        w[0] == b'T'
            && w[1].is_ascii_digit()
            && w[2].is_ascii_digit()
            && w[3] == b':'
            && w[4].is_ascii_digit()
            && w[5].is_ascii_digit()
    })
}

/// The explicit date when given (must be valid), else the first valid of
/// `scheduled`, `due`, else today.
fn resolve_target(input: &Map<String, Value>, today: &str) -> Result<Value, OpError> {
    if let Some(explicit) = input.get("explicitDate").and_then(Value::as_str) {
        return written(explicit)
            .map(|d| json!({"value": d}))
            .ok_or_else(|| invalid_date(explicit));
    }
    for key in ["scheduled", "due"] {
        if let Some(d) = input.get(key).and_then(Value::as_str).and_then(written) {
            return Ok(json!({"value": d}));
        }
    }
    Ok(json!({"value": today}))
}

/// Merges `patch` into `original`; `changed` says whether any value moved.
fn update_patch(input: &Map<String, Value>) -> Result<Value, OpError> {
    let mut frontmatter = object(input, "original")?.clone();
    let patch = object(input, "patch")?;
    let mut changed = false;
    for (key, value) in patch {
        if frontmatter.get(key) != Some(value) {
            changed = true;
            frontmatter.insert(key.clone(), value.clone());
        }
    }
    Ok(json!({"changed": changed, "frontmatter": frontmatter}))
}

/// Sets the first closed status (in `completedValues` order, default done and
/// cancelled) and the completed date: `explicitDate`, else today. A task already
/// closed with a completed date keeps both, so a repeated completion changes nothing.
fn complete_nonrecurring(input: &Map<String, Value>, today: &str) -> Result<Value, OpError> {
    let frontmatter = object(input, "frontmatter")?;
    let completed = if input.contains_key("completedValues") {
        strings(input, "completedValues")?
    } else {
        vec!["done".to_string(), "cancelled".to_string()]
    };
    let current = frontmatter.get("status").and_then(Value::as_str);
    if let (Some(status), Some(date)) = (
        current,
        frontmatter.get("completedDate").and_then(Value::as_str),
    ) {
        if completed.iter().any(|c| c == status) {
            return Ok(json!({"status": status, "completedDate": date}));
        }
    }
    let Some(status) = completed.first() else {
        return Err(OpError::new("invalid_input", "completedValues is empty"));
    };
    let date = match input.get("explicitDate").and_then(Value::as_str) {
        Some(explicit) => written(explicit)
            .ok_or_else(|| invalid_date(explicit))?
            .to_string(),
        None => today.to_string(),
    };
    Ok(json!({"status": status, "completedDate": date}))
}

/// Whether repeating the operation on the state it left changes nothing. Only the
/// completion is defined here; other operations are not implemented.
fn idempotency_check(input: &Map<String, Value>, today: &str) -> Result<Value, OpError> {
    match text(input, "operation")? {
        "complete_nonrecurring" => {
            let second = object(input, "second")?;
            let mut again = Map::new();
            again.insert("frontmatter".into(), Value::Object(second.clone()));
            let after = complete_nonrecurring(&again, today)?;
            let same = after["status"] == second.get("status").cloned().unwrap_or(Value::Null)
                && after["completedDate"]
                    == second.get("completedDate").cloned().unwrap_or(Value::Null);
            Ok(json!({"idempotent": same}))
        }
        other => Err(OpError::new(
            "unsupported_operation",
            format!("idempotency of {other} is not implemented"),
        )),
    }
}

/// Checks a frontmatter as the task type writes it: a title, a string status, and
/// datetimes or dates where dates are written.
fn mutate_with_validation(input: &Map<String, Value>) -> Result<Value, OpError> {
    let fm = object(input, "frontmatter")?;
    match fm.get("title") {
        None => {
            return Err(OpError::new(
                "missing_required_field",
                "missing_required_field: title is required",
            ));
        }
        Some(Value::String(t)) if !t.trim().is_empty() => {}
        Some(_) => {
            return Err(OpError::new(
                "invalid_type",
                "title must be a non-empty string",
            ));
        }
    }
    if let Some(status) = fm.get("status") {
        if !status.is_string() {
            return Err(OpError::new(
                "invalid_type",
                "invalid_type: status must be a string",
            ));
        }
    }
    for key in ["dateCreated", "dateModified"] {
        if let Some(value) = fm.get(key) {
            let ok = value
                .as_str()
                .is_some_and(|v| temporal::written_date(v).is_some());
            if !ok {
                return Err(OpError::new(
                    "invalid_date",
                    format!("invalid_date: {key} is not a date"),
                ));
            }
        }
    }
    Ok(json!({"value": "accepted"}))
}

/// Puts back the default status; the completed date is cleared or kept.
fn uncomplete_nonrecurring(input: &Map<String, Value>) -> Result<Value, OpError> {
    let frontmatter = object(input, "frontmatter")?;
    let default = text(input, "defaultStatus")?;
    let clear = input.get("clearCompletedDate").and_then(Value::as_bool) == Some(true);
    let date = if clear {
        Value::Null
    } else {
        frontmatter
            .get("completedDate")
            .cloned()
            .unwrap_or(Value::Null)
    };
    Ok(json!({"status": default, "completedDate": date}))
}

/// Applies the patch unless the simulated write fails after it, in which case the
/// original persists and `committed` is false.
fn atomic_write(input: &Map<String, Value>) -> Result<Value, OpError> {
    let mut merged = object(input, "original")?.clone();
    for (key, value) in object(input, "patch")? {
        merged.insert(key.clone(), value.clone());
    }
    let failed = input
        .get("simulateFailureAfterWrite")
        .and_then(Value::as_bool)
        == Some(true);
    if failed {
        let original = object(input, "original")?.clone();
        return Ok(json!({"committed": false, "persisted": original}));
    }
    Ok(json!({"committed": true, "persisted": merged}))
}

/// Removes a note; a note with broken backlinks is kept unless forced.
fn delete_remove(input: &Map<String, Value>) -> Result<Value, OpError> {
    let broken = strings(input, "brokenLinks").unwrap_or_default();
    let force = input.get("force").and_then(Value::as_bool) == Some(true);
    let check = input.get("checkBacklinks").and_then(Value::as_bool) == Some(true);
    if check && !force && !broken.is_empty() {
        return Err(OpError::new(
            "has_backlinks",
            format!(
                "the note has backlinks from {}; use force to remove it",
                broken.join(", ")
            ),
        ));
    }
    Ok(json!({"deleted": true}))
}

fn object<'a>(input: &'a Map<String, Value>, key: &str) -> Result<&'a Map<String, Value>, OpError> {
    input
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| OpError {
            code: "invalid_input",
            message: format!("{key} must be an object"),
            field: Some(key.to_string()),
        })
}

fn strings(input: &Map<String, Value>, key: &str) -> Result<Vec<String>, OpError> {
    match input.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str().map(str::to_string).ok_or_else(|| OpError {
                    code: "invalid_input",
                    message: format!("{key} must hold strings"),
                    field: Some(key.to_string()),
                })
            })
            .collect(),
        Some(_) => Err(OpError {
            code: "invalid_input",
            message: format!("{key} must be an array"),
            field: Some(key.to_string()),
        }),
    }
}

/// Creates a note of a task type: fills the `where` equalities, the type's defaults
/// and the datetime fields from `fixedNow`, then renders the path pattern. A forced
/// error is returned as its own code, as the suite expects it.
fn create_compat(input: &Map<String, Value>) -> Result<Value, OpError> {
    if let Some(code) = input.get("forceCreateError").and_then(Value::as_str) {
        return Err(OpError {
            code: "forced_error",
            message: code.to_string(),
            field: None,
        });
    }
    let task_type = object(input, "taskType")?;
    let mut fm = object(input, "frontmatter")?.clone();
    let pattern = task_type
        .get("path_pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| OpError::new("invalid_input", "taskType.path_pattern must be a string"))?;
    let now = input
        .get("fixedNow")
        .and_then(Value::as_str)
        .and_then(temporal::datetime);
    let fields = task_type.get("fields").and_then(Value::as_object);

    if let Some(where_) = task_type
        .get("match")
        .and_then(|m| m.get("where"))
        .and_then(Value::as_object)
    {
        for (key, cond) in where_ {
            if fm.contains_key(key) {
                continue;
            }
            let value = match cond {
                Value::Object(o) if o.contains_key("eq") => o["eq"].clone(),
                Value::Object(o) if o.contains_key("contains") => json!([o["contains"]]),
                Value::Object(o) if o.contains_key("exists") => json!(true),
                other => other.clone(),
            };
            fm.insert(key.clone(), value);
        }
    }

    for (name, def) in fields.into_iter().flatten() {
        if fm.contains_key(name) {
            continue;
        }
        if let Some(default) = def.get("default") {
            fm.insert(name.clone(), default.clone());
        } else if matches!(name.as_str(), "dateCreated" | "dateModified") {
            if let Some(fixed) = input.get("fixedNow") {
                fm.insert(name.clone(), fixed.clone());
            }
        }
    }

    let path = render_path(pattern, &fm, now.as_ref())?;
    let path = if path.ends_with(".md") {
        path
    } else {
        format!("{path}.md")
    };
    Ok(json!({"path": path, "frontmatter": fm}))
}

/// Renders `{name}` placeholders. A name is a date token of `fixedNow`, a title or
/// status form, or a frontmatter key (`dueDate` and `scheduledDate` read `due` and
/// `scheduled`). Any other name is missing.
fn render_path(
    pattern: &str,
    fm: &Map<String, Value>,
    now: Option<&DateTime>,
) -> Result<String, OpError> {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            return Err(OpError::new(
                "invalid_input",
                "unclosed placeholder in path_pattern",
            ));
        };
        let name = &rest[open + 1..open + close];
        match token(name, fm, now) {
            Some(value) => out.push_str(&value),
            None => {
                return Err(OpError::new(
                    "path_required",
                    format!("missing template values: {name}"),
                ));
            }
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn token(name: &str, fm: &Map<String, Value>, now: Option<&DateTime>) -> Option<String> {
    let title = fm.get("title").and_then(Value::as_str);
    let status = fm.get("status").and_then(Value::as_str);
    let priority = fm.get("priority").and_then(Value::as_str);
    let text = |key: &str| fm.get(key).and_then(Value::as_str).map(str::to_string);
    let first = |s: Option<&str>| {
        s.and_then(|s| s.chars().next())
            .map(|c| c.to_uppercase().to_string())
    };
    match name {
        "titleLower" => return title.map(|t| t.to_lowercase()),
        "titleUpper" => return title.map(|t| t.to_uppercase()),
        "titleKebab" => return title.map(|t| words(t).join("-")),
        "titleSnake" => return title.map(|t| words(t).join("_")),
        "titleCamel" => return title.map(camel),
        "titlePascal" => return title.map(|t| words(t).iter().map(|w| capitalize(w)).collect()),
        "statusShort" => return first(status),
        "priorityShort" => return first(priority),
        "dueDate" => return text("due"),
        "scheduledDate" => return text("scheduled"),
        _ => {}
    }
    if let Some(now) = now {
        if let Some(value) = clock_token(name, now) {
            return Some(value);
        }
    }
    match fm.get(name) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn clock_token(name: &str, now: &DateTime) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const DAYS: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    let (y, m, d) = (now.year, now.month, now.day);
    let date = format!("{y:04}-{m:02}-{d:02}");
    let time = format!("{:02}:{:02}", now.hour, now.minute);
    let month_name = MONTHS[(m - 1) as usize];
    let day_name = DAYS[(now.iso_weekday() - 1) as usize];
    Some(match name {
        "date" => date,
        "time" | "time24" => time,
        "year" => format!("{y:04}"),
        "shortYear" => format!("{:02}", y % 100),
        "month" => format!("{m:02}"),
        "monthName" => month_name.to_string(),
        "monthNameShort" => month_name[..3].to_string(),
        "day" => format!("{d:02}"),
        "dayName" => day_name.to_string(),
        "dayNameShort" => day_name[..3].to_string(),
        "shortDate" => format!("{:02}{m:02}{d:02}", y % 100),
        "dateTime" => format!("{date}-{:02}{:02}", now.hour, now.minute),
        "timestamp" => format!("{date}-{:02}{:02}{:02}", now.hour, now.minute, now.second),
        "week" => format!("{:02}", now.iso_week()),
        "quarter" => ((m - 1) / 3 + 1).to_string(),
        "hour" => format!("{:02}", now.hour),
        "minute" => format!("{:02}", now.minute),
        "second" => format!("{:02}", now.second),
        "unix" => now.unix_seconds().to_string(),
        "unixMs" => (now.unix_seconds() * 1000).to_string(),
        "timezone" | "utcOffset" => {
            let sign = if now.offset < 0 { '-' } else { '+' };
            format!(
                "{sign}{:02}:{:02}",
                now.offset.abs() / 60,
                now.offset.abs() % 60
            )
        }
        "zettel" => {
            let secs = now.hour * 3600 + now.minute * 60 + now.second;
            format!("{y:04}{m:02}{d:02}{}", base36(secs))
        }
        _ => return None,
    })
}

fn base36(mut n: i64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// The lowercase words of a title, split at any non-alphanumeric character.
fn words(title: &str) -> Vec<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn camel(title: &str) -> String {
    let words = words(title);
    words
        .iter()
        .enumerate()
        .map(|(i, w)| if i == 0 { w.clone() } else { capitalize(w) })
        .collect()
}
