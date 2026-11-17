//! The collection's tasks configuration, read from its JSON form.
//!
//! Every key is optional; an absent key takes its default. A key that is present
//! with the wrong shape is an error (`config_invalid`), never silently ignored.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::TaskError;

/// Tasks settings of a collection.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TasksConfig {
    /// Fields that must be present when a task is created (collection field names).
    pub required_on_create: Vec<String>,
    /// Person alias to the wikilinks that stand for that person in `assignees`.
    pub people: BTreeMap<String, Vec<String>>,
    /// Assignee values refused on write, mapped to the alias they should use instead.
    pub refused_assignees: BTreeMap<String, String>,
    /// Status a new task starts with, when the collection sets one.
    pub initial_status: Option<String>,
    /// Glob patterns of the paths where project notes live (`*` matches within one
    /// path segment). Empty: every record of the `projeto` type is a project.
    pub project_paths: Vec<String>,
}

impl TasksConfig {
    /// Reads the config from its JSON object. `null` gives the defaults.
    pub fn from_json(v: &Value) -> Result<TasksConfig, TaskError> {
        if v.is_null() {
            return Ok(TasksConfig::default());
        }
        let obj = v
            .as_object()
            .ok_or_else(|| invalid("the tasks config must be an object"))?;

        let required_on_create = match obj.get("required_on_create") {
            None => Vec::new(),
            Some(value) => strings(value, "required_on_create")?,
        };

        let people = match obj.get("people") {
            None => BTreeMap::new(),
            Some(value) => {
                let map = value
                    .as_object()
                    .ok_or_else(|| invalid("people must be an object"))?;
                map.iter()
                    .map(|(alias, list)| Ok((alias.clone(), strings(list, "people")?)))
                    .collect::<Result<_, TaskError>>()?
            }
        };

        let refused_assignees = match obj.get("refused_assignees") {
            None => BTreeMap::new(),
            Some(value) => {
                let map = value
                    .as_object()
                    .ok_or_else(|| invalid("refused_assignees must be an object"))?;
                map.iter()
                    .map(|(name, alias)| {
                        let alias = alias
                            .as_str()
                            .ok_or_else(|| invalid("refused_assignees values must be strings"))?;
                        Ok((name.clone(), alias.to_string()))
                    })
                    .collect::<Result<_, TaskError>>()?
            }
        };

        let initial_status = match obj.get("initial_status") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(invalid("initial_status must be a string")),
        };

        let project_paths = match obj.get("project_paths") {
            None => Vec::new(),
            Some(value) => strings(value, "project_paths")?,
        };

        Ok(TasksConfig {
            required_on_create,
            people,
            refused_assignees,
            initial_status,
            project_paths,
        })
    }
}

fn invalid(message: &str) -> TaskError {
    TaskError::new("config_invalid", message)
}

fn strings(v: &Value, key: &str) -> Result<Vec<String>, TaskError> {
    let arr = v
        .as_array()
        .ok_or_else(|| invalid(&format!("{key} must be an array of strings")))?;
    arr.iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| invalid(&format!("{key} must be an array of strings")))
        })
        .collect()
}
