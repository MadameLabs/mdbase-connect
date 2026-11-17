//! Tests for the person's timer on a task: start, stop and retime of time entries.

use mdbn_tasks::{Binding, TaskView, open_entries, retime, start, stop};
use serde_json::{Map, Value, json};

const TASK_TYPE: &str = include_str!("fixtures/task-type.json");
const AGORA: &str = "2026-11-16T10:00:00.000Z";
const HOJE: &str = "2026-11-16";
const ARTHUR: &str = "[[Arthur Cardoso]]";
const MAYARA: &str = "[[Mayara Almeida]]";

fn binding() -> Binding {
    let def: Value = serde_json::from_str(TASK_TYPE).expect("fixture is valid JSON");
    Binding::from_type(&def).expect("task type binds")
}

fn fm(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn tarefa(frontmatter: Value) -> TaskView {
    TaskView {
        path: "06-tarefas/A.md".to_string(),
        revision: "rev-1".to_string(),
        frontmatter: fm(frontmatter),
    }
}

fn campos(op: &Value) -> &Map<String, Value> {
    assert_eq!(op["op"], json!("update"));
    assert_eq!(op["if_revision"], json!("rev-1"));
    op["fields"].as_object().expect("fields object")
}

fn unica_operacao(ops: &[Value]) -> &Value {
    assert_eq!(ops.len(), 1, "one update op with status and entries");
    &ops[0]
}

#[test]
fn start_abre_entrada_e_poe_em_andamento() {
    let b = binding();
    let t = tarefa(json!({"status": "pausa", "timeEntries": []}));
    let ops = start(&b, &t, ARTHUR, AGORA).expect("starts");
    let op = unica_operacao(&ops);
    assert_eq!(campos(op).get("status"), Some(&json!("em-andamento")));
    assert_eq!(
        campos(op).get("timeEntries"),
        Some(&json!([{"startTime": AGORA, "description": "pessoa: [[Arthur Cardoso]]"}]))
    );
}

#[test]
fn start_repetido_nao_abre_outra() {
    let b = binding();
    let t = tarefa(json!({
        "status": "em-andamento",
        "timeEntries": [{"startTime": "2026-11-16T09:00:00.000Z", "description": "pessoa: [[Arthur Cardoso]]"}]
    }));
    let ops = start(&b, &t, ARTHUR, AGORA).expect("no error");
    assert!(ops.is_empty());
}

#[test]
fn start_de_outra_pessoa_abre_a_sua() {
    let b = binding();
    let t = tarefa(json!({
        "timeEntries": [{"startTime": "2026-11-16T09:00:00.000Z", "description": "pessoa: [[Arthur Cardoso]]"}]
    }));
    let ops = start(&b, &t, MAYARA, AGORA).expect("starts");
    let lista = campos(unica_operacao(&ops))
        .get("timeEntries")
        .cloned()
        .expect("entries");
    assert_eq!(lista.as_array().map(Vec::len), Some(2));
}

#[test]
fn start_sem_pessoa_recusa() {
    let b = binding();
    let err = start(&b, &tarefa(json!({})), "", AGORA).expect_err("no person");
    assert_eq!(err.code, "missing_person");
}

#[test]
fn stop_exige_status_valido() {
    let b = binding();
    let t = tarefa(json!({}));
    for ruim in ["", "ativo"] {
        let err = stop(&b, &t, ARTHUR, ruim, AGORA, HOJE).expect_err(ruim);
        assert_eq!(err.code, "missing_or_invalid_status", "{ruim}");
    }
}

#[test]
fn stop_fecha_so_a_da_pessoa() {
    let b = binding();
    let t = tarefa(json!({
        "status": "em-andamento",
        "timeEntries": [
            {"startTime": "2026-11-16T09:00:00.000Z", "description": "pessoa: [[Arthur Cardoso]]"},
            {"startTime": "2026-11-16T09:30:00.000Z", "description": "pessoa: [[Mayara Almeida]]"}
        ]
    }));
    let ops = stop(&b, &t, ARTHUR, "pausa", AGORA, HOJE).expect("stops");
    let fields = campos(unica_operacao(&ops));
    assert_eq!(fields.get("status"), Some(&json!("pausa")));
    assert_eq!(
        fields.get("timeEntries"),
        Some(&json!([
            {"startTime": "2026-11-16T09:00:00.000Z", "description": "pessoa: [[Arthur Cardoso]]", "endTime": AGORA},
            {"startTime": "2026-11-16T09:30:00.000Z", "description": "pessoa: [[Mayara Almeida]]"}
        ]))
    );
}

#[test]
fn stop_sem_entrada_aberta_so_grava_status() {
    let b = binding();
    let t = tarefa(json!({
        "timeEntries": [{"startTime": "2026-11-16T09:00:00.000Z", "endTime": "2026-11-16T09:10:00.000Z"}]
    }));
    let ops = stop(&b, &t, ARTHUR, "pausa", AGORA, HOJE).expect("no error");
    let fields = campos(unica_operacao(&ops));
    assert_eq!(fields.get("status"), Some(&json!("pausa")));
    assert!(!fields.contains_key("timeEntries"));
}

#[test]
fn stop_fechando_grava_data() {
    let b = binding();
    let t = tarefa(json!({"status": "em-andamento"}));
    let ops = stop(&b, &t, ARTHUR, "concluido", AGORA, HOJE).expect("stops");
    let fields = campos(unica_operacao(&ops));
    assert_eq!(fields.get("status"), Some(&json!("concluido")));
    assert_eq!(fields.get("completedDate"), Some(&json!(HOJE)));
}

#[test]
fn entrada_sem_dono_so_para_quem_nao_tem_a_sua() {
    let b = binding();
    let antiga = json!({"startTime": "2026-11-16T09:00:00.000Z"});

    // Arthur has no entry of his own: he closes the ownerless one.
    let t = tarefa(json!({"timeEntries": [antiga.clone()]}));
    let ops = stop(&b, &t, ARTHUR, "pausa", AGORA, HOJE).expect("stops");
    let fechada = campos(unica_operacao(&ops))
        .get("timeEntries")
        .cloned()
        .expect("entries");
    assert_eq!(fechada[0]["endTime"], json!(AGORA));

    // Mayara has her own open entry: the ownerless one stays open.
    let t = tarefa(json!({"timeEntries": [
        antiga.clone(),
        {"startTime": "2026-11-16T09:30:00.000Z", "description": "pessoa: [[Mayara Almeida]]"}
    ]}));
    let ops = stop(&b, &t, MAYARA, "pausa", AGORA, HOJE).expect("stops");
    let lista = campos(unica_operacao(&ops))
        .get("timeEntries")
        .cloned()
        .expect("entries");
    assert!(lista[0].get("endTime").is_none());
    assert_eq!(lista[1]["endTime"], json!(AGORA));
}

#[test]
fn open_entries_mostra_quem_esta_correndo() {
    let b = binding();
    let t = tarefa(json!({"timeEntries": [
        {"startTime": "2026-11-16T09:00:00.000Z", "endTime": "2026-11-16T09:10:00.000Z", "description": "pessoa: [[Arthur Cardoso]]"},
        {"startTime": "2026-11-16T09:20:00.000Z", "description": "pessoa: [[Mayara Almeida]]"},
        {"startTime": "2026-11-16T09:30:00.000Z"}
    ]}));
    let abertas = open_entries(&b, &t);
    assert_eq!(abertas.len(), 2);
    assert_eq!(abertas[0].person, "[[Mayara Almeida]]");
    assert_eq!(abertas[0].start, "2026-11-16T09:20:00.000Z");
    assert_eq!(abertas[1].person, "");
}

#[test]
fn retime_move_o_fim_da_entrada_pelo_inicio_gravado() {
    let b = binding();
    let t = tarefa(json!({"timeEntries": [
        {"startTime": "2026-10-12T10:00:00.000Z", "endTime": "2026-10-12T11:00:00.000Z"},
        {"startTime": "2026-10-12T17:57:02.436Z", "endTime": "2026-10-13T10:42:50.699Z", "duration": 999}
    ]}));
    let op = retime(
        &b,
        &t,
        "2026-10-12T17:57:02.436Z",
        None,
        Some("2026-10-12T18:07:08.457Z"),
        "sessão morreu",
        AGORA,
    )
    .expect("retimes");
    let lista = campos(&op).get("timeEntries").cloned().expect("entries");
    assert_eq!(
        lista[0],
        json!({"startTime": "2026-10-12T10:00:00.000Z", "endTime": "2026-10-12T11:00:00.000Z"})
    );
    assert_eq!(
        lista[1],
        json!({"startTime": "2026-10-12T17:57:02.436Z", "endTime": "2026-10-12T18:07:08.457Z"})
    );
}

#[test]
fn retime_exige_motivo() {
    let b = binding();
    let t = tarefa(json!({"timeEntries": [
        {"startTime": "2026-10-12T10:00:00.000Z", "endTime": "2026-10-12T11:00:00.000Z"}
    ]}));
    let err = retime(
        &b,
        &t,
        "2026-10-12T10:00:00.000Z",
        None,
        Some("2026-10-12T10:30:00.000Z"),
        "  ",
        AGORA,
    )
    .expect_err("no reason");
    assert_eq!(err.code, "missing_reason");
}

#[test]
fn retime_recusa_o_que_mentiria_sobre_o_historico() {
    let b = binding();
    let t = tarefa(json!({"timeEntries": [
        {"startTime": "2026-10-12T10:00:00.000Z", "endTime": "2026-10-12T11:00:00.000Z"},
        {"startTime": "2026-10-12T17:57:02.436Z", "endTime": "2026-10-13T10:42:50.699Z"}
    ]}));
    let motivo = "correção";
    let casos: [(&str, Option<&str>, Option<&str>, &str); 6] = [
        (
            "2026-10-12T09:00:00.000Z",
            None,
            Some("2026-10-12T09:30:00.000Z"),
            "time_entry_not_found",
        ),
        (
            "2026-10-12T17:57:02.436Z",
            None,
            Some("2026-10-12T17:00:00.000Z"),
            "end_not_after_start",
        ),
        (
            "2026-10-12T17:57:02.436Z",
            Some("2026-10-12T10:30:00.000Z"),
            None,
            "time_entry_overlap",
        ),
        (
            "2026-10-12T17:57:02.436Z",
            None,
            Some("2999-01-01T00:00:00.000Z"),
            "end_in_future",
        ),
        ("2026-10-12T17:57:02.436Z", None, None, "missing_new_time"),
        (
            "",
            None,
            Some("2026-10-12T18:00:00.000Z"),
            "missing_entry_start_time",
        ),
    ];
    for (entrada, novo_inicio, novo_fim, codigo) in casos {
        let err = retime(&b, &t, entrada, novo_inicio, novo_fim, motivo, AGORA).expect_err(codigo);
        assert_eq!(err.code, codigo);
    }
}

#[test]
fn retime_recusa_entrada_ainda_aberta() {
    let b = binding();
    let t = tarefa(json!({"timeEntries": [{"startTime": "2026-10-12T10:00:00.000Z"}]}));
    let err = retime(
        &b,
        &t,
        "2026-10-12T10:00:00.000Z",
        Some("2026-10-12T09:00:00.000Z"),
        None,
        "motivo",
        AGORA,
    )
    .expect_err("still open");
    assert_eq!(err.code, "entry_still_open");
}

#[test]
fn retime_normaliza_o_horario_para_utc_com_milissegundos() {
    let b = binding();
    let t = tarefa(json!({"timeEntries": [
        {"startTime": "2026-10-12T05:00:00.000Z", "endTime": "2026-10-12T05:30:00.000Z"}
    ]}));
    let op = retime(
        &b,
        &t,
        "2026-10-12T05:00:00.000Z",
        None,
        Some("2026-10-12T08:30:15+02:00"),
        "motivo",
        AGORA,
    )
    .expect("retimes");
    let lista = campos(&op).get("timeEntries").cloned().expect("entries");
    assert_eq!(lista[0]["endTime"], json!("2026-10-12T06:30:15.000Z"));
}

#[test]
fn retime_recusa_horario_que_nao_e_iso() {
    let b = binding();
    let t = tarefa(json!({"timeEntries": [
        {"startTime": "2026-10-12T05:00:00.000Z", "endTime": "2026-10-12T05:30:00.000Z"}
    ]}));
    let entrada = "2026-10-12T05:00:00.000Z";
    for ruim in [
        "2026-02-30T05:00:00Z",
        "2026-10-12 05:00",
        "2026-10-12T25:00:00Z",
        "hoje",
    ] {
        let err = retime(&b, &t, entrada, None, Some(ruim), "motivo", AGORA).expect_err(ruim);
        assert_eq!(err.code, "invalid_new_end", "{ruim}");
    }
    let err = retime(
        &b,
        &t,
        entrada,
        None,
        Some("2026-10-12T05:10:00+24:00"),
        "motivo",
        AGORA,
    )
    .expect_err("offset out of range");
    assert_eq!(err.code, "invalid_new_end");
}
