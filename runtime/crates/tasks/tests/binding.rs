//! Tests for the task type binding and the tasks config.

use mdbn_tasks::{Binding, TaskError, TasksConfig};
use serde_json::{Map, Value, json};

const TASK_TYPE: &str = include_str!("fixtures/task-type.json");

fn task_type() -> Value {
    serde_json::from_str(TASK_TYPE).expect("fixture is valid JSON")
}

fn fm(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

#[test]
fn binding_le_o_tipo_da_biblioteca() {
    let b = Binding::from_type(&task_type()).expect("task type binds");

    assert_eq!(b.type_name, "task");
    assert_eq!(b.field("projects"), "projects");
    assert_eq!(b.field("recurrenceAnchor"), "recurrence_anchor");
    assert_eq!(b.completed, ["concluido", "cancelado"]);
    assert_eq!(b.default_status, "backlog");
    assert!(b.statuses.iter().any(|s| s == "em-andamento"));
    assert_eq!(b.archived_tag, "archived");
    assert_eq!(b.folder.as_deref(), Some("06-tarefas/Tasks"));
    assert_eq!(b.path_template.as_deref(), Some("{{title}}"));
    assert_eq!(b.read_defaults.get("priority"), Some(&json!("nenhuma")));
}

#[test]
fn effective_aplica_padrao_sem_sobrescrever() {
    let b = Binding::from_type(&task_type()).expect("task type binds");

    let vazio = b.effective(&fm(json!({})));
    assert_eq!(vazio.get("status"), Some(&json!("backlog")));
    assert_eq!(vazio.get("priority"), Some(&json!("nenhuma")));

    let pausada = b.effective(&fm(json!({"status": "pausa"})));
    assert_eq!(pausada.get("status"), Some(&json!("pausa")));
    assert_eq!(pausada.get("priority"), Some(&json!("nenhuma")));
}

#[test]
fn is_closed_segue_completed_values() {
    let b = Binding::from_type(&task_type()).expect("task type binds");
    assert!(b.is_closed("concluido"));
    assert!(b.is_closed("cancelado"));
    assert!(!b.is_closed("pausa"));
}

#[test]
fn config_traduz_apelidos_e_recusas() {
    let cfg = TasksConfig::from_json(&json!({
        "required_on_create": ["assignees", "brand", "projects"],
        "people": {
            "arthur": ["[[Arthur Cardoso]]"],
            "labs": ["[[Arthur Cardoso]]", "[[Mayara Almeida]]"]
        },
        "refused_assignees": {"Madame_Labs": "labs"}
    }))
    .expect("config parses");

    assert_eq!(cfg.required_on_create, ["assignees", "brand", "projects"]);
    assert_eq!(
        cfg.people.get("labs"),
        Some(&vec![
            "[[Arthur Cardoso]]".to_string(),
            "[[Mayara Almeida]]".to_string()
        ])
    );
    assert_eq!(
        cfg.refused_assignees.get("Madame_Labs").map(String::as_str),
        Some("labs")
    );
}

#[test]
fn tipo_sem_contrato_falha() {
    let sem_contrato = json!({"name": "task", "implements": []});
    match Binding::from_type(&sem_contrato) {
        Err(TaskError { code, .. }) => assert_eq!(code, "type_without_tasknotes_contract"),
        Ok(_) => panic!("type without tasknotes.task contract must fail"),
    }
}
