//! `target_type: any` em `collection.links`: o TaskNotes grava esse valor em `projects[]` e
//! `occurrence_template` querendo dizer "qualquer tipo", mas a spec não define curinga e o
//! validador o comparava como nome de tipo, acusando todo link (`link_target_type_mismatch`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mdbn_core::ids::Uuid;
use mdbn_core::state::MemState;
use mdbn_core::validate;

fn colecao(alvo: &str) -> MemState {
    let mut state = MemState::new();
    state.insert_resource("mdbase.yaml", "spec_version: '0.3.0'\n");
    state.insert_resource(
        "_types/tarefa.md",
        &format!(
            "---\nkind: mdbase.type\nname: tarefa\nmatch:\n  expr: \"file.path.startsWith('t/')\"\nschema:\n  dialect: json-schema-2020-12\n  value: {{type: object}}\ncollection:\n  links:\n    projects[]:\n      target_type: {alvo}\n      validate_exists: false\n---\n"
        ),
    );
    state.insert_resource(
        "_types/projeto.md",
        "---\nkind: mdbase.type\nname: projeto\nmatch:\n  expr: \"file.path.startsWith('p/')\"\nschema:\n  dialect: json-schema-2020-12\n  value: {type: object}\n---\n",
    );
    state.insert_record(Uuid([1; 16]), "t/a.md", "---\nprojects: [\"[[x]]\"]\n---\n");
    state.insert_record(Uuid([2; 16]), "p/x.md", "---\ntitle: X\n---\n");
    state
}

fn divergencias(alvo: &str) -> usize {
    validate::cross_record_issues(&colecao(alvo), Uuid([1; 16]))
        .iter()
        .filter(|i| i.code == "link_target_type_mismatch")
        .count()
}

#[test]
fn target_type_any_aceita_qualquer_tipo() {
    assert_eq!(divergencias("any"), 0);
}

#[test]
fn target_type_de_verdade_continua_conferido() {
    assert_eq!(divergencias("projeto"), 0);
    assert_eq!(divergencias("pessoa"), 1);
}
