//! Tests for creating and promoting tasks under the collection's rules, and for
//! reading person and brand names into assignees.

use mdbn_tasks::{
    Binding, NewTask, TaskView, TasksConfig, create, path_for, promote, resolve_person,
};
use serde_json::{Map, Value, json};

const TASK_TYPE: &str = include_str!("fixtures/task-type.json");

fn binding() -> Binding {
    let def: Value = serde_json::from_str(TASK_TYPE).expect("fixture is valid JSON");
    Binding::from_type(&def).expect("task type binds")
}

fn config(v: Value) -> TasksConfig {
    TasksConfig::from_json(&v).expect("config parses")
}

fn nova(title: &str) -> NewTask {
    NewTask {
        title: title.to_string(),
        assignees: Vec::new(),
        brand: None,
        projects: Vec::new(),
    }
}

fn fm(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

fn nota(frontmatter: Value) -> TaskView {
    TaskView {
        path: "01-planejamento/Ideia.md".to_string(),
        revision: "rev-1".to_string(),
        frontmatter: fm(frontmatter),
    }
}

fn campos(op: &Value) -> &Map<String, Value> {
    op["fields"].as_object().expect("fields object")
}

#[test]
fn caminho_vem_do_tipo() {
    let b = binding();
    assert_eq!(path_for(&b, "Revisar X"), "06-tarefas/Tasks/Revisar X.md");
}

#[test]
fn caminho_troca_caractere_proibido() {
    let b = binding();
    let caminho = path_for(&b, "A/B: C");
    let nome = caminho
        .strip_prefix("06-tarefas/Tasks/")
        .expect("file sits in the type folder");
    assert!(!nome.contains('/'), "{nome}");
    assert!(!nome.contains(':'), "{nome}");
    assert_eq!(caminho, "06-tarefas/Tasks/AB C.md");
}

#[test]
fn criar_exige_os_obrigatorios_da_config() {
    let b = binding();
    let c = config(json!({"required_on_create": ["assignees", "projects"]}));
    let mut n = nova("Revisar X");
    n.assignees = vec!["labs".to_string()];
    let err = create(&b, &c, &n).expect_err("projects is required");
    assert_eq!(err.code, "missing_required_field");
    assert!(err.message.contains("projects"), "{}", err.message);
}

#[test]
fn sem_config_nada_e_obrigatorio() {
    let b = binding();
    let op = create(&b, &TasksConfig::default(), &nova("Revisar X")).expect("title is enough");
    assert_eq!(op["op"], json!("create"));
    assert_eq!(op["type"], json!("task"));
    assert_eq!(op["path"], json!("06-tarefas/Tasks/Revisar X.md"));
    assert!(!campos(&op).contains_key("assignees"));
}

#[test]
fn apelido_vira_pessoa() {
    let c = config(json!({"people": {"labs": ["[[Arthur Cardoso]]", "[[Mayara Almeida]]"]}}));
    assert_eq!(
        resolve_person(&c, "labs").expect("alias resolves"),
        ["[[Arthur Cardoso]]", "[[Mayara Almeida]]"]
    );
}

#[test]
fn marca_como_responsavel_recusa() {
    let c = config(json!({
        "people": {"labs": ["[[Arthur Cardoso]]", "[[Mayara Almeida]]"]},
        "refused_assignees": {"Madame_Labs": "labs"}
    }));
    let err = resolve_person(&c, "Madame_Labs").expect_err("brand name is refused");
    assert_eq!(err.code, "responsavel_antigo");
    assert!(err.message.contains("labs"), "{}", err.message);
}

#[test]
fn criar_nasce_no_status_inicial_e_com_tag() {
    let b = binding();
    let com = config(json!({"initial_status": "to-do"}));
    let op = create(&b, &com, &nova("Revisar X")).expect("creates");
    assert_eq!(campos(&op).get("status"), Some(&json!("to-do")));
    assert_eq!(campos(&op).get("tags"), Some(&json!(["task"])));

    let op = create(&b, &TasksConfig::default(), &nova("Revisar X")).expect("creates");
    assert_eq!(campos(&op).get("status"), Some(&json!("backlog")));
    assert_eq!(campos(&op).get("tags"), Some(&json!(["task"])));
}

#[test]
fn promover_acrescenta_task_sem_perder_tags() {
    let b = binding();
    let n = nova("Ideia");
    let t = nota(json!({"tags": ["ideia"]}));
    let op = promote(&b, &TasksConfig::default(), &t, &n).expect("promotes");
    assert_eq!(op["op"], json!("update"));
    assert_eq!(op["path"], json!("01-planejamento/Ideia.md"));
    assert_eq!(op["if_revision"], json!("rev-1"));
    assert_eq!(campos(&op).get("tags"), Some(&json!(["ideia", "task"])));
    assert_eq!(campos(&op).get("status"), Some(&json!("backlog")));
}

#[test]
fn pessoa_sem_config_vira_wikilink() {
    assert_eq!(
        resolve_person(&TasksConfig::default(), "Lucilene").expect("used as given"),
        ["[[Lucilene]]"]
    );
    assert_eq!(
        resolve_person(&TasksConfig::default(), "[[Lucilene]]").expect("used as given"),
        ["[[Lucilene]]"]
    );
}

#[test]
fn pessoa_vazia_recusa() {
    let err = resolve_person(&TasksConfig::default(), "   ").expect_err("empty name");
    assert_eq!(err.code, "responsavel_desconhecido");
}

#[test]
fn criar_nao_grava_campos_do_ciclo_de_vida() {
    let b = binding();
    let op = create(&b, &TasksConfig::default(), &nova("Revisar X")).expect("creates");
    for proibido in ["id", "dateCreated", "dateModified"] {
        assert!(!campos(&op).contains_key(proibido), "{proibido}");
    }
}

#[test]
fn criar_grava_pessoa_marca_e_projeto_como_wikilink() {
    let b = binding();
    let c = config(json!({"people": {"labs": ["[[Arthur Cardoso]]", "[[Mayara Almeida]]"]}}));
    let n = NewTask {
        title: "Revisar X".to_string(),
        assignees: vec!["labs".to_string()],
        brand: Some("Madame Labs".to_string()),
        projects: vec!["mdbase-tasks".to_string(), "[[mdbase-tasks]]".to_string()],
    };
    let op = create(&b, &c, &n).expect("creates");
    assert_eq!(
        campos(&op).get("assignees"),
        Some(&json!(["[[Arthur Cardoso]]", "[[Mayara Almeida]]"]))
    );
    assert_eq!(campos(&op).get("brand"), Some(&json!(["[[Madame Labs]]"])));
    assert_eq!(
        campos(&op).get("projects"),
        Some(&json!(["[[mdbase-tasks]]"]))
    );
}

#[test]
fn criar_recusa_status_inicial_fora_do_tipo() {
    let b = binding();
    let c = config(json!({"initial_status": "inventado"}));
    let err = create(&b, &c, &nova("Revisar X")).expect_err("unknown status");
    assert_eq!(err.code, "invalid_status");
}

#[test]
fn criar_recusa_titulo_vazio() {
    let b = binding();
    let err = create(&b, &TasksConfig::default(), &nova("  ")).expect_err("empty title");
    assert_eq!(err.code, "missing_title");
}

#[test]
fn promover_nao_duplica_task() {
    let b = binding();
    let t = nota(json!({"tags": ["task", "ideia"]}));
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia")).expect("promotes");
    assert_eq!(campos(&op).get("tags"), Some(&json!(["task", "ideia"])));
}

#[test]
fn promover_mantem_pessoas_e_projetos_da_nota() {
    let b = binding();
    let c = config(json!({"people": {"arthur": ["[[Arthur Cardoso]]"]}}));
    let t = nota(json!({"assignees": ["[[Mayara Almeida]]"], "projects": ["[[Projeto]]"]}));
    let n = NewTask {
        title: "Ideia".to_string(),
        assignees: vec!["arthur".to_string()],
        brand: None,
        projects: vec!["Projeto".to_string(), "Outro".to_string()],
    };
    let op = promote(&b, &c, &t, &n).expect("promotes");
    assert_eq!(
        campos(&op).get("assignees"),
        Some(&json!(["[[Mayara Almeida]]", "[[Arthur Cardoso]]"]))
    );
    assert_eq!(
        campos(&op).get("projects"),
        Some(&json!(["[[Projeto]]", "[[Outro]]"]))
    );
}

#[test]
fn promover_exige_os_obrigatorios_da_config() {
    let b = binding();
    let c = config(json!({"required_on_create": ["assignees"]}));
    let err = promote(&b, &c, &nota(json!({})), &nova("Ideia")).expect_err("assignees required");
    assert_eq!(err.code, "missing_required_field");
    assert!(err.message.contains("assignees"), "{}", err.message);
}

#[test]
fn promover_recusa_responsavel_antigo() {
    let b = binding();
    let c = config(json!({
        "people": {"labs": ["[[Arthur Cardoso]]", "[[Mayara Almeida]]"]},
        "refused_assignees": {"Madame_Labs": "labs"}
    }));
    let n = NewTask {
        title: "Ideia".to_string(),
        assignees: vec!["Madame_Labs".to_string()],
        brand: None,
        projects: Vec::new(),
    };
    let err = promote(&b, &c, &nota(json!({})), &n).expect_err("refused");
    assert_eq!(err.code, "responsavel_antigo");
}
