//! Madame Labs fork: the native Postgres deletion registry against an isolated
//! throwaway database (MDBN_TEST_PG_URL / MDBN_LOGSVC_PG_URL). Each test creates
//! its own database so revisions and page scans are deterministic.
use super::*;
use mdbn_log_service::Code;
use mdbn_log_service::auth::Principal;
use mdbn_log_service::deletion::CollectionDeletionRecord;
use mdbn_log_service::testkit::id16;
use mdbn_wire::cbor::Cbor;
use mdbn_wire::schema::Wire;

async fn fresh_url() -> Option<String> {
    let Ok(url) =
        std::env::var("MDBN_TEST_PG_URL").or_else(|_| std::env::var("MDBN_LOGSVC_PG_URL"))
    else {
        eprintln!("MDBN_TEST_PG_URL not set: skipping the Postgres deletion registry run");
        return None;
    };
    let mut raw = [0u8; 8];
    getrandom::fill(&mut raw).unwrap();
    let name = format!(
        "piso_{}",
        raw.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let cfg: tokio_postgres::Config = url.parse().unwrap();
    let (client, conn) = cfg.connect(tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(conn);
    client
        .batch_execute(&format!("CREATE DATABASE {name}"))
        .await
        .unwrap();
    let (base, _) = url.rsplit_once('/').unwrap();
    Some(format!("{base}/{name}"))
}

fn rec(label: &str, epoch: u64) -> CollectionDeletionRecord {
    CollectionDeletionRecord {
        collection: id16(&format!("{label}/collection")),
        deletion_id: id16(&format!("{label}/deletion")),
        lifecycle_epoch: epoch,
    }
}

fn record_params(r: CollectionDeletionRecord) -> Cbor {
    Cbor::Map(vec![
        (Cbor::Uint(0), B16([0; 16]).to_cbor()),
        (Cbor::Uint(1), r.collection.to_cbor()),
        (Cbor::Uint(2), r.deletion_id.to_cbor()),
        (Cbor::Uint(3), Cbor::Uint(r.lifecycle_epoch)),
    ])
}

fn field(v: &Cbor, k: u64) -> Option<&Cbor> {
    match v {
        Cbor::Map(m) => m
            .iter()
            .find(|(key, _)| *key == Cbor::Uint(k))
            .map(|(_, v)| v),
        _ => None,
    }
}

async fn record(reg: &PgDeletionRegistry, r: CollectionDeletionRecord) -> Result<Cbor> {
    reg.call(
        &Principal::ControlPlane,
        "registry_record_collection_deletion",
        &record_params(r),
    )
    .await
    .expect("registry method")
}

#[tokio::test]
async fn absent_floor_is_authoritative_none_and_read_fresh_after_record() {
    let Some(url) = fresh_url().await else { return };
    let reg = PgDeletionRegistry::connect(&url, 2).await.unwrap();
    let r = rec("fresh", u64::MAX);
    assert_eq!(reg.read(r.collection).await.unwrap(), None);
    let receipt = record(&reg, r).await.unwrap();
    assert_eq!(field(&receipt, 0), Some(&Cbor::Uint(1)));
    assert_eq!(field(&receipt, 1), Some(&r.collection.to_cbor()));
    assert_eq!(field(&receipt, 2), Some(&r.deletion_id.to_cbor()));
    assert_eq!(field(&receipt, 3), Some(&Cbor::Uint(u64::MAX)));
    assert_eq!(field(&receipt, 4), Some(&Cbor::Uint(1)));
    assert_eq!(reg.read(r.collection).await.unwrap(), Some(r));
}

#[tokio::test]
async fn floor_survives_a_new_registry_on_the_same_database() {
    let Some(url) = fresh_url().await else { return };
    let r = rec("durable", 7);
    {
        let reg = PgDeletionRegistry::connect(&url, 1).await.unwrap();
        record(&reg, r).await.unwrap();
    }
    let reg = PgDeletionRegistry::connect(&url, 1).await.unwrap();
    assert_eq!(reg.read(r.collection).await.unwrap(), Some(r));
}

#[tokio::test]
async fn first_floor_is_immutable_and_repeat_is_idempotent() {
    let Some(url) = fresh_url().await else { return };
    let reg = PgDeletionRegistry::connect(&url, 2).await.unwrap();
    let r = rec("first", 3);
    let first = record(&reg, r).await.unwrap();
    assert_eq!(
        record(&reg, r).await.unwrap(),
        first,
        "repeat is the same receipt"
    );
    for other in [
        CollectionDeletionRecord {
            lifecycle_epoch: 4,
            ..r
        },
        CollectionDeletionRecord {
            deletion_id: id16("first/other"),
            ..r
        },
    ] {
        let e = record(&reg, other).await.unwrap_err();
        assert_eq!(e.code, Code::Forbidden);
        assert_eq!(e.reason.as_deref(), Some("collection_deletion_conflict"));
        assert_eq!(
            e.details.as_ref(),
            Some(&first),
            "conflict carries the first receipt"
        );
    }
    assert_eq!(reg.read(r.collection).await.unwrap(), Some(r));
    assert_eq!(
        field(&record(&reg, r).await.unwrap(), 4),
        Some(&Cbor::Uint(1))
    );
}

#[tokio::test]
async fn record_requires_control_plane_nil_route_and_exact_shape() {
    let Some(url) = fresh_url().await else { return };
    let reg = PgDeletionRegistry::connect(&url, 2).await.unwrap();
    let r = rec("shape", 1);
    let device = Principal::Device {
        id: id16("shape/device"),
        sign_pk: B32([7; 32]),
        collection: Some(r.collection),
    };
    let e = reg
        .call(
            &device,
            "registry_record_collection_deletion",
            &record_params(r),
        )
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(e.code, Code::Forbidden);
    let nonnil_route = Cbor::Map(vec![
        (Cbor::Uint(0), r.collection.to_cbor()),
        (Cbor::Uint(1), r.collection.to_cbor()),
        (Cbor::Uint(2), r.deletion_id.to_cbor()),
        (Cbor::Uint(3), Cbor::Uint(1)),
    ]);
    let zero_epoch = record_params(CollectionDeletionRecord {
        lifecycle_epoch: 0,
        ..r
    });
    let nil_collection = record_params(CollectionDeletionRecord {
        collection: B16([0; 16]),
        ..r
    });
    let nil_deletion = record_params(CollectionDeletionRecord {
        deletion_id: B16([0; 16]),
        ..r
    });
    for bad in [
        Cbor::Null,
        Cbor::Map(vec![]),
        Cbor::Map(vec![(Cbor::Uint(0), B16([0; 16]).to_cbor())]),
        nonnil_route,
        zero_epoch,
        nil_collection,
        nil_deletion,
    ] {
        let e = reg
            .call(
                &Principal::ControlPlane,
                "registry_record_collection_deletion",
                &bad,
            )
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(e.code, Code::Invalid, "{bad:?}");
    }
    assert_eq!(
        reg.read(r.collection).await.unwrap(),
        None,
        "refusals record nothing"
    );
    assert!(
        reg.call(&Principal::ControlPlane, "append", &record_params(r))
            .await
            .is_none(),
        "non-registry methods fall through to the service"
    );
}

#[tokio::test]
async fn malformed_stored_row_is_unavailable_never_absence() {
    let Some(url) = fresh_url().await else { return };
    let reg = PgDeletionRegistry::connect(&url, 2).await.unwrap();
    let c = id16("corrupt/collection");
    let client = reg.pool.get().await.unwrap();
    client
        .execute(
            "INSERT INTO ls_collection_deletion_floor VALUES ($1, $2, $3)",
            &[&&c.0[..], &&id16("corrupt/deletion").0[..], &&[0u8, 1][..]],
        )
        .await
        .unwrap();
    drop(client);
    let e = reg.read(c).await.unwrap_err();
    assert_eq!(e.code, Code::Unavailable);
    assert_eq!(
        e.reason.as_deref(),
        Some("collection_deletion_floor_unavailable")
    );
}

#[tokio::test]
async fn unreachable_database_is_unavailable() {
    let reg = PgDeletionRegistry {
        pool: Pool::new("host=127.0.0.1 port=1 user=x dbname=x".parse().unwrap(), 1).unwrap(),
    };
    let e = reg.read(id16("unreachable/collection")).await.unwrap_err();
    assert_eq!(e.code, Code::Unavailable);
}

#[tokio::test]
async fn single_lookup_and_generation_pinned_pages() {
    let Some(url) = fresh_url().await else { return };
    let reg = PgDeletionRegistry::connect(&url, 2).await.unwrap();
    let nil = || B16([0; 16]).to_cbor();
    let page = |after: Cbor, generation: Cbor| {
        Cbor::Map(vec![
            (Cbor::Uint(0), nil()),
            (Cbor::Uint(1), after),
            (Cbor::Uint(2), generation),
        ])
    };
    let call = |m: &'static str, p: Cbor| {
        let reg = &reg;
        async move { reg.call(&Principal::ControlPlane, m, &p).await.unwrap() }
    };
    let empty = call(
        "registry_collection_deletions",
        page(Cbor::Null, Cbor::Null),
    )
    .await
    .unwrap();
    assert_eq!(field(&empty, 1), Some(&Cbor::Uint(0)));
    assert_eq!(field(&empty, 2), Some(&Cbor::Array(vec![])));
    assert_eq!(field(&empty, 4), Some(&Cbor::Bool(true)));

    let mut all: Vec<CollectionDeletionRecord> =
        (0..130).map(|i| rec(&format!("page-{i}"), i + 1)).collect();
    for r in &all {
        record(&reg, *r).await.unwrap();
    }
    all.sort_by_key(|r| r.collection.0);
    let row = |r: &CollectionDeletionRecord| {
        Cbor::Array(vec![
            r.collection.to_cbor(),
            r.deletion_id.to_cbor(),
            Cbor::Uint(r.lifecycle_epoch),
        ])
    };
    let first = call(
        "registry_collection_deletions",
        page(Cbor::Null, Cbor::Null),
    )
    .await
    .unwrap();
    assert_eq!(field(&first, 1), Some(&Cbor::Uint(130)));
    assert_eq!(
        field(&first, 2),
        Some(&Cbor::Array(all[..128].iter().map(row).collect()))
    );
    assert_eq!(field(&first, 3), Some(&all[127].collection.to_cbor()));
    assert_eq!(field(&first, 4), Some(&Cbor::Bool(false)));
    let cursor = field(&first, 3).unwrap().clone();
    let e = call(
        "registry_collection_deletions",
        page(cursor.clone(), Cbor::Null),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, Code::Invalid, "cursor without generation");
    let e = call(
        "registry_collection_deletions",
        page(cursor.clone(), Cbor::Uint(129)),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, Code::Unavailable, "stale generation");
    let second = call(
        "registry_collection_deletions",
        page(cursor, Cbor::Uint(130)),
    )
    .await
    .unwrap();
    assert_eq!(
        field(&second, 2),
        Some(&Cbor::Array(all[128..].iter().map(row).collect()))
    );
    assert_eq!(field(&second, 4), Some(&Cbor::Bool(true)));

    let one = |c: Uuid| Cbor::Map(vec![(Cbor::Uint(0), nil()), (Cbor::Uint(1), c.to_cbor())]);
    let hit = call("registry_collection_deletion", one(all[0].collection))
        .await
        .unwrap();
    assert_eq!(field(&hit, 2), Some(&row(&all[0])));
    assert_eq!(field(&hit, 3), Some(&Cbor::Uint(130)));
    let miss = call("registry_collection_deletion", one(id16("page/none")))
        .await
        .unwrap();
    assert_eq!(field(&miss, 2), Some(&Cbor::Null));
}

#[tokio::test]
async fn record_waits_for_an_in_flight_writer_of_the_collection() {
    let Some(url) = fresh_url().await else { return };
    let reg = PgDeletionRegistry::connect(&url, 2).await.unwrap();
    let r = rec("race", 1);
    // A writer holding the collection row lock, as PgTxn does from begin to commit.
    let backend = PgBackend::connect(&url, 2).await.unwrap();
    let mut writer = backend.pool().get().await.unwrap();
    writer
        .execute(
            "INSERT INTO ls_collections (id, head, head_chain, meta) VALUES ($1, 0, $2, $3)",
            &[&&r.collection.0[..], &&[0u8; 32][..], &&[0u8][..]],
        )
        .await
        .unwrap();
    let tx = writer.transaction().await.unwrap();
    tx.query_one(
        "SELECT meta FROM ls_collections WHERE id = $1 FOR UPDATE",
        &[&&r.collection.0[..]],
    )
    .await
    .unwrap();
    let pending = record(&reg, r);
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), &mut pending)
            .await
            .is_err(),
        "the floor must not land while a writer of the collection is open"
    );
    tx.commit().await.unwrap();
    pending.await.unwrap();
    assert_eq!(reg.read(r.collection).await.unwrap(), Some(r));
}
