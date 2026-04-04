//! Tax snapshot repository — SQL layer for ap_tax_snapshots.
//!
//! All raw SQL for ap_tax_snapshots lives here.
//! The service layer calls these functions and owns business logic
//! (hash computation, idempotency decisions, provider dispatch).

use chrono::DateTime;
use chrono::Utc;
use sqlx::PgConnection;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::ApTaxSnapshot;

const SNAPSHOT_COLS: &str =
    "id, bill_id, tenant_id, provider, provider_quote_ref, provider_commit_ref, \
     quote_hash, total_tax_minor, tax_by_line, status, quoted_at, committed_at, \
     voided_at, void_reason, created_at, updated_at";

// ============================================================================
// Reads
// ============================================================================

/// Fetch the active (non-voided) tax snapshot for a bill, if any.
pub async fn find_active_snapshot(
    pool: &PgPool,
    bill_id: Uuid,
) -> Result<Option<ApTaxSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, ApTaxSnapshot>(&format!(
        "SELECT {} FROM ap_tax_snapshots WHERE bill_id = $1 AND status != 'voided' LIMIT 1",
        SNAPSHOT_COLS
    ))
    .bind(bill_id)
    .fetch_optional(pool)
    .await
}

/// Fetch a snapshot by its primary key.
pub async fn fetch_snapshot_by_id(pool: &PgPool, id: Uuid) -> Result<ApTaxSnapshot, sqlx::Error> {
    sqlx::query_as::<_, ApTaxSnapshot>(&format!(
        "SELECT {} FROM ap_tax_snapshots WHERE id = $1",
        SNAPSHOT_COLS
    ))
    .bind(id)
    .fetch_one(pool)
    .await
}

// ============================================================================
// Writes
// ============================================================================

/// Supersede the active snapshot (content changed — void it before creating new one).
pub async fn void_superseded_snapshot(pool: &PgPool, snap_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'voided', voided_at = NOW(), \
             void_reason = 'superseded by new quote', updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(snap_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Insert a new quoted snapshot row.
pub async fn insert_snapshot(
    pool: &PgPool,
    id: Uuid,
    bill_id: Uuid,
    tenant_id: &str,
    provider: &str,
    provider_quote_ref: &str,
    quote_hash: &str,
    total_tax_minor: i64,
    tax_by_line: &serde_json::Value,
    quoted_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ap_tax_snapshots \
         (id, bill_id, tenant_id, provider, provider_quote_ref, quote_hash, \
          total_tax_minor, tax_by_line, status, quoted_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'quoted', $9)",
    )
    .bind(id)
    .bind(bill_id)
    .bind(tenant_id)
    .bind(provider)
    .bind(provider_quote_ref)
    .bind(quote_hash)
    .bind(total_tax_minor)
    .bind(tax_by_line)
    .bind(quoted_at)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Mark a snapshot as committed with the provider's commit reference.
pub async fn update_snapshot_committed(
    pool: &PgPool,
    id: Uuid,
    provider_commit_ref: &str,
    committed_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'committed', provider_commit_ref = $1, committed_at = $2, \
             updated_at = NOW() \
         WHERE id = $3",
    )
    .bind(provider_commit_ref)
    .bind(committed_at)
    .bind(id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Void a quoted snapshot (voided_at = NOW(), no provider call needed).
pub async fn void_snapshot_now(
    pool: &PgPool,
    id: Uuid,
    void_reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'voided', voided_at = NOW(), void_reason = $1, updated_at = NOW() \
         WHERE id = $2",
    )
    .bind(void_reason)
    .bind(id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Void a committed snapshot with an explicit timestamp from the provider.
pub async fn void_snapshot_at(
    pool: &PgPool,
    id: Uuid,
    voided_at: DateTime<Utc>,
    void_reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'voided', voided_at = $1, void_reason = $2, updated_at = NOW() \
         WHERE id = $3",
    )
    .bind(voided_at)
    .bind(void_reason)
    .bind(id)
    .execute(pool)
    .await
    .map(|_| ())
}

// ============================================================================
// Transaction-bound variants (for use within another service's transaction)
// ============================================================================

/// Fetch the active (non-voided) tax snapshot for a bill within an existing transaction.
pub async fn find_active_snapshot_tx(
    conn: &mut PgConnection,
    bill_id: Uuid,
) -> Result<Option<super::models::ApTaxSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, super::models::ApTaxSnapshot>(&format!(
        "SELECT {} FROM ap_tax_snapshots \
         WHERE bill_id = $1 AND status != 'voided' LIMIT 1",
        SNAPSHOT_COLS
    ))
    .bind(bill_id)
    .fetch_optional(&mut *conn)
    .await
}

/// Mark a snapshot as committed within an existing transaction.
pub async fn commit_snapshot_tx(
    conn: &mut PgConnection,
    id: Uuid,
    provider_commit_ref: &str,
    committed_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'committed', provider_commit_ref = $1, committed_at = $2, \
             updated_at = NOW() \
         WHERE id = $3",
    )
    .bind(provider_commit_ref)
    .bind(committed_at)
    .bind(id)
    .execute(&mut *conn)
    .await
    .map(|_| ())
}
