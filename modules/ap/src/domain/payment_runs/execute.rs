//! Payment run execution: Guard → Mutation → Outbox atomicity.
//!
//! `execute_payment_run`:
//!   Idempotency: if run is already completed or failed, return existing state.
//!   Guard:    Run must be in 'pending' or 'executing' status (supports retry).
//!   Mutation (per item, in a single transaction):
//!     - Submit payment via integrations::payments (deterministic payment_id).
//!     - INSERT payment_run_executions (UNIQUE on run_id + item_id → no duplicates).
//!     - For each bill in the item: INSERT allocation (payment_run_id set).
//!     - UPDATE vendor_bills.status → 'paid' (full open balance allocated).
//!     - INSERT outbox event: ap.payment_executed.
//!   Completion:
//!     - UPDATE payment_runs.status → 'completed', executed_at = NOW().

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_ap_payment_executed_envelope, ApPaymentExecutedPayload,
    EVENT_TYPE_AP_PAYMENT_EXECUTED,
};
use crate::integrations::payments::{submit_payment, PaymentInstruction};
use crate::outbox::enqueue_event_tx;

use super::{PaymentRun, PaymentRunError, PaymentRunItemRow};

// ============================================================================
// Result types
// ============================================================================

/// Record of a single vendor payment within the run.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
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

/// Result returned by `execute_payment_run`.
#[derive(Debug)]
pub struct ExecuteResult {
    pub run: PaymentRun,
    pub executions: Vec<ExecutionRecord>,
}

// ============================================================================
// Public API
// ============================================================================

/// Execute a payment run: submit payments, record allocations, emit events.
///
/// Safe to call multiple times — fully idempotent:
/// - If `run_id` is already completed: returns the existing state.
/// - If `run_id` is executing (partial retry): re-processes only unfinished items.
/// - `allocation_id` is derived from `run_id + bill_id` (UUID v5), so re-inserting
///   would violate the UNIQUE constraint and be caught as idempotent duplication.
pub async fn execute_payment_run(
    pool: &PgPool,
    tenant_id: &str,
    run_id: Uuid,
) -> Result<ExecuteResult, PaymentRunError> {
    let mut tx = pool.begin().await?;

    // Guard: lock run row to prevent concurrent execution
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
    .fetch_optional(&mut *tx)
    .await?;

    let run = run.ok_or(PaymentRunError::RunNotFound(run_id))?;

    // Idempotency: already terminal — return existing state
    if run.status == "completed" || run.status == "failed" {
        let executions = fetch_executions(&mut *tx, run_id).await?;
        tx.commit().await?;
        return Ok(ExecuteResult { run, executions });
    }

    // Guard: only pending or executing (retry) are valid
    if run.status != "pending" && run.status != "executing" {
        return Err(PaymentRunError::RunNotPending(run.status.clone()));
    }

    // Transition to 'executing' (no-op if already executing)
    if run.status == "pending" {
        sqlx::query(
            "UPDATE payment_runs SET status = 'executing' WHERE run_id = $1 AND tenant_id = $2",
        )
        .bind(run_id)
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;
    }

    // Load all items
    let items: Vec<PaymentRunItemRow> = sqlx::query_as(
        "SELECT id, run_id, vendor_id, bill_ids, amount_minor, currency, created_at \
         FROM payment_run_items WHERE run_id = $1 ORDER BY id ASC",
    )
    .bind(run_id)
    .fetch_all(&mut *tx)
    .await?;

    let mut executions: Vec<ExecutionRecord> = Vec::with_capacity(items.len());

    for item in &items {
        // Idempotency: skip if already executed
        if let Some(exec) = fetch_execution_by_item(&mut *tx, run_id, item.id).await? {
            executions.push(exec);
            continue;
        }

        // Submit payment instruction to disbursement layer
        let payment_result = submit_payment(&PaymentInstruction {
            run_id,
            vendor_id: item.vendor_id,
            amount_minor: item.amount_minor,
            currency: item.currency.trim().to_string(),
            payment_method: run.payment_method.clone(),
            tenant_id: tenant_id.to_string(),
        });

        let mut bills_paid: Vec<Uuid> = Vec::new();
        let mut actual_amount: i64 = 0;

        for &bill_id in &item.bill_ids {
            let open_balance = query_open_balance(&mut *tx, tenant_id, bill_id).await?;
            if open_balance <= 0 {
                continue; // Already fully paid — skip
            }

            // Derive stable allocation_id from run_id + bill_id (UUID v5)
            let alloc_key = format!("{}:{}", run_id, bill_id);
            let allocation_id =
                Uuid::new_v5(&Uuid::NAMESPACE_OID, alloc_key.as_bytes());

            // Allocate the full open balance to this bill
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
            .bind(item.currency.trim())
            .execute(&mut *tx)
            .await?;

            // Transition bill to 'paid' (open_balance was the entire remainder)
            sqlx::query(
                "UPDATE vendor_bills SET status = 'paid' \
                 WHERE bill_id = $1 AND tenant_id = $2 \
                   AND status IN ('approved', 'partially_paid')",
            )
            .bind(bill_id)
            .bind(tenant_id)
            .execute(&mut *tx)
            .await?;

            actual_amount += open_balance;
            bills_paid.push(bill_id);
        }

        // Record execution
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
        .bind(item.id)
        .bind(payment_result.payment_id)
        .bind(item.vendor_id)
        .bind(actual_amount.max(0))
        .bind(item.currency.trim())
        .fetch_one(&mut *tx)
        .await?;

        // Emit ap.payment_executed event via outbox
        let payload = ApPaymentExecutedPayload {
            payment_id: payment_result.payment_id,
            run_id,
            tenant_id: tenant_id.to_string(),
            vendor_id: item.vendor_id,
            bill_ids: bills_paid,
            amount_minor: actual_amount.max(0),
            currency: item.currency.trim().to_string(),
            payment_method: run.payment_method.clone(),
            bank_reference: payment_result.bank_reference,
            bank_account_last4: None,
            executed_at: payment_result.executed_at,
        };

        let envelope = build_ap_payment_executed_envelope(
            Uuid::new_v4(),
            tenant_id.to_string(),
            run_id.to_string(),
            Some(run_id.to_string()),
            payload,
        );

        enqueue_event_tx(
            &mut tx,
            envelope.event_id,
            EVENT_TYPE_AP_PAYMENT_EXECUTED,
            "payment_run",
            &run_id.to_string(),
            &envelope,
        )
        .await?;

        executions.push(exec);
    }

    // Mark run as completed
    let completed_run: PaymentRun = sqlx::query_as(
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
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(ExecuteResult {
        run: completed_run,
        executions,
    })
}

// ============================================================================
// Private helpers
// ============================================================================

async fn fetch_executions(
    conn: &mut sqlx::PgConnection,
    run_id: Uuid,
) -> Result<Vec<ExecutionRecord>, PaymentRunError> {
    let rows: Vec<ExecutionRecord> = sqlx::query_as(
        "SELECT id, run_id, item_id, payment_id, vendor_id, amount_minor, \
                currency, status, executed_at \
         FROM payment_run_executions WHERE run_id = $1 ORDER BY id ASC",
    )
    .bind(run_id)
    .fetch_all(conn)
    .await?;
    Ok(rows)
}

async fn fetch_execution_by_item(
    conn: &mut sqlx::PgConnection,
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
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

async fn query_open_balance(
    conn: &mut sqlx::PgConnection,
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

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::bills::models::test_fixtures::{
        cleanup, create_bill_with_line, create_vendor, make_pool,
    };
    use crate::domain::payment_runs::builder::create_payment_run;
    use crate::domain::payment_runs::{CreatePaymentRunRequest, PaymentRunError};
    use chrono::Utc;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-execute";

    fn run_req(run_id: Uuid) -> CreatePaymentRunRequest {
        CreatePaymentRunRequest {
            run_id,
            currency: "USD".to_string(),
            scheduled_date: Utc::now() + chrono::Duration::days(1),
            payment_method: "ach".to_string(),
            created_by: "user-1".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: None,
        }
    }

    async fn cleanup_all(db: &PgPool) {
        for q in [
            "DELETE FROM payment_run_executions WHERE run_id IN \
             (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
            "DELETE FROM ap_allocations WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM payment_run_items WHERE run_id IN \
             (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
            "DELETE FROM payment_runs WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(db).await.ok();
        }
        cleanup(db, TEST_TENANT).await;
    }

    async fn setup_run(db: &PgPool) -> (Uuid, Uuid) {
        let vendor_id = create_vendor(db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(db, TEST_TENANT, vendor_id, "approved").await;
        let run_id = Uuid::new_v4();
        create_payment_run(db, TEST_TENANT, &run_req(run_id))
            .await
            .expect("create run");
        (run_id, bill_id)
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_completes_run_and_marks_bills_paid() {
        let db = make_pool().await;
        cleanup_all(&db).await;

        let (run_id, bill_id) = setup_run(&db).await;

        let result = execute_payment_run(&db, TEST_TENANT, run_id)
            .await
            .expect("execute run");

        assert_eq!(result.run.status, "completed");
        assert!(result.run.executed_at.is_some());
        assert_eq!(result.executions.len(), 1);
        assert_eq!(result.executions[0].status, "success");

        // Bill must be 'paid'
        let (status,): (String,) =
            sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1")
                .bind(bill_id)
                .fetch_one(&db)
                .await
                .expect("fetch status");
        assert_eq!(status, "paid");

        cleanup_all(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_creates_allocation_with_run_id() {
        let db = make_pool().await;
        cleanup_all(&db).await;

        let (run_id, bill_id) = setup_run(&db).await;
        execute_payment_run(&db, TEST_TENANT, run_id)
            .await
            .expect("execute");

        let (payment_run_id,): (Option<Uuid>,) = sqlx::query_as(
            "SELECT payment_run_id FROM ap_allocations WHERE bill_id = $1 AND tenant_id = $2",
        )
        .bind(bill_id)
        .bind(TEST_TENANT)
        .fetch_one(&db)
        .await
        .expect("fetch allocation");

        assert_eq!(payment_run_id, Some(run_id), "allocation must reference run_id");

        cleanup_all(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_emits_payment_executed_event() {
        let db = make_pool().await;
        cleanup_all(&db).await;

        let (run_id, _) = setup_run(&db).await;
        execute_payment_run(&db, TEST_TENANT, run_id)
            .await
            .expect("execute");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox \
             WHERE event_type = $1 AND aggregate_id = $2",
        )
        .bind(EVENT_TYPE_AP_PAYMENT_EXECUTED)
        .bind(run_id.to_string())
        .fetch_one(&db)
        .await
        .expect("count outbox");

        assert_eq!(count, 1, "ap.payment_executed must be in outbox");

        cleanup_all(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_idempotent_second_call_returns_same_state() {
        let db = make_pool().await;
        cleanup_all(&db).await;

        let (run_id, _) = setup_run(&db).await;

        let r1 = execute_payment_run(&db, TEST_TENANT, run_id)
            .await
            .expect("first execute");
        let r2 = execute_payment_run(&db, TEST_TENANT, run_id)
            .await
            .expect("second execute (idempotent)");

        assert_eq!(r1.run.status, r2.run.status);
        assert_eq!(r1.executions.len(), r2.executions.len());
        assert_eq!(
            r1.executions[0].payment_id,
            r2.executions[0].payment_id,
            "same payment_id on retry"
        );

        // Only one allocation per bill
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ap_allocations WHERE payment_run_id = $1",
        )
        .bind(run_id)
        .fetch_one(&db)
        .await
        .expect("count allocs");
        assert_eq!(count, 1, "idempotent: only one allocation per bill");

        cleanup_all(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_balance_reconciles_bill_total_minus_existing_allocations() {
        let db = make_pool().await;
        cleanup_all(&db).await;

        // Partially allocate the bill before run creation (20000 of 50000)
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "approved").await;
        sqlx::query(
            "INSERT INTO ap_allocations \
             (allocation_id, bill_id, tenant_id, amount_minor, currency, allocation_type, created_at) \
             VALUES ($1, $2, $3, 20000, 'USD', 'partial', NOW())",
        )
        .bind(Uuid::new_v4())
        .bind(bill_id)
        .bind(TEST_TENANT)
        .execute(&db)
        .await
        .expect("pre-alloc");
        sqlx::query(
            "UPDATE vendor_bills SET status = 'partially_paid' WHERE bill_id = $1",
        )
        .bind(bill_id)
        .execute(&db)
        .await
        .expect("update status");

        // Create run — open balance = 30000
        let run_id = Uuid::new_v4();
        create_payment_run(&db, TEST_TENANT, &run_req(run_id))
            .await
            .expect("create run");

        execute_payment_run(&db, TEST_TENANT, run_id)
            .await
            .expect("execute");

        // Execution allocation should be 30000 (the open balance)
        let (alloc_amount,): (i64,) = sqlx::query_as(
            "SELECT amount_minor FROM ap_allocations WHERE bill_id = $1 AND payment_run_id = $2",
        )
        .bind(bill_id)
        .bind(run_id)
        .fetch_one(&db)
        .await
        .expect("fetch alloc");
        assert_eq!(alloc_amount, 30000, "must allocate only the remaining open balance");

        cleanup_all(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_execute_not_found_returns_error() {
        let db = make_pool().await;
        cleanup_all(&db).await;

        let result = execute_payment_run(&db, TEST_TENANT, Uuid::new_v4()).await;
        assert!(
            matches!(result, Err(PaymentRunError::RunNotFound(_))),
            "non-existent run_id should return RunNotFound"
        );

        cleanup_all(&db).await;
    }
}
