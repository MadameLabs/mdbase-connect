//! Tests for the write planners over an existing task: status, projects,
//! blockers, archive and skipped occurrences.

use mdbn_tasks::{
    Binding, TaskView, archive, block, is_open, link_projects, set_status, skip, unblock, unskip,
    wikilink,
};
use serde_json::{Map, Value, json};

const TASK_TYPE: &str = include_str!("fixtures/task-type.json");
const HOJE: &str = "2026-11-17";

fn binding() -> Binding {
    let def: Value = serde_json::from_str(TASK_TYPE).expect("fixture is valid JSON");
    Binding::from_type(&def).expect("task type binds")
}

fn fm(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn tarefa(path: &str, frontmatter: Value) -> TaskView {
    TaskView {
        path: path.to_string(),
        revision: "rev-1".to_string(),
        frontmatter: fm(frontmatter),
    }
}

fn campos(op: &Value) -> &Map<String, Value> {
    assert_eq!(op["op"], json!("update"));
    op["fields"].as_object().expect("fields object")
}

#[test]
fn fechar_grava_data_de_conclusao() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({"status": "em-andamento"}));
    let op = set_status(&b, &t, "concluido", None, HOJE).expect("closes");
    assert_eq!(campos(&op).get("status"), Some(&json!("concluido")));
    assert_eq!(campos(&op).get("completedDate"), Some(&json!("2026-11-17")));
}

#[test]
fn data_de_conclusao_informada_vale_no_fechamento() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({}));
    let op = set_status(&b, &t, "cancelado", Some("2026-11-10"), HOJE).expect("closes");
    assert_eq!(campos(&op).get("completedDate"), Some(&json!("2026-11-10")));
}

#[test]
fn data_de_conclusao_so_em_fechada() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({}));
    let err = set_status(&b, &t, "pausa", Some("2026-11-10"), HOJE).expect_err("pausa is open");
    assert_eq!(err.code, "completed_date_requires_closed_status");
}

#[test]
fn data_de_conclusao_no_futuro_recusa() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({}));
    let err = set_status(&b, &t, "concluido", Some("2026-11-18"), HOJE).expect_err("future date");
    assert_eq!(err.code, "invalid_completed_date");
}

#[test]
fn data_de_conclusao_mal_formada_recusa() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({}));
    for ruim in [
        "17/11/2026",
        "2026-11-1",
        "2026-13-01",
        "2026-11-00",
        "hoje",
    ] {
        let err = set_status(&b, &t, "concluido", Some(ruim), HOJE).expect_err(ruim);
        assert_eq!(err.code, "invalid_completed_date", "{ruim}");
    }
}

#[test]
fn status_fora_do_tipo_recusa() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({}));
    let err = set_status(&b, &t, "ativo", None, HOJE).expect_err("not in the type");
    assert_eq!(err.code, "invalid_status");
}

#[test]
fn sem_status_gravado_conta_como_backlog() {
    let b = binding();
    assert!(is_open(&b, &tarefa("06-tarefas/A.md", json!({}))));
    assert!(!is_open(
        &b,
        &tarefa("06-tarefas/A.md", json!({"status": "cancelado"}))
    ));
    assert!(!is_open(
        &b,
        &tarefa("06-tarefas/A.md", json!({"status": "concluido"}))
    ));
    assert!(is_open(
        &b,
        &tarefa("06-tarefas/A.md", json!({"status": "pausa"}))
    ));
}

#[test]
fn arquivada_nao_e_aberta() {
    let b = binding();
    let t = tarefa(
        "06-tarefas/A.md",
        json!({"status": "pausa", "tags": ["task", "archived"]}),
    );
    assert!(!is_open(&b, &t));
}

#[test]
fn projetos_viram_wikilink_e_trocam_a_lista() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({"projects": ["[[velho]]"]}));
    let op = link_projects(&b, &t, &["mdbase-tasks".to_string()]).expect("links");
    assert_eq!(
        campos(&op).get("projects"),
        Some(&json!(["[[mdbase-tasks]]"]))
    );
}

#[test]
fn projetos_vazios_recusa() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({"projects": ["[[velho]]"]}));
    let err = link_projects(&b, &t, &[]).expect_err("empty project list");
    assert_eq!(err.code, "missing_project");
}

#[test]
fn lista_gravada_como_texto_vira_um_item() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({"tags": "task"}));
    let op = archive(&b, &t).expect("archives");
    assert_eq!(campos(&op).get("tags"), Some(&json!(["task", "archived"])));
}

#[test]
fn wikilink_nao_duplica_colchetes_nem_caminho() {
    assert_eq!(wikilink("mdbase-tasks"), "[[mdbase-tasks]]");
    assert_eq!(wikilink("[[mdbase-tasks]]"), "[[mdbase-tasks]]");
    assert_eq!(wikilink("02-marcas/x/Projetos/Foo.md"), "[[Foo]]");
}

#[test]
fn bloqueio_sem_duplicar_e_sem_si_mesmo() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({}));
    let existing = ["06-tarefas/X.md".to_string()];

    let primeira = block(&b, &t, &["X.md".to_string()], &existing).expect("blocks");
    let lista = campos(&primeira)
        .get("blockedBy")
        .cloned()
        .expect("blockedBy");
    assert_eq!(lista, json!([{"uid": "[[X]]", "reltype": "FINISHTOSTART"}]));

    // Second block, now reading the list the first one wrote.
    let depois = tarefa("06-tarefas/A.md", json!({"blockedBy": lista}));
    let segunda = block(&b, &depois, &["06-tarefas/X.md".to_string()], &existing).expect("blocks");
    assert_eq!(
        campos(&segunda).get("blockedBy"),
        Some(&json!([{"uid": "[[X]]", "reltype": "FINISHTOSTART"}]))
    );
}

#[test]
fn bloqueio_preserva_itens_existentes() {
    let b = binding();
    let atual = json!([{"uid": "[[Etapa 3|apelido]]", "reltype": "STARTTOSTART"}, "[[Outra]]"]);
    let t = tarefa("06-tarefas/A.md", json!({"blockedBy": atual.clone()}));
    let existing = ["Nova.md".to_string()];
    let op = block(&b, &t, &["Nova.md".to_string()], &existing).expect("blocks");
    assert_eq!(
        campos(&op).get("blockedBy"),
        Some(&json!([
            {"uid": "[[Etapa 3|apelido]]", "reltype": "STARTTOSTART"},
            "[[Outra]]",
            {"uid": "[[Nova]]", "reltype": "FINISHTOSTART"}
        ]))
    );
}

#[test]
fn bloquear_por_si_mesma_recusa() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({}));
    let err = block(
        &b,
        &t,
        &["06-tarefas/A.md".to_string()],
        &["06-tarefas/A.md".to_string()],
    )
    .expect_err("self block");
    assert_eq!(err.code, "task_cannot_block_itself");
}

#[test]
fn bloqueador_fora_da_colecao_recusa() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({}));
    let err =
        block(&b, &t, &["Fantasma.md".to_string()], &["X.md".to_string()]).expect_err("unknown");
    assert_eq!(err.code, "blocker_not_found");
}

#[test]
fn desbloquear_tira_pelo_nome_da_nota() {
    let b = binding();
    let t = tarefa(
        "06-tarefas/A.md",
        json!({"blockedBy": [{"uid": "[[06-tarefas/X]]", "reltype": "FINISHTOSTART"}, {"uid": "[[Fica]]"}]}),
    );
    let op = unblock(&b, &t, &["X.md".to_string()]).expect("unblocks");
    assert_eq!(
        campos(&op).get("blockedBy"),
        Some(&json!([{"uid": "[[Fica]]"}]))
    );
}

#[test]
fn arquivar_acrescenta_a_tag() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({"tags": ["task", "projeto-x"]}));
    let op = archive(&b, &t).expect("archives");
    assert_eq!(
        campos(&op).get("tags"),
        Some(&json!(["task", "projeto-x", "archived"]))
    );
}

#[test]
fn pular_e_desfazer_ocorrencia() {
    let b = binding();
    let t = tarefa(
        "06-tarefas/A.md",
        json!({"skipped_instances": ["2026-11-01"]}),
    );

    let pulada = skip(&b, &t, "2026-11-08").expect("skips");
    assert_eq!(
        campos(&pulada).get("skipped_instances"),
        Some(&json!(["2026-11-01", "2026-11-08"]))
    );

    let repetida = skip(&b, &t, "2026-11-01").expect("skip twice is harmless");
    assert_eq!(
        campos(&repetida).get("skipped_instances"),
        Some(&json!(["2026-11-01"]))
    );

    let desfeita = unskip(&b, &t, "2026-11-01").expect("unskips");
    assert_eq!(campos(&desfeita).get("skipped_instances"), Some(&json!([])));
}

#[test]
fn skip_recusa_data_mal_formada() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({}));
    let err = skip(&b, &t, "08/11/2026").expect_err("bad date");
    assert_eq!(err.code, "invalid_date");
}

#[test]
fn todo_writeop_traz_if_revision() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({"blockedBy": []}));
    let ops = [
        set_status(&b, &t, "pausa", None, HOJE).expect("status"),
        link_projects(&b, &t, &["P".to_string()]).expect("projects"),
        block(&b, &t, &["X.md".to_string()], &["X.md".to_string()]).expect("block"),
        unblock(&b, &t, &["X.md".to_string()]).expect("unblock"),
        archive(&b, &t).expect("archive"),
        skip(&b, &t, "2026-11-08").expect("skip"),
        unskip(&b, &t, "2026-11-08").expect("unskip"),
    ];
    for op in ops {
        assert_eq!(op["if_revision"], json!("rev-1"));
        assert_eq!(op["path"], json!("06-tarefas/A.md"));
    }
}

#[test]
fn data_de_conclusao_inexistente_recusa() {
    let b = binding();
    let t = tarefa("06-tarefas/A.md", json!({"status": "em-andamento"}));
    for inexistente in ["2026-02-30", "2025-02-29", "2026-04-31", "2026-13-01"] {
        let err = set_status(&b, &t, "concluido", Some(inexistente), HOJE).unwrap_err();
        assert_eq!(err.code, "invalid_completed_date", "{inexistente}");
    }
    let ok = set_status(&b, &t, "concluido", Some("2024-02-29"), HOJE);
    assert!(ok.is_ok(), "2024 is a leap year");
}

#[test]
fn apagar_nota_com_backlink_quebrado_recusa_sem_force() {
    use mdbn_tasks::delete_check;
    let quebrados = vec!["tasks/other.md".to_string()];
    assert_eq!(
        delete_check(&quebrados, true, false).unwrap_err().code,
        "has_backlinks"
    );
    assert!(
        delete_check(&quebrados, true, true).is_ok(),
        "force removes it"
    );
    assert!(delete_check(&[], true, false).is_ok(), "no broken links");
    assert!(
        delete_check(&quebrados, false, false).is_ok(),
        "backlinks not checked"
    );
}
