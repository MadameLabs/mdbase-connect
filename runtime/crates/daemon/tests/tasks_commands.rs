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
/// The `projeto` type of the project notes, as the BIBLIOTECA declares it.
const PROJECT_TYPE: &str = include_str!("../../tasks/tests/fixtures/projeto-type.md");

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
    std::fs::write(root.join("_types/projeto.md"), PROJECT_TYPE).unwrap();
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

/// Writes a project note with the given frontmatter lines (YAML, without the fences).
fn project_file(root: &Path, rel: &str, frontmatter: &str) {
    std::fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
    std::fs::write(root.join(rel), format!("---\n{frontmatter}\n---\nCorpo.\n")).unwrap();
}

/// Writes a task note that links to projects and carries extra frontmatter lines.
fn task_linked(root: &Path, rel: &str, projects: &str, extra: &str) {
    let text = format!(
        "---\ntags: [task]\ntitle: \"Tarefa\"\nstatus: to-do\nprojects: [\"{projects}\"]\n{extra}dateCreated: \"2026-11-01T09:00:00.000-03:00\"\n---\nCorpo.\n"
    );
    std::fs::write(root.join(rel), text).unwrap();
}

const Q: &str = "02-marcas/Madame_Labs/Projetos/q.md";
const R: &str = "02-marcas/Madame_Labs/Projetos/r.md";

/// The project `q`, active and owned by Arthur, and `r`, active and without an owner.
fn two_projects(f: &Fixture) {
    project_file(
        &f.root,
        Q,
        "type: projeto\nbrand: \"[[Madame_Labs]]\"\nstatus: em-andamento\nowner: \"[[Arthur]]\"",
    );
    project_file(
        &f.root,
        R,
        "type: projeto\nbrand: \"[[Madame_Labs]]\"\nstatus: em-andamento",
    );
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

// --- projects (cli.test.mjs, projectCli) ---

#[test]
fn projeto_status_grava_desfecho_e_recusa_fechar_sem_ele() {
    let f = fixture("pst", Some("{}"));
    two_projects(&f);
    let before = f.read(Q);

    let (code, v) = f.json(&["tasks", "projects", "status", "q", "--status", "concluido"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "desfecho_obrigatorio", "{v}");
    assert_eq!(
        f.read(Q),
        before,
        "a refused status must not touch the note"
    );

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "status",
        "q",
        "--status",
        "concluido",
        "--desfecho",
        "entregue",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["path"], Q, "{v}");
    assert_eq!(v["result"]["changed"], true, "{v}");
    let (_, shown) = f.json(&["tasks", "show", Q]);
    let fm = &shown["result"]["frontmatter"];
    assert_eq!(fm["status"], "concluido", "{shown}");
    assert_eq!(fm["outcome"], "entregue", "{shown}");
    assert!(fm["outcomeDate"].is_string(), "{shown}");

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "status",
        "q",
        "--status",
        "em-andamento",
    ]);
    assert_eq!(code, 0, "{v}");
    let (_, shown) = f.json(&["tasks", "show", Q]);
    let fm = &shown["result"]["frontmatter"];
    assert!(
        fm.get("outcome").is_none() || fm["outcome"].is_null(),
        "{shown}"
    );
}

#[test]
fn projeto_pausa_checa_a_retomada_contra_o_relogio() {
    let f = fixture("tpr", Some("{}"));
    two_projects(&f);

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "status",
        "q",
        "--status",
        "pausa",
        "--desfecho",
        "cliente sumiu",
        "--retomar-em",
        "2000-01-01",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "retomada_no_passado", "{v}");

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "status",
        "q",
        "--status",
        "pausa",
        "--desfecho",
        "cliente sumiu",
        "--retomar-em",
        "2999-01-01",
    ]);
    assert_eq!(code, 0, "{v}");
    let (_, shown) = f.json(&["tasks", "show", Q]);
    assert_eq!(
        shown["result"]["frontmatter"]["resumeOn"], "2999-01-01",
        "{shown}"
    );
}

#[test]
fn desfecho_sem_texto_nao_engole_a_opcao_seguinte() {
    let f = fixture("tdn", Some("{}"));
    two_projects(&f);
    let before = f.read(Q);
    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "status",
        "q",
        "--status",
        "pausa",
        "--desfecho",
        "--retomar-em",
        "2999-12-30",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "desfecho_obrigatorio", "{v}");
    assert_eq!(f.read(Q), before);
}

#[test]
fn projeto_assign_grava_owner_e_recusa_grupo() {
    let f = fixture(
        "tpa",
        Some("{\"people\":{\"labs\":[\"[[Arthur]]\",\"[[Mayara]]\"]}}"),
    );
    two_projects(&f);

    let (code, v) = f.json(&["tasks", "projects", "assign", "r", "--responsavel", "labs"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "responsavel_grupo", "{v}");

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "assign",
        "r",
        "--responsavel",
        "Mayara",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["path"], R, "{v}");
    let (_, shown) = f.json(&["tasks", "show", R]);
    assert_eq!(
        shown["result"]["frontmatter"]["owner"], "[[Mayara]]",
        "{shown}"
    );

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "assign",
        "r",
        "--responsavel",
        "Fulano",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "responsavel_desconhecido", "{v}");

    let (code, v) = f.json(&["tasks", "projects", "list", "--responsavel", "Mayara"]);
    assert_eq!(code, 0, "{v}");
    let names: Vec<&str> = v["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["r"], "{v}");
}

#[test]
fn projects_list_resume_tarefas_e_review_traz_quem_nao_tem_responsavel() {
    let f = fixture("tpl", Some("{}"));
    two_projects(&f);
    task_linked(
        &f.root,
        "06-tarefas/Tasks/t1.md",
        "[[q]]",
        "timeEntries:\n  - startTime: \"2026-11-09T12:00:00Z\"\n    endTime: \"2026-11-09T13:00:00Z\"\n    description: \"pessoa: Arthur\"\n",
    );

    let (code, v) = f.json(&["tasks", "projects", "list"]);
    assert_eq!(code, 0, "{v}");
    let items = v["result"]["items"].as_array().unwrap();
    let q_item = items.iter().find(|i| i["name"] == "q").expect("q listed");
    assert_eq!(q_item["openTasks"], 1, "{v}");
    assert_eq!(q_item["totalTasks"], 1, "{v}");
    assert_eq!(q_item["owner"], "[[Arthur]]", "{v}");
    assert_eq!(q_item["lastActivity"], "2026-11-09", "{v}");

    let (code, v) = f.json(&["tasks", "projects", "review", "--responsavel", "Arthur"]);
    assert_eq!(code, 0, "{v}");
    let queue = v["result"]["queue"].as_array().unwrap();
    let r_item = queue.iter().find(|i| i["name"] == "r").expect("r queued");
    assert_eq!(r_item["reason"], "sem_responsavel", "{v}");
    assert!(v["result"]["orphanLinks"].is_array(), "{v}");
}

#[test]
fn projects_status_de_projeto_invalido_e_recusado() {
    let f = fixture("tpi", Some("{}"));
    two_projects(&f);
    let (code, v) = f.json(&["tasks", "projects", "list", "--status", "ativo"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "invalid_project_status", "{v}");
}

#[test]
fn projeto_milestone_grava_e_projects_mostra() {
    let f = fixture("tms", Some("{}"));
    two_projects(&f);

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "milestone",
        "q",
        "--marco",
        "lançar",
        "--data",
        "2026-12-01",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["nextMilestone"], "lançar", "{v}");

    let (_, listed) = f.json(&["tasks", "projects", "list"]);
    let q_item = listed["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "q")
        .expect("q listed")
        .clone();
    assert_eq!(q_item["nextMilestoneDate"], "2026-12-01", "{listed}");

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "milestone",
        "q",
        "--marco",
        "x",
        "--data",
        "ontem",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "data_invalida", "{v}");

    let (code, v) = f.json(&["tasks", "projects", "milestone", "q", "--limpar"]);
    assert_eq!(code, 0, "{v}");
    let (_, shown) = f.json(&["tasks", "show", Q]);
    assert!(
        shown["result"]["frontmatter"]["nextMilestone"].is_null(),
        "{shown}"
    );
}

#[test]
fn projeto_rename_status_ensaia_antes_de_trocar() {
    let f = fixture("trs", Some("{}"));
    two_projects(&f);
    let velho = "02-marcas/Madame_Labs/Projetos/velho.md";
    project_file(
        &f.root,
        velho,
        "type: projeto\nbrand: \"[[Madame_Labs]]\"\nstatus: ativo\nowner: \"[[Arthur]]\"",
    );
    let before = f.read(velho);

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "rename-status",
        "--de",
        "ativo",
        "--para",
        "em-andamento",
        "--dry-run",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["result"]["dryRun"], true, "{v}");
    assert_eq!(v["result"]["paths"], serde_json::json!([velho]), "{v}");
    assert_eq!(f.read(velho), before, "a dry run must not write");

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "rename-status",
        "--de",
        "ativo",
        "--para",
        "em-andamento",
    ]);
    assert_eq!(code, 0, "{v}");
    let (_, shown) = f.json(&["tasks", "show", velho]);
    assert_eq!(
        shown["result"]["frontmatter"]["status"], "em-andamento",
        "{shown}"
    );

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "rename-status",
        "--de",
        "ativo",
        "--para",
        "feito",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "invalid_project_status", "{v}");
}

#[test]
fn projeto_com_mesmo_nome_em_duas_marcas_pede_o_caminho() {
    let f = fixture("tpd", Some("{}"));
    for brand in ["A", "B"] {
        project_file(
            &f.root,
            &format!("02-marcas/{brand}/Projetos/dup.md"),
            "type: projeto\nbrand: \"[[Madame_Labs]]\"\nstatus: em-andamento\nowner: \"[[Arthur]]\"",
        );
    }
    let (code, v) = f.json(&["tasks", "projects", "status", "dup", "--status", "backlog"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "project_ambiguous", "{v}");

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "status",
        "02-marcas/A/Projetos/dup.md",
        "--status",
        "backlog",
    ]);
    assert_eq!(code, 0, "{v}");

    let (code, v) = f.json(&[
        "tasks",
        "projects",
        "status",
        "nao-existe",
        "--status",
        "backlog",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "project_not_found", "{v}");
}

#[test]
fn molde_fora_dos_caminhos_nao_e_projeto() {
    let f = fixture(
        "tmo",
        Some("{\"project_paths\":[\"02-marcas/*/Projetos/*.md\"]}"),
    );
    two_projects(&f);
    // The template has the project type, but lives outside the project paths.
    project_file(
        &f.root,
        "08-templates/projeto.md",
        "type: projeto\nbrand: \"[[NOME_DA_MARCA]]\"\nstatus: backlog\nowner: \"[[NOME_DA_PESSOA]]\"",
    );

    let (code, v) = f.json(&["tasks", "projects", "review"]);
    assert_eq!(code, 0, "{v}");
    let names: Vec<&str> = v["result"]["queue"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"projeto"), "{v}");

    let (code, v) = f.json(&["tasks", "projects", "list"]);
    assert_eq!(code, 0, "{v}");
    let names: Vec<&str> = v["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"projeto"), "{v}");

    let (code, v) = f.json(&[
        "tasks", "projects", "status", "projeto", "--status", "backlog",
    ]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["error"], "project_not_found", "{v}");
}

// --- gate (hooks.test.mjs, cli.test.mjs hook-start) ---

/// A SessionStart payload as the host sends it.
const HOOK: &[u8] =
    b"{\"session_id\":\"abc\",\"cwd\":\"/tmp\",\"hook_event_name\":\"SessionStart\"}";

impl Fixture {
    /// Runs with `stdin` piped in and `env` set; the collection is `-C`.
    fn run_stdin(&self, args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Output {
        use std::io::Write;
        use std::process::Stdio;
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_mdbase"));
        cmd.arg("--state-dir")
            .arg(&self.state)
            .args(args)
            .arg("-C")
            .arg(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            cmd.env(key, value);
        }
        let mut child = cmd.spawn().unwrap();
        // The child may exit before reading (an invalid command): not an error here.
        let _ = child.stdin.take().unwrap().write_all(stdin);
        child.wait_with_output().unwrap()
    }
}

/// The gate's stdout: exit 0, one JSON line, parsed.
fn gate_envelope(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout.clone()).unwrap();
    assert_eq!(
        text.lines().count(),
        1,
        "stdout must be one envelope: {text}"
    );
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[test]
fn gate_claude_tem_envelope() {
    let f = fixture("tgc", Some("{}"));
    let out = f.run_stdin(
        &["tasks", "gate", "--agent", "claude"],
        HOOK,
        &[("USER", "ninguem")],
    );
    let v = gate_envelope(&out);
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(
        v["hookSpecificOutput"]["hookEventName"], "SessionStart",
        "{v}"
    );
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.contains("Gate TaskNotes obrigatório"), "{ctx}");
    assert!(
        ctx.contains("mdbase tasks list --responsavel <p> -C "),
        "{ctx}"
    );
    assert!(ctx.contains(&format!("-C {}", f.root.display())), "{ctx}");
    assert!(
        ctx.contains("mdbase tasks start \"CAMINHO_EXATO\""),
        "{ctx}"
    );
    assert!(ctx.contains("--status <status>"), "{ctx}");
    assert!(ctx.contains("aprovacao") && ctx.contains("pausa"), "{ctx}");
    for old in [
        "--session",
        "--agent",
        "--backend",
        "--config",
        "tasknotes-agent",
    ] {
        assert!(
            !ctx.contains(old),
            "{old} must not survive in the context: {ctx}"
        );
    }
}

#[test]
fn gate_codex_sem_ok() {
    let f = fixture("tgx", Some("{}"));
    let out = f.run_stdin(
        &["tasks", "gate", "--agent", "codex"],
        HOOK,
        &[("USER", "ninguem")],
    );
    let v = gate_envelope(&out);
    assert!(v.get("ok").is_none(), "codex envelope has no ok: {v}");
    assert_eq!(
        v["hookSpecificOutput"]["hookEventName"], "SessionStart",
        "{v}"
    );
    assert!(
        v["hookSpecificOutput"]["additionalContext"].is_string(),
        "{v}"
    );
}

#[test]
fn gate_lista_cronometros_abertos_da_pessoa() {
    let f = fixture("tgl", Some("{\"people\":{\"arthur\":[\"[[Arthur]]\"]}}"));
    let mut paths = Vec::new();
    for (title, who) in [("Tarefa A", "Arthur"), ("Tarefa B", "Mayara")] {
        let (code, v) = f.json(&["tasks", "create", title, "--responsavel", who]);
        assert_eq!(code, 0, "{v}");
        let path = v["result"]["path"].as_str().unwrap().to_string();
        let (code, v) = f.json(&["tasks", "start", &path, "--responsavel", who]);
        assert_eq!(code, 0, "{v}");
        paths.push(path);
    }
    let out = f.run_stdin(
        &[
            "tasks",
            "gate",
            "--agent",
            "claude",
            "--responsavel",
            "arthur",
        ],
        HOOK,
        &[("USER", "ninguem")],
    );
    let v = gate_envelope(&out);
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.contains("Cronômetros abertos de arthur:"), "{ctx}");
    assert!(
        ctx.contains(&paths[0]),
        "Arthur's open timer is listed: {ctx}"
    );
    assert!(
        !ctx.contains(&paths[1]),
        "Mayara's timer is not Arthur's: {ctx}"
    );
    assert!(ctx.contains(", há "), "the elapsed time is printed: {ctx}");
    // The timers come last, as the adapter's context ends with them.
    let last = ctx.trim_end().lines().last().unwrap();
    assert!(last.starts_with("- ") && last.contains(&paths[0]), "{ctx}");
}

#[test]
fn gate_usa_o_usuario_do_ambiente_sem_responsavel() {
    let f = fixture("tgu", Some("{\"people\":{\"arthur\":[\"[[Arthur]]\"]}}"));
    let (code, v) = f.json(&["tasks", "create", "Minha", "--responsavel", "Arthur"]);
    assert_eq!(code, 0, "{v}");
    let path = v["result"]["path"].as_str().unwrap().to_string();
    let (code, v) = f.json(&["tasks", "start", &path, "--responsavel", "Arthur"]);
    assert_eq!(code, 0, "{v}");

    // USER is lowercased before the `people` lookup.
    let out = f.run_stdin(
        &["tasks", "gate", "--agent", "omp"],
        HOOK,
        &[("USER", "Arthur")],
    );
    let v = gate_envelope(&out);
    assert_eq!(v["ok"], true, "{v}");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.contains("Cronômetros abertos de arthur:"), "{ctx}");
    assert!(ctx.contains(&path), "{ctx}");
}

#[test]
fn gate_sem_pessoa_omite_cronometros() {
    let f = fixture("tgs", Some("{}"));
    let out = f.run_stdin(
        &["tasks", "gate", "--agent", "claude"],
        HOOK,
        &[("USER", "ninguem")],
    );
    let v = gate_envelope(&out);
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(!ctx.contains("Cronômetros abertos"), "{ctx}");
}

#[test]
fn gate_nunca_falha_com_stdin_invalido() {
    let f = fixture("tgf", Some("{}"));
    let cases: [&[u8]; 5] = [
        b"",
        b"isto nao e json",
        b"\xEF\xBB\xBF{\"session_id\":\"abc\"}",
        b"\xEF\xBB\xBF{invalido",
        b"[1,2,3]",
    ];
    for stdin in cases {
        let out = f.run_stdin(
            &["tasks", "gate", "--agent", "claude"],
            stdin,
            &[("USER", "ninguem")],
        );
        let v = gate_envelope(&out);
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"], "SessionStart",
            "{v}"
        );
        assert!(
            v["hookSpecificOutput"]["additionalContext"].is_string(),
            "{v}"
        );
    }
}

#[test]
fn gate_agente_desconhecido_tem_envelope() {
    let f = fixture("tga", Some("{}"));
    let out = f.run_stdin(
        &["tasks", "gate", "--agent", "inventado"],
        HOOK,
        &[("USER", "ninguem")],
    );
    let v = gate_envelope(&out);
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(
        v["hookSpecificOutput"]["hookEventName"], "SessionStart",
        "{v}"
    );
    assert!(
        v["hookSpecificOutput"]["additionalContext"].is_string(),
        "{v}"
    );
}

#[test]
fn gate_sem_colecao_da_envelope_minimo() {
    let f = fixture("tgm", Some("{}"));
    // An empty folder: no collection, so no `task` type to read the statuses from.
    let empty =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tgm-vazia{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    let elsewhere = Fixture {
        root: empty,
        state: f.state.clone(),
    };
    let out = elsewhere.run_stdin(
        &["tasks", "gate", "--agent", "claude"],
        HOOK,
        &[("USER", "ninguem")],
    );
    let v = gate_envelope(&out);
    assert_eq!(v["ok"], true, "{v}");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        ctx.contains("O contexto completo não pôde ser montado"),
        "{ctx}"
    );
    assert!(
        !ctx.contains("Listar:"),
        "the minimal context has no commands: {ctx}"
    );
}

#[test]
fn gate_sem_perfil_da_envelope_minimo() {
    // No `--state-dir` and no HOME: the profile cannot be resolved.
    let f = fixture("tgp", Some("{}"));
    let out = Command::new(env!("CARGO_BIN_EXE_mdbase"))
        .args(["tasks", "gate", "--agent", "claude", "-C"])
        .arg(&f.root)
        .env_remove("HOME")
        .env_remove("MDBASE_HOME")
        .env("USER", "ninguem")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let v = gate_envelope(&out);
    assert_eq!(v["ok"], true, "{v}");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.contains("profile_unavailable"), "{ctx}");
}

#[test]
fn gate_usa_status_do_tipo() {
    let f = fixture("tgt2", Some("{}"));
    // The collection's type names its statuses otherwise than the rules do.
    let mut def: Value = serde_json::from_str(TASK_TYPE).unwrap();
    let status = &mut def["implements"][0]["binding"]["status"];
    status["values"] = serde_json::json!(["rascunho", "feito", "descartado"]);
    status["default"] = serde_json::json!("rascunho");
    status["completed_values"] = serde_json::json!(["feito"]);
    status["definitions"] = serde_json::json!([]);
    let schema_status = &mut def["schema"]["value"]["properties"]["status"];
    schema_status["enum"] = serde_json::json!(["rascunho", "feito", "descartado"]);
    schema_status["default"] = serde_json::json!("rascunho");
    std::fs::write(
        f.root.join("_types/task.md"),
        format!(
            "---\n{}\n---\n",
            serde_json::to_string_pretty(&def).unwrap()
        ),
    )
    .unwrap();

    let out = f.run_stdin(
        &["tasks", "gate", "--agent", "claude"],
        HOOK,
        &[("USER", "ninguem")],
    );
    let v = gate_envelope(&out);
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.contains("--status feito antes de a seguinte"), "{ctx}");
    assert!(ctx.contains("rascunho (status do tipo task)"), "{ctx}");
    assert!(ctx.contains("feito (status do tipo task)"), "{ctx}");
    assert!(ctx.contains("descartado (status do tipo task)"), "{ctx}");
    for fixed in ["concluido", "aprovacao", "cancelado", "--status concluido"] {
        assert!(
            !ctx.contains(fixed),
            "{fixed} is the rules' name, not the type's: {ctx}"
        );
    }
}

/// `tasks exec` (hidden): one envelope on stdout, exit 0, for any stdin.
fn exec_envelope(operation: &str, stdin: &[u8]) -> Value {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_mdbase"))
        .args(["tasks", "exec", operation])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)))
}

#[test]
fn exec_operacao_fora_do_contrato_da_envelope() {
    let v = exec_envelope("no.such.op", b"{}");
    assert_eq!(v["ok"], false, "{v}");
    assert!(v["error"].is_string(), "{v}");
    assert_eq!(v["error_details"]["operation"], "no.such.op", "{v}");
    assert_eq!(v["error_details"]["code"], "unsupported_operation", "{v}");
    assert!(v["error_details"]["message"].is_string(), "{v}");
}

#[test]
fn exec_entrada_invalida_da_envelope() {
    for stdin in [&b""[..], b"isto nao e json", b"[1,2,3]"] {
        let v = exec_envelope("meta.claim", stdin);
        assert_eq!(v["ok"], false, "{v}");
        assert_eq!(v["error_details"]["code"], "invalid_input", "{v}");
    }
}

#[test]
fn exec_meta_claim_declara_o_perfil_e_nenhuma_capacidade() {
    let v = exec_envelope("meta.claim", b"{}");
    assert_eq!(v["ok"], true, "{v}");
    let r = &v["result"];
    assert_eq!(r["implementation"], "mdbase-tasks", "{v}");
    assert_eq!(r["profiles"], serde_json::json!(["core-lite"]), "{v}");
    assert_eq!(r["capabilities"], serde_json::json!([]), "{v}");
    assert_eq!(r["validation_modes"], serde_json::json!(["strict"]), "{v}");
    assert_eq!(r["spec_version"], "0.3.0", "{v}");
    assert!(r["version"].is_string(), "{v}");
}

#[test]
fn exec_fica_fora_da_ajuda() {
    let out = Command::new(env!("CARGO_BIN_EXE_mdbase"))
        .args(["tasks", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!text.contains("exec"), "{text}");
}

/// A collection whose records keep their type under `type` and `types`, as the
/// BIBLIOTECA does, with the `plano` type declared.
fn fixture_tipado(tag: &str) -> Fixture {
    let f = fixture(tag, None);
    std::fs::write(
        f.root.join("mdbase.yaml"),
        "spec_version: \"0.3.0\"\nname: Test\nsettings:\n  explicit_type_keys: [type, types]\n",
    )
    .unwrap();
    std::fs::write(
        f.root.join("_types/plano.md"),
        "---\nkind: mdbase.type\nname: plano\nmatch:\n  path_glob: \"01-planejamento/**/*.md\"\nschema:\n  dialect: json-schema-2020-12\n  value:\n    type: object\n---\n",
    )
    .unwrap();
    f
}

#[test]
fn promote_documento_tipado_aparece_no_list() {
    let f = fixture_tipado("tpt");
    std::fs::create_dir_all(f.root.join("01-planejamento")).unwrap();
    std::fs::write(
        f.root.join("01-planejamento/Ideia.md"),
        "---\ntype: plano\ntags: [ideia]\ntitle: \"Ideia\"\ndateCreated: \"2026-11-01T09:00:00.000-03:00\"\n---\nCorpo.\n",
    )
    .unwrap();
    // A note without dateCreated: its `data` date becomes the creation date.
    std::fs::write(
        f.root.join("01-planejamento/Sem data.md"),
        "---\ntype: plano\ntags: [ideia]\ntitle: \"Sem data\"\ndata: '2026-11-14'\n---\nCorpo.\n",
    )
    .unwrap();
    let (code, v) = f.json(&["tasks", "promote", "01-planejamento/Ideia.md"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = f.json(&["tasks", "promote", "01-planejamento/Sem data.md"]);
    assert_eq!(code, 0, "{v}");
    // A note with no dates at all: its creation date is the clock's `now`.
    std::fs::write(
        f.root.join("01-planejamento/Agora.md"),
        "---\ntype: plano\ntags: [ideia]\ntitle: \"Agora\"\n---\nCorpo.\n",
    )
    .unwrap();
    let (code, v) = f.json(&["tasks", "promote", "01-planejamento/Agora.md"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = f.json(&["tasks", "list"]);
    assert_eq!(code, 0, "{v}");
    let paths: Vec<&str> = v["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["path"].as_str())
        .collect();
    assert!(paths.contains(&"01-planejamento/Ideia.md"), "{v}");
    assert!(paths.contains(&"01-planejamento/Sem data.md"), "{v}");
    assert!(paths.contains(&"01-planejamento/Agora.md"), "{v}");
    assert!(f.read("01-planejamento/Ideia.md").contains("type: plano"));

    // The date-only value keeps its local day: midnight with the clock's offset,
    // whatever the machine's timezone is.
    let data = creation_of(&f, "01-planejamento/Sem data.md");
    let agora = creation_of(&f, "01-planejamento/Agora.md");
    assert!(data.starts_with("2026-11-14T00:00:00"), "{data}");
    assert_eq!(offset_of(&data), offset_of(&agora), "{data} vs {agora}");
}

/// The `dateCreated` value written in a note's frontmatter.
fn creation_of(f: &Fixture, rel: &str) -> String {
    f.read(rel)
        .lines()
        .find_map(|l| l.strip_prefix("dateCreated: "))
        .expect("dateCreated written")
        .trim_matches('"')
        .to_string()
}

/// The zone part of a date-time: `Z` or the trailing `+HH:MM` / `-HH:MM`.
fn offset_of(date_time: &str) -> &str {
    match date_time.rfind(['+', '-']).filter(|i| *i > 10) {
        Some(i) => &date_time[i..],
        None => &date_time[date_time.len() - 1..],
    }
}
