//! `mdbase tasks` against a real collection, in process (`-C`): the task type is the
//! fixture of `mdbn-tasks`, copied into the collection's `_types`.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// The `task` type of the tasks crate's tests, as the collection's type file.
const TASK_TYPE: &str = include_str!("../../tasks/tests/fixtures/task-type.json");
/// The `tasknotes.task` contract the type implements; a type whose contract is
/// missing does not load.
const TASK_CONTRACT: &str = include_str!("../../tasks/tests/fixtures/tasknotes-task-contract.md");

struct Fixture {
    root: PathBuf,
    state: PathBuf,
}

/// A collection with the task type, one person note (`Arthur`) and the tasks
/// configuration `config` (JSON), or none when `config` is `None`.
fn fixture(tag: &str, config: Option<&str>) -> Fixture {
    let base =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{tag}{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("col");
    std::fs::create_dir_all(root.join("_types")).unwrap();
    std::fs::create_dir_all(root.join("06-tarefas/Tasks")).unwrap();
    std::fs::create_dir_all(root.join("pessoas")).unwrap();
    std::fs::write(
        root.join("mdbase.yaml"),
        "spec_version: \"0.3.0\"\nname: Test\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("_contracts")).unwrap();
    std::fs::write(root.join("_contracts/tasknotes.task.md"), TASK_CONTRACT).unwrap();
    std::fs::write(
        root.join("_types/task.md"),
        format!("---\n{TASK_TYPE}\n---\n"),
    )
    .unwrap();
    // The person and brand types: a person is a record of the type that marks
    // people (here `pessoa`, the fallback), a brand is a record of another type.
    std::fs::write(
        root.join("_types/pessoa.md"),
        "---\nkind: mdbase.type\nname: pessoa\nmatch:\n  path_glob: \"pessoas/**/*.md\"\nschema:\n  dialect: json-schema-2020-12\n  value:\n    type: object\n---\n",
    )
    .unwrap();
    std::fs::write(
        root.join("_types/marca.md"),
        "---\nkind: mdbase.type\nname: marca\nmatch:\n  path_glob: \"02-marcas/**/*.md\"\nschema:\n  dialect: json-schema-2020-12\n  value:\n    type: object\n---\n",
    )
    .unwrap();
    for person in ["Arthur", "Mayara"] {
        std::fs::write(
            root.join(format!("pessoas/{person}.md")),
            format!("---\ntype: pessoa\ntitle: {person}\n---\n"),
        )
        .unwrap();
    }
    std::fs::create_dir_all(root.join("02-marcas")).unwrap();
    std::fs::write(
        root.join("02-marcas/Madame_Labs.md"),
        "---\ntype: marca\ntitle: Madame Labs\n---\n",
    )
    .unwrap();
    if let Some(text) = config {
        std::fs::create_dir_all(root.join("_config")).unwrap();
        std::fs::write(root.join("_config/mdbase-tasks.json"), text).unwrap();
    }
    Fixture {
        root,
        // Short: the daemon's Unix socket lives under it.
        state: base.join("s"),
    }
}

/// Writes a task note directly, as the adapter would have it.
fn task_file(root: &Path, rel: &str, title: &str, status: &str) {
    let text = format!(
        "---\ntags: [task]\ntitle: \"{title}\"\nstatus: {status}\ndateCreated: \"2026-11-01T09:00:00.000-03:00\"\n---\nCorpo.\n"
    );
    std::fs::write(root.join(rel), text).unwrap();
}

impl Fixture {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mdbase"))
            .arg("--state-dir")
            .arg(&self.state)
            .args(args)
            .arg("-C")
            .arg(&self.root)
            .output()
            .unwrap()
    }

    /// Runs with `--json` and returns the exit code and the parsed envelope.
    fn json(&self, args: &[&str]) -> (i32, Value) {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self.run(&all);
        let v = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)));
        (out.status.code().unwrap(), v)
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }

    /// The task's revision, from `show`.
    fn revision(&self, path: &str) -> String {
        let (code, v) = self.json(&["tasks", "show", path]);
        assert_eq!(code, 0, "{v}");
        v["result"]["revision"].as_str().unwrap().to_string()
    }
}

#[test]
fn list_pagina_ate_o_fim() {
    let f = fixture("tls", None);
    for i in 0..120 {
        task_file(
            &f.root,
            &format!("06-tarefas/Tasks/t{i:03}.md"),
            &format!("Tarefa {i:03}"),
            "to-do",
        );
    }
    let (code, v) = f.json(&["tasks", "list"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["result"].as_array().unwrap().len(), 120, "{v}");
}

#[test]
fn start_e_stop_numa_gravacao_cada() {
    let f = fixture("tss", Some("{}"));
    let (code, v) = f.json(&["tasks", "create", "Escrever doc", "--responsavel", "Arthur"]);
    assert_eq!(code, 0, "{v}");
    let path = v["result"]["path"].as_str().unwrap().to_string();

    let (code, v) = f.json(&["tasks", "start", &path, "--responsavel", "Arthur"]);
    assert_eq!(code, 0, "{v}");
    let (_, v) = f.json(&["tasks", "show", &path]);
    let fm = &v["result"]["frontmatter"];
    assert_eq!(fm["status"], "em-andamento", "{v}");
    let entries = fm["timeEntries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{v}");
    assert!(entries[0].get("endTime").is_none(), "{v}");
    assert_eq!(entries[0]["description"], "pessoa: Arthur");

    let (code, v) = f.json(&[
        "tasks",
        "stop",
        &path,
        "--responsavel",
        "Arthur",
        "--status",
        "aprovacao",
    ]);
    assert_eq!(code, 0, "{v}");
    let (_, v) = f.json(&["tasks", "show", &path]);
    let fm = &v["result"]["frontmatter"];
    assert_eq!(fm["status"], "aprovacao", "{v}");
    let entries = fm["timeEntries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{v}");
    assert!(entries[0]["endTime"].is_string(), "{v}");
}

#[test]
fn revisao_mudou_no_meio_da_erro() {
    let f = fixture("trv", Some("{}"));
    task_file(&f.root, "06-tarefas/Tasks/rev.md", "Revisar", "backlog");
    let path = "06-tarefas/Tasks/rev.md";
    let stale = f.revision(path);

    // The note changes after the read the write was planned from.
    let out = f.run(&["update", path, "--fields", "{\"status\":\"to-do\"}"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let before = f.read(path);

    let (code, v) = f.json(&[
        "tasks",
        "set-status",
        path,
        "--status",
        "concluido",
        "--if-revision",
        &stale,
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(v["error"], "revision_conflict", "{v}");
    assert!(v["message"].is_string(), "{v}");
    assert_eq!(f.read(path), before, "the note must stay as it was");
}

#[test]
fn create_usa_config_da_colecao() {
    let f = fixture("tcf", Some("{\"required_on_create\":[\"projects\"]}"));
    let (code, v) = f.json(&["tasks", "create", "Sem projeto"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "missing_required_field", "{v}");

    let (code, v) = f.json(&["tasks", "create", "Com projeto", "--projeto", "Projeto X"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn responsavel_sem_nota_recusa() {
    let f = fixture("tsn", Some("{}"));
    let (code, v) = f.json(&["tasks", "create", "Sem dona", "--responsavel", "Fulano"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "responsavel_desconhecido", "{v}");
    assert!(v["message"].as_str().unwrap().contains("Fulano.md"), "{v}");
}

#[test]
fn marca_nao_passa_como_pessoa() {
    // `Madame_Labs.md` exists, but it is a brand (another type), not a person.
    let f = fixture("tmp", Some("{}"));
    let (code, v) = f.json(&[
        "tasks",
        "create",
        "Sem dona",
        "--responsavel",
        "Madame_Labs",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "responsavel_desconhecido", "{v}");
}

#[test]
fn timer_status_lista_so_os_da_pessoa() {
    let f = fixture("tts", Some("{}"));
    for (title, who) in [("Tarefa A", "Arthur"), ("Tarefa B", "Mayara")] {
        let (code, v) = f.json(&["tasks", "create", title, "--responsavel", who]);
        assert_eq!(code, 0, "{v}");
        let path = v["result"]["path"].as_str().unwrap().to_string();
        let (code, v) = f.json(&["tasks", "start", &path, "--responsavel", who]);
        assert_eq!(code, 0, "{v}");
    }

    let (code, v) = f.json(&["tasks", "timer", "status"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"].as_array().unwrap().len(), 2, "{v}");

    let (code, v) = f.json(&["tasks", "timer", "status", "--responsavel", "Mayara"]);
    assert_eq!(code, 0, "{v}");
    let entries = v["result"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{v}");
    assert_eq!(entries[0]["person"], "Mayara", "{v}");
}

#[test]
fn responsavel_grupo_recusa_o_cronometro() {
    let f = fixture(
        "tgr",
        Some("{\"people\":{\"labs\":[\"[[Arthur]]\",\"[[Fulano]]\"]}}"),
    );
    let (code, v) = f.json(&[
        "tasks",
        "create",
        "Tarefa de grupo",
        "--responsavel",
        "labs",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "responsavel_desconhecido", "{v}");

    task_file(&f.root, "06-tarefas/Tasks/grupo.md", "Grupo", "to-do");
    let (code, v) = f.json(&[
        "tasks",
        "start",
        "06-tarefas/Tasks/grupo.md",
        "--responsavel",
        "labs",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "responsavel_grupo", "{v}");
}

#[test]
fn saida_json_estavel() {
    let f = fixture("tjs", Some("{}"));
    task_file(&f.root, "06-tarefas/Tasks/ok.md", "Ok", "to-do");
    let (code, v) = f.json(&["tasks", "show", "06-tarefas/Tasks/ok.md"]);
    assert_eq!(code, 0);
    assert_eq!(v["ok"], true);
    assert!(v.get("result").is_some());
    assert!(v.get("error").is_none());

    let (code, v) = f.json(&["tasks", "show", "06-tarefas/Tasks/nao-existe.md"]);
    assert_eq!(code, 1);
    assert_eq!(v["ok"], false);
    assert!(v["error"].is_string(), "{v}");
    assert!(v["message"].is_string(), "{v}");
    assert_eq!(v.as_object().unwrap().len(), 3, "{v}");
}
