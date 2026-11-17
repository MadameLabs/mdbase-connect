//! Tests for the crate's date rules: calendar validity, the date a value stands
//! for, and the date an operation targets.

use mdbn_tasks::temporal;

#[test]
fn has_time_procura_t_hh_mm() {
    assert!(temporal::has_time("2026-02-20T10:00"));
    assert!(temporal::has_time("T10:00"));
    assert!(temporal::has_time("2026-02-20TT10:00"));
    assert!(!temporal::has_time("2026-02-20 10:00:00"));
    assert!(!temporal::has_time("2026-02-20t10:00"));
    assert!(!temporal::has_time("2026-02-20T1:00"));
}

#[test]
fn data_alvo_da_operacao_segue_explicita_agendada_prazo_hoje() {
    assert_eq!(
        temporal::operation_target(
            Some("2026-02-20"),
            Some("2026-01-01"),
            Some("2026-01-02"),
            "2026-11-16"
        )
        .unwrap(),
        "2026-02-20"
    );
    assert_eq!(
        temporal::operation_target(None, Some("bad"), Some("2026-04-01"), "2026-11-16").unwrap(),
        "2026-04-01"
    );
    assert_eq!(
        temporal::operation_target(None, Some("2023-02-29"), Some("2024-03-01"), "2026-11-16")
            .unwrap(),
        "2024-03-01"
    );
    assert_eq!(
        temporal::operation_target(None, Some("2026-11-05T23:59:59-08:00"), None, "2026-11-16")
            .unwrap(),
        "2026-11-05"
    );
    assert_eq!(
        temporal::operation_target(None, None, None, "2026-11-16").unwrap(),
        "2026-11-16"
    );
}

#[test]
fn data_alvo_explicita_invalida_recusa() {
    assert!(
        temporal::operation_target(Some("2026-02-30"), Some("2026-01-01"), None, "2026-11-16")
            .is_err()
    );
}
