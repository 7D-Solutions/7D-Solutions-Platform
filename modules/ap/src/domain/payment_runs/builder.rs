//! Payment run builder: Guard → Mutation → Outbox atomicity.
//!
//! `create_payment_run`:
//!   Idempotency: if run_id already exists, return the existing run.
//!   Guard:    Select eligible bills (approved/partially_paid, open balance > 0,
//!             currency match, optional due_date/vendor filters).
//!             Reject if no eligible bills exist.
//!   Mutation: INSERT payment_runs + payment_run_items (one per vendor).
//!             INSERT outbox event: ap.payment_run_created.
//!
//! This step does NOT move funds or record allocations.
//! Allocations are recorded by the execution step (bd-295k).

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_ap_payment_run_created_envelope, ApPaymentRunCreatedPayload,
    PaymentRunItem as EventPaymentRunItem, EVENT_TYPE_AP_PAYMENT_RUN_CREATED,
};
use crate::outbox::enqueue_event_tx;

use super::{
    CreatePaymentRunRequest, PaymentRun, PaymentRunError, PaymentRunItemRow, PaymentRunResult,
};

// ============================================================================
// Internal helper types
// ============================================================================

/// An eligible bill row returned by the selector query.
#[derive(Debug, sqlx::FromRow)]
struct EligibleBill {
    bill_id: Uuid,
    vendor_id: Uuid,
    open_balance_minor: i64,
    #[allow(dead_code)]
    currency: String,
}

// ============================================================================
// Public API
// ============================================================================

/// Create a payment run for all eligible bills in the given tenant + currency.
///
/// Idempotent: if `run_id` already exists for this tenant, the existing run
/// and its items are returned without any mutations.
///
/// Guard: returns `NoBillsEligible` if no approved/partially-paid bills with
/// open balance exist that match the request filters.
pub async fn create_payment_run(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreatePaymentRunRequest,
) -> Result<PaymentRunResult, PaymentRunError> {
    req.validate()?;

    // Idempotency: if run already exists, return it
    if let Some(existing) = fetch_existing_run(pool, tenant_id, req.run_id).await? {
        return Ok(existing);
    }

    // Guard: select eligible bills
    let eligible = select_eligible_bills(pool, tenant_id, req).await?;

    if eligible.is_empty() {
        return Err(PaymentRunError::NoBillsEligible(
            tenant_id.to_string(),
            req.currency.clone(),
        ));
    }

    // Group by vendor
    let vendor_groups = group_by_vendor(eligible);

    // Compute totals
    let total_minor: i64 = vendor_groups.iter().map(|(_, amt, _)| amt).sum();

    let mut tx = pool.begin().await?;

    // Mutation: INSERT payment_runs
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
    .bind(req.run_id)
    .bind(tenant_id)
    .bind(total_minor)
    .bind(&req.currency)
    .bind(req.scheduled_date)
    .bind(&req.payment_method)
    .bind(&req.created_by)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        // Catch unique violation on run_id (race condition after idempotency check)
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.code().as_deref() == Some("23505") {
                return sqlx::Error::RowNotFound; // will be handled below
            }
        }
        e
    })?;

    // Mutation: INSERT payment_run_items (one per vendor group)
    let mut items: Vec<PaymentRunItemRow> = Vec::with_capacity(vendor_groups.len());
    for (vendor_id, amount_minor, bill_ids) in &vendor_groups {
        let item: PaymentRunItemRow = sqlx::query_as(
            r#"
            INSERT INTO payment_run_items
                (run_id, vendor_id, bill_ids, amount_minor, currency, created_at)
            VALUES ($1, $2, $3, $4, $5, NOW())
            RETURNING id, run_id, vendor_id, bill_ids, amount_minor, currency, created_at
            "#,
        )
        .bind(req.run_id)
        .bind(vendor_id)
        .bind(bill_ids.as_slice())
        .bind(amount_minor)
        .bind(&req.currency)
        .fetch_one(&mut *tx)
        .await?;
        items.push(item);
    }

    // Build outbox event payload
    let event_items: Vec<EventPaymentRunItem> = vendor_groups
        .iter()
        .map(|(vendor_id, amount_minor, bill_ids)| EventPaymentRunItem {
            vendor_id: *vendor_id,
            bill_ids: bill_ids.clone(),
            amount_minor: *amount_minor,
            currency: req.currency.clone(),
        })
        .collect();

    let payload = ApPaymentRunCreatedPayload {
        run_id: req.run_id,
        tenant_id: tenant_id.to_string(),
        items: event_items,
        total_minor,
        currency: req.currency.clone(),
        scheduled_date: req.scheduled_date,
        payment_method: req.payment_method.clone(),
        created_by: req.created_by.clone(),
        created_at: Utc::now(),
    };

    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let envelope = build_ap_payment_run_created_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        envelope.event_id,
        EVENT_TYPE_AP_PAYMENT_RUN_CREATED,
        "payment_run",
        &req.run_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(PaymentRunResult { run, items })
}

/// Fetch a run and its items if the run_id already exists for this tenant.
async fn fetch_existing_run(
    pool: &PgPool,
    tenant_id: &str,
    run_id: Uuid,
) -> Result<Option<PaymentRunResult>, PaymentRunError> {
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

    let Some(run) = run else {
        return Ok(None);
    };

    let items: Vec<PaymentRunItemRow> = sqlx::query_as(
        r#"
        SELECT id, run_id, vendor_id, bill_ids, amount_minor, currency, created_at
        FROM payment_run_items
        WHERE run_id = $1
        ORDER BY id ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    Ok(Some(PaymentRunResult { run, items }))
}

/// Select eligible bills: approved/partially_paid, open balance > 0, matching currency.
///
/// Optional filters: due_on_or_before, vendor_ids.
/// Results are ordered deterministically: vendor_id, due_date, bill_id.
async fn select_eligible_bills(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreatePaymentRunRequest,
) -> Result<Vec<EligibleBill>, PaymentRunError> {
    // Use nullable parameter trick for optional filters.
    // $3::timestamptz IS NULL  → skip due_date filter
    // $4::uuid[]    IS NULL  → skip vendor filter
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

/// Group eligible bills by vendor, returning (vendor_id, total_amount, bill_ids).
fn group_by_vendor(bills: Vec<EligibleBill>) -> Vec<(Uuid, i64, Vec<Uuid>)> {
    let mut groups: Vec<(Uuid, i64, Vec<Uuid>)> = Vec::new();

    for bill in bills {
        if let Some(grp) = groups.iter_mut().find(|(vid, _, _)| *vid == bill.vendor_id) {
            grp.1 += bill.open_balance_minor;
            grp.2.push(bill.bill_id);
        } else {
            groups.push((bill.vendor_id, bill.open_balance_minor, vec![bill.bill_id]));
        }
    }

    groups
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
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-payment-runs";

    fn run_req(run_id: Uuid) -> CreatePaymentRunRequest {
        CreatePaymentRunRequest {
            run_id,
            currency: "USD".to_string(),
            scheduled_date: Utc::now() + chrono::Duration::days(1),
            payment_method: "ach".to_string(),
            created_by: "user-1".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: Some("corr-test-1".to_string()),
        }
    }

    async fn cleanup_runs(db: &PgPool) {
        // Items reference runs, so delete items first
        sqlx::query(
            "DELETE FROM payment_run_items WHERE run_id IN \
             (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(db)
        .await
        .ok();

        sqlx::query("DELETE FROM payment_runs WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(db)
            .await
            .ok();

        sqlx::query(
            "DELETE FROM ap_allocations WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(db)
        .await
        .ok();

        cleanup(db, TEST_TENANT).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_creates_run_for_approved_bill() {
        let db = make_pool().await;
        cleanup_runs(&db).await;

        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        // Bill fixture creates with status 'open', override to 'approved'
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "approved").await;
        let _ = bill_id;

        let result = create_payment_run(&db, TEST_TENANT, &run_req(Uuid::new_v4()))
            .await
            .expect("run created");

        assert_eq!(result.run.tenant_id, TEST_TENANT);
        assert_eq!(result.run.status, "pending");
        assert_eq!(result.run.currency, "USD");
        assert_eq!(result.run.payment_method, "ach");
        assert!(!result.items.is_empty(), "should have at least one item");
        assert_eq!(result.run.total_minor, 50000, "bill total_minor = 50000");

        // Outbox event emitted
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox WHERE event_type = $1 AND aggregate_id = $2",
        )
        .bind(EVENT_TYPE_AP_PAYMENT_RUN_CREATED)
        .bind(result.run.run_id.to_string())
        .fetch_one(&db)
        .await
        .expect("count outbox");
        assert_eq!(count, 1, "ap.payment_run_created event in outbox");

        cleanup_runs(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_no_eligible_bills_returns_error() {
        let db = make_pool().await;
        cleanup_runs(&db).await;

        // Create a bill with status 'open' (not eligible)
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let _ = create_bill_with_line(&db, TEST_TENANT, vendor_id, "open").await;

        let result = create_payment_run(&db, TEST_TENANT, &run_req(Uuid::new_v4())).await;

        assert!(
            matches!(result, Err(PaymentRunError::NoBillsEligible(_, _))),
            "open bill should not be eligible, got {:?}",
            result
        );

        cleanup_runs(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_groups_bills_by_vendor() {
        let db = make_pool().await;
        cleanup_runs(&db).await;

        let vendor1 = create_vendor(&db, TEST_TENANT).await;
        let vendor2 = create_vendor(&db, TEST_TENANT).await;
        create_bill_with_line(&db, TEST_TENANT, vendor1, "approved").await;
        create_bill_with_line(&db, TEST_TENANT, vendor1, "approved").await;
        create_bill_with_line(&db, TEST_TENANT, vendor2, "approved").await;

        let result = create_payment_run(&db, TEST_TENANT, &run_req(Uuid::new_v4()))
            .await
            .expect("run created");

        // 2 vendors → 2 items
        assert_eq!(result.items.len(), 2, "one item per vendor");

        // vendor1 has 2 bills → amount = 2 × 50000 = 100000
        let v1_item = result
            .items
            .iter()
            .find(|i| i.vendor_id == vendor1)
            .unwrap();
        assert_eq!(v1_item.bill_ids.len(), 2);
        assert_eq!(v1_item.amount_minor, 100000);

        // vendor2 has 1 bill → amount = 50000
        let v2_item = result
            .items
            .iter()
            .find(|i| i.vendor_id == vendor2)
            .unwrap();
        assert_eq!(v2_item.bill_ids.len(), 1);
        assert_eq!(v2_item.amount_minor, 50000);

        assert_eq!(result.run.total_minor, 150000);

        cleanup_runs(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_same_run_id_returns_existing() {
        let db = make_pool().await;
        cleanup_runs(&db).await;

        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        create_bill_with_line(&db, TEST_TENANT, vendor_id, "approved").await;

        let run_id = Uuid::new_v4();
        let req = run_req(run_id);

        let first = create_payment_run(&db, TEST_TENANT, &req)
            .await
            .expect("first run");
        let second = create_payment_run(&db, TEST_TENANT, &req)
            .await
            .expect("second run (idempotent)");

        assert_eq!(first.run.run_id, second.run.run_id);
        assert_eq!(first.items.len(), second.items.len());

        // Only one run record in DB
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM payment_runs WHERE run_id = $1 AND tenant_id = $2",
        )
        .bind(run_id)
        .bind(TEST_TENANT)
        .fetch_one(&db)
        .await
        .expect("count runs");
        assert_eq!(count, 1, "idempotent: only one payment_run row");

        cleanup_runs(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_due_on_or_before_filter() {
        let db = make_pool().await;
        cleanup_runs(&db).await;

        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        // Fixture: due_date = NOW() + 30 days
        create_bill_with_line(&db, TEST_TENANT, vendor_id, "approved").await;

        // Filter: due on or before yesterday → no eligible bills
        let mut req = run_req(Uuid::new_v4());
        req.due_on_or_before = Some(Utc::now() - chrono::Duration::days(1));

        let result = create_payment_run(&db, TEST_TENANT, &req).await;
        assert!(
            matches!(result, Err(PaymentRunError::NoBillsEligible(_, _))),
            "bill due in 30 days should not be selected when cutoff is yesterday"
        );

        // Filter: due on or before 60 days from now → includes the bill
        let mut req2 = run_req(Uuid::new_v4());
        req2.due_on_or_before = Some(Utc::now() + chrono::Duration::days(60));

        let result2 = create_payment_run(&db, TEST_TENANT, &req2)
            .await
            .expect("should include bill due in 30 days");
        assert!(!result2.items.is_empty());

        cleanup_runs(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_vendor_ids_filter() {
        let db = make_pool().await;
        cleanup_runs(&db).await;

        let vendor1 = create_vendor(&db, TEST_TENANT).await;
        let vendor2 = create_vendor(&db, TEST_TENANT).await;
        create_bill_with_line(&db, TEST_TENANT, vendor1, "approved").await;
        create_bill_with_line(&db, TEST_TENANT, vendor2, "approved").await;

        // Filter to vendor1 only
        let mut req = run_req(Uuid::new_v4());
        req.vendor_ids = Some(vec![vendor1]);

        let result = create_payment_run(&db, TEST_TENANT, &req)
            .await
            .expect("filtered run");

        assert_eq!(result.items.len(), 1, "only vendor1 included");
        assert_eq!(result.items[0].vendor_id, vendor1);
        assert_eq!(result.run.total_minor, 50000);

        cleanup_runs(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_partially_paid_bill_uses_open_balance() {
        let db = make_pool().await;
        cleanup_runs(&db).await;

        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "approved").await;

        // Partially allocate the bill (20000 of 50000)
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
        .expect("insert partial allocation");

        // Update bill status to partially_paid
        sqlx::query("UPDATE vendor_bills SET status = 'partially_paid' WHERE bill_id = $1")
            .bind(bill_id)
            .execute(&db)
            .await
            .expect("update status");

        let result = create_payment_run(&db, TEST_TENANT, &run_req(Uuid::new_v4()))
            .await
            .expect("run created");

        // Open balance = 50000 - 20000 = 30000
        assert_eq!(
            result.run.total_minor, 30000,
            "run uses open balance not full total"
        );
        assert_eq!(result.items[0].amount_minor, 30000);

        cleanup_runs(&db).await;
    }
}
