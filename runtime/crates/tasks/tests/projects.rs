//! Tests for the project lifecycle rules: transitions, milestones, renames, the
//! task summary per project and the review queue.
//!
//! Ported case by case from the adapter's `test/projects.test.mjs` and
//! `test/project-summary.test.mjs`. Dates are passed in, as the rules take no clock.

use mdbn_tasks::{
    Binding, ProjectItem, ProjectsSummary, ReviewItem, TaskError, TaskView, filter_queue_by_owner,
    list_projects, project_assign, project_milestone, project_name, project_rename_status,
    project_review, project_status, projects_summary,
};
use serde_json::{Map, Value, json};

const TASK_TYPE: &str = include_str!("fixtures/task-type.json");
const HOJE: &str = "2026-11-11";
const ARTHUR: &str = "[[Arthur Cardoso]]";

fn binding() -> Binding {
    let def: Value = serde_json::from_str(TASK_TYPE).expect("fixture is valid JSON");
    Binding::from_type(&def).expect("task type binds")
}

/// Merges `extra` into `base`. A `null` value is the same as a missing key.
fn merged(base: Value, extra: Value) -> Map<String, Value> {
    let mut out = base.as_object().expect("object").clone();
    for (key, value) in extra.as_object().expect("object") {
        out.insert(key.clone(), value.clone());
    }
    out
}

/// A project note under the brand folder `a`, active and owned by Arthur by default.
fn projeto(name: &str, extra: Value) -> TaskView {
    let base =
        json!({"type": "projeto", "brand": "[[A]]", "status": "em-andamento", "owner": ARTHUR});
    TaskView {
        path: format!("02-marcas/a/Projetos/{name}.md"),
        revision: "rev-1".to_string(),
        frontmatter: merged(base, extra),
    }
}

/// A task note, `to-do` by default.
fn tarefa(name: &str, extra: Value) -> TaskView {
    let base = json!({"status": "to-do", "tags": ["task"]});
    TaskView {
        path: format!("06-tarefas/Tasks/{name}.md"),
        revision: "rev-1".to_string(),
        frontmatter: merged(base, extra),
    }
}

fn campos(op: &Value) -> &Map<String, Value> {
    assert_eq!(op["op"], json!("update"), "{op}");
    op["fields"].as_object().expect("fields object")
}

fn code<T: std::fmt::Debug>(r: Result<T, TaskError>) -> &'static str {
    r.expect_err("the rule must refuse").code
}

fn summary(projects: &[TaskView], tasks: &[TaskView]) -> ProjectsSummary {
    projects_summary(&binding(), projects, tasks)
}

fn item<'a>(s: &'a ProjectsSummary, name: &str) -> &'a ProjectItem {
    s.items
        .iter()
        .find(|entry| entry.name == name)
        .expect("project in summary")
}

fn queue(items: &[ProjectItem]) -> Vec<ReviewItem> {
    project_review(items, HOJE, 30)
}

fn pairs(q: &[ReviewItem]) -> Vec<(String, &'static str)> {
    q.iter()
        .map(|entry| (entry.item.name.clone(), entry.reason))
        .collect()
}

// --- transitions (projects.test.mjs) ---

#[test]
fn os_papeis_de_projeto_sao_os_do_tipo_task() {
    assert_eq!(
        binding().statuses,
        [
            "backlog",
            "to-do",
            "em-andamento",
            "aprovacao",
            "pausa",
            "concluido",
            "cancelado"
        ]
    );
}

#[test]
fn concluir_grava_desfecho_e_data_de_hoje() {
    let p = projeto("q", json!({}));
    let op = project_status(&binding(), &p, "concluido", Some("entregue"), None, HOJE)
        .expect("closes with an outcome");
    let f = campos(&op);
    assert_eq!(f.get("status"), Some(&json!("concluido")));
    assert_eq!(f.get("outcome"), Some(&json!("entregue")));
    assert_eq!(f.get("outcomeDate"), Some(&json!(HOJE)));
    assert_eq!(f.get("resumeOn"), Some(&Value::Null));
    assert_eq!(op["if_revision"], json!("rev-1"));
    assert_eq!(op["path"], json!("02-marcas/a/Projetos/q.md"));
}

#[test]
fn pausar_grava_o_motivo_e_a_data_de_retomada() {
    let p = projeto("q", json!({}));
    let op = project_status(
        &binding(),
        &p,
        "pausa",
        Some("cliente sumiu"),
        Some("2026-12-01"),
        HOJE,
    )
    .expect("pauses");
    let f = campos(&op);
    assert_eq!(f.get("status"), Some(&json!("pausa")));
    assert_eq!(f.get("outcome"), Some(&json!("cliente sumiu")));
    assert_eq!(f.get("outcomeDate"), Some(&json!(HOJE)));
    assert_eq!(f.get("resumeOn"), Some(&json!("2026-12-01")));
}

#[test]
fn voltar_a_ativo_apaga_desfecho_data_e_retomada() {
    let p = projeto(
        "q",
        json!({"status": "concluido", "outcome": "x", "outcomeDate": HOJE}),
    );
    let op = project_status(&binding(), &p, "em-andamento", None, None, HOJE).expect("reopens");
    let f = campos(&op);
    assert_eq!(f.get("status"), Some(&json!("em-andamento")));
    assert_eq!(f.get("outcome"), Some(&Value::Null));
    assert_eq!(f.get("outcomeDate"), Some(&Value::Null));
    assert_eq!(f.get("resumeOn"), Some(&Value::Null));
}

#[test]
fn fechar_sem_desfecho_e_recusado() {
    let p = projeto("q", json!({}));
    assert_eq!(
        code(project_status(
            &binding(),
            &p,
            "concluido",
            None,
            None,
            HOJE
        )),
        "desfecho_obrigatorio"
    );
    assert_eq!(
        code(project_status(
            &binding(),
            &p,
            "cancelado",
            Some("   "),
            None,
            HOJE
        )),
        "desfecho_obrigatorio"
    );
}

#[test]
fn pausa_exige_data_de_retomada_valida_e_futura() {
    let p = projeto("q", json!({}));
    let b = binding();
    assert_eq!(
        code(project_status(&b, &p, "pausa", Some("x"), None, HOJE)),
        "retomada_obrigatoria"
    );
    assert_eq!(
        code(project_status(
            &b,
            &p,
            "pausa",
            Some("x"),
            Some("11/12/2026"),
            HOJE
        )),
        "retomada_invalida"
    );
    assert_eq!(
        code(project_status(
            &b,
            &p,
            "pausa",
            Some("x"),
            Some("2026-02-30"),
            HOJE
        )),
        "retomada_invalida"
    );
    assert_eq!(
        code(project_status(
            &b,
            &p,
            "pausa",
            Some("x"),
            Some("2026-11-11"),
            HOJE
        )),
        "retomada_no_passado"
    );
}

#[test]
fn data_de_retomada_so_cabe_em_pausa() {
    let p = projeto("q", json!({}));
    assert_eq!(
        code(project_status(
            &binding(),
            &p,
            "concluido",
            Some("x"),
            Some("2026-12-01"),
            HOJE
        )),
        "retomada_so_em_pausa"
    );
}

#[test]
fn sem_responsavel_nenhuma_transicao_passa() {
    let p = projeto("q", json!({"owner": null}));
    assert_eq!(
        code(project_status(
            &binding(),
            &p,
            "em-andamento",
            None,
            None,
            HOJE
        )),
        "owner_obrigatorio"
    );
}

#[test]
fn papel_desconhecido_e_recusado_pelo_nome() {
    let p = projeto("q", json!({}));
    let err = project_status(&binding(), &p, "feito", Some("x"), None, HOJE).expect_err("refused");
    assert_eq!(err.code, "invalid_project_status");
}

#[test]
fn o_valor_e_conferido_contra_o_binding_do_tipo_task() {
    // A status the task type does not allow is refused, even if the rules know it.
    let mut def: Value = serde_json::from_str(TASK_TYPE).expect("fixture is valid JSON");
    let values = def["implements"][0]["binding"]["status"]["values"]
        .as_array_mut()
        .expect("status values");
    values.retain(|v| v != "aprovacao");
    let b = Binding::from_type(&def).expect("binds");
    let p = projeto("q", json!({}));
    assert_eq!(
        code(project_status(&b, &p, "aprovacao", Some("x"), None, HOJE)),
        "invalid_project_status"
    );
}

#[test]
fn owner_legado_em_texto_ou_lista_nao_conta_como_responsavel() {
    let b = binding();
    let texto = projeto("q", json!({"owner": "Arthur"}));
    assert_eq!(
        code(project_status(
            &b,
            &texto,
            "concluido",
            Some("x"),
            None,
            HOJE
        )),
        "owner_obrigatorio"
    );
    let lista = projeto("q", json!({"owner": ["[[Arthur Cardoso]]"]}));
    assert_eq!(
        code(project_status(
            &b,
            &lista,
            "cancelado",
            Some("x"),
            None,
            HOJE
        )),
        "owner_obrigatorio"
    );
}

#[test]
fn backlog_e_todo_apagam_desfecho_e_pedem_so_responsavel() {
    let b = binding();
    for role in ["backlog", "to-do"] {
        let p = projeto("q", json!({}));
        let op = project_status(&b, &p, role, None, None, HOJE).expect("open status");
        let f = campos(&op);
        assert_eq!(f.get("status"), Some(&json!(role)));
        assert_eq!(f.get("outcome"), Some(&Value::Null));
        assert_eq!(f.get("outcomeDate"), Some(&Value::Null));
        assert_eq!(f.get("resumeOn"), Some(&Value::Null));
    }
}

// --- assign, milestone, rename (projects.test.mjs and cli.test.mjs) ---

#[test]
fn responsavel_grava_owner_como_wikilink() {
    let p = projeto("r", json!({}));
    let op = project_assign(&p, &["[[Lucilene]]".to_string()]).expect("one person");
    assert_eq!(campos(&op).get("owner"), Some(&json!("[[Lucilene]]")));
}

#[test]
fn grupo_como_responsavel_e_recusado() {
    let p = projeto("r", json!({}));
    let grupo = ["[[Arthur]]".to_string(), "[[Fulano]]".to_string()];
    assert_eq!(code(project_assign(&p, &grupo)), "responsavel_grupo");
}

#[test]
fn proximo_marco_texto_obrigatorio_data_opcional_e_valida_limpar_apaga() {
    let p = projeto("q", json!({}));
    let op = project_milestone(&p, Some(" lançar "), Some("2026-12-01"), false).expect("sets");
    assert_eq!(campos(&op).get("nextMilestone"), Some(&json!("lançar")));
    assert_eq!(
        campos(&op).get("nextMilestoneDate"),
        Some(&json!("2026-12-01"))
    );

    let sem_data = project_milestone(&p, Some("lançar"), None, false).expect("sets without date");
    assert_eq!(
        campos(&sem_data).get("nextMilestoneDate"),
        Some(&Value::Null)
    );

    assert_eq!(
        code(project_milestone(&p, None, None, false)),
        "marco_obrigatorio"
    );
    assert_eq!(
        code(project_milestone(&p, Some("x"), Some("1/12"), false)),
        "data_invalida"
    );

    let limpo = project_milestone(&p, None, None, true).expect("clears");
    assert_eq!(campos(&limpo).get("nextMilestone"), Some(&Value::Null));
    assert_eq!(campos(&limpo).get("nextMilestoneDate"), Some(&Value::Null));
}

#[test]
fn renomear_status_troca_so_o_valor_e_recusa_destino_invalido() {
    let velho = projeto("velho", json!({"status": "ativo"}));
    let q = projeto("q", json!({"status": "em-andamento"}));
    let ops =
        project_rename_status(&binding(), &[velho, q], "ativo", "em-andamento").expect("renames");
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["path"], json!("02-marcas/a/Projetos/velho.md"));
    assert_eq!(ops[0]["if_revision"], json!("rev-1"));
    assert_eq!(
        campos(&ops[0]),
        &merged(json!({}), json!({"status": "em-andamento"}))
    );

    let outro = projeto("outro", json!({"status": "ativo"}));
    assert_eq!(
        code(project_rename_status(
            &binding(),
            &[outro],
            "ativo",
            "feito"
        )),
        "invalid_project_status"
    );
}

#[test]
fn renomear_status_sem_projeto_no_valor_antigo_nao_planeja_nada() {
    let q = projeto("q", json!({"status": "em-andamento"}));
    let ops = project_rename_status(&binding(), &[q], "ativo", "em-andamento").expect("no-op");
    assert!(ops.is_empty());
}

// --- summary (project-summary.test.mjs) ---

#[test]
fn o_nome_do_link_tira_colchetes_alias_pasta_e_extensao() {
    let p = [projeto("p", json!({}))];
    let tasks = [
        tarefa("a", json!({"projects": ["[[02-marcas/x/Projetos/p|P]]"]})),
        tarefa("b", json!({"projects": ["[[p.md]]"]})),
        tarefa("c", json!({"projects": ["p"]})),
    ];
    assert_eq!(item(&summary(&p, &tasks), "p").total_tasks, 3);
}

#[test]
fn atividade_vem_do_cronometro_nunca_do_datemodified() {
    let p = [projeto("p", json!({}))];
    let com_cronometro = tarefa(
        "a",
        json!({"projects": ["[[p]]"], "timeEntries": [{"startTime": "2026-11-09T12:00:00Z"}], "dateModified": "2026-11-10T00:00:00Z"}),
    );
    assert_eq!(
        item(&summary(&p, &[com_cronometro]), "p")
            .last_activity
            .as_deref(),
        Some("2026-11-09")
    );

    let so_modificada = tarefa(
        "b",
        json!({"projects": ["[[p]]"], "dateModified": "2026-11-10T00:00:00Z"}),
    );
    assert_eq!(
        item(&summary(&p, &[so_modificada]), "p").last_activity,
        None
    );

    let concluida = tarefa(
        "c",
        json!({"projects": ["[[p]]"], "completedDate": "2026-10-01", "dateCreated": "2026-09-01T10:00:00-03:00"}),
    );
    assert_eq!(
        item(&summary(&p, &[concluida]), "p")
            .last_activity
            .as_deref(),
        Some("2026-10-01")
    );
}

#[test]
fn tarefa_fechada_conta_no_total_e_nao_nas_abertas() {
    let p = [projeto("p", json!({}))];
    let tasks = [
        tarefa(
            "a",
            json!({"projects": ["[[p]]"], "status": "concluido", "completedDate": HOJE}),
        ),
        tarefa("b", json!({"projects": ["[[p|P]]"]})),
    ];
    let s = summary(&p, &tasks);
    let entry = item(&s, "p");
    assert_eq!(entry.total_tasks, 2);
    assert_eq!(entry.open_tasks, 1);
    assert_eq!(entry.open_task_paths, ["06-tarefas/Tasks/b.md"]);
}

#[test]
fn link_para_projeto_inexistente_vira_orfao_com_o_caminho_da_tarefa() {
    let orfa = tarefa("o", json!({"projects": ["[[sumiu]]"]}));
    let s = summary(&[projeto("p", json!({}))], &[orfa]);
    assert_eq!(s.orphans.len(), 1);
    assert_eq!(s.orphans[0].name, "sumiu");
    assert_eq!(s.orphans[0].tasks, ["06-tarefas/Tasks/o.md"]);
}

#[test]
fn status_fora_do_tipo_e_owner_que_nao_e_wikilink_saem_nulos() {
    let s = summary(
        &[
            projeto("s", json!({"status": "em andamento"})),
            projeto("t", json!({"owner": "Arthur"})),
            projeto("u", json!({"owner": ["[[A]]"]})),
            projeto("v", json!({"status": null, "owner": null})),
        ],
        &[],
    );
    assert_eq!(item(&s, "s").status, None);
    assert_eq!(item(&s, "t").owner, None);
    assert_eq!(item(&s, "u").owner, None);
    assert_eq!(item(&s, "v").status, None);
}

#[test]
fn o_resumo_traz_o_proximo_marco() {
    let s = summary(
        &[projeto(
            "p",
            json!({"nextMilestone": "lançar", "nextMilestoneDate": "2026-12-01"}),
        )],
        &[],
    );
    assert_eq!(item(&s, "p").next_milestone.as_deref(), Some("lançar"));
    assert_eq!(
        item(&s, "p").next_milestone_date.as_deref(),
        Some("2026-12-01")
    );
}

#[test]
fn fila_retomada_vencida_primeiro_depois_sem_responsavel_status_desconhecido_parado_e_aprovacao() {
    let s = summary(
        &[
            projeto("aprov", json!({"status": "aprovacao", "outcome": "x"})),
            projeto("parado", json!({})),
            projeto("recente", json!({})),
            projeto("estranho", json!({"status": "em andamento"})),
            projeto("orfao", json!({"owner": "Arthur"})),
            projeto(
                "retomar",
                json!({"status": "pausa", "outcome": "x", "resumeOn": HOJE}),
            ),
            projeto(
                "pausa-futura",
                json!({"status": "pausa", "outcome": "x", "resumeOn": "2026-12-01"}),
            ),
        ],
        &[
            tarefa(
                "p1",
                json!({"projects": ["[[parado]]"], "timeEntries": [{"startTime": "2026-10-01T12:00:00Z"}]}),
            ),
            tarefa(
                "r1",
                json!({"projects": ["[[recente]]"], "timeEntries": [{"startTime": "2026-10-20T12:00:00Z"}]}),
            ),
        ],
    );
    let q = queue(&s.items);
    assert_eq!(
        pairs(&q),
        [
            ("retomar".to_string(), "retomada_vencida"),
            ("orfao".to_string(), "sem_responsavel"),
            ("estranho".to_string(), "status_desconhecido"),
            ("parado".to_string(), "parado"),
            ("aprov".to_string(), "aguardando_aprovacao"),
        ]
    );
}

#[test]
fn projeto_fechado_nao_entra_na_fila_nem_sem_responsavel() {
    let s = summary(
        &[projeto(
            "fim",
            json!({"status": "concluido", "outcome": "x", "owner": null}),
        )],
        &[],
    );
    assert!(queue(&s.items).is_empty());
}

#[test]
fn pausado_sem_data_de_retomada_entra_na_fila_como_pausa_sem_retomada() {
    let s = summary(
        &[projeto("leg", json!({"status": "pausa", "outcome": "x"}))],
        &[],
    );
    assert_eq!(
        pairs(&queue(&s.items)),
        [("leg".to_string(), "pausa_sem_retomada")]
    );
}

#[test]
fn filtro_por_pessoa_mantem_todo_projeto_sem_responsavel_qualquer_que_seja_o_motivo() {
    let s = summary(
        &[
            projeto(
                "venceu",
                json!({"status": "pausa", "outcome": "x", "resumeOn": "2026-11-01", "owner": null}),
            ),
            projeto("da-mayara", json!({"owner": "[[Mayara Almeida]]"})),
            projeto("meu", json!({})),
        ],
        &[],
    );
    let mine = filter_queue_by_owner(queue(&s.items), ARTHUR);
    let mut names: Vec<String> = mine.iter().map(|e| e.item.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["meu", "venceu"]);
}

#[test]
fn fechado_ou_pausado_sem_desfecho_vem_antes_de_tudo_mesmo_fechado() {
    let s = summary(
        &[
            projeto(
                "venceu",
                json!({"status": "pausa", "outcome": "x", "resumeOn": "2026-11-01"}),
            ),
            projeto("arrastado", json!({"status": "concluido"})),
            projeto(
                "certo",
                json!({"status": "concluido", "outcome": "entregue"}),
            ),
        ],
        &[],
    );
    assert_eq!(
        pairs(&queue(&s.items)),
        [
            ("arrastado".to_string(), "desfecho_ausente"),
            ("venceu".to_string(), "retomada_vencida"),
        ]
    );
}

#[test]
fn parado_vale_para_em_andamento_e_backlog_parado_nao_entra_na_fila() {
    let s = summary(
        &[
            projeto("fila", json!({})),
            projeto("espera", json!({"status": "backlog"})),
        ],
        &[],
    );
    assert_eq!(pairs(&queue(&s.items)), [("fila".to_string(), "parado")]);
}

#[test]
fn parado_usa_o_limite_de_dias_passado() {
    let p = [projeto("p", json!({}))];
    let tarefas = [tarefa(
        "a",
        json!({"projects": ["[[p]]"], "timeEntries": [{"startTime": "2026-10-20T12:00:00Z"}]}),
    )];
    let s = summary(&p, &tarefas);
    // The last activity is 22 days before HOJE: fresh at 30 days, stale at 21.
    assert!(project_review(&s.items, HOJE, 30).is_empty());
    let apertada = project_review(&s.items, HOJE, 21);
    assert_eq!(pairs(&apertada), [("p".to_string(), "parado")]);
}

// --- listing (cli.test.mjs: projects) ---

#[test]
fn lista_padrao_esconde_fechados_e_filtra_por_status_e_dono() {
    let b = binding();
    let s = summary(
        &[
            projeto("ativo", json!({})),
            projeto("fechado", json!({"status": "concluido", "outcome": "x"})),
            projeto(
                "da-mayara",
                json!({"owner": "[[Mayara Almeida]]", "status": "aprovacao", "outcome": "x"}),
            ),
        ],
        &[],
    );
    let names =
        |items: Vec<ProjectItem>| -> Vec<String> { items.into_iter().map(|i| i.name).collect() };
    assert_eq!(
        names(list_projects(&b, &s.items, None, None).unwrap()),
        ["ativo", "da-mayara"]
    );
    assert_eq!(
        names(list_projects(&b, &s.items, Some("concluido"), None).unwrap()),
        ["fechado"]
    );
    assert_eq!(
        names(list_projects(&b, &s.items, None, Some(ARTHUR)).unwrap()),
        ["ativo"]
    );
    assert_eq!(
        list_projects(&b, &s.items, Some("ativo"), None)
            .unwrap_err()
            .code,
        "invalid_project_status"
    );
}

#[test]
fn o_nome_do_projeto_vem_do_caminho_ou_do_link() {
    assert_eq!(project_name("02-marcas/a/Projetos/p.md"), "p");
    assert_eq!(project_name("[[02-marcas/a/Projetos/p|P]]"), "p");
}

// --- project paths (review fix: the template is not a project) ---

#[test]
fn path_matches_asterisco_fica_num_segmento() {
    use mdbn_tasks::path_matches;
    let p = "02-marcas/*/Projetos/*.md";
    assert!(path_matches(p, "02-marcas/madame-labs/Projetos/x.md"));
    assert!(!path_matches(p, "08-templates/projeto.md"));
    assert!(!path_matches(p, "02-marcas/a/b/Projetos/x.md"));
    assert!(!path_matches(p, "02-marcas/madame-labs/Projetos/x.md.bak"));
    assert!(path_matches(
        "02-marcas/*/Projetos/*",
        "02-marcas/a/Projetos/x.md"
    ));
    assert!(!path_matches(
        "02-marcas/Projetos",
        "02-marcas/Projetos/x.md"
    ));
}

#[test]
fn config_sem_project_paths_aceita_todo_projeto() {
    use mdbn_tasks::{TasksConfig, is_project_path};
    let vazio = TasksConfig::default();
    assert!(is_project_path(&vazio, "08-templates/projeto.md"));
    let com =
        TasksConfig::from_json(&json!({"project_paths": ["02-marcas/*/Projetos/*.md"]})).unwrap();
    assert!(is_project_path(&com, "02-marcas/a/Projetos/x.md"));
    assert!(!is_project_path(&com, "08-templates/projeto.md"));
    let invalido = TasksConfig::from_json(&json!({"project_paths": "02-marcas/*"}));
    assert_eq!(invalido.unwrap_err().code, "config_invalid");
}
