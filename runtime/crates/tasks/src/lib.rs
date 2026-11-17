//! # mdbn-tasks: task semantics for `mdbase tasks`
//!
//! **Responsibility.** Pure rules for the task commands: how a collection's `task`
//! type maps onto task fields, statuses and paths ([`Binding`]), and how the
//! collection's tasks configuration is read ([`TasksConfig`]). Nothing here is
//! hardcoded to TaskNotes names: every field, status, archive tag and path comes
//! from the type definition the collection carries.
//!
//! Portable: no internal dependencies, no I/O, no clock, no entropy.

mod binding;
mod config;
pub mod conformance;
mod create;
mod plan;
mod projects;
mod temporal;
mod timer;

pub use binding::Binding;
pub use config::TasksConfig;
pub use create::{NewTask, create, path_for, promote, resolve_person};
pub use plan::{
    TaskView, archive, block, is_open, link_projects, set_status, skip, unblock, unskip, wikilink,
};
pub use projects::{
    Orphan, ProjectItem, ProjectsSummary, ReviewItem, filter_queue_by_owner, is_project_path,
    is_valid_date, list_projects, path_matches, project_assign, project_milestone, project_name,
    project_rename_status, project_review, project_status, projects_summary,
};
pub use timer::{OpenEntry, PERSON_PREFIX, open_entries, parse_instant, retime, start, stop};

/// An error with a stable machine code and a human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskError {
    /// Stable code, e.g. `type_without_tasknotes_contract`.
    pub code: &'static str,
    /// Human-readable detail.
    pub message: String,
}

impl TaskError {
    /// Builds an error from a code and a message.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
