//! Madame Labs fork: native Postgres collection deletion registry.
//!
//! Upstream ships the Postgres backend as a non-launch fallback with no floor
//! producer, so every publishing write answers `unavailable`. This registry is
//! the producer for a single-operator self-host: the CP records the permanent
//! nil floor here (`registry_record_collection_deletion`, the same RPCs and
//! receipts as the Durable Object registry), and the backend reads it freshly
//! before every publishing commit.
//!
//! **Trade-off accepted by the operator.** The floors live in the same Postgres
//! as the log. A restore of that database to a point before a deletion also
//! forgets the floor. Upstream keeps the floor independent of log/CP backups
//! for exactly this reason; here the backup policy carries that risk.
//!
//! **Ordering.** Recording a floor takes the collection's row lock first (the
//! row lock *is* the collection's actor, see `pg.rs`), so a floor never lands
//! between a writer's floor read and its commit. A writer that starts after the
//! floor commits reads it at commit and is refused `Gone`.
//!
//! The registry uses its own pool: a writer holds a backend connection while
//! it reads the floor, so a shared pool could starve under load.
use super::*;
use mdbn_log_service::Code;
use mdbn_log_service::auth::Principal;
use mdbn_log_service::deletion::CollectionDeletionRecord;
use mdbn_wire::cbor::Cbor;
use mdbn_wire::schema::Wire;

const PAGE_ROWS: i64 = 128;
const NIL: Uuid = B16([0; 16]);

/// Migration 3 (fork): the floor table and its single revision counter.
pub const DELETION_REGISTRY_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS ls_collection_deletion_floor (
  collection bytea PRIMARY KEY,
  deletion_id bytea NOT NULL,
  lifecycle_epoch bytea NOT NULL
);
CREATE TABLE IF NOT EXISTS ls_collection_deletion_revision (
  k integer PRIMARY KEY CHECK (k = 0),
  revision bytea NOT NULL
);
INSERT INTO ls_collection_deletion_revision VALUES (0, '\x0000000000000000')
  ON CONFLICT (k) DO NOTHING;
"#;

/// Durable deletion floors in Postgres.
pub struct PgDeletionRegistry {
    pool: Pool,
}

fn unavailable() -> ServiceError {
    CollectionDeletionRecord::unavailable()
}
fn invalid() -> ServiceError {
    ServiceError::invalid("collection_deletion_request")
}
/// Any database failure is unknown authority: fail closed.
fn down(e: impl std::fmt::Display) -> ServiceError {
    let mut error = unavailable();
    error.message = Some(format!("postgres: {e}"));
    error
}

fn params(value: &Cbor, count: usize) -> Result<Vec<&Cbor>> {
    let Cbor::Map(fields) = value else {
        return Err(invalid());
    };
    if fields.len() != count
        || fields
            .iter()
            .enumerate()
            .any(|(i, (k, _))| *k != Cbor::Uint(i as u64))
    {
        return Err(invalid());
    }
    Ok(fields.iter().map(|(_, v)| v).collect())
}
fn uuid(value: &Cbor) -> Result<Uuid> {
    let id = Uuid::from_cbor(value).map_err(|_| invalid())?;
    if id == NIL {
        return Err(invalid());
    }
    Ok(id)
}
fn uint(value: &Cbor) -> Result<u64> {
    match value {
        Cbor::Uint(n) => Ok(*n),
        _ => Err(invalid()),
    }
}
fn stored_u64(bytes: &[u8]) -> Result<u64> {
    Ok(u64::from_be_bytes(
        bytes.try_into().map_err(|_| unavailable())?,
    ))
}
fn stored_record(row: &Row) -> Result<CollectionDeletionRecord> {
    let id = |i: usize| -> Result<Uuid> {
        Ok(B16(row
            .try_get::<_, &[u8]>(i)
            .map_err(down)?
            .try_into()
            .map_err(|_| unavailable())?))
    };
    CollectionDeletionRecord {
        collection: id(0)?,
        deletion_id: id(1)?,
        lifecycle_epoch: stored_u64(row.try_get::<_, &[u8]>(2).map_err(down)?)?,
    }
    .validate()
}
fn row_cbor(r: CollectionDeletionRecord) -> Cbor {
    Cbor::Array(vec![
        r.collection.to_cbor(),
        r.deletion_id.to_cbor(),
        Cbor::Uint(r.lifecycle_epoch),
    ])
}
fn reply(fields: Vec<(u64, Cbor)>) -> Cbor {
    Cbor::Map(
        fields
            .into_iter()
            .map(|(k, v)| (Cbor::Uint(k), v))
            .collect(),
    )
}

const SELECT_FLOOR: &str = "SELECT collection, deletion_id, lifecycle_epoch \
     FROM ls_collection_deletion_floor WHERE collection = $1";
const SELECT_REVISION: &str = "SELECT revision FROM ls_collection_deletion_revision WHERE k = 0";

impl PgDeletionRegistry {
    /// Connect on its own pool and apply pending migrations (idempotent).
    pub async fn connect(url: &str, pool_size: usize) -> Result<Self> {
        let cfg: tokio_postgres::Config = url.parse().map_err(db)?;
        let pool = Pool::new(cfg, pool_size).map_err(db)?;
        let mut c = pool.get().await.map_err(db)?;
        migrate(&mut c).await?;
        drop(c);
        Ok(PgDeletionRegistry { pool })
    }

    async fn floor(&self, collection: Uuid) -> Result<Option<CollectionDeletionRecord>> {
        let c = self.pool.get().await.map_err(down)?;
        let row = c
            .query_opt(SELECT_FLOOR, &[&&collection.0[..]])
            .await
            .map_err(down)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let record = stored_record(&row)?;
        if record.collection != collection {
            return Err(unavailable());
        }
        Ok(Some(record))
    }

    /// Registry RPCs; `None` for any other method (it belongs to the service).
    pub async fn call(
        &self,
        principal: &Principal,
        method: &str,
        value: &Cbor,
    ) -> Option<Result<Cbor>> {
        let count = match method {
            "registry_record_collection_deletion" => 4,
            "registry_collection_deletions" => 3,
            "registry_collection_deletion" => 2,
            _ => return None,
        };
        Some(self.dispatch(principal, method, count, value).await)
    }

    async fn dispatch(
        &self,
        principal: &Principal,
        method: &str,
        count: usize,
        value: &Cbor,
    ) -> Result<Cbor> {
        if !matches!(principal, Principal::ControlPlane) {
            return Err(ServiceError::reason(Code::Forbidden, "principal"));
        }
        let fields = params(value, count)?;
        if Uuid::from_cbor(fields[0]).map_err(|_| invalid())? != NIL {
            return Err(invalid());
        }
        match method {
            "registry_record_collection_deletion" => {
                let requested = CollectionDeletionRecord {
                    collection: uuid(fields[1])?,
                    deletion_id: uuid(fields[2])?,
                    lifecycle_epoch: uint(fields[3])?,
                };
                if requested.lifecycle_epoch == 0 {
                    return Err(invalid());
                }
                self.record(requested).await
            }
            "registry_collection_deletion" => {
                let collection = uuid(fields[1])?;
                let mut c = self.pool.get().await.map_err(down)?;
                let tx = c
                    .build_transaction()
                    .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
                    .read_only(true)
                    .start()
                    .await
                    .map_err(down)?;
                let floor = tx
                    .query_opt(SELECT_FLOOR, &[&&collection.0[..]])
                    .await
                    .map_err(down)?
                    .map(|r| stored_record(&r))
                    .transpose()?;
                let revision = stored_u64(
                    tx.query_one(SELECT_REVISION, &[])
                        .await
                        .map_err(down)?
                        .try_get(0)
                        .map_err(down)?,
                )?;
                tx.commit().await.map_err(down)?;
                Ok(reply(vec![
                    (0, Cbor::Uint(1)),
                    (1, collection.to_cbor()),
                    (2, floor.map_or(Cbor::Null, row_cbor)),
                    (3, Cbor::Uint(revision)),
                ]))
            }
            _ => {
                let after = match fields[1] {
                    Cbor::Null => None,
                    v => Some(uuid(v)?),
                };
                let expected = match fields[2] {
                    Cbor::Null => None,
                    v => Some(uint(v)?),
                };
                if after.is_some() && expected.is_none() {
                    return Err(invalid());
                }
                self.page(after, expected).await
            }
        }
    }

    async fn page(&self, after: Option<Uuid>, expected: Option<u64>) -> Result<Cbor> {
        let mut c = self.pool.get().await.map_err(down)?;
        let tx = c
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .await
            .map_err(down)?;
        let revision = stored_u64(
            tx.query_one(SELECT_REVISION, &[])
                .await
                .map_err(down)?
                .try_get(0)
                .map_err(down)?,
        )?;
        if expected.is_some_and(|n| n != revision) {
            return Err(unavailable());
        }
        let rows = tx
            .query(
                "SELECT collection, deletion_id, lifecycle_epoch \
                 FROM ls_collection_deletion_floor WHERE collection > $1 \
                 ORDER BY collection LIMIT $2",
                &[&&after.unwrap_or(NIL).0[..], &PAGE_ROWS],
            )
            .await
            .map_err(down)?;
        tx.commit().await.map_err(down)?;
        let records: Vec<CollectionDeletionRecord> =
            rows.iter().map(stored_record).collect::<Result<_>>()?;
        let next = records.last().map(|r| r.collection).or(after);
        Ok(reply(vec![
            (0, Cbor::Uint(1)),
            (1, Cbor::Uint(revision)),
            (2, Cbor::Array(records.into_iter().map(row_cbor).collect())),
            (3, next.map_or(Cbor::Null, |id| id.to_cbor())),
            (4, Cbor::Bool((rows.len() as i64) < PAGE_ROWS)),
        ]))
    }

    async fn record(&self, requested: CollectionDeletionRecord) -> Result<Cbor> {
        let mut c = self.pool.get().await.map_err(down)?;
        let tx = c.transaction().await.map_err(down)?;
        let id = &requested.collection.0[..];
        // The collection's actor first: wait out any open writer (see module doc).
        tx.query_opt(
            "SELECT 1 FROM ls_collections WHERE id = $1 FOR UPDATE",
            &[&id],
        )
        .await
        .map_err(down)?;
        // Then the registry: one floor insert and revision bump at a time.
        let revision = stored_u64(
            tx.query_one(&format!("{SELECT_REVISION} FOR UPDATE"), &[])
                .await
                .map_err(down)?
                .try_get(0)
                .map_err(down)?,
        )?;
        let current = tx
            .query_opt(SELECT_FLOOR, &[&id])
            .await
            .map_err(down)?
            .map(|r| stored_record(&r))
            .transpose()?;
        let (actual, revision) = match current {
            Some(actual) => (actual, revision),
            None => {
                let next = revision.checked_add(1).ok_or_else(unavailable)?;
                tx.execute(
                    "INSERT INTO ls_collection_deletion_floor VALUES ($1, $2, $3)",
                    &[
                        &id,
                        &&requested.deletion_id.0[..],
                        &&requested.lifecycle_epoch.to_be_bytes()[..],
                    ],
                )
                .await
                .map_err(down)?;
                tx.execute(
                    "UPDATE ls_collection_deletion_revision SET revision = $1 WHERE k = 0",
                    &[&&next.to_be_bytes()[..]],
                )
                .await
                .map_err(down)?;
                (requested, next)
            }
        };
        tx.commit().await.map_err(down)?;
        let receipt = reply(vec![
            (0, Cbor::Uint(1)),
            (1, actual.collection.to_cbor()),
            (2, actual.deletion_id.to_cbor()),
            (3, Cbor::Uint(actual.lifecycle_epoch)),
            (4, Cbor::Uint(revision)),
        ]);
        // The first floor is immutable: a different tuple is refused with the
        // first receipt, never replaced or reported as success.
        if actual != requested {
            let mut error = ServiceError::reason(Code::Forbidden, "collection_deletion_conflict");
            error.details = Some(receipt);
            return Err(error);
        }
        Ok(receipt)
    }
}

impl IndependentFloorReader for PgDeletionRegistry {
    /// Fresh read on every call: no cache, absence only when the row is absent.
    fn read<'a>(
        &'a self,
        collection: Uuid,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<CollectionDeletionRecord>>> + Send + 'a>,
    > {
        Box::pin(self.floor(collection))
    }
}

#[cfg(test)]
mod tests;
