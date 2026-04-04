//! Payment run repository — SQL layer for payment_runs, payment_run_items,
//! payment_run_executions, and ap_allocations.
//!
//! Builder queries (pool-based) handle the run-creation path.
//! Execute queries (conn-based) handle the execution path within a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use sqlx::PgPool;
use uuid::Uuid;

use super::{CreatePaymentRunRequest, PaymentRun, PaymentRunError, PaymentRunItemRow};

// ============================================================================
// Execution result type (owned here; re-exported from execute.rs)
// ============================================================================

/// Record of a single vendor payment within a payment run execution.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct ExecutionRecord {
    pub id: i64,
    pub run_id: Uuid,
    pub item_id: i64,
    pub payment_id: Uuid,
    pub vendor_id: Uuid,
    pub amount_minor: i64,
    pub currency: String,
    pub status: String,
    pub executed_at: DateTime<Utc>,
}

// ============================================================================
// Internal helper type
// ============================================================================

/// An eligible bill row returned by `select_eligible_bills`.
#[derive(Debug, sqlx::FromRow)]
pub(super) struct EligibleBill {
    pub bill_id: Uuid,
    pub vendor_id: Uuid,
    pub open_balance_minor: i64,
    #[allow(dead_code)]
    pub currency: String,
}

// ============================================================================
// Builder queries (pool-based — no transaction needed)
// ============================================================================

/// Fetch a payment run header by run_id + tenant. Returns None if not found.
pub async fn fetch_payment_run(
    pool: &PgPool,
    tenant_id: &str,
    run_id: Uuid,
) -> Result<Option<PaymentRun>, PaymentRunError> {
    let run: Option<PaymentRun> = sqlx::query_as(
        r#"
        SELECT run_id, tenant_id, total_minor, currency, scheduled_date,
               payment_method, status, created_by, created_at, executed_at
        FROM payment_runs
        WHERE run_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(run)
}

/// Fetch all items for a payment run (pool-based, for the idempotency path).
pub async fn fetch_run_items_pool(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<PaymentRunItemRow>, PaymentRunError> {
    let items: Vec<PaymentRunItemRow> = sqlx::query_as(
        "SELECT id, run_id, vendor_id, bill_ids, amount_minor, currency, created_at \
         FROM payment_run_items WHERE run_id = $1 ORDER BY id ASC",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    Ok(items)
}

/// Select eligible bills: approved/partially_paid, open balance > 0, matching currency.
///
/// Optional filters: due_on_or_before, vendor_ids.
/// Results are ordered deterministically: vendor_id, due_date, bill_id.
pub(super) async fn select_eligible_bills(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreatePaymentRunRequest,
) -> Result<Vec<EligibleBill>, PaymentRunError> {
    let rows: Vec<EligibleBill> = sqlx::query_as(
        r#"
        SELECT
            vb.bill_id,
            vb.vendor_id,
            vb.currency,
            (vb.total_minor - COALESCE(SUM(aa.amount_minor), 0))::bigint AS open_balance_minor
        FROM vendor_bills vb
        LEFT JOIN ap_allocations aa
            ON aa.bill_id = vb.bill_id AND aa.tenant_id = vb.tenant_id
        WHERE vb.tenant_id = $1
          AND vb.status IN ('approved', 'partially_paid')
          AND vb.currency = $2
          AND ($3::timestamptz IS NULL OR vb.due_date <= $3)
          AND ($4::uuid[] IS NULL OR vb.vendor_id = ANY($4))
        GROUP BY vb.bill_id, vb.vendor_id, vb.currency, vb.total_minor
        HAVING (vb.total_minor - COALESCE(SUM(aa.amount_minor), 0)) > 0
        ORDER BY vb.vendor_id, vb.due_date, vb.bill_id
        "#,
    )
    .bind(tenant_id)
    .bind(&req.currency)
    .bind(req.due_on_or_before)
    .bind(req.vendor_ids.as_deref())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// INSERT a payment_run header row. Returns the inserted record.
///
/// Maps the 23505 unique-violation (race on run_id) to `PaymentRunError::DuplicateRunId`.
pub async fn insert_payment_run(
    conn: &mut PgConnection,
    run_id: Uuid,
    tenant_id: &str,
    total_minor: i64,
    currency: &str,
    scheduled_date: DateTime<Utc>,
    payment_method: &str,
    created_by: &str,
) -> Result<PaymentRun, PaymentRunError> {
    let run: PaymentRun = sqlx::query_as(
        r#"
        INSERT INTO payment_runs
            (run_id, tenant_id, total_minor, currency, scheduled_date,
             payment_method, status, created_by, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, NOW())
        RETURNING run_id, tenant_id, total_minor, currency, scheduled_date,
                  payment_method, status, created_by, created_at, executed_at
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .bind(total_minor)
    .bind(currency)
    .bind(scheduled_date)
    .bind(payment_method)
    .bind(created_by)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.code().as_deref() == Some("23505") {
                return PaymentRunError::DuplicateRunId(run_id);
            }
        }
        PaymentRunError::Database(e)
    })?;
    Ok(run)
}

/// INSERT a payment_run_items row (one per vendor group). Returns the inserted record.
pub async fn insert_payment_run_item(
    conn: &mut PgConnection,
    run_id: Uuid,
    vendor_id: Uuid,
    bill_ids: &[Uuid],
    amount_minor: i64,
    currency: &str,
) -> Result<PaymentRunItemRow, PaymentRunError> {
    let item: PaymentRunItemRow = sqlx::query_as(
        r#"
        INSERT INTO payment_run_items
            (run_id, vendor_id, bill_ids, amount_minor, currency, created_at)
        VALUES ($1, $2, $3, $4, $5, NOW())
        RETURNING id, run_id, vendor_id, bill_ids, amount_minor, currency, created_at
        "#,
    )
    .bind(run_id)
    .bind(vendor_id)
    .bind(bill_ids)
    .bind(amount_minor)
    .bind(currency)
    .fetch_one(&mut *conn)
    .await?;
    Ok(item)
}

// ============================================================================
// Execute queries (conn-based — called within a transaction)
// ============================================================================

/// SELECT … FOR UPDATE on the payment_runs row. Locks it to prevent concurrent execution.
pub async fn lock_payment_run(
    conn: &mut PgConnection,
    run_id: Uuid,
    tenant_id: &str,
) -> Result<Option<PaymentRun>, PaymentRunError> {
    let run: Option<PaymentRun> = sqlx::query_as(
        r#"
        SELECT run_id, tenant_id, total_minor, currency, scheduled_date,
               payment_method, status, created_by, created_at, executed_at
        FROM payment_runs
        WHERE run_id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(run)
}

/// UPDATE payment_runs SET status = 'executing'.
pub async fn set_run_executing(
    conn: &mut PgConnection,
    run_id: Uuid,
    tenant_id: &str,
) -> Result<(), PaymentRunError> {
    sqlx::query(
        "UPDATE payment_runs SET status = 'executing' \
         WHERE run_id = $1 AND tenant_id = $2",
    )
    .bind(run_id)
    .bind(tenant_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Fetch all items for a payment run (conn-based, within a transaction).
pub async fn fetch_run_items_tx(
    conn: &mut PgConnection,
    run_id: Uuid,
) -> Result<Vec<PaymentRunItemRow>, PaymentRunError> {
    let items: Vec<PaymentRunItemRow> = sqlx::query_as(
        "SELECT id, run_id, vendor_id, bill_ids, amount_minor, currency, created_at \
         FROM payment_run_items WHERE run_id = $1 ORDER BY id ASC",
    )
    .bind(run_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(items)
}

/// Fetch all execution records for a run (conn-based).
pub async fn fetch_executions(
    conn: &mut PgConnection,
    run_id: Uuid,
) -> Result<Vec<ExecutionRecord>, PaymentRunError> {
    let rows: Vec<ExecutionRecord> = sqlx::query_as(
        "SELECT id, run_id, item_id, payment_id, vendor_id, amount_minor, \
                currency, status, executed_at \
         FROM payment_run_executions WHERE run_id = $1 ORDER BY id ASC",
    )
    .bind(run_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}

/// Fetch the execution record for a specific item in a run (for idempotency check).
pub async fn fetch_execution_by_item(
    conn: &mut PgConnection,
    run_id: Uuid,
    item_id: i64,
) -> Result<Option<ExecutionRecord>, PaymentRunError> {
    let row: Option<ExecutionRecord> = sqlx::query_as(
        "SELECT id, run_id, item_id, payment_id, vendor_id, amount_minor, \
                currency, status, executed_at \
         FROM payment_run_executions WHERE run_id = $1 AND item_id = $2",
    )
    .bind(run_id)
    .bind(item_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// Compute open balance for a bill: total_minor minus sum of existing allocations.
pub async fn query_open_balance(
    conn: &mut PgConnection,
    tenant_id: &str,
    bill_id: Uuid,
) -> Result<i64, PaymentRunError> {
    let (total,): (i64,) = sqlx::query_as(
        "SELECT total_minor FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_one(&mut *conn)
    .await?;

    let (allocated,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount_minor), 0)::bigint \
         FROM ap_allocations WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_one(&mut *conn)
    .await?;

    Ok(total - allocated)
}

/// INSERT an ap_allocations row. ON CONFLICT DO NOTHING for idempotency.
pub async fn insert_allocation(
    conn: &mut PgConnection,
    allocation_id: Uuid,
    bill_id: Uuid,
    run_id: Uuid,
    tenant_id: &str,
    open_balance: i64,
    currency: &str,
) -> Result<(), PaymentRunError> {
    sqlx::query(
        r#"
        INSERT INTO ap_allocations
            (allocation_id, bill_id, payment_run_id, tenant_id,
             amount_minor, currency, allocation_type, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, 'full', NOW())
        ON CONFLICT (allocation_id) DO NOTHING
        "#,
    )
    .bind(allocation_id)
    .bind(bill_id)
    .bind(run_id)
    .bind(tenant_id)
    .bind(open_balance)
    .bind(currency)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// UPDATE vendor_bills SET status = 'paid' (only if currently approved or partially_paid).
pub async fn mark_bill_paid(
    conn: &mut PgConnection,
    bill_id: Uuid,
    tenant_id: &str,
) -> Result<(), PaymentRunError> {
    sqlx::query(
        "UPDATE vendor_bills SET status = 'paid' \
         WHERE bill_id = $1 AND tenant_id = $2 \
           AND status IN ('approved', 'partially_paid')",
    )
    .bind(bill_id)
    .bind(tenant_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// INSERT a payment_run_executions row. Returns the inserted record.
pub async fn insert_execution(
    conn: &mut PgConnection,
    run_id: Uuid,
    item_id: i64,
    payment_id: Uuid,
    vendor_id: Uuid,
    amount_minor: i64,
    currency: &str,
) -> Result<ExecutionRecord, PaymentRunError> {
    let exec: ExecutionRecord = sqlx::query_as(
        r#"
        INSERT INTO payment_run_executions
            (run_id, item_id, payment_id, vendor_id, amount_minor, currency,
             status, executed_at)
        VALUES ($1, $2, $3, $4, $5, $6, 'success', NOW())
        RETURNING id, run_id, item_id, payment_id, vendor_id, amount_minor,
                  currency, status, executed_at
        "#,
    )
    .bind(run_id)
    .bind(item_id)
    .bind(payment_id)
    .bind(vendor_id)
    .bind(amount_minor)
    .bind(currency)
    .fetch_one(&mut *conn)
    .await?;
    Ok(exec)
}

/// UPDATE payment_runs SET status = 'completed', executed_at = NOW().
/// Returns the updated run header.
pub async fn complete_run(
    conn: &mut PgConnection,
    run_id: Uuid,
    tenant_id: &str,
) -> Result<PaymentRun, PaymentRunError> {
    let run: PaymentRun = sqlx::query_as(
        r#"
        UPDATE payment_runs
           SET status = 'completed', executed_at = NOW()
         WHERE run_id = $1 AND tenant_id = $2
        RETURNING run_id, tenant_id, total_minor, currency, scheduled_date,
                  payment_method, status, created_by, created_at, executed_at
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(run)
}
