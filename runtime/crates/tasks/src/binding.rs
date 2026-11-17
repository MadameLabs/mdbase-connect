//! The collection's `task` type, read once into the names the task commands need.
//!
//! Everything comes from the type definition: field names through the
//! `tasknotes.task` contract, statuses and their completed set from its `binding`,
//! the archive tag from `binding.archive`, and the path and defaults from
//! `collection`. Nothing is hardcoded to TaskNotes spelling.

use serde_json::{Map, Value};

use crate::TaskError;

/// The contract a task type must implement for the task commands.
const TASK_CONTRACT: &str = "tasknotes.task";

/// The task type of a collection, bound to the names the commands use.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// The type's name (`task`).
    pub type_name: String,
    /// Contract field name to collection field name (e.g. `recurrenceAnchor` to
    /// `recurrence_anchor`).
    pub fields: std::collections::BTreeMap<String, String>,
    /// All status values the type allows, in declaration order.
    pub statuses: Vec<String>,
    /// The status a task gets when none is written.
    pub default_status: String,
    /// Statuses that close a task.
    pub completed: Vec<String>,
    /// Tag that marks an archived task; empty when the type declares no archive.
    pub archived_tag: String,
    /// Folder where tasks live (`collection.path.folder`).
    pub folder: Option<String>,
    /// Filename template for new tasks (`collection.path.template`).
    pub path_template: Option<String>,
    /// Defaults applied on read (`collection.read_defaults`).
    pub read_defaults: Map<String, Value>,
}

/// The field roles the contract names. A field whose name is a role maps to it
/// without `tn_role`.
pub const ROLES: &[&str] = &[
    "title",
    "status",
    "priority",
    "due",
    "scheduled",
    "completedDate",
    "tags",
    "contexts",
    "projects",
    "attachments",
    "timeEstimate",
    "dateCreated",
    "dateModified",
    "recurrence",
    "recurrenceAnchor",
    "completeInstances",
    "skippedInstances",
    "timeEntries",
];

/// Status names read as closed when a status field declares no completed values.
const CLOSED_NAMES: &[&str] = &["done", "completed", "cancelled", "canceled"];
/// Completed values when neither the field declares them nor its names say so.
const DEFAULT_COMPLETED: &[&str] = &["done", "cancelled"];

impl Binding {
    /// Binds a parsed `task` type definition (the JSON of its frontmatter).
    ///
    /// Fails with `type_without_tasknotes_contract` when the type does not implement
    /// the `tasknotes.task` contract, and with `type_without_task_status` when that
    /// contract declares no status values.
    pub fn from_type(def: &Value) -> Result<Binding, TaskError> {
        let implements = def
            .get("implements")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        let contract = implements
            .into_iter()
            .find(|c| c.get("contract").and_then(Value::as_str) == Some(TASK_CONTRACT))
            .ok_or_else(|| {
                TaskError::new(
                    "type_without_tasknotes_contract",
                    format!("the type does not implement {TASK_CONTRACT}"),
                )
            })?;

        let fields = contract
            .get("fields")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let status = contract
            .get("binding")
            .and_then(|b| b.get("status"))
            .ok_or_else(|| {
                TaskError::new(
                    "type_without_task_status",
                    "the tasknotes.task binding declares no status",
                )
            })?;
        let statuses = string_list(status.get("values"));
        if statuses.is_empty() {
            return Err(TaskError::new(
                "type_without_task_status",
                "the tasknotes.task binding declares no status values",
            ));
        }
        let default_status = status
            .get("default")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let completed = string_list(status.get("completed_values"));

        let archived_tag = contract
            .get("binding")
            .and_then(|b| b.get("archive"))
            .and_then(|a| a.get("archived_tag"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let collection = def.get("collection");
        let path = collection.and_then(|c| c.get("path"));
        let folder = path
            .and_then(|p| p.get("folder"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let path_template = path
            .and_then(|p| p.get("template"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let read_defaults = collection
            .and_then(|c| c.get("read_defaults"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        Ok(Binding {
            type_name: def
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            fields,
            statuses,
            default_status,
            completed,
            archived_tag,
            folder,
            path_template,
            read_defaults,
        })
    }

    /// The collection field name for a contract field. A contract field the type
    /// does not map is returned unchanged.
    pub fn field<'a>(&'a self, contract_name: &'a str) -> &'a str {
        self.fields
            .get(contract_name)
            .map(String::as_str)
            .unwrap_or(contract_name)
    }

    /// The frontmatter with the type's `read_defaults` filled in. Keys already present
    /// keep their value.
    pub fn effective(&self, frontmatter: &Map<String, Value>) -> Map<String, Value> {
        let mut out = frontmatter.clone();
        for (key, value) in &self.read_defaults {
            out.entry(key.clone()).or_insert_with(|| value.clone());
        }
        out
    }

    /// Binds a collection's own field names to the roles. A field takes its `tn_role`,
    /// or its name when the name is a role; other fields are not mapped; when two fields
    /// take one role, the first in name order maps to it. The status
    /// field gives the statuses and the completed ones: its `tn_completed_values`,
    /// else the values whose names read as closed, else done and cancelled.
    pub fn from_roles(fields: &Map<String, Value>) -> Binding {
        let mut mapped = std::collections::BTreeMap::new();
        let mut status_def: Option<&Map<String, Value>> = None;
        for (name, def) in fields {
            let def = def.as_object();
            let role = def
                .and_then(|d| d.get("tn_role"))
                .and_then(Value::as_str)
                .or_else(|| ROLES.contains(&name.as_str()).then_some(name.as_str()));
            let Some(role) = role else { continue };
            if role == "status" && status_def.is_none() {
                status_def = def;
            }
            // The first field of a role, in name order, is the one it maps to.
            mapped
                .entry(role.to_string())
                .or_insert_with(|| name.clone());
        }
        let values = status_def
            .and_then(|d| d.get("values"))
            .map(|v| string_list(Some(v)))
            .unwrap_or_default();
        let completed = match status_def.and_then(|d| d.get("tn_completed_values")) {
            Some(explicit) => string_list(Some(explicit)),
            None => {
                let inferred: Vec<String> = values
                    .iter()
                    .filter(|v| CLOSED_NAMES.contains(&v.as_str()))
                    .cloned()
                    .collect();
                if inferred.is_empty() {
                    DEFAULT_COMPLETED.iter().map(|s| s.to_string()).collect()
                } else {
                    inferred
                }
            }
        };
        let default_status = values.first().cloned().unwrap_or_default();
        Binding {
            type_name: String::new(),
            fields: mapped,
            statuses: values,
            default_status,
            completed,
            archived_tag: String::new(),
            folder: None,
            path_template: None,
            read_defaults: Map::new(),
        }
    }

    /// The role a collection field plays, when it maps to one.
    pub fn role_of(&self, field: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(_, f)| f.as_str() == field)
            .map(|(role, _)| role.as_str())
    }

    /// The frontmatter with each mapped field renamed to its role; other keys stay.
    pub fn normalize(&self, frontmatter: &Map<String, Value>) -> Map<String, Value> {
        frontmatter
            .iter()
            .map(|(key, value)| {
                let name = self.role_of(key).unwrap_or(key).to_string();
                (name, value.clone())
            })
            .collect()
    }

    /// The inverse of [`Binding::normalize`]: each role renamed to its field; other keys stay.
    pub fn denormalize(&self, role_data: &Map<String, Value>) -> Map<String, Value> {
        role_data
            .iter()
            .map(|(key, value)| (self.field(key).to_string(), value.clone()))
            .collect()
    }

    /// The title shown for a task: the value of `key` when it is not blank; for a
    /// title field, the `title` value; else the note's file name without `.md`.
    /// Blank values count as absent. Values come back as written.
    pub fn display_title(
        &self,
        frontmatter: &Map<String, Value>,
        key: &str,
        task_path: Option<&str>,
    ) -> Option<String> {
        let text = |k: &str| {
            frontmatter
                .get(k)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
        };
        text(key)
            .or_else(|| {
                (self.role_of(key) == Some("title"))
                    .then(|| text("title"))
                    .flatten()
            })
            .or_else(|| {
                let name = task_path?.rsplit('/').next()?;
                let stem = name.strip_suffix(".md").unwrap_or(name);
                (!stem.is_empty()).then(|| stem.to_string())
            })
    }

    /// Whether a status closes a task.
    pub fn is_closed(&self, status: &str) -> bool {
        self.completed.iter().any(|c| c == status)
    }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
