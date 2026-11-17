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
