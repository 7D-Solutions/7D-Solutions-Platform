//! Allocation service — Guard → Mutation for append-only payment application.
//!
//! `apply_allocation`:
//!   Guard:    Lock bill row; verify status; check open balance vs requested amount.
//!             Idempotency: if allocation_id already exists, return existing record.
//!   Mutation: INSERT ap_allocations (append-only); UPDATE vendor_bills.status.
//!
//! `get_allocations`: List all allocations for a bill (ordered by creation time).
//! `get_bill_balance`:  Return remaining open balance for a bill.
//!
//! No outbox events at this layer — execution (payment disbursement) is handled
//! by the payment runs system and fires its own events.

use sqlx::PgPool;
use uuid::Uuid;

use super::{derive_bill_status, AllocationError, AllocationRecord, CreateAllocationRequest};

// ============================================================================
// Internal read helpers
// ============================================================================

#[derive(sqlx::FromRow)]
struct BillHeaderForAlloc {
    total_minor: i64,
    status: String,
}

// ============================================================================
// Public API
// ============================================================================

/// Apply a payment allocation to an approved or partially-paid bill.
///
/// Guard:
///   - Locks the bill row to prevent concurrent allocation races.
///   - Verifies bill status is 'approved' or 'partially_paid'.
///   - Computes remaining open balance; rejects if amount > open balance.
///   - Returns existing AllocationRecord if allocation_id already exists (idempotent).
///
/// Mutation (within the same transaction):
///   - INSERTs ap_allocations row (append-only; no UPDATE ever).
///   - Derives new bill status ('partially_paid' or 'paid') and updates vendor_bills.
pub async fn apply_allocation(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
    req: &CreateAllocationRequest,
) -> Result<AllocationRecord, AllocationError> {
    req.validate()?;

    let mut tx = pool.begin().await?;

    // Idempotency: return existing record if allocation_id already present
    let existing: Option<AllocationRecord> = sqlx::query_as(
        r#"
        SELECT id, allocation_id, bill_id, payment_run_id, tenant_id,
               amount_minor, currency, allocation_type, created_at
        FROM ap_allocations
        WHERE allocation_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(req.allocation_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(record) = existing {
        tx.commit().await?;
        return Ok(record);
    }

    // Guard: lock bill row
    let header: Option<BillHeaderForAlloc> = sqlx::query_as(
        r#"
        SELECT total_minor, status
        FROM vendor_bills
        WHERE bill_id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let header = header.ok_or(AllocationError::BillNotFound(bill_id))?;

    // Guard: only approved/partially_paid bills accept allocations
    if !matches!(header.status.as_str(), "approved" | "partially_paid") {
        return Err(AllocationError::InvalidBillStatus(header.status.clone()));
    }

    // Guard: compute open balance (sum of prior allocations)
    let (already_allocated,): (i64,) = sqlx::query_as(
        r#"
        SELECT COALESCE(SUM(amount_minor), 0)::bigint
        FROM ap_allocations
        WHERE bill_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    let open_balance = header.total_minor - already_allocated;

    if req.amount_minor > open_balance {
        return Err(AllocationError::OverAllocation {
            available: open_balance,
            requested: req.amount_minor,
        });
    }

    // Determine allocation_type for this record
    let new_allocated = already_allocated + req.amount_minor;
    let allocation_type = if new_allocated >= header.total_minor {
        "full"
    } else {
        "partial"
    };

    // Mutation: INSERT allocation (append-only)
    let record: AllocationRecord = sqlx::query_as(
        r#"
        INSERT INTO ap_allocations
            (allocation_id, bill_id, payment_run_id, tenant_id,
             amount_minor, currency, allocation_type, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        RETURNING id, allocation_id, bill_id, payment_run_id, tenant_id,
                  amount_minor, currency, allocation_type, created_at
        "#,
    )
    .bind(req.allocation_id)
    .bind(bill_id)
    .bind(req.payment_run_id)
    .bind(tenant_id)
    .bind(req.amount_minor)
    .bind(&req.currency)
    .bind(allocation_type)
    .fetch_one(&mut *tx)
    .await?;

    // Mutation: derive new bill status and update vendor_bills
    let new_status = derive_bill_status(header.total_minor, new_allocated);
    sqlx::query(
        r#"
        UPDATE vendor_bills
        SET status = $1
        WHERE bill_id = $2 AND tenant_id = $3
        "#,
    )
    .bind(new_status)
    .bind(bill_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(record)
}

/// List all allocations for a bill in insertion order.
pub async fn get_allocations(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
) -> Result<Vec<AllocationRecord>, AllocationError> {
    let rows: Vec<AllocationRecord> = sqlx::query_as(
        r#"
        SELECT id, allocation_id, bill_id, payment_run_id, tenant_id,
               amount_minor, currency, allocation_type, created_at
        FROM ap_allocations
        WHERE bill_id = $1 AND tenant_id = $2
        ORDER BY id ASC
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Return remaining open balance for a bill (total_minor - sum of allocations).
///
/// Returns None if the bill is not found for this tenant.
pub async fn get_bill_balance(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
) -> Result<Option<super::BillBalanceSummary>, AllocationError> {
    let header: Option<BillHeaderForAlloc> = sqlx::query_as(
        "SELECT total_minor, status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let Some(header) = header else {
        return Ok(None);
    };

    let (allocated,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(amount_minor), 0)::bigint \
         FROM ap_allocations WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(Some(super::BillBalanceSummary {
        bill_id,
        total_minor: header.total_minor,
        allocated_minor: allocated,
        open_balance_minor: header.total_minor - allocated,
        status: header.status,
    }))
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

    const TEST_TENANT: &str = "test-tenant-allocations";

    async fn make_approved_bill(db: &PgPool) -> Uuid {
        let vendor_id = create_vendor(db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(db, TEST_TENANT, vendor_id, "approved").await;
        bill_id
    }

    fn alloc_req(amount_minor: i64) -> CreateAllocationRequest {
        CreateAllocationRequest {
            allocation_id: Uuid::new_v4(),
            amount_minor,
            currency: "USD".to_string(),
            payment_run_id: None,
        }
    }

    async fn cleanup_allocations(db: &PgPool) {
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
    async fn test_full_allocation_marks_bill_paid() {
        let db = make_pool().await;
        cleanup_allocations(&db).await;
        let bill_id = make_approved_bill(&db).await;

        // Bill total_minor is 50000 (from test fixture)
        let result = apply_allocation(&db, TEST_TENANT, bill_id, &alloc_req(50000))
            .await
            .expect("full allocation");

        assert_eq!(result.allocation_type, "full");

        // Bill status should be 'paid'
        let (status,): (String,) =
            sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
                .bind(bill_id)
                .bind(TEST_TENANT)
                .fetch_one(&db)
                .await
                .expect("fetch status");
        assert_eq!(status, "paid");

        cleanup_allocations(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_partial_allocation_marks_bill_partially_paid() {
        let db = make_pool().await;
        cleanup_allocations(&db).await;
        let bill_id = make_approved_bill(&db).await;

        let result = apply_allocation(&db, TEST_TENANT, bill_id, &alloc_req(20000))
            .await
            .expect("partial allocation");

        assert_eq!(result.allocation_type, "partial");

        let (status,): (String,) =
            sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
                .bind(bill_id)
                .bind(TEST_TENANT)
                .fetch_one(&db)
                .await
                .expect("fetch status");
        assert_eq!(status, "partially_paid");

        cleanup_allocations(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_two_partial_allocations_sum_to_paid() {
        let db = make_pool().await;
        cleanup_allocations(&db).await;
        let bill_id = make_approved_bill(&db).await;

        apply_allocation(&db, TEST_TENANT, bill_id, &alloc_req(30000))
            .await
            .expect("first allocation");

        apply_allocation(&db, TEST_TENANT, bill_id, &alloc_req(20000))
            .await
            .expect("second allocation");

        let (status,): (String,) =
            sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
                .bind(bill_id)
                .bind(TEST_TENANT)
                .fetch_one(&db)
                .await
                .expect("fetch status");
        assert_eq!(status, "paid");

        cleanup_allocations(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_over_allocation_rejected() {
        let db = make_pool().await;
        cleanup_allocations(&db).await;
        let bill_id = make_approved_bill(&db).await;

        let result = apply_allocation(&db, TEST_TENANT, bill_id, &alloc_req(60000)).await;

        assert!(
            matches!(result, Err(AllocationError::OverAllocation { .. })),
            "over-allocation should be rejected, got {:?}",
            result
        );

        cleanup_allocations(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_duplicate_allocation_id_returns_existing() {
        let db = make_pool().await;
        cleanup_allocations(&db).await;
        let bill_id = make_approved_bill(&db).await;

        let req = alloc_req(20000);
        let first = apply_allocation(&db, TEST_TENANT, bill_id, &req)
            .await
            .expect("first");
        let second = apply_allocation(&db, TEST_TENANT, bill_id, &req)
            .await
            .expect("second (idempotent)");

        assert_eq!(first.allocation_id, second.allocation_id);
        assert_eq!(first.id, second.id);

        // Only one allocation row should exist
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ap_allocations WHERE bill_id = $1 AND tenant_id = $2",
        )
        .bind(bill_id)
        .bind(TEST_TENANT)
        .fetch_one(&db)
        .await
        .expect("count");
        assert_eq!(count, 1, "idempotent: only one allocation row");

        cleanup_allocations(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_allocation_on_invalid_status_rejected() {
        let db = make_pool().await;
        cleanup_allocations(&db).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "open").await;

        let result = apply_allocation(&db, TEST_TENANT, bill_id, &alloc_req(10000)).await;

        assert!(
            matches!(result, Err(AllocationError::InvalidBillStatus(_))),
            "open bill should not accept allocations, got {:?}",
            result
        );

        cleanup_allocations(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_wrong_tenant_returns_not_found() {
        let db = make_pool().await;
        cleanup_allocations(&db).await;
        let bill_id = make_approved_bill(&db).await;

        let result = apply_allocation(&db, "wrong-tenant", bill_id, &alloc_req(10000)).await;

        assert!(
            matches!(result, Err(AllocationError::BillNotFound(_))),
            "wrong tenant should return not found, got {:?}",
            result
        );

        cleanup_allocations(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_get_allocations_returns_in_insertion_order() {
        let db = make_pool().await;
        cleanup_allocations(&db).await;
        let bill_id = make_approved_bill(&db).await;

        apply_allocation(&db, TEST_TENANT, bill_id, &alloc_req(10000))
            .await
            .expect("first");
        apply_allocation(&db, TEST_TENANT, bill_id, &alloc_req(20000))
            .await
            .expect("second");

        let allocations = get_allocations(&db, TEST_TENANT, bill_id)
            .await
            .expect("get_allocations");

        assert_eq!(allocations.len(), 2);
        assert_eq!(allocations[0].amount_minor, 10000);
        assert_eq!(allocations[1].amount_minor, 20000);

        cleanup_allocations(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_get_bill_balance_reflects_allocations() {
        let db = make_pool().await;
        cleanup_allocations(&db).await;
        let bill_id = make_approved_bill(&db).await;

        let before = get_bill_balance(&db, TEST_TENANT, bill_id)
            .await
            .expect("before balance")
            .expect("bill exists");
        assert_eq!(before.open_balance_minor, 50000);

        apply_allocation(&db, TEST_TENANT, bill_id, &alloc_req(20000))
            .await
            .expect("allocation");

        let after = get_bill_balance(&db, TEST_TENANT, bill_id)
            .await
            .expect("after balance")
            .expect("bill exists");
        assert_eq!(after.open_balance_minor, 30000);
        assert_eq!(after.allocated_minor, 20000);

        cleanup_allocations(&db).await;
    }
}
