//! The lifecycle of project notes: status and owner transitions, the next milestone,
//! the status rename, and the summary and review queue over the project's tasks.
//!
//! A project is a record of the `projeto` type. Its `status` takes the values of the
//! collection's `task` type ([`Binding::statuses`]). The rules are the ones the
//! tasknotes-agent adapter applies: a status that closes or pauses the project
//! demands an outcome, and `pausa` demands a future resume date. Writes are planned as
//! `update` ops ([`crate::plan`]), carrying the revision the project was read at.
//!
//! "Last real activity" comes from the time entries, the completion date or the
//! creation date of the linked tasks, never from `dateModified`.
//!
//! Dates are `YYYY-MM-DD` strings and "today" is passed in. Nothing here reads a
//! clock or does I/O. Portable: serde_json only.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::plan::{list_of, note_name, update};
use crate::temporal::{self, civil_from_days, days_from_civil};
use crate::{Binding, TaskError, TaskView, TasksConfig, wikilink};

/// Statuses that keep the project open: they clear outcome, outcome date and resume.
const OPEN: [&str; 3] = ["backlog", "to-do", "em-andamento"];
/// Statuses that need an outcome to be written.
const NEEDS_OUTCOME: [&str; 4] = ["pausa", "aprovacao", "concluido", "cancelado"];
const PAUSA: &str = "pausa";
const CONCLUIDO: &str = "concluido";
const CANCELADO: &str = "cancelado";
const APROVACAO: &str = "aprovacao";
const EM_ANDAMENTO: &str = "em-andamento";

/// The review reasons, in the order the queue lists them.
const REASONS: [&str; 7] = [
    "desfecho_ausente",
    "retomada_vencida",
    "pausa_sem_retomada",
    "sem_responsavel",
    "status_desconhecido",
    "parado",
    "aguardando_aprovacao",
];

/// Plans a status change of a project. Returns the `update` op.
///
/// `desfecho` and `retomar_em` are `None` when not given. The owner must be a single
/// wikilink. `retomar_em` only goes with `pausa`. An open status clears the outcome,
/// its date and the resume date. Any other status records the outcome and `today`,
/// and `pausa` records the resume date, which must be a valid date after `today`.
pub fn project_status(
    b: &Binding,
    p: &TaskView,
    status: &str,
    desfecho: Option<&str>,
    retomar_em: Option<&str>,
    today: &str,
) -> Result<Value, TaskError> {
    check_status(b, status)?;
    if owner_link(p.frontmatter.get("owner")).is_none() {
        return Err(TaskError::new(
            "owner_obrigatorio",
            "the project needs an owner, a person's wikilink, before its status changes",
        ));
    }
    if retomar_em.is_some() && status != PAUSA {
        return Err(TaskError::new(
            "retomada_so_em_pausa",
            "a resume date only goes with the pausa status",
        ));
    }

    let mut fields = Map::new();
    fields.insert("status".into(), json!(status));
    if OPEN.contains(&status) {
        fields.insert("outcome".into(), Value::Null);
        fields.insert("outcomeDate".into(), Value::Null);
        fields.insert("resumeOn".into(), Value::Null);
        return Ok(update(p, fields));
    }

    let outcome = desfecho.map_or("", str::trim);
    if outcome.is_empty() {
        return Err(TaskError::new(
            "desfecho_obrigatorio",
            format!("status {status} needs an outcome"),
        ));
    }
    let resume_on = if status == PAUSA {
        let date = retomar_em
            .ok_or_else(|| TaskError::new("retomada_obrigatoria", "pausa needs a resume date"))?;
        if !is_valid_date(date) {
            return Err(TaskError::new(
                "retomada_invalida",
                format!("{date} is not a YYYY-MM-DD date"),
            ));
        }
        if date <= today {
            return Err(TaskError::new(
                "retomada_no_passado",
                format!("the resume date {date} must be after {today}"),
            ));
        }
        Value::String(date.to_string())
    } else {
        Value::Null
    };
    fields.insert("outcome".into(), json!(outcome));
    fields.insert("outcomeDate".into(), json!(today));
    fields.insert("resumeOn".into(), resume_on);
    Ok(update(p, fields))
}

/// Plans the owner of a project. `people` are the wikilinks the name stands for. A
/// group (more than one person) is refused: a project has one owner.
pub fn project_assign(p: &TaskView, people: &[String]) -> Result<Value, TaskError> {
    match people {
        [] => Err(TaskError::new("missing_responsavel", "no person given")),
        [person] => {
            let mut fields = Map::new();
            fields.insert("owner".into(), json!(wikilink(person)));
            Ok(update(p, fields))
        }
        _ => Err(TaskError::new(
            "responsavel_grupo",
            "a project has one owner, not a group",
        )),
    }
}

/// Plans the next milestone: a non-empty text and an optional valid date, which may
/// be in the past. `limpar` clears both. The milestone needs no owner.
pub fn project_milestone(
    p: &TaskView,
    marco: Option<&str>,
    data: Option<&str>,
    limpar: bool,
) -> Result<Value, TaskError> {
    let mut fields = Map::new();
    if limpar {
        fields.insert("nextMilestone".into(), Value::Null);
        fields.insert("nextMilestoneDate".into(), Value::Null);
        return Ok(update(p, fields));
    }
    let text = marco.map_or("", str::trim);
    if text.is_empty() {
        return Err(TaskError::new(
            "marco_obrigatorio",
            "the milestone needs a text",
        ));
    }
    if let Some(date) = data.filter(|d| !is_valid_date(d)) {
        return Err(TaskError::new(
            "data_invalida",
            format!("{date} is not a YYYY-MM-DD date"),
        ));
    }
    fields.insert("nextMilestone".into(), json!(text));
    fields.insert(
        "nextMilestoneDate".into(),
        data.map_or(Value::Null, |d| json!(d)),
    );
    Ok(update(p, fields))
}

/// Plans the priority of a project: one of the values the project type allows in
/// its `priority` enum, trimmed and matched exactly. `limpar` clears it. The priority
/// needs no owner.
pub fn project_priority(
    p: &TaskView,
    valor: Option<&str>,
    permitidos: &[String],
    limpar: bool,
) -> Result<Value, TaskError> {
    let mut fields = Map::new();
    if limpar {
        fields.insert("priority".into(), Value::Null);
        return Ok(update(p, fields));
    }
    if permitidos.is_empty() {
        return Err(TaskError::new(
            "prioridade_indisponivel",
            "the project type declares no priority values",
        ));
    }
    let value = valor.map_or("", str::trim);
    if !permitidos.iter().any(|v| v == value) {
        return Err(TaskError::new(
            "prioridade_invalida",
            format!("{value:?} is not one of: {}", permitidos.join(", ")),
        ));
    }
    fields.insert("priority".into(), json!(value));
    Ok(update(p, fields))
}

/// Plans the rename of a status value: one op per project whose `status` is exactly
/// `de`, setting it to `para`. No transition rules apply, so a legacy project keeps
/// its owner and outcome as they are. `para` must be a status of the task type. The
/// ops are sorted by path. Nothing is written here.
pub fn project_rename_status(
    b: &Binding,
    projects: &[TaskView],
    de: &str,
    para: &str,
) -> Result<Vec<Value>, TaskError> {
    check_status(b, para)?;
    let mut targets: Vec<&TaskView> = projects
        .iter()
        .filter(|p| p.frontmatter.get("status").and_then(Value::as_str) == Some(de))
        .collect();
    targets.sort_by(|a, c| a.path.cmp(&c.path));
    Ok(targets
        .into_iter()
        .map(|p| {
            let mut fields = Map::new();
            fields.insert("status".into(), json!(para));
            update(p, fields)
        })
        .collect())
}

/// A project as the commands list it: its fields, and the state of its tasks.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectItem {
    /// Path of the project note.
    pub path: String,
    /// The note's name: its file name without `.md`.
    pub name: String,
    /// The `brand` field as stored (`null` when absent).
    pub brand: Value,
    /// The status, when it is a status of the task type; otherwise `None`.
    pub status: Option<String>,
    /// The owner, when it is a single wikilink; otherwise `None`.
    pub owner: Option<String>,
    /// The outcome, as stored.
    pub outcome: Option<String>,
    /// The day the outcome was recorded.
    pub outcome_date: Option<String>,
    /// The day the project resumes (`pausa`).
    pub resume_on: Option<String>,
    /// The next milestone, as stored.
    pub next_milestone: Option<String>,
    /// The day of the next milestone, when it has one.
    pub next_milestone_date: Option<String>,
    /// The priority, as stored.
    pub priority: Option<String>,
    /// Linked tasks that are not closed.
    pub open_tasks: usize,
    /// Linked tasks, closed or not.
    pub total_tasks: usize,
    /// The latest real activity of the linked tasks.
    pub last_activity: Option<String>,
    /// Paths of the linked tasks that are not closed, in the order given.
    pub open_task_paths: Vec<String>,
}

impl ProjectItem {
    /// The item as the commands print it.
    pub fn to_json(&self) -> Value {
        json!({
            "path": self.path,
            "name": self.name,
            "brand": self.brand,
            "status": self.status,
            "owner": self.owner,
            "outcome": self.outcome,
            "outcomeDate": self.outcome_date,
            "resumeOn": self.resume_on,
            "nextMilestone": self.next_milestone,
            "nextMilestoneDate": self.next_milestone_date,
            "priority": self.priority,
            "openTasks": self.open_tasks,
            "totalTasks": self.total_tasks,
            "lastActivity": self.last_activity,
            "openTaskPaths": self.open_task_paths,
        })
    }
}

/// A task link naming no project that exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    /// The name the links use.
    pub name: String,
    /// The tasks that link to it.
    pub tasks: Vec<String>,
}

/// Every project with its task counts, and the links to projects that do not exist.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectsSummary {
    /// One item per project, sorted by path.
    pub items: Vec<ProjectItem>,
    /// Orphan links, sorted by name.
    pub orphans: Vec<Orphan>,
}

/// Summarizes the projects against the tasks. Counts are information only: nothing
/// here changes a project's status.
pub fn projects_summary(b: &Binding, projects: &[TaskView], tasks: &[TaskView]) -> ProjectsSummary {
    let names: BTreeSet<String> = projects.iter().map(|p| note_name(&p.path)).collect();
    let mut by_project: BTreeMap<String, Vec<&TaskView>> = BTreeMap::new();
    let mut orphans: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for task in tasks {
        let linked: BTreeSet<String> = list_of(task, b.field("projects"))
            .iter()
            .filter_map(Value::as_str)
            .map(note_name)
            .filter(|name| !name.is_empty())
            .collect();
        for name in linked {
            if names.contains(&name) {
                by_project.entry(name).or_default().push(task);
            } else {
                orphans.entry(name).or_default().push(task.path.clone());
            }
        }
    }

    let mut sorted: Vec<&TaskView> = projects.iter().collect();
    sorted.sort_by(|a, c| a.path.cmp(&c.path));
    let items = sorted
        .into_iter()
        .map(|p| {
            let name = note_name(&p.path);
            let linked: &[&TaskView] = by_project.get(&name).map_or(&[], Vec::as_slice);
            let open: Vec<&&TaskView> = linked
                .iter()
                .filter(|t| !b.is_closed(&status_of(b, t)))
                .collect();
            let fm = &p.frontmatter;
            let status =
                string_of(fm.get("status")).filter(|s| b.statuses.iter().any(|known| known == s));
            ProjectItem {
                path: p.path.clone(),
                name,
                brand: fm.get("brand").cloned().unwrap_or(Value::Null),
                status,
                owner: owner_link(fm.get("owner")),
                outcome: string_of(fm.get("outcome")),
                outcome_date: day(string_of(fm.get("outcomeDate")).as_deref()),
                resume_on: day(string_of(fm.get("resumeOn")).as_deref()),
                next_milestone: string_of(fm.get("nextMilestone")),
                next_milestone_date: day(string_of(fm.get("nextMilestoneDate")).as_deref()),
                priority: string_of(fm.get("priority")),
                open_tasks: open.len(),
                total_tasks: linked.len(),
                last_activity: linked.iter().filter_map(|t| last_activity(b, t)).max(),
                open_task_paths: open.iter().map(|t| t.path.clone()).collect(),
            }
        })
        .collect();

    let orphans = orphans
        .into_iter()
        .map(|(name, tasks)| Orphan { name, tasks })
        .collect();
    ProjectsSummary { items, orphans }
}

/// The projects `projects list` shows. With a status, only those in it (a status of
/// the task type, refused otherwise). Without one, every open project, so closed
/// ones are hidden. With an owner, only that person's projects.
pub fn list_projects(
    b: &Binding,
    items: &[ProjectItem],
    status: Option<&str>,
    owner: Option<&str>,
) -> Result<Vec<ProjectItem>, TaskError> {
    if let Some(wanted) = status {
        check_status(b, wanted)?;
    }
    Ok(items
        .iter()
        .filter(|item| {
            let status_ok = match status {
                Some(wanted) => item.status.as_deref() == Some(wanted),
                None => item.status.as_deref().is_some_and(|s| !b.is_closed(s)),
            };
            status_ok && owner.is_none_or(|o| item.owner.as_deref() == Some(o))
        })
        .cloned()
        .collect())
}

/// Whether a record at `path` is a project note under the collection's config: any
/// path when the config names no project paths, otherwise one that matches a pattern.
pub fn is_project_path(config: &TasksConfig, path: &str) -> bool {
    config.project_paths.is_empty() || config.project_paths.iter().any(|p| path_matches(p, path))
}

/// Whether a collection path matches a glob pattern. `*` matches any run of
/// characters within one path segment (never a `/`); everything else matches itself.
/// The whole path must match.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    glob_match(pattern.as_bytes(), path.as_bytes())
}

fn glob_match(pattern: &[u8], path: &[u8]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((b'*', rest)) => {
            let segment = path.iter().position(|&c| c == b'/').unwrap_or(path.len());
            (0..=segment).any(|i| glob_match(rest, &path[i..]))
        }
        Some((&c, rest)) => path
            .split_first()
            .is_some_and(|(&pc, path_rest)| pc == c && glob_match(rest, path_rest)),
    }
}

/// The name a project is known by, from its path or a link to it: the note name.
pub fn project_name(value: &str) -> String {
    note_name(value)
}

/// A project in the review queue, with the reason it is there.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewItem {
    /// Why the project needs a decision; one of the review reasons.
    pub reason: &'static str,
    /// The project.
    pub item: ProjectItem,
}

impl ReviewItem {
    /// The item as the commands print it: the project's fields and its `reason`.
    pub fn to_json(&self) -> Value {
        let mut out = self.item.to_json();
        out["reason"] = json!(self.reason);
        out
    }
}

/// The review queue: the projects that need a decision, most urgent reason first and
/// by name within a reason. Projects that are closed are only listed when their
/// status demands an outcome they lack. A project not active for more than `parado_ha`
/// days before `today` (or never active) counts as stalled. `today` is the caller's
/// local date.
pub fn project_review(items: &[ProjectItem], today: &str, parado_ha: u32) -> Vec<ReviewItem> {
    let limite = shift_days(today, parado_ha).unwrap_or_else(|| today.to_string());
    let mut queue: Vec<ReviewItem> = items
        .iter()
        .filter_map(|item| {
            reason_for(item, today, &limite).map(|reason| ReviewItem {
                reason,
                item: item.clone(),
            })
        })
        .collect();
    queue.sort_by(|a, c| {
        reason_rank(a.reason)
            .cmp(&reason_rank(c.reason))
            .then_with(|| a.item.name.cmp(&c.item.name))
    });
    queue
}

/// A person's view of the queue: the projects they own, and every project without an
/// owner, whatever the reason it is there, since assigning it is the triage.
pub fn filter_queue_by_owner(queue: Vec<ReviewItem>, owner: &str) -> Vec<ReviewItem> {
    queue
        .into_iter()
        .filter(|entry| entry.item.owner.as_deref().is_none_or(|o| o == owner))
        .collect()
}

fn reason_for(item: &ProjectItem, today: &str, limite: &str) -> Option<&'static str> {
    let status = item.status.as_deref();
    let outcome_missing = item.outcome.as_deref().is_none_or(str::is_empty);
    if status.is_some_and(|s| NEEDS_OUTCOME.contains(&s)) && outcome_missing {
        return Some("desfecho_ausente");
    }
    if matches!(status, Some(CONCLUIDO | CANCELADO)) {
        return None;
    }
    if status == Some(PAUSA) {
        match item.resume_on.as_deref() {
            Some(resume) if resume <= today => return Some("retomada_vencida"),
            None => return Some("pausa_sem_retomada"),
            Some(_) => {}
        }
    }
    if item.owner.is_none() {
        return Some("sem_responsavel");
    }
    if item.status.is_none() {
        return Some("status_desconhecido");
    }
    if status == Some(EM_ANDAMENTO)
        && item
            .last_activity
            .as_deref()
            .is_none_or(|last| last < limite)
    {
        return Some("parado");
    }
    if status == Some(APROVACAO) {
        return Some("aguardando_aprovacao");
    }
    None
}

fn reason_rank(reason: &str) -> usize {
    REASONS
        .iter()
        .position(|r| *r == reason)
        .unwrap_or(REASONS.len())
}

/// Whether `value` is a calendar date `YYYY-MM-DD` that exists.
pub fn is_valid_date(value: &str) -> bool {
    temporal::calendar_date(value).is_some()
}

/// The date `days` before `date`, as `YYYY-MM-DD`.
fn shift_days(date: &str, days: u32) -> Option<String> {
    let (y, m, d) = temporal::ymd(date)?;
    let (y, m, d) = civil_from_days(days_from_civil(y, m, d) - i64::from(days));
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// The day of a stored timestamp or date: its first ten characters, when they are a
/// `YYYY-MM-DD` prefix.
fn day(value: Option<&str>) -> Option<String> {
    let text = value?;
    let head = text.get(..10)?;
    let shape = head.bytes().enumerate().all(|(i, c)| match i {
        4 | 7 => c == b'-',
        _ => c.is_ascii_digit(),
    });
    shape.then(|| head.to_string())
}

/// The status a task has, with the type's default when none is written.
fn status_of(b: &Binding, t: &TaskView) -> String {
    b.effective(&t.frontmatter)
        .get(b.field("status"))
        .and_then(Value::as_str)
        .map_or_else(|| b.default_status.clone(), str::to_string)
}

/// The latest day a task shows activity: a time entry's start, its completion date
/// or its creation date. Never `dateModified`.
fn last_activity(b: &Binding, t: &TaskView) -> Option<String> {
    let entries = list_of(t, b.field("timeEntries"))
        .into_iter()
        .filter_map(|entry| day(entry.get("startTime").and_then(Value::as_str)));
    let stamps = [b.field("completedDate"), b.field("dateCreated")]
        .into_iter()
        .filter_map(|key| day(t.frontmatter.get(key).and_then(Value::as_str)));
    entries.chain(stamps).max()
}

/// The owner of a project as a single wikilink, or `None` for anything else: plain
/// text and lists are legacy and count as no owner.
fn owner_link(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    let inner = text.strip_prefix("[[")?.strip_suffix("]]")?;
    (!inner.is_empty() && !inner.contains(']')).then(|| text.to_string())
}

fn string_of(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

/// Refuses a status the task type does not allow.
fn check_status(b: &Binding, status: &str) -> Result<(), TaskError> {
    if b.statuses.iter().any(|s| s == status) {
        return Ok(());
    }
    Err(TaskError::new(
        "invalid_project_status",
        format!("{status} is not a status of the task type"),
    ))
}
