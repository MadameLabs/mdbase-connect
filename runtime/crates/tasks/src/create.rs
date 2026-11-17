//! Creating and promoting tasks under the collection's rules.
//!
//! `create` plans one `create` write for a new task; `promote` plans one `update`
//! write that turns an existing note into a task. Both read the collection's
//! tasks configuration ([`TasksConfig`]) for the initial status, the required
//! fields and the person aliases, and the type ([`Binding`]) for the folder, the
//! filename template and the statuses. Lifecycle fields (`id`, `dateCreated`,
//! `dateModified`) are not written: the type sets them.
//!
//! Portable: no I/O, no clock, no entropy.

use serde_json::{Map, Value, json};

use crate::plan::{check_status, is_date, list_of, update};
use crate::{Binding, TaskError, TaskView, TasksConfig, wikilink};

/// Tag that makes a note a task; the type's `where` rule matches on it.
const TASK_TAG: &str = "task";
/// Frontmatter key of the scalar type of a note.
const TYPE_KEY: &str = "type";
/// Frontmatter key of the list of types of a note.
const TYPES_KEY: &str = "types";
/// Frontmatter key of a date-only creation date, read when `dateCreated` is missing.
const CREATED_KEY: &str = "created";
/// Frontmatter key of a date-only date, read when neither `dateCreated` nor
/// `created` is set.
const DATA_KEY: &str = "data";
/// Placeholder in the type's filename template.
const TITLE_PLACEHOLDER: &str = "{{title}}";
/// Characters the TaskNotes filename sanitizer removes (not replaces).
const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*', '#', '[', ']'];

/// What the user gave for a new task or a promoted note.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewTask {
    /// Title of the task.
    pub title: String,
    /// Person names or aliases, resolved through [`resolve_person`].
    pub assignees: Vec<String>,
    /// Brand the task belongs to; stored under the `brand` field.
    pub brand: Option<String>,
    /// Project names or notes, stored as wikilinks.
    pub projects: Vec<String>,
}

/// The path a new task gets: the type's folder, its filename template with
/// `{{title}}` replaced by the sanitized title, and `.md`.
pub fn path_for(b: &Binding, title: &str) -> String {
    let name = sanitize_segment(title);
    let template = b.path_template.as_deref().unwrap_or(TITLE_PLACEHOLDER);
    let file = template.replace(TITLE_PLACEHOLDER, &name);
    match b.folder.as_deref().filter(|folder| !folder.is_empty()) {
        Some(folder) => format!("{folder}/{file}.md"),
        None => format!("{file}.md"),
    }
}

/// Plans a `create` write for a new task. The task starts in the configured
/// initial status (or the type's default) and carries the `task` tag. Every field
/// in `required_on_create` must be filled.
pub fn create(b: &Binding, c: &TasksConfig, n: &NewTask) -> Result<Value, TaskError> {
    let title = title_of(n)?;
    let mut fields = Map::new();
    fields.insert(b.field("title").to_string(), json!(title));
    fields.insert(b.field("status").to_string(), json!(initial_status(b, c)?));
    fields.insert(b.field("tags").to_string(), json!([TASK_TAG]));
    insert_list(
        &mut fields,
        b.field("assignees"),
        people_of(c, &n.assignees)?,
    );
    insert_list(&mut fields, "brand", brand_of(n));
    insert_list(&mut fields, b.field("projects"), links(&n.projects));

    require(c, &fields, &Map::new())?;
    Ok(json!({
        "op": "create",
        "path": path_for(b, &title),
        "type": b.type_name,
        "fields": fields,
    }))
}

/// Plans an `update` write that promotes an existing note to a task. The `task`
/// tag is added to the note's tags, the initial status is set, and assignees,
/// brand and projects are added to the note's own lists without duplicates. The
/// note's explicit type list (`types`, or the scalar `type`) gets the task type
/// added, so the collection's type rules see the note as a task too; `type` itself
/// is never rewritten. The write carries the note's revision. The title is not
/// changed.
///
/// A note without a creation date (`dateCreated`) gets one: from its `created`
/// field, else its `data` field (a date becomes midnight in the zone of `now`), else `now`, the
/// caller's clock as a date-time. An existing `dateCreated` is never overwritten.
///
/// A note that already carries the `task` tag is repaired, not restarted: its
/// status is kept as written and only what is missing is filled in.
pub fn promote(
    b: &Binding,
    c: &TasksConfig,
    note: &TaskView,
    n: &NewTask,
    now: &str,
) -> Result<Value, TaskError> {
    let mut fields = Map::new();
    if !has_task_tag(b, note) {
        fields.insert(b.field("status").to_string(), json!(initial_status(b, c)?));
    }
    let created = b.field("dateCreated");
    if !filled(note.frontmatter.get(created)) {
        fields.insert(created.to_string(), json!(creation_date(note, now)));
    }
    fields.insert(
        b.field("tags").to_string(),
        Value::Array(merge(
            list_of(note, b.field("tags")),
            &[TASK_TAG.to_string()],
        )),
    );
    if let Some(types) = types_with_task(b, note) {
        fields.insert(TYPES_KEY.to_string(), Value::Array(types));
    }
    merge_into(
        &mut fields,
        note,
        b.field("assignees"),
        &people_of(c, &n.assignees)?,
    );
    merge_into(&mut fields, note, "brand", &brand_of(n));
    merge_into(&mut fields, note, b.field("projects"), &links(&n.projects));

    require(c, &fields, &note.frontmatter)?;
    Ok(update(note, fields))
}

/// The creation date for a note that has none: `created`, else `data`, else `now`.
/// A date-only value becomes midnight in the zone of `now`, so it keeps its local
/// day; a value that already has a time is used as written.
fn creation_date(note: &TaskView, now: &str) -> String {
    let zone = zone_of(now);
    [CREATED_KEY, DATA_KEY]
        .iter()
        .find_map(|key| as_date_time(note.frontmatter.get(*key), zone))
        .unwrap_or_else(|| now.to_string())
}

/// The zone of a date-time: `Z`, or the trailing `±HH:MM`. A value without one
/// counts as `Z`.
fn zone_of(date_time: &str) -> &str {
    match date_time.rfind(['+', '-']).filter(|i| *i > 10) {
        Some(i) => &date_time[i..],
        None => "Z",
    }
}

fn as_date_time(value: Option<&Value>, zone: &str) -> Option<String> {
    let text = value?.as_str()?.trim();
    if text.len() == 10 && is_date(text) {
        return Some(format!("{text}T00:00:00{zone}"));
    }
    let day = text.get(..10)?;
    let rest = text.get(10..)?;
    (is_date(day) && rest.starts_with('T')).then(|| text.to_string())
}

fn has_task_tag(b: &Binding, note: &TaskView) -> bool {
    list_of(note, b.field("tags"))
        .iter()
        .any(|v| v.as_str() == Some(TASK_TAG))
}

/// The note's explicit types with the task type added, or `None` when there is
/// nothing to add. The list is `types` when the note has one; otherwise the scalar
/// `type`. A note with no types, or already typed as a task, gets no change.
fn types_with_task(b: &Binding, note: &TaskView) -> Option<Vec<Value>> {
    let task = b.type_name.as_str();
    let mut base = list_of(note, TYPES_KEY);
    if base.is_empty() {
        let scalar = note
            .frontmatter
            .get(TYPE_KEY)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        base = scalar.map(|s| json!(s)).into_iter().collect();
    }
    if base.is_empty() || base.iter().any(|v| v.as_str() == Some(task)) {
        return None;
    }
    Some(merge(base, &[task.to_string()]))
}

/// The people a name or alias stands for, as wikilinks.
///
/// A refused name (`refused_assignees`) fails with `responsavel_antigo`, naming the
/// alias to use. An alias (`people`) gives its configured wikilinks. Any other
/// name is used as given, as a wikilink; checking that a person note exists is the
/// caller's job.
pub fn resolve_person(c: &TasksConfig, who: &str) -> Result<Vec<String>, TaskError> {
    let raw = strip_brackets(who);
    if raw.is_empty() {
        return Err(TaskError::new(
            "responsavel_desconhecido",
            "the person name is empty",
        ));
    }
    if let Some(alias) = c.refused_assignees.get(raw) {
        let detail = match c.people.get(alias) {
            Some(list) => format!("use {alias} ({})", list.join(", ")),
            None => format!("use {alias}"),
        };
        return Err(TaskError::new("responsavel_antigo", detail));
    }
    if let Some(list) = c.people.get(&raw.to_lowercase()) {
        return Ok(list.clone());
    }
    Ok(vec![wikilink(raw)])
}

fn title_of(n: &NewTask) -> Result<String, TaskError> {
    let title = n.title.trim();
    if sanitize_segment(title).is_empty() {
        return Err(TaskError::new("missing_title", "the task needs a title"));
    }
    Ok(title.to_string())
}

fn initial_status(b: &Binding, c: &TasksConfig) -> Result<String, TaskError> {
    let status = c
        .initial_status
        .clone()
        .unwrap_or_else(|| b.default_status.clone());
    check_status(b, &status)?;
    Ok(status)
}

/// Resolves every name and keeps each person once, in order.
fn people_of(c: &TasksConfig, names: &[String]) -> Result<Vec<String>, TaskError> {
    let mut people: Vec<String> = Vec::new();
    for name in names {
        for person in resolve_person(c, name)? {
            if !people.contains(&person) {
                people.push(person);
            }
        }
    }
    Ok(people)
}

fn brand_of(n: &NewTask) -> Vec<String> {
    match &n.brand {
        Some(brand) => links(std::slice::from_ref(brand)),
        None => Vec::new(),
    }
}

/// Wikilinks for the given names, blanks dropped and duplicates removed.
fn links(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for value in values {
        if value.trim().is_empty() {
            continue;
        }
        let link = wikilink(value);
        if !out.contains(&link) {
            out.push(link);
        }
    }
    out
}

/// Adds the given values to a list. Items already there keep their place, and
/// each new value is added once.
fn merge(existing: Vec<Value>, added: &[String]) -> Vec<Value> {
    let mut out = existing;
    for value in added {
        if !out.iter().any(|item| item.as_str() == Some(value.as_str())) {
            out.push(json!(value));
        }
    }
    out
}

fn merge_into(fields: &mut Map<String, Value>, note: &TaskView, key: &str, added: &[String]) {
    if added.is_empty() {
        return;
    }
    fields.insert(
        key.to_string(),
        Value::Array(merge(list_of(note, key), added)),
    );
}

fn insert_list(fields: &mut Map<String, Value>, key: &str, values: Vec<String>) {
    if !values.is_empty() {
        fields.insert(key.to_string(), json!(values));
    }
}

/// Fails with `missing_required_field` when a required field is neither written by
/// this op nor already filled in the note.
fn require(
    c: &TasksConfig,
    fields: &Map<String, Value>,
    existing: &Map<String, Value>,
) -> Result<(), TaskError> {
    for name in &c.required_on_create {
        let value = fields.get(name).or_else(|| existing.get(name));
        if !filled(value) {
            return Err(TaskError::new(
                "missing_required_field",
                format!("the required field {name} is missing"),
            ));
        }
    }
    Ok(())
}

fn filled(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(_) => true,
    }
}

/// The name as TaskNotes writes it in a file name: whitespace collapsed, forbidden
/// characters and control characters removed, leading and trailing dots dropped.
fn sanitize_segment(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let kept: String = collapsed
        .chars()
        .filter(|c| !FORBIDDEN.contains(c) && !c.is_control())
        .collect();
    kept.trim_matches('.').trim().to_string()
}

fn strip_brackets(value: &str) -> &str {
    let inner = value.trim();
    let inner = inner.strip_prefix("[[").unwrap_or(inner);
    let inner = inner.strip_suffix("]]").unwrap_or(inner);
    inner.trim()
}
