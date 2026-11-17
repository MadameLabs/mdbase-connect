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

#[test]
fn tipo_sem_implements_falha() {
    let sem_implements = json!({"name": "task"});
    match Binding::from_type(&sem_implements) {
        Err(TaskError { code, .. }) => assert_eq!(code, "type_without_tasknotes_contract"),
        Ok(_) => panic!("type without an implements key must fail"),
    }
}

// Roles: a collection's own field names bound to the contract's roles.

fn roles(v: Value) -> Map<String, Value> {
    v.as_object().expect("object").clone()
}

#[test]
fn papel_sem_tn_role_e_o_proprio_nome_do_campo() {
    let b = mdbn_tasks::Binding::from_roles(&roles(
        json!({"title": {"type": "string"}, "dateCreated": {"type": "datetime"}}),
    ));
    assert_eq!(b.field("title"), "title");
    assert_eq!(b.field("dateCreated"), "dateCreated");
    assert_eq!(b.role_of("title"), Some("title"));
}

#[test]
fn tn_role_troca_o_campo_e_role_of_volta() {
    let b = mdbn_tasks::Binding::from_roles(&roles(
        json!({"myTitle": {"tn_role": "title"}, "myDue": {"tn_role": "due"}}),
    ));
    assert_eq!(b.field("title"), "myTitle");
    assert_eq!(b.field("due"), "myDue");
    assert_eq!(b.role_of("myDue"), Some("due"));
    assert_eq!(b.role_of("vendorField"), None);
}

#[test]
fn concluidos_vem_de_tn_completed_values_ou_do_nome_ou_do_padrao() {
    let explicit = mdbn_tasks::Binding::from_roles(&roles(
        json!({"taskStatus": {"tn_role": "status", "values": ["open", "done", "cancelled"], "tn_completed_values": ["done", "cancelled"]}}),
    ));
    assert_eq!(explicit.completed, vec!["done", "cancelled"]);
    assert!(explicit.is_closed("done"));
    assert!(!explicit.is_closed("open"));

    let inferred = mdbn_tasks::Binding::from_roles(&roles(
        json!({"myStatus": {"tn_role": "status", "values": ["open", "in-progress", "completed", "cancelled"]}}),
    ));
    assert_eq!(inferred.completed, vec!["completed", "cancelled"]);

    let fallback = mdbn_tasks::Binding::from_roles(&roles(
        json!({"state": {"tn_role": "status", "values": ["todo", "doing", "backlog"]}}),
    ));
    assert_eq!(fallback.completed, vec!["done", "cancelled"]);
}

#[test]
fn normalize_e_denormalize_trocam_nome_de_campo_por_papel() {
    let b = mdbn_tasks::Binding::from_roles(&roles(
        json!({"myTitle": {"tn_role": "title"}, "myCreated": {"tn_role": "dateCreated"}}),
    ));
    let normalized = b.normalize(&roles(
        json!({"myTitle": "Plan", "myCreated": "2026-02-19T10:00:00Z", "vendorField": "ZX-42"}),
    ));
    assert_eq!(normalized["title"], "Plan");
    assert_eq!(normalized["dateCreated"], "2026-02-19T10:00:00Z");
    assert_eq!(normalized["vendorField"], "ZX-42", "unmapped keys stay");
    let back = b.denormalize(&normalized);
    assert_eq!(back["myTitle"], "Plan");
    assert_eq!(back["myCreated"], "2026-02-19T10:00:00Z");
    assert_eq!(back["vendorField"], "ZX-42");
}

#[test]
fn titulo_de_exibicao_cai_para_titulo_depois_para_o_nome_do_arquivo() {
    let second =
        mdbn_tasks::Binding::from_roles(&roles(json!({"secondaryTitle": {"tn_role": "title"}})));
    let custom =
        mdbn_tasks::Binding::from_roles(&roles(json!({"customTitle": {"tn_role": "title"}})));
    let fm = |v: Value| roles(v);
    assert_eq!(
        second
            .display_title(
                &fm(json!({"title": "Primary", "secondaryTitle": "Fallback"})),
                "secondaryTitle",
                Some("tasks/FromPath.md")
            )
            .as_deref(),
        Some("Fallback")
    );
    assert_eq!(
        custom
            .display_title(
                &fm(json!({"title": "Title fallback"})),
                "customTitle",
                Some("tasks/FromPath.md")
            )
            .as_deref(),
        Some("Title fallback")
    );
    assert_eq!(
        custom
            .display_title(&fm(json!({})), "customTitle", Some("tasks/FromPath.md"))
            .as_deref(),
        Some("FromPath")
    );
    assert_eq!(
        custom.display_title(&fm(json!({})), "customTitle", Some("")),
        None
    );
    assert_eq!(
        custom
            .display_title(
                &fm(json!({"title": "", "customTitle": ""})),
                "customTitle",
                Some("tasks/FallbackOnly.md")
            )
            .as_deref(),
        Some("FallbackOnly")
    );
}

#[test]
fn dois_campos_com_o_mesmo_papel_o_primeiro_vence() {
    let b = mdbn_tasks::Binding::from_roles(&roles(
        json!({"zTitle": {"tn_role": "title"}, "aTitle": {"tn_role": "title"}}),
    ));
    assert_eq!(b.field("title"), "aTitle");
}
