//! `mdbase tasks`: the task commands over the collection's `task` type.
//!
//! The rules live in `mdbn-tasks`, which plans each write as a JSON `WriteOp`. This
//! module reads the task, runs the planner, and sends the plan through the data
//! commands' write path ([`crate::data::apply`]), with the revision the task was read
//! at (or `--if-revision`). The collection's `_config/mdbase-tasks.json` is read from
//! the collection folder on disk.
//!
//! Output: under `--json`, `{"ok":true,"result":...}` on success and
//! `{"ok":false,"error":CODE,"message":...}` on failure, exit 1.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use mdbn_tasks::{Binding, NewTask, OpenEntry, TaskError, TaskView, TasksConfig};
use mdbn_wire::Wire;
use mdbn_wire::cbor::Cbor;
use mdbn_wire::client::{PublishState, QueryResult, Receipt, RecordView};
use serde_json::{Map, Value, json};
use time::{OffsetDateTime, UtcOffset};

use crate::client::ClientError;
use crate::control::ControlError;
use crate::data::{self, Method, Target, Transport, WriteOp};
use crate::paths::Profile;
use crate::secrets::{hex, uuid_string};

/// The collection file that holds the tasks configuration, from the collection root.
const CONFIG_FILE: &str = "_config/mdbase-tasks.json";
/// The type the task commands work on.
const TASK_TYPE: &str = "task";
/// Records per `query` page; `list` and the other scans page until the end.
const PAGE: u64 = 50;

/// The task commands.
#[derive(Debug, Subcommand)]
pub(crate) enum TasksCmd {
    /// Open tasks, or the tasks with `--status`. One path per line (`--json`: the tasks).
    List {
        #[command(flatten)]
        target: Target,
        /// Only the tasks of this person (name or alias).
        #[arg(long, value_name = "P")]
        responsavel: Option<String>,
        /// Only the tasks with this status, open or closed.
        #[arg(long, value_name = "S")]
        status: Option<String>,
    },
    /// A task's frontmatter and revision.
    Show {
        #[command(flatten)]
        target: Target,
        /// Task path, relative to the collection root.
        path: String,
    },
    /// Create a task under the collection's rules.
    Create {
        #[command(flatten)]
        target: Target,
        /// The title of the task.
        title: String,
        /// Person, alias or group (repeatable).
        #[arg(long = "responsavel", value_name = "P")]
        responsavel: Vec<String>,
        /// The brand the task belongs to.
        #[arg(long, value_name = "M")]
        marca: Option<String>,
        /// A project, by name or note (repeatable).
        #[arg(long = "projeto", value_name = "X")]
        projeto: Vec<String>,
    },
    /// Turn an existing note into a task.
    Promote {
        #[command(flatten)]
        target: Target,
        /// Note path.
        path: String,
        /// Person, alias or group (repeatable).
        #[arg(long = "responsavel", value_name = "P")]
        responsavel: Vec<String>,
        /// The brand the task belongs to.
        #[arg(long, value_name = "M")]
        marca: Option<String>,
        /// A project, by name or note (repeatable).
        #[arg(long = "projeto", value_name = "X")]
        projeto: Vec<String>,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// Change a task's status.
    SetStatus {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        /// The new status.
        #[arg(long, value_name = "S")]
        status: String,
        /// The day it was completed (closed statuses only; default: today).
        #[arg(long, value_name = "D")]
        completed_date: Option<String>,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// Set the projects of a task.
    LinkProject {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        /// A project, by name or note (repeatable).
        #[arg(long = "projeto", value_name = "X", required = true)]
        projeto: Vec<String>,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// Add blockers to a task.
    Block {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        /// A task that must finish first (repeatable).
        #[arg(long = "by", value_name = "Y", required = true)]
        by: Vec<String>,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// Remove blockers from a task.
    Unblock {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        /// A blocker to remove (repeatable).
        #[arg(long = "by", value_name = "Y", required = true)]
        by: Vec<String>,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// Archive a task.
    Archive {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// Skip a recurrence on a date.
    Skip {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        /// The date to skip (YYYY-MM-DD).
        #[arg(long, value_name = "D")]
        date: String,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// Undo a skip on a date.
    Unskip {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        /// The skipped date (YYYY-MM-DD).
        #[arg(long, value_name = "D")]
        date: String,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// Start a person's timer on a task (sets the status to em-andamento).
    Start {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        /// The person running the timer (one person, not a group).
        #[arg(long, value_name = "P")]
        responsavel: String,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// Stop a person's timer on a task, with the status it ends in.
    Stop {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        /// The person whose timer stops (one person, not a group).
        #[arg(long, value_name = "P")]
        responsavel: String,
        /// The status the task ends in.
        #[arg(long, value_name = "S")]
        status: String,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// The time entries of a task.
    #[command(subcommand)]
    Timer(TimerCmd),
}

/// `timer` subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum TimerCmd {
    /// Correct a closed time entry, found by its stored start time.
    Fix {
        #[command(flatten)]
        target: Target,
        /// Task path.
        path: String,
        /// The stored start time of the entry.
        #[arg(long, value_name = "INICIO")]
        entry: String,
        /// The new start time (ISO 8601).
        #[arg(long, value_name = "A")]
        start: Option<String>,
        /// The new end time (ISO 8601).
        #[arg(long, value_name = "B")]
        end: Option<String>,
        /// Why the entry is corrected (kept by the caller's audit log, not the entry).
        #[arg(long, value_name = "R")]
        reason: String,
        #[command(flatten)]
        write: TaskWrite,
    },
    /// The entries still running, on every task, or only those of one person.
    Status {
        #[command(flatten)]
        target: Target,
        /// Only the entries of this person.
        #[arg(long, value_name = "P")]
        responsavel: Option<String>,
    },
}

/// Options every task write takes.
#[derive(Debug, Args)]
pub(crate) struct TaskWrite {
    /// Write only if the task is still at this revision (from `show`), in place of
    /// the revision the task was read at.
    #[arg(long, value_name = "REVISION")]
    if_revision: Option<String>,
}

/// Runs a task command and prints its result.
pub(crate) async fn run(cmd: TasksCmd, profile: &Profile, json: bool) -> ExitCode {
    match execute(cmd, profile).await {
        Ok(result) => {
            if json {
                println!("{}", pretty(&json!({"ok": true, "result": result})));
            } else if let Some(items) = result.as_array() {
                for item in items {
                    let path = item.get("path").and_then(Value::as_str);
                    println!("{}", path.map_or_else(|| item.to_string(), str::to_string));
                }
            } else {
                println!("{}", pretty(&result));
            }
            ExitCode::SUCCESS
        }
        Err(f) => {
            if json {
                println!(
                    "{}",
                    pretty(&json!({"ok": false, "error": f.code, "message": f.message}))
                );
            } else {
                eprintln!("error: {}: {}", f.code, f.message);
            }
            ExitCode::from(1)
        }
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// A failed command: the stable code and the detail for people.
#[derive(Debug)]
struct Failure {
    code: String,
    message: String,
}

impl Failure {
    fn new(code: &str, message: impl Into<String>) -> Failure {
        Failure {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

impl From<TaskError> for Failure {
    fn from(e: TaskError) -> Failure {
        Failure::new(e.code, e.message)
    }
}

impl From<ControlError> for Failure {
    fn from(e: ControlError) -> Failure {
        Failure::new(&e.code, e.message)
    }
}

impl From<ClientError> for Failure {
    fn from(e: ClientError) -> Failure {
        match e {
            ClientError::Remote(err) => err.into(),
            ClientError::NotRunning => Failure::new(
                "daemon_not_running",
                "the mdbase daemon is not running; start it with `mdbase daemon start`",
            ),
            other => Failure::new("daemon_unreachable", other.to_string()),
        }
    }
}

async fn execute(cmd: TasksCmd, profile: &Profile) -> Result<Value, Failure> {
    match cmd {
        TasksCmd::List {
            target,
            responsavel,
            status,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let people = match &responsavel {
                Some(who) => Some(s.config_links(who)?),
                None => None,
            };
            let mut out = Vec::new();
            for record in all_records(&mut s.t, &[TASK_TYPE]).await? {
                let view = view_of(&record);
                let effective = s.binding.effective(&view.frontmatter);
                let keep = match &status {
                    Some(wanted) => text_of(&effective, s.binding.field("status")) == Some(wanted),
                    None => mdbn_tasks::is_open(
                        &s.binding,
                        &TaskView {
                            frontmatter: effective.clone(),
                            ..view.clone()
                        },
                    ),
                };
                let mine = match &people {
                    Some(links) => assignees_of(&s.binding, &effective)
                        .iter()
                        .any(|a| links.contains(a)),
                    None => true,
                };
                if keep && mine {
                    out.push(json!({
                        "path": view.path,
                        "status": text_of(&effective, s.binding.field("status")),
                        "title": text_of(&effective, s.binding.field("title")),
                        "assignees": assignees_of(&s.binding, &effective),
                    }));
                }
            }
            Ok(Value::Array(out))
        }
        TasksCmd::Show { target, path } => {
            let mut s = Session::open(&target, profile).await?;
            let view = s.view(&path).await?;
            Ok(json!({
                "path": view.path,
                "revision": view.revision,
                "frontmatter": Value::Object(view.frontmatter),
            }))
        }
        TasksCmd::Create {
            target,
            title,
            responsavel,
            marca,
            projeto,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let new = NewTask {
                title,
                assignees: responsavel,
                brand: marca,
                projects: projeto,
            };
            let op = mdbn_tasks::create(&s.binding, &s.config(), &new)?;
            s.check_op_people(&op).await?;
            let path = op_path(&op);
            s.write(&path, vec![op], None).await
        }
        TasksCmd::Promote {
            target,
            path,
            responsavel,
            marca,
            projeto,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let note = s.view(&path).await?;
            let new = NewTask {
                title: String::new(),
                assignees: responsavel,
                brand: marca,
                projects: projeto,
            };
            let op = mdbn_tasks::promote(&s.binding, &s.config(), &note, &new)?;
            s.check_op_people(&op).await?;
            s.write(&path, vec![op], write.if_revision.as_deref()).await
        }
        TasksCmd::SetStatus {
            target,
            path,
            status,
            completed_date,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let view = s.view(&path).await?;
            let op = mdbn_tasks::set_status(
                &s.binding,
                &view,
                &status,
                completed_date.as_deref(),
                &s.today,
            )?;
            s.write(&path, vec![op], write.if_revision.as_deref()).await
        }
        TasksCmd::LinkProject {
            target,
            path,
            projeto,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let view = s.view(&path).await?;
            let op = mdbn_tasks::link_projects(&s.binding, &view, &projeto)?;
            s.write(&path, vec![op], write.if_revision.as_deref()).await
        }
        TasksCmd::Block {
            target,
            path,
            by,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let view = s.view(&path).await?;
            let existing: Vec<String> = all_records(&mut s.t, &[TASK_TYPE])
                .await?
                .iter()
                .map(|r| r.path.clone())
                .collect();
            let op = mdbn_tasks::block(&s.binding, &view, &by, &existing)?;
            s.write(&path, vec![op], write.if_revision.as_deref()).await
        }
        TasksCmd::Unblock {
            target,
            path,
            by,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let view = s.view(&path).await?;
            let op = mdbn_tasks::unblock(&s.binding, &view, &by)?;
            s.write(&path, vec![op], write.if_revision.as_deref()).await
        }
        TasksCmd::Archive {
            target,
            path,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let view = s.view(&path).await?;
            let op = mdbn_tasks::archive(&s.binding, &view)?;
            s.write(&path, vec![op], write.if_revision.as_deref()).await
        }
        TasksCmd::Skip {
            target,
            path,
            date,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let view = s.view(&path).await?;
            let op = mdbn_tasks::skip(&s.binding, &view, &date)?;
            s.write(&path, vec![op], write.if_revision.as_deref()).await
        }
        TasksCmd::Unskip {
            target,
            path,
            date,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let view = s.view(&path).await?;
            let op = mdbn_tasks::unskip(&s.binding, &view, &date)?;
            s.write(&path, vec![op], write.if_revision.as_deref()).await
        }
        TasksCmd::Start {
            target,
            path,
            responsavel,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let person = s.timer_person(&responsavel).await?;
            let view = s.view(&path).await?;
            let ops = mdbn_tasks::start(&s.binding, &view, &person, &s.now)?;
            s.write(&path, ops, write.if_revision.as_deref()).await
        }
        TasksCmd::Stop {
            target,
            path,
            responsavel,
            status,
            write,
        } => {
            let mut s = Session::open(&target, profile).await?;
            let person = s.timer_person(&responsavel).await?;
            let view = s.view(&path).await?;
            let ops = mdbn_tasks::stop(&s.binding, &view, &person, &status, &s.now, &s.today)?;
            s.write(&path, ops, write.if_revision.as_deref()).await
        }
        TasksCmd::Timer(TimerCmd::Fix {
            target,
            path,
            entry,
            start,
            end,
            reason,
            write,
        }) => {
            let mut s = Session::open(&target, profile).await?;
            let view = s.view(&path).await?;
            let op = mdbn_tasks::retime(
                &s.binding,
                &view,
                &entry,
                start.as_deref(),
                end.as_deref(),
                &reason,
                &s.now,
            )?;
            s.write(&path, vec![op], write.if_revision.as_deref()).await
        }
        TasksCmd::Timer(TimerCmd::Status {
            target,
            responsavel,
        }) => {
            let mut s = Session::open(&target, profile).await?;
            let people = match &responsavel {
                Some(who) => Some(s.config_links(who)?),
                None => None,
            };
            let mut out = Vec::new();
            for record in all_records(&mut s.t, &[TASK_TYPE]).await? {
                let view = view_of(&record);
                for OpenEntry { person, start } in mdbn_tasks::open_entries(&s.binding, &view) {
                    let mine = match &people {
                        Some(links) => links.contains(&strip_link(&person)),
                        None => true,
                    };
                    if mine {
                        out.push(json!({"path": view.path, "person": person, "start": start}));
                    }
                }
            }
            Ok(Value::Array(out))
        }
    }
}

/// The task type, the tasks configuration and the clock of one command, on the
/// route the target resolves to.
struct Session {
    t: Transport,
    binding: Binding,
    /// The collection's tasks configuration; `None` when the collection has no file.
    config: Option<TasksConfig>,
    /// Local time with its offset, to the millisecond.
    now: String,
    /// Local date, `YYYY-MM-DD`.
    today: String,
}

impl Session {
    async fn open(target: &Target, profile: &Profile) -> Result<Session, Failure> {
        let root = folder_of(target, profile).await?;
        let mut t = data::open(target, profile).await?;
        let definition = task_definition(&mut t).await?;
        let binding = Binding::from_type(&definition)?;
        let config = read_config(&root)?;
        let (now, today) = clock()?;
        Ok(Session {
            t,
            binding,
            config,
            now,
            today,
        })
    }

    /// The configuration the planners use: the file's, or the defaults.
    fn config(&self) -> TasksConfig {
        self.config.clone().unwrap_or_default()
    }

    /// The wikilinks a name or alias stands for, without checking that the people
    /// exist. For filters, which only compare.
    fn config_links(&self, who: &str) -> Result<Vec<String>, Failure> {
        Ok(mdbn_tasks::resolve_person(&self.config(), who)?)
    }

    /// The one person a timer runs for. A name that stands for several people is
    /// refused: a timer has one owner.
    async fn timer_person(&mut self, who: &str) -> Result<String, Failure> {
        let links = self.config_links(who)?;
        match links.len() {
            1 => {
                self.check_people(&links).await?;
                Ok(strip_link(&links[0]))
            }
            0 => Err(Failure::new(
                "responsavel_desconhecido",
                format!("{who} stands for no person"),
            )),
            _ => Err(Failure::new(
                "responsavel_grupo",
                format!("{who} stands for {} people; a timer needs one", links.len()),
            )),
        }
    }

    /// Every person link a planned write names (`assignees`) must have a person note.
    async fn check_op_people(&mut self, op: &Value) -> Result<(), Failure> {
        let key = self.binding.field("assignees");
        let links: Vec<String> = op
            .get("fields")
            .and_then(|f| f.get(key))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        self.check_people(&links).await
    }

    /// Checks that each link has a note named `<name>.md` in the collection. Only
    /// done when the collection has a tasks configuration.
    async fn check_people(&mut self, links: &[String]) -> Result<(), Failure> {
        if self.config.is_none() || links.is_empty() {
            return Ok(());
        }
        let names: HashSet<String> = all_records(&mut self.t, &[])
            .await?
            .iter()
            .filter_map(|r| file_name(&r.path).map(str::to_string))
            .collect();
        for link in links {
            let name = strip_link(link);
            if !names.contains(&format!("{name}.md")) {
                return Err(Failure::new(
                    "responsavel_desconhecido",
                    format!("no person note named {name}.md in the collection"),
                ));
            }
        }
        Ok(())
    }

    /// The task at `path`, with the revision it is read at.
    async fn view(&mut self, path: &str) -> Result<TaskView, Failure> {
        let record: RecordView = data::decode(
            &self
                .t
                .call(
                    Method::Get,
                    data::params(vec![
                        Cbor::Text(path.to_string()),
                        data::include(false, false, false).to_cbor(),
                    ]),
                )
                .await?,
        )?;
        Ok(view_of(&record))
    }

    /// Sends the planned writes as one mutation. An `if_revision` given on the
    /// command replaces the revision of each update. A held or merged write is an
    /// error, never a success.
    async fn write(
        &mut self,
        path: &str,
        ops: Vec<Value>,
        if_revision: Option<&str>,
    ) -> Result<Value, Failure> {
        if ops.is_empty() {
            return Ok(json!({"path": path, "changed": false}));
        }
        let mut batch = Vec::with_capacity(ops.len());
        for mut op in ops {
            let is_update = op.get("op").and_then(Value::as_str) == Some("update");
            if let (true, Some(revision)) = (is_update, if_revision) {
                op["if_revision"] = json!(revision);
            }
            let write: WriteOp = serde_json::from_value(op)
                .map_err(|e| Failure::new("internal", format!("the planned write: {e}")))?;
            batch.push(write);
        }
        let applied = data::apply(&mut self.t, batch, false)
            .await
            .map_err(revision_failure)?;
        held(&applied.receipt)?;
        let receipt = applied.receipt;
        let record = receipt.records.as_ref().and_then(|r| r.first());
        Ok(json!({
            "path": record.map_or(path, |r| r.path.as_str()),
            "changed": true,
            "revision": record.map(|r| hex(&r.revision.0)),
            "mutation": uuid_string(&receipt.mutation.0),
        }))
    }
}

/// A write the store rejected with `conflict` is a revision that moved: the write
/// is planned with `ConflictMode::Reject`, so nothing else answers with it.
fn revision_failure(e: ClientError) -> Failure {
    match e {
        ClientError::Remote(err) if err.code == "conflict" => {
            Failure::new("revision_conflict", err.message)
        }
        other => other.into(),
    }
}

/// A write the store did not take as planned: held, merged, or not in the files.
fn held(receipt: &Receipt) -> Result<(), Failure> {
    if let Some(conflicts) = receipt.conflicts.as_ref().filter(|c| !c.is_empty()) {
        return Err(Failure::new(
            "write_held",
            format!(
                "the write was merged with a concurrent change: {} conflict(s) to resolve",
                conflicts.len()
            ),
        ));
    }
    if receipt.published == Some(PublishState::NotPublished) {
        return Err(Failure::new(
            "write_held",
            "the change is recorded but its file was not written: the file changed or is held",
        ));
    }
    Ok(())
}

/// The folder the command works on: the registered collection's, or `-C`'s, or the
/// current directory.
async fn folder_of(target: &Target, profile: &Profile) -> Result<PathBuf, Failure> {
    if let Some(arg) = &target.collection {
        let list = crate::cli::collections(profile).await?;
        return Ok(crate::cli::pick(&list, arg)?.root.clone());
    }
    match &target.root {
        Some(root) => Ok(root.clone()),
        None => std::env::current_dir()
            .map_err(|e| Failure::new("internal", format!("current directory: {e}"))),
    }
}

/// The `task` type's definition, as `Binding::from_type` reads it.
async fn task_definition(t: &mut Transport) -> Result<Value, Failure> {
    let result = t
        .call(Method::Describe, data::params(vec![Cbor::Bool(true)]))
        .await?;
    data::types(&result)?
        .into_iter()
        .find(|ty| ty.name == TASK_TYPE)
        .and_then(|ty| ty.definition)
        .ok_or_else(|| {
            Failure::new(
                "type_missing",
                "the collection has no task type (_types/task.md)",
            )
        })
}

/// The collection's tasks configuration, or `None` when the file is absent.
fn read_config(root: &Path) -> Result<Option<TasksConfig>, Failure> {
    let file = root.join(CONFIG_FILE);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Failure::new(
                "config_unreadable",
                format!("{}: {e}", file.display()),
            ));
        }
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::new("config_invalid", format!("{}: {e}", file.display())))?;
    Ok(Some(TasksConfig::from_json(&value)?))
}

/// Local time to the millisecond with its offset, and the local date.
fn clock() -> Result<(String, String), Failure> {
    let offset = UtcOffset::current_local_offset()
        .map_err(|_| Failure::new("clock_unavailable", "the local time zone cannot be read"))?;
    let local = OffsetDateTime::now_utc().to_offset(offset);
    let sign = if offset.is_negative() { '-' } else { '+' };
    let zone = format!(
        "{sign}{:02}:{:02}",
        offset.whole_hours().unsigned_abs(),
        offset.minutes_past_hour().unsigned_abs()
    );
    let today = format!(
        "{:04}-{:02}-{:02}",
        local.year(),
        u8::from(local.month()),
        local.day()
    );
    let now = format!(
        "{today}T{:02}:{:02}:{:02}.{:03}{zone}",
        local.hour(),
        local.minute(),
        local.second(),
        local.millisecond()
    );
    Ok((now, today))
}

/// Every record of the given types (every record when none), a page at a time,
/// until a page says there are no more.
async fn all_records(t: &mut Transport, types: &[&str]) -> Result<Vec<RecordView>, Failure> {
    let mut out = Vec::new();
    let mut offset = 0u64;
    loop {
        let mut query = Map::new();
        query.insert("limit".into(), json!(PAGE));
        query.insert("offset".into(), json!(offset));
        if !types.is_empty() {
            query.insert("types".into(), json!(types));
        }
        let result: QueryResult = data::decode(
            &t.call(
                Method::Query,
                data::params(vec![
                    data::to_wire(&Value::Object(query)).to_cbor(),
                    data::include(false, false, false).to_cbor(),
                ]),
            )
            .await?,
        )?;
        let got = result.records.len() as u64;
        out.extend(result.records);
        if result.has_more != Some(true) || got == 0 {
            return Ok(out);
        }
        offset += got;
    }
}

fn view_of(record: &RecordView) -> TaskView {
    TaskView {
        path: record.path.clone(),
        revision: hex(&record.revision.0),
        frontmatter: data::map_json(&record.frontmatter)
            .as_object()
            .cloned()
            .unwrap_or_default(),
    }
}

fn text_of<'a>(fields: &'a Map<String, Value>, key: &str) -> Option<&'a String> {
    match fields.get(key) {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}

/// The assignees of a task as wikilinks, as the type stores them.
fn assignees_of(b: &Binding, fields: &Map<String, Value>) -> Vec<String> {
    fields
        .get(b.field("assignees"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// The name in `[[name]]`, or the value itself when it is not a link.
fn strip_link(value: &str) -> String {
    let inner = value.trim();
    let inner = inner.strip_prefix("[[").unwrap_or(inner);
    let inner = inner.strip_suffix("]]").unwrap_or(inner);
    inner.trim().to_string()
}

/// The last segment of a record path.
fn file_name(path: &str) -> Option<&str> {
    path.rsplit('/').next()
}

fn op_path(op: &Value) -> String {
    op.get("path")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
