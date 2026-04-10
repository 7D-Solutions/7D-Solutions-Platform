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
/// Scoped to tenant_id so cross-tenant reads are impossible at the SQL layer.
pub async fn find_active_snapshot(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
) -> Result<Option<ApTaxSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, ApTaxSnapshot>(&format!(
        "SELECT {} FROM ap_tax_snapshots \
         WHERE bill_id = $1 AND status != 'voided' AND tenant_id = $2 LIMIT 1",
        SNAPSHOT_COLS
    ))
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// Fetch a snapshot by its primary key, scoped to tenant_id.
pub async fn fetch_snapshot_by_id(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<ApTaxSnapshot, sqlx::Error> {
    sqlx::query_as::<_, ApTaxSnapshot>(&format!(
        "SELECT {} FROM ap_tax_snapshots WHERE id = $1 AND tenant_id = $2",
        SNAPSHOT_COLS
    ))
    .bind(id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await
}

// ============================================================================
// Writes
// ============================================================================

/// Supersede the active snapshot (content changed — void it before creating new one).
/// Scoped to tenant_id as a defense-in-depth write guard.
pub async fn void_superseded_snapshot(
    pool: &PgPool,
    tenant_id: &str,
    snap_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'voided', voided_at = NOW(), \
             void_reason = 'superseded by new quote', updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(snap_id)
    .bind(tenant_id)
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
/// Scoped to tenant_id as a defense-in-depth write guard.
pub async fn update_snapshot_committed(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    provider_commit_ref: &str,
    committed_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'committed', provider_commit_ref = $1, committed_at = $2, \
             updated_at = NOW() \
         WHERE id = $3 AND tenant_id = $4",
    )
    .bind(provider_commit_ref)
    .bind(committed_at)
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Void a quoted snapshot (voided_at = NOW(), no provider call needed).
/// Scoped to tenant_id as a defense-in-depth write guard.
pub async fn void_snapshot_now(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    void_reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'voided', voided_at = NOW(), void_reason = $1, updated_at = NOW() \
         WHERE id = $2 AND tenant_id = $3",
    )
    .bind(void_reason)
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Void a committed snapshot with an explicit timestamp from the provider.
/// Scoped to tenant_id as a defense-in-depth write guard.
pub async fn void_snapshot_at(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    voided_at: DateTime<Utc>,
    void_reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'voided', voided_at = $1, void_reason = $2, updated_at = NOW() \
         WHERE id = $3 AND tenant_id = $4",
    )
    .bind(voided_at)
    .bind(void_reason)
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .map(|_| ())
}

// ============================================================================
// Transaction-bound variants (for use within another service's transaction)
// ============================================================================

/// Fetch the active (non-voided) tax snapshot for a bill within an existing transaction.
/// Scoped to tenant_id so cross-tenant reads are impossible at the SQL layer.
pub async fn find_active_snapshot_tx(
    conn: &mut PgConnection,
    tenant_id: &str,
    bill_id: Uuid,
) -> Result<Option<super::models::ApTaxSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, super::models::ApTaxSnapshot>(&format!(
        "SELECT {} FROM ap_tax_snapshots \
         WHERE bill_id = $1 AND status != 'voided' AND tenant_id = $2 LIMIT 1",
        SNAPSHOT_COLS
    ))
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await
}

/// Mark a snapshot as committed within an existing transaction.
/// Scoped to tenant_id as a defense-in-depth write guard.
pub async fn commit_snapshot_tx(
    conn: &mut PgConnection,
    tenant_id: &str,
    id: Uuid,
    provider_commit_ref: &str,
    committed_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE ap_tax_snapshots \
         SET status = 'committed', provider_commit_ref = $1, committed_at = $2, \
             updated_at = NOW() \
         WHERE id = $3 AND tenant_id = $4",
    )
    .bind(provider_commit_ref)
    .bind(committed_at)
    .bind(id)
    .bind(tenant_id)
    .execute(&mut *conn)
    .await
    .map(|_| ())
}
