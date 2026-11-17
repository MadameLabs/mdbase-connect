//! The operations the conformance adapter reports on, in the shapes the
//! tasknotes-spec suite gives them (`ADAPTER_CONTRACT.md`). Cases are copied from
//! the suite's fixtures; the suite itself runs through the adapter.

use mdbn_tasks::conformance::{self, OpError};
use serde_json::{Value, json};

const TODAY: &str = "2026-11-16";

fn run(op: &str, input: Value) -> Result<Value, OpError> {
    conformance::execute(op, &input, TODAY)
}

fn value(op: &str, input: Value) -> Value {
    run(op, input).unwrap_or_else(|e| panic!("{op}: {e:?}"))
}

fn code(op: &str, input: Value) -> &'static str {
    match run(op, input) {
        Err(e) => e.code,
        Ok(v) => panic!("{op} should fail, got {v}"),
    }
}

// meta

#[test]
fn meta_has_capability_nenhuma_capacidade_e_reclamada() {
    for cap in [
        "dependencies",
        "reminders",
        "time-tracking",
        "archive",
        "templating",
    ] {
        assert_eq!(
            value("meta.has_capability", json!({"capability": cap})),
            json!({"value": false}),
            "{cap}"
        );
    }
}

#[test]
fn meta_has_profile_diz_se_o_perfil_e_reclamado() {
    assert_eq!(
        value("meta.has_profile", json!({"profile": "core-lite"})),
        json!({"value": true})
    );
    assert_eq!(
        value("meta.has_profile", json!({"profile": "recurrence"})),
        json!({"value": false})
    );
    assert_eq!(
        value("meta.has_profile", json!({"profile": "extended"})),
        json!({"value": false})
    );
}

#[test]
fn meta_has_sem_o_nome_do_argumento_e_entrada_invalida() {
    assert_eq!(code("meta.has_capability", json!({})), "invalid_input");
    assert_eq!(
        code("meta.has_profile", json!({"profile": 3})),
        "invalid_input"
    );
}

// date: a date is YYYY-MM-DD, with the calendar's days; a datetime carries an
// offset (Z or +HH:MM); dates are compared as written, but parse_utc and parse_local
// read the datetime's instant in UTC.

#[test]
fn date_validate_aceita_so_data_de_calendario() {
    assert_eq!(
        value("date.validate", json!({"value": "1999-01-01"})),
        json!({"value": "1999-01-01"})
    );
    assert_eq!(
        value("date.validate", json!({"value": "2024-02-29"})),
        json!({"value": "2024-02-29"})
    );
    assert_eq!(
        value("date.validate", json!({"value": "1900-02-28"})),
        json!({"value": "1900-02-28"})
    );
    for bad in [
        "",
        " ",
        "-2026-02-20",
        "26-02-01",
        "2026-2-1",
        "2026-00-01",
        "2026-13-01",
        "2026-01-00",
        "2026-01-32",
        "2026-02-29",
        "2025-02-29",
        "1900-02-29",
        "2026/02/01",
        "not-a-date",
        "2026-02-20T23:60:00Z",
        "2026-02-20T23:59:60Z",
    ] {
        assert_eq!(
            code("date.validate", json!({"value": bad})),
            "invalid_date",
            "{bad:?}"
        );
    }
}

#[test]
fn date_parse_utc_e_parse_local_leem_a_data_e_convertem_o_instante_para_utc() {
    assert_eq!(
        value("date.parse_utc", json!({"value": "2024-03-01"})),
        json!({"date": "2024-03-01"})
    );
    assert_eq!(
        value("date.parse_local", json!({"value": "2018-07-05"})),
        json!({"localDate": "2018-07-05"})
    );
    // 2028-06-30T18:30 at -09:00 is 2028-07-01 in UTC.
    assert_eq!(
        value(
            "date.parse_utc",
            json!({"value": "2028-06-30T18:30:00-09:00"})
        ),
        json!({"date": "2028-07-01"})
    );
    assert_eq!(
        value(
            "date.parse_local",
            json!({"value": "2026-02-19T23:00:00-01:00"})
        ),
        json!({"isoDate": "2026-02-20"})
    );
    assert_eq!(
        value(
            "date.parse_utc",
            json!({"value": "2030-01-01T10:00:00.123Z"})
        ),
        json!({"date": "2030-01-01"})
    );
    assert_eq!(
        value(
            "date.parse_utc",
            json!({"value": "2044-08-21T14:15:16+03:00"})
        ),
        json!({"date": "2044-08-21"})
    );
    for bad in ["", "not-a-date", "2026-02-30", "2026-02-20T23:60:00Z"] {
        assert_eq!(
            code("date.parse_utc", json!({"value": bad})),
            "invalid_date",
            "{bad:?}"
        );
        assert_eq!(
            code("date.parse_local", json!({"value": bad})),
            "invalid_date",
            "{bad:?}"
        );
    }
}

#[test]
fn date_get_part_devolve_a_data_como_escrita() {
    assert_eq!(
        value("date.get_part", json!({"value": "2026-02-20"})),
        json!({"value": "2026-02-20"})
    );
    assert_eq!(
        value(
            "date.get_part",
            json!({"value": "2026-02-20T12:34:56+05:30"})
        ),
        json!({"value": "2026-02-20"})
    );
    assert_eq!(
        value("date.get_part", json!({"value": "2026-02-20T00:00:00Z"})),
        json!({"value": "2026-02-20"})
    );
}

#[test]
fn date_has_time_procura_hora_hh_mm_depois_do_t() {
    for (s, want) in [
        ("", false),
        ("2026-02-20", false),
        ("2026-02-20 10:00:00", false),
        ("2026-02-20T10", false),
        ("2026-02-20T10:0", false),
        ("2026-02-20T1:00", false),
        ("2026-02-20t10:00", false),
        ("no-time-component", false),
        ("2026-02-20T10:00", true),
        ("2026-02-20T10:00:00Z", true),
        ("2026-02-20T99:99", true),
        ("2026-02-20TT10:00", true),
        ("T10:00", true),
        ("textT10:00", true),
    ] {
        assert_eq!(
            value("date.has_time", json!({"value": s})),
            json!({"value": want}),
            "{s:?}"
        );
    }
}

#[test]
fn date_is_same_e_is_before_comparam_a_data_escrita() {
    let same = |a: &str, b: &str| value("date.is_same", json!({"a": a, "b": b}));
    assert_eq!(same("2026-02-20", "2026-02-20"), json!({"value": true}));
    assert_eq!(
        same("2026-02-20T00:00:00Z", "2026-02-20"),
        json!({"value": true})
    );
    assert_eq!(
        same("2028-06-30T18:30:00-09:00", "2028-06-30"),
        json!({"value": true})
    );
    assert_eq!(
        same("1999-12-31T23:00:00-01:00", "2000-01-01"),
        json!({"value": false})
    );
    assert_eq!(same("invalid", "2026-02-20"), json!({"value": false}));
    assert_eq!(same("", "2026-02-20"), json!({"value": false}));

    let before = |a: &str, b: &str| value("date.is_before", json!({"a": a, "b": b}));
    assert_eq!(before("2026-02-19", "2026-02-20"), json!({"value": true}));
    assert_eq!(
        before("2026-02-20T23:59:59Z", "2026-02-20"),
        json!({"value": false})
    );
    assert_eq!(
        before("2026-02-20", "2026-02-20T00:00:00Z"),
        json!({"value": false})
    );
    assert_eq!(before("2024-02-29", "2024-03-01"), json!({"value": true}));
    assert_eq!(before("invalid", "2026-02-20"), json!({"value": false}));
}

#[test]
fn date_resolve_operation_target_prefere_explicit_depois_scheduled_depois_due() {
    let target = |input: Value| value("date.resolve_operation_target", input);
    assert_eq!(
        target(json!({"explicitDate": "2026-02-20"})),
        json!({"value": "2026-02-20"})
    );
    assert_eq!(
        target(
            json!({"explicitDate": "2024-02-29", "scheduled": "2026-01-01", "due": "2026-01-02"})
        ),
        json!({"value": "2024-02-29"})
    );
    assert_eq!(
        target(json!({"scheduled": "2026-03-10T09:00:00Z"})),
        json!({"value": "2026-03-10"})
    );
    assert_eq!(
        target(json!({"scheduled": "bad", "due": "2026-04-01"})),
        json!({"value": "2026-04-01"})
    );
    assert_eq!(
        target(json!({"scheduled": "2023-02-29", "due": "2024-03-01"})),
        json!({"value": "2024-03-01"})
    );
    assert_eq!(
        target(json!({"scheduled": "2026-11-05T23:59:59-08:00"})),
        json!({"value": "2026-11-05"})
    );
    // Nothing given: today, from the clock.
    assert_eq!(target(json!({})), json!({"value": TODAY}));
    assert_eq!(
        target(json!({"scheduled": " ", "due": " "})),
        json!({"value": TODAY})
    );
    assert_eq!(
        code(
            "date.resolve_operation_target",
            json!({"explicitDate": "2026-02-30"})
        ),
        "invalid_date"
    );
}

// op and delete

#[test]
fn op_update_patch_funde_o_patch_e_diz_se_mudou() {
    let r = value(
        "op.update_patch",
        json!({"original": {"title": "W", "status": "open", "vendorTicket": "ZX-42"}, "patch": {"status": "open"}}),
    );
    assert_eq!(
        r,
        json!({"changed": false, "frontmatter": {"title": "W", "status": "open", "vendorTicket": "ZX-42"}})
    );
    let r = value(
        "op.update_patch",
        json!({"original": {"title": "W", "status": "open"}, "patch": {"status": "in-progress"}}),
    );
    assert_eq!(
        r,
        json!({"changed": true, "frontmatter": {"title": "W", "status": "in-progress"}})
    );
}

#[test]
fn op_complete_nonrecurring_fecha_no_primeiro_status_fechado_com_a_data() {
    let r = value(
        "op.complete_nonrecurring",
        json!({"frontmatter": {"title": "B", "status": "open"}, "completedValues": ["done", "cancelled"], "explicitDate": "2026-02-20"}),
    );
    assert_eq!(r, json!({"status": "done", "completedDate": "2026-02-20"}));
    let r = value(
        "op.complete_nonrecurring",
        json!({"frontmatter": {"title": "B", "status": "open"}, "completedValues": ["cancelled", "done"], "explicitDate": "2026-02-20"}),
    );
    assert_eq!(
        r,
        json!({"status": "cancelled", "completedDate": "2026-02-20"})
    );
}

#[test]
fn op_uncomplete_nonrecurring_volta_ao_status_padrao() {
    let r = value(
        "op.uncomplete_nonrecurring",
        json!({"frontmatter": {"status": "done", "completedDate": "2026-02-20"}, "defaultStatus": "open", "clearCompletedDate": true}),
    );
    assert_eq!(r, json!({"status": "open", "completedDate": null}));
    let r = value(
        "op.uncomplete_nonrecurring",
        json!({"frontmatter": {"status": "cancelled", "completedDate": "2026-02-20"}, "defaultStatus": "open", "clearCompletedDate": false}),
    );
    assert_eq!(r, json!({"status": "open", "completedDate": "2026-02-20"}));
}

#[test]
fn op_error_shape_devolve_codigo_e_mensagem() {
    let r = value(
        "op.error_shape",
        json!({"operation": "update", "code": "invalid_type", "message": "bad value", "field": "status"}),
    );
    assert_eq!(r["code"], "invalid_type");
    assert_eq!(r["operation"], "update");
    assert_eq!(r["message"], "bad value");
}

#[test]
fn delete_remove_recusa_remover_com_links_quebrados_sem_force() {
    assert_eq!(
        value(
            "delete.remove",
            json!({"path": "tasks/demo.md", "checkBacklinks": true, "force": true, "brokenLinks": ["tasks/other.md"]})
        ),
        json!({"deleted": true})
    );
    assert_eq!(
        code(
            "delete.remove",
            json!({"path": "tasks/demo.md", "checkBacklinks": true, "force": false, "brokenLinks": ["tasks/other.md"]})
        ),
        "has_backlinks"
    );
    assert_eq!(
        value(
            "delete.remove",
            json!({"path": "tasks/demo.md", "checkBacklinks": true, "force": false})
        ),
        json!({"deleted": true})
    );
}

#[test]
fn op_complete_nonrecurring_ja_fechada_mantem_a_data() {
    let r = value(
        "op.complete_nonrecurring",
        json!({"frontmatter": {"title": "B", "status": "done", "completedDate": "2026-02-19"}, "completedValues": ["done", "cancelled"], "explicitDate": "2026-02-20"}),
    );
    assert_eq!(r, json!({"status": "done", "completedDate": "2026-02-19"}));
}

#[test]
fn op_idempotency_check_repetir_complete_nao_muda_o_estado() {
    let r = value(
        "op.idempotency_check",
        json!({"operation": "complete_nonrecurring", "first": {"status": "open", "completedDate": null}, "second": {"status": "done", "completedDate": "2026-02-20"}}),
    );
    assert_eq!(r, json!({"idempotent": true}));
    let r = value(
        "op.idempotency_check",
        json!({"operation": "complete_nonrecurring", "first": {"status": "open"}, "second": {"status": "open"}}),
    );
    assert_eq!(r, json!({"idempotent": false}));
}

#[test]
fn claim_usa_o_nome_mdbase_tasks_e_nao_reclama_capacidades() {
    let c = value("meta.claim", json!({}));
    assert_eq!(c["implementation"], "mdbase-tasks");
    assert_eq!(c["capabilities"], json!([]));
    assert_eq!(
        value("meta.has_capability", json!({"capability": "dependencies"})),
        json!({"value": false})
    );
}

#[test]
fn operacao_fora_do_contrato_responde_sem_nome_da_operacao() {
    let e = run(
        "date.day_in_timezone",
        json!({"instant": "2026-02-20T00:30:00Z", "timezone": "Invalid/Zone"}),
    )
    .unwrap_err();
    assert_eq!(e.code, "unsupported_operation");
    assert_eq!(e.message, "operation not implemented");
}

#[test]
fn delete_remove_recusa_brokenlinks_que_nao_e_lista() {
    assert_eq!(
        code(
            "delete.remove",
            json!({"path": "t.md", "checkBacklinks": true, "force": false, "brokenLinks": "x"})
        ),
        "invalid_input"
    );
}

#[test]
fn op_idempotency_check_create_e_idempotente() {
    let r = value(
        "op.idempotency_check",
        json!({"operation": "create", "first": null, "second": {"title": "New task", "status": "open"}}),
    );
    assert_eq!(r, json!({"idempotent": true}));
}

#[test]
fn operacoes_sem_correspondente_no_crate_respondem_nao_implementada() {
    for op in [
        "create_compat.create",
        "op.mutate_with_validation",
        "op.atomic_write",
        "date.day_in_timezone",
    ] {
        let e = run(op, json!({"value": "x"})).unwrap_err();
        assert_eq!(e.code, "unsupported_operation", "{op}");
        assert_eq!(e.message, "operation not implemented", "{op}");
    }
}
