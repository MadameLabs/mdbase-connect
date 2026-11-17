//! Planned writes over an existing task.
//!
//! Each planner returns a CLI `WriteOp` as JSON:
//! `{"op":"update","path":..,"fields":{..},"if_revision":..}`. Fields replace the
//! whole value of each key, so list edits are computed here from the frontmatter
//! as read, and the whole new list is sent. Field names go through
//! [`Binding::field`], never hardcoded collection names.
//!
//! Dates are `YYYY-MM-DD` strings; "today" is passed in. Nothing here reads a
//! clock or does I/O.

use serde_json::{Map, Value, json};

use crate::{Binding, TaskError};

/// A task as read from the collection: where it is, the revision it was read at,
/// and its frontmatter.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskView {
    /// Path of the task note in the collection.
    pub path: String,
    /// Revision the task was read at; the write is refused if it moved.
    pub revision: String,
    /// The task's frontmatter as stored.
    pub frontmatter: Map<String, Value>,
}

/// Plans a status change. A closed status also records `completedDate`: the given
/// date, or `today` when none is given. Dates must be well formed and not later
/// than `today`. A date may only accompany a closed status.
pub fn set_status(
    b: &Binding,
    t: &TaskView,
    status: &str,
    completed_date: Option<&str>,
    today: &str,
) -> Result<Value, TaskError> {
    Ok(update(t, status_fields(b, status, completed_date, today)?))
}

/// The status fields of a write: the status, plus `completedDate` when the status
/// closes the task. Shared by `set_status` and the timer, so both close alike.
pub(crate) fn status_fields(
    b: &Binding,
    status: &str,
    completed_date: Option<&str>,
    today: &str,
) -> Result<Map<String, Value>, TaskError> {
    check_status(b, status)?;
    let mut fields = Map::new();
    fields.insert(b.field("status").to_string(), json!(status));

    match completed_date {
        Some(date) => {
            if !b.is_closed(status) {
                return Err(TaskError::new(
                    "completed_date_requires_closed_status",
                    format!("completed date needs a closed status, not {status}"),
                ));
            }
            if !is_date(date) || !is_date(today) || date > today {
                return Err(TaskError::new(
                    "invalid_completed_date",
                    format!("completed date {date} must be a past or today date"),
                ));
            }
            fields.insert(b.field("completedDate").to_string(), json!(date));
        }
        None if b.is_closed(status) => {
            if !is_date(today) {
                return Err(TaskError::new(
                    "invalid_date",
                    format!("today {today} is not a date"),
                ));
            }
            fields.insert(b.field("completedDate").to_string(), json!(today));
        }
        None => {}
    }
    Ok(fields)
}

/// Refuses a status the task type does not allow.
pub(crate) fn check_status(b: &Binding, status: &str) -> Result<(), TaskError> {
    if !b.statuses.iter().any(|s| s == status) {
        return Err(TaskError::new(
            "invalid_status",
            format!("status {status} is not in the task type"),
        ));
    }
    Ok(())
}

/// Plans the project list of a task: the given projects replace the list, as
/// wikilinks, without duplicates.
pub fn link_projects(b: &Binding, t: &TaskView, projects: &[String]) -> Result<Value, TaskError> {
    if projects.is_empty() {
        return Err(TaskError::new("missing_project", "no project given"));
    }
    let mut links: Vec<String> = Vec::new();
    for project in projects {
        let link = wikilink(project);
        if !links.contains(&link) {
            links.push(link);
        }
    }
    let mut fields = Map::new();
    fields.insert(b.field("projects").to_string(), json!(links));
    Ok(update(t, fields))
}

/// Plans adding blockers to `blockedBy`. Each blocker must be one of `existing`
/// (the task notes of the collection) and not the task itself. A blocker already
/// listed is not added twice. New items are `FINISHTOSTART`, as the plugin writes
/// them; items already there keep their form.
pub fn block(
    b: &Binding,
    t: &TaskView,
    blockers: &[String],
    existing: &[String],
) -> Result<Value, TaskError> {
    if blockers.is_empty() {
        return Err(TaskError::new("missing_blocker", "no blocker given"));
    }
    let me = note_name(&t.path);
    let mut next = list_of(t, b.field("blockedBy"));
    for blocker in blockers {
        let name = note_name(blocker);
        if name == me {
            return Err(TaskError::new(
                "task_cannot_block_itself",
                "a task cannot block itself",
            ));
        }
        if !existing.iter().any(|e| note_name(e) == name) {
            return Err(TaskError::new(
                "blocker_not_found",
                format!("blocker {name} is not a task in the collection"),
            ));
        }
        if !next.iter().any(|item| item_name(item) == name) {
            next.push(json!({"uid": format!("[[{name}]]"), "reltype": "FINISHTOSTART"}));
        }
    }
    let mut fields = Map::new();
    fields.insert(b.field("blockedBy").to_string(), Value::Array(next));
    Ok(update(t, fields))
}

/// Plans removing blockers from `blockedBy`, matched by note name whatever the
/// written form.
pub fn unblock(b: &Binding, t: &TaskView, blockers: &[String]) -> Result<Value, TaskError> {
    let names: Vec<String> = blockers.iter().map(|x| note_name(x)).collect();
    let next: Vec<Value> = list_of(t, b.field("blockedBy"))
        .into_iter()
        .filter(|item| !names.contains(&item_name(item)))
        .collect();
    let mut fields = Map::new();
    fields.insert(b.field("blockedBy").to_string(), Value::Array(next));
    Ok(update(t, fields))
}

/// Plans archiving: adds the type's archive tag, keeping the other tags.
pub fn archive(b: &Binding, t: &TaskView) -> Result<Value, TaskError> {
    if b.archived_tag.is_empty() {
        return Err(TaskError::new(
            "type_without_archive",
            "the task type declares no archive tag",
        ));
    }
    let mut tags = list_of(t, b.field("tags"));
    if !has_tag(t, b) {
        tags.push(json!(b.archived_tag));
    }
    let mut fields = Map::new();
    fields.insert(b.field("tags").to_string(), Value::Array(tags));
    Ok(update(t, fields))
}

/// Plans skipping one occurrence of a recurring task. Skipping a date already
/// skipped changes nothing.
pub fn skip(b: &Binding, t: &TaskView, date: &str) -> Result<Value, TaskError> {
    if !is_date(date) {
        return Err(TaskError::new(
            "invalid_date",
            format!("{date} is not a date"),
        ));
    }
    let mut skipped = list_of(t, b.field("skippedInstances"));
    if !skipped.iter().any(|v| v.as_str() == Some(date)) {
        skipped.push(json!(date));
    }
    let mut fields = Map::new();
    fields.insert(
        b.field("skippedInstances").to_string(),
        Value::Array(skipped),
    );
    Ok(update(t, fields))
}

/// Plans undoing a skip of one occurrence.
pub fn unskip(b: &Binding, t: &TaskView, date: &str) -> Result<Value, TaskError> {
    if !is_date(date) {
        return Err(TaskError::new(
            "invalid_date",
            format!("{date} is not a date"),
        ));
    }
    let kept: Vec<Value> = list_of(t, b.field("skippedInstances"))
        .into_iter()
        .filter(|v| v.as_str() != Some(date))
        .collect();
    let mut fields = Map::new();
    fields.insert(b.field("skippedInstances").to_string(), Value::Array(kept));
    Ok(update(t, fields))
}

/// Whether a task is still open: its status, with the type's default when none is
/// written, is not closed, and it does not carry the archive tag.
pub fn is_open(b: &Binding, t: &TaskView) -> bool {
    if !b.archived_tag.is_empty() && has_tag(t, b) {
        return false;
    }
    let effective = b.effective(&t.frontmatter);
    let status = effective
        .get(b.field("status"))
        .and_then(Value::as_str)
        .unwrap_or(b.default_status.as_str());
    !b.is_closed(status)
}

/// The wikilink for a note name or path: `mdbase-tasks` and `02-x/mdbase-tasks.md`
/// both become `[[mdbase-tasks]]`. A value already in wikilink form is kept.
pub fn wikilink(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
        return trimmed.to_string();
    }
    format!("[[{}]]", note_name(trimmed))
}

pub(crate) fn update(t: &TaskView, fields: Map<String, Value>) -> Value {
    json!({
        "op": "update",
        "path": t.path,
        "fields": fields,
        "if_revision": t.revision,
    })
}

/// The list stored under `key`. A plain string counts as a one-item list, so a
/// note written as `tags: task` keeps its tag; a missing key is an empty list.
pub(crate) fn list_of(t: &TaskView, key: &str) -> Vec<Value> {
    match t.frontmatter.get(key) {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(s)) => vec![Value::String(s.clone())],
        _ => Vec::new(),
    }
}

fn has_tag(t: &TaskView, b: &Binding) -> bool {
    list_of(t, b.field("tags"))
        .iter()
        .any(|v| v.as_str() == Some(b.archived_tag.as_str()))
}

/// The note name of a path or wikilink: no brackets, alias, heading, folder or
/// `.md` extension.
pub(crate) fn note_name(value: &str) -> String {
    let inner = value.trim();
    let inner = inner.strip_prefix("[[").unwrap_or(inner);
    let inner = inner.strip_suffix("]]").unwrap_or(inner);
    let inner = inner.split('|').next().unwrap_or(inner);
    let inner = inner.split('#').next().unwrap_or(inner);
    let inner = inner.rsplit('/').next().unwrap_or(inner);
    let lower = inner.to_ascii_lowercase();
    let inner = if lower.ends_with(".md") {
        &inner[..inner.len() - 3]
    } else {
        inner
    };
    inner.trim().to_string()
}

/// The note name of a `blockedBy` item, which is a string or an object with `uid`.
fn item_name(item: &Value) -> String {
    match item {
        Value::String(s) => note_name(s),
        Value::Object(o) => note_name(o.get("uid").and_then(Value::as_str).unwrap_or_default()),
        _ => String::new(),
    }
}

/// A `YYYY-MM-DD` date with a month of 01 to 12 and a day of 01 to 31.
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    if !b
        .iter()
        .enumerate()
        .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return false;
    }
    let month = (b[5] - b'0') * 10 + (b[6] - b'0');
    let day = (b[8] - b'0') * 10 + (b[9] - b'0');
    (1..=12).contains(&month) && (1..=31).contains(&day)
}
