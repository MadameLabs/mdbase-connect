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

const AGORA: &str = "2026-11-17T10:00:00.000-03:00";

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
    let op = promote(&b, &TasksConfig::default(), &t, &n, AGORA).expect("promotes");
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
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia"), AGORA).expect("promotes");
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
    let op = promote(&b, &c, &t, &n, AGORA).expect("promotes");
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
    let err =
        promote(&b, &c, &nota(json!({})), &nova("Ideia"), AGORA).expect_err("assignees required");
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
    let err = promote(&b, &c, &nota(json!({})), &n, AGORA).expect_err("refused");
    assert_eq!(err.code, "responsavel_antigo");
}

#[test]
fn promover_documento_tipado_acrescenta_task_em_types() {
    let b = binding();
    let t = nota(json!({"type": "plano", "tags": ["ideia"]}));
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia"), AGORA).expect("promotes");
    assert_eq!(campos(&op).get("types"), Some(&json!(["plano", "task"])));
    assert_eq!(campos(&op).get("tags"), Some(&json!(["ideia", "task"])));
}

#[test]
fn promover_mantem_o_campo_type() {
    let b = binding();
    let t = nota(json!({"type": "plano", "tags": ["ideia"]}));
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia"), AGORA).expect("promotes");
    assert!(!campos(&op).contains_key("type"), "{:?}", campos(&op));
}

#[test]
fn promover_types_existente_acrescenta_task() {
    let b = binding();
    let t = nota(json!({"types": ["plano", "briefing"], "tags": ["ideia"]}));
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia"), AGORA).expect("promotes");
    assert_eq!(
        campos(&op).get("types"),
        Some(&json!(["plano", "briefing", "task"]))
    );

    let ja = nota(json!({"types": ["plano", "task"], "tags": ["task"]}));
    let op = promote(&b, &TasksConfig::default(), &ja, &nova("Ideia"), AGORA).expect("promotes");
    assert!(!campos(&op).contains_key("types"), "{:?}", campos(&op));
}

#[test]
fn promover_sem_tipo_ou_com_type_task_nao_grava_types() {
    let b = binding();
    for frontmatter in [
        json!({"tags": ["ideia"]}),
        json!({"type": "task", "tags": []}),
    ] {
        let op = promote(
            &b,
            &TasksConfig::default(),
            &nota(frontmatter),
            &nova("Ideia"),
            AGORA,
        )
        .expect("promotes");
        assert!(!campos(&op).contains_key("types"), "{:?}", campos(&op));
    }
}

#[test]
fn reparo_de_tarefa_ja_promovida_mantem_status() {
    let b = binding();
    let t = nota(json!({"tags": ["task"], "status": "em-andamento"}));
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia"), AGORA).expect("repairs");
    assert!(!campos(&op).contains_key("status"), "{:?}", campos(&op));
    assert_eq!(campos(&op).get("tags"), Some(&json!(["task"])));
}

fn nota_tipada(frontmatter: Value) -> TaskView {
    let mut t = nota(frontmatter);
    t.frontmatter.insert("tags".to_string(), json!(["ideia"]));
    t
}

#[test]
fn promover_sem_data_de_criacao_usa_created() {
    let b = binding();
    let t = nota_tipada(json!({"type": "plano", "created": "2026-11-16", "data": "2026-11-14"}));
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia"), AGORA).expect("promotes");
    assert_eq!(
        campos(&op).get("dateCreated"),
        Some(&json!("2026-11-16T00:00:00Z"))
    );
}

#[test]
fn promover_sem_data_de_criacao_usa_data() {
    let b = binding();
    let t = nota_tipada(json!({"type": "plano", "data": "2026-11-14"}));
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia"), AGORA).expect("promotes");
    assert_eq!(
        campos(&op).get("dateCreated"),
        Some(&json!("2026-11-14T00:00:00Z"))
    );
}

#[test]
fn promover_sem_data_de_criacao_usa_agora() {
    let b = binding();
    let t = nota_tipada(json!({"type": "plano"}));
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia"), AGORA).expect("promotes");
    assert_eq!(campos(&op).get("dateCreated"), Some(&json!(AGORA)));
}

#[test]
fn promover_nao_sobrescreve_data_de_criacao() {
    let b = binding();
    let t = nota_tipada(json!({
        "type": "plano",
        "dateCreated": "2026-01-01T09:00:00.000-03:00",
        "created": "2026-11-16"
    }));
    let op = promote(&b, &TasksConfig::default(), &t, &nova("Ideia"), AGORA).expect("promotes");
    assert!(
        !campos(&op).contains_key("dateCreated"),
        "{:?}",
        campos(&op)
    );
}
