//! Adapter shapes for the tasknotes-spec conformance suite (`ADAPTER_CONTRACT.md`).
//!
//! Every operation delegates to the crate's public API. This module only reads the
//! input object, calls the crate, and writes the result in the suite's shape. An
//! operation the crate does not implement answers `unsupported_operation` with the
//! message `operation not implemented`.
//!
//! Portable: no I/O and no clock; the caller passes `today`.

use serde_json::{Map, Value, json};

use crate::temporal;
use crate::{Binding, NewTask, TaskError, TaskView, TasksConfig};

/// The name the adapter reports for this implementation.
pub const IMPLEMENTATION: &str = "mdbase-tasks";
/// The `tasknotes-spec` version the claim targets (the clone's `package.json`).
pub const SPEC_VERSION: &str = "0.3.0";
/// The profiles claimed.
pub const PROFILES: &[&str] = &["core-lite"];
/// The capabilities claimed. None: the suite's capability fixtures sit under the
/// `extended` profile, which is not claimed.
pub const CAPABILITIES: &[&str] = &[];
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

    fn unsupported() -> Self {
        Self::new("unsupported_operation", "operation not implemented")
    }
}

impl From<TaskError> for OpError {
    fn from(e: TaskError) -> Self {
        Self::new(e.code, e.message)
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

/// Runs one operation over its input object. `today` is the local date
/// (`YYYY-MM-DD`). `Ok` is the result; `Err` is the failure the envelope reports.
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
        "date.has_time" => Ok(json!({"value": temporal::has_time(text(input, "value")?)})),
        "date.is_same" => {
            let (a, b) = (written(input, "a")?, written(input, "b")?);
            Ok(json!({"value": a.is_some() && a == b}))
        }
        "date.is_before" => {
            let (a, b) = (written(input, "a")?, written(input, "b")?);
            Ok(json!({"value": matches!((a, b), (Some(a), Some(b)) if a < b)}))
        }
        "date.resolve_operation_target" => {
            let target = temporal::operation_target(
                opt_text(input, "explicitDate"),
                opt_text(input, "scheduled"),
                opt_text(input, "due"),
                today,
            )?;
            Ok(json!({"value": target}))
        }
        "field.default_mapping" => {
            let role = text(input, "checkRole")?;
            let b = Binding::from_roles(&Map::new());
            Ok(json!({
                "roleToField": {role: b.field(role)},
                "fieldToRole": {b.field(role): role},
                "displayNameKey": b.field("title"),
            }))
        }
        "field.build_mapping" => {
            let b = Binding::from_roles(object(input, "fields")?);
            let mut out = json!({
                "roleToField": role_to_field(&b),
                "fieldToRole": field_to_role(&b),
                "completedStatuses": b.completed,
            });
            if let Some(key) = opt_text(input, "displayNameKey") {
                out["displayNameKey"] = json!(key);
            }
            Ok(out)
        }
        "field.is_completed_status" => {
            let b = Binding::from_roles(object(input, "fields")?);
            Ok(json!({"value": b.is_closed(text(input, "status")?)}))
        }
        "field.default_completed_status" => {
            let b = Binding::from_roles(object(input, "fields")?);
            let value = b.completed.first().cloned().unwrap_or_default();
            Ok(json!({"value": value}))
        }
        "field.normalize" => {
            let b = Binding::from_roles(object(input, "fields")?);
            let normalized = b.normalize(object(input, "frontmatter")?);
            Ok(json!({"normalized": normalized}))
        }
        "field.denormalize" => {
            let b = Binding::from_roles(object(input, "fields")?);
            let denormalized = b.denormalize(object(input, "roleData")?);
            Ok(json!({"denormalized": denormalized}))
        }
        "field.resolve_display_title" => {
            let b = Binding::from_roles(object(input, "fields")?);
            let key = text(input, "displayNameKey")?;
            let value = b.display_title(
                object(input, "frontmatter")?,
                key,
                opt_text(input, "taskPath"),
            );
            Ok(json!({"value": value}))
        }
        "op.update_patch" => update_patch(input),
        "op.complete_nonrecurring" => {
            let completed = completed_values(input)?;
            let frontmatter = object(input, "frontmatter")?;
            let explicit = opt_text(input, "explicitDate");
            let (status, date) = complete(frontmatter, &completed, explicit, today)?;
            Ok(json!({"status": status, "completedDate": date}))
        }
        "op.uncomplete_nonrecurring" => uncomplete(input, today),
        "op.error_shape" => Ok(json!({
            "operation": text(input, "operation")?,
            "code": text(input, "code")?,
            "message": text(input, "message")?,
        })),
        "op.idempotency_check" => idempotency_check(input, today),
        "delete.remove" => {
            let broken = string_list(input, "brokenLinks")?;
            let check = input.get("checkBacklinks").and_then(Value::as_bool) == Some(true);
            let force = input.get("force").and_then(Value::as_bool) == Some(true);
            crate::delete_check(&broken, check, force)?;
            Ok(json!({"deleted": true}))
        }
        _ => Err(OpError::unsupported()),
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

/// The suite's `roleToField`: every role, with the field it maps to (the role's own
/// name when nothing maps it).
fn role_to_field(b: &Binding) -> Value {
    let map: Map<String, Value> = crate::ROLES
        .iter()
        .map(|r| (r.to_string(), json!(b.field(r))))
        .collect();
    Value::Object(map)
}

/// The suite's `fieldToRole`: the inverse of [`role_to_field`].
fn field_to_role(b: &Binding) -> Value {
    let map: Map<String, Value> = crate::ROLES
        .iter()
        .map(|r| (b.field(r).to_string(), json!(r)))
        .collect();
    Value::Object(map)
}

/// Applies a planned write's fields over a frontmatter.
fn apply(frontmatter: &Map<String, Value>, fields: &Map<String, Value>) -> Map<String, Value> {
    let mut merged = frontmatter.clone();
    for (key, value) in fields {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

/// The fields of a planned write.
fn planned_fields(op: &Value) -> Map<String, Value> {
    op["fields"].as_object().cloned().unwrap_or_default()
}

/// A task view over a frontmatter, for the planners.
fn view(frontmatter: &Map<String, Value>) -> TaskView {
    TaskView {
        path: String::new(),
        revision: String::new(),
        frontmatter: frontmatter.clone(),
    }
}

/// Binds the statuses the suite gives: the listed ones, with the completed ones.
fn status_binding(statuses: &[String], completed: &[String]) -> Binding {
    let mut field = Map::new();
    field.insert(
        "status".into(),
        json!({"tn_role": "status", "values": statuses, "tn_completed_values": completed}),
    );
    Binding::from_roles(&field)
}

/// The completed values of the input, default done and cancelled.
fn completed_values(input: &Map<String, Value>) -> Result<Vec<String>, OpError> {
    if input.contains_key("completedValues") {
        let values = string_list(input, "completedValues")?;
        if values.is_empty() {
            return Err(OpError::new("invalid_input", "completedValues is empty"));
        }
        Ok(values)
    } else {
        Ok(vec!["done".into(), "cancelled".into()])
    }
}

/// Closes a frontmatter through the crate's `set_status`. A task already closed with a
/// completed date keeps both; otherwise the status is the first completed value and
/// the date is `explicit`, else today.
fn complete(
    frontmatter: &Map<String, Value>,
    completed: &[String],
    explicit: Option<&str>,
    today: &str,
) -> Result<(String, String), OpError> {
    let current = frontmatter.get("status").and_then(Value::as_str);
    let stored = frontmatter.get("completedDate").and_then(Value::as_str);
    if let (Some(status), Some(date)) = (current, stored) {
        if completed.iter().any(|c| c == status) {
            return Ok((status.to_string(), date.to_string()));
        }
    }
    let target = completed.first().cloned().unwrap_or_default();
    let mut statuses = completed.to_vec();
    if let Some(status) = current {
        statuses.push(status.to_string());
    }
    let b = status_binding(&statuses, completed);
    let op = crate::set_status(&b, &view(frontmatter), &target, explicit, today)?;
    let merged = apply(frontmatter, &planned_fields(&op));
    Ok((
        merged["status"].as_str().unwrap_or_default().to_string(),
        merged["completedDate"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    ))
}

/// Merges a patch with the crate's field planner and reports whether a value moved.
fn update_patch(input: &Map<String, Value>) -> Result<Value, OpError> {
    let original = object(input, "original")?;
    let patch = object(input, "patch")?;
    let op = crate::plan_fields(&view(original), patch.clone());
    let frontmatter = apply(original, &planned_fields(&op));
    let changed = patch.iter().any(|(k, v)| original.get(k) != Some(v));
    Ok(json!({"changed": changed, "frontmatter": frontmatter}))
}

/// Puts back the default status through `set_status`; the completed date is cleared
/// when asked, else kept.
fn uncomplete(input: &Map<String, Value>, today: &str) -> Result<Value, OpError> {
    let frontmatter = object(input, "frontmatter")?;
    let default = text(input, "defaultStatus")?;
    let mut statuses = vec![default.to_string()];
    if let Some(current) = frontmatter.get("status").and_then(Value::as_str) {
        statuses.push(current.to_string());
    }
    let b = status_binding(&statuses, &[]);
    let op = crate::set_status(&b, &view(frontmatter), default, None, today)?;
    let mut merged = apply(frontmatter, &planned_fields(&op));
    if input.get("clearCompletedDate").and_then(Value::as_bool) == Some(true) {
        merged.insert("completedDate".into(), Value::Null);
    }
    Ok(json!({
        "status": merged.get("status").cloned().unwrap_or(Value::Null),
        "completedDate": merged.get("completedDate").cloned().unwrap_or(Value::Null),
    }))
}

/// `complete` is idempotent when completing its own result changes nothing; `create`
/// is idempotent when planning the same task twice gives the same write.
fn idempotency_check(input: &Map<String, Value>, today: &str) -> Result<Value, OpError> {
    match text(input, "operation")? {
        "complete_nonrecurring" => {
            let second = object(input, "second")?;
            let completed = completed_values(input)?;
            let (status, date) = complete(second, &completed, None, today)?;
            let same = second.get("status").and_then(Value::as_str) == Some(status.as_str())
                && second.get("completedDate").and_then(Value::as_str) == Some(date.as_str());
            Ok(json!({"idempotent": same}))
        }
        "create" => {
            let second = object(input, "second")?;
            let title = text(second, "title")?;
            let mut field = Map::new();
            field.insert(
                "status".into(),
                json!({"tn_role": "status", "values": ["open"]}),
            );
            let b = Binding::from_roles(&field);
            let config = TasksConfig::from_json(&json!({}))?;
            let n = NewTask {
                title: title.to_string(),
                ..NewTask::default()
            };
            let first = crate::create(&b, &config, &n)?;
            let again = crate::create(&b, &config, &n)?;
            Ok(json!({"idempotent": first == again}))
        }
        _ => Err(OpError::unsupported()),
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

fn opt_text<'a>(input: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

fn written<'a>(input: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, OpError> {
    Ok(temporal::written_date(text(input, key)?))
}

fn invalid_date(value: &str) -> OpError {
    OpError::new("invalid_date", format!("Invalid date: {value:?}"))
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

/// A list of strings. An absent key is empty; any other shape is refused.
fn string_list(input: &Map<String, Value>, key: &str) -> Result<Vec<String>, OpError> {
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
