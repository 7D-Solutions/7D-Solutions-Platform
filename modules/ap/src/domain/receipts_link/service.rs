//! Receipt link service: idempotent ingestion of goods receipts into AP.
//!
//! Idempotency strategy: INSERT … ON CONFLICT (po_line_id, receipt_id) DO NOTHING.
//! Replaying the same inventory.item_received event is safe — no duplicate rows.
//! No cross-module DB writes: only the AP database is mutated.

use sqlx::PgPool;
use uuid::Uuid;

use super::{IngestReceiptLinkRequest, ReceiptLinkError};

// ============================================================================
// Public API
// ============================================================================

/// Ingest a goods receipt link into AP's po_receipt_links table.
///
/// Idempotent: if a row already exists for (po_line_id, receipt_id), this is a no-op.
/// The caller must have resolved po_line_id and all other fields before calling.
pub async fn ingest_receipt_link(
    pool: &PgPool,
    req: &IngestReceiptLinkRequest,
) -> Result<(), ReceiptLinkError> {
    req.validate()?;

    sqlx::query(
        r#"
        INSERT INTO po_receipt_links
            (po_id, po_line_id, vendor_id, receipt_id,
             quantity_received, unit_of_measure, unit_price_minor,
             currency, gl_account_code, received_at, received_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (po_line_id, receipt_id) DO NOTHING
        "#,
    )
    .bind(req.po_id)
    .bind(req.po_line_id)
    .bind(req.vendor_id)
    .bind(req.receipt_id)
    .bind(req.quantity_received)
    .bind(&req.unit_of_measure)
    .bind(req.unit_price_minor)
    .bind(&req.currency)
    .bind(&req.gl_account_code)
    .bind(req.received_at)
    .bind(&req.received_by)
    .execute(pool)
    .await?;

    tracing::info!(
        po_id = %req.po_id,
        po_line_id = %req.po_line_id,
        receipt_id = %req.receipt_id,
        "AP: receipt link ingested (or already present)"
    );

    Ok(())
}

/// Count receipt links for a PO line (used for 3-way match readiness checks).
pub async fn count_receipt_links_for_line(
    pool: &PgPool,
    tenant_id: &str,
    po_line_id: Uuid,
) -> Result<i64, ReceiptLinkError> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM po_receipt_links \
         WHERE po_line_id = $1 \
           AND po_id IN (SELECT po_id FROM purchase_orders WHERE tenant_id = $2)",
    )
    .bind(po_line_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-receipt-svc";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to AP test DB")
    }

    /// Insert minimal vendor → PO → PO line fixtures; returns (vendor_id, po_id, line_id).
    async fn setup_fixtures(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days,
               is_active, created_at, updated_at)
               VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())"#,
        )
        .bind(vendor_id)
        .bind(TEST_TENANT)
        .bind(format!("Vendor-{}", vendor_id))
        .execute(pool)
        .await
        .expect("insert vendor failed");

        let po_id = Uuid::new_v4();
        let po_number = format!("PO-{}", &po_id.to_string()[..8]);
        sqlx::query(
            r#"INSERT INTO purchase_orders
               (po_id, tenant_id, vendor_id, po_number, currency,
                total_minor, status, created_by, created_at)
               VALUES ($1, $2, $3, $4, 'USD', 10000, 'approved', 'system', NOW())"#,
        )
        .bind(po_id)
        .bind(TEST_TENANT)
        .bind(vendor_id)
        .bind(&po_number)
        .execute(pool)
        .await
        .expect("insert PO failed");

        let line_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO po_lines
               (line_id, po_id, description, quantity, unit_of_measure,
                unit_price_minor, line_total_minor, gl_account_code, created_at)
               VALUES ($1, $2, 'Widgets', 10.0, 'each', 1000, 10000, '6100', NOW())"#,
        )
        .bind(line_id)
        .bind(po_id)
        .execute(pool)
        .await
        .expect("insert PO line failed");

        (vendor_id, po_id, line_id)
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM po_receipt_links WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM po_lines WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM po_status WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'po' \
             AND aggregate_id IN \
             (SELECT po_id::TEXT FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query("DELETE FROM purchase_orders WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();

        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
             AND aggregate_id IN \
             (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();
    }

    fn sample_req(
        vendor_id: Uuid,
        po_id: Uuid,
        line_id: Uuid,
        receipt_id: Uuid,
    ) -> IngestReceiptLinkRequest {
        IngestReceiptLinkRequest {
            po_id,
            po_line_id: line_id,
            vendor_id,
            receipt_id,
            quantity_received: 5.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 1000,
            currency: "USD".to_string(),
            gl_account_code: "6100".to_string(),
            received_at: Utc::now(),
            received_by: "system:inventory-consumer".to_string(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_ingest_receipt_link_persists_row() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (vendor_id, po_id, line_id) = setup_fixtures(&pool).await;
        let receipt_id = Uuid::new_v4();

        ingest_receipt_link(&pool, &sample_req(vendor_id, po_id, line_id, receipt_id))
            .await
            .expect("ingest failed");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM po_receipt_links \
             WHERE po_id = $1 AND po_line_id = $2 AND receipt_id = $3 \
             AND po_id IN (SELECT po_id FROM purchase_orders WHERE tenant_id = $4)",
        )
        .bind(po_id)
        .bind(line_id)
        .bind(receipt_id)
        .bind(TEST_TENANT)
        .fetch_one(&pool)
        .await
        .expect("count query failed");

        assert_eq!(count, 1);
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_ingest_receipt_link_is_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (vendor_id, po_id, line_id) = setup_fixtures(&pool).await;
        let receipt_id = Uuid::new_v4();
        let req = sample_req(vendor_id, po_id, line_id, receipt_id);

        ingest_receipt_link(&pool, &req)
            .await
            .expect("first ingest failed");
        ingest_receipt_link(&pool, &req)
            .await
            .expect("second ingest must not error");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM po_receipt_links WHERE po_line_id = $1 AND receipt_id = $2 \
             AND po_id IN (SELECT po_id FROM purchase_orders WHERE tenant_id = $3)",
        )
        .bind(line_id)
        .bind(receipt_id)
        .bind(TEST_TENANT)
        .fetch_one(&pool)
        .await
        .expect("count query failed");

        assert_eq!(count, 1, "idempotent ingest must not create duplicate rows");
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_different_receipt_ids_create_separate_rows() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (vendor_id, po_id, line_id) = setup_fixtures(&pool).await;

        ingest_receipt_link(
            &pool,
            &sample_req(vendor_id, po_id, line_id, Uuid::new_v4()),
        )
        .await
        .expect("first ingest failed");
        ingest_receipt_link(
            &pool,
            &sample_req(vendor_id, po_id, line_id, Uuid::new_v4()),
        )
        .await
        .expect("second ingest failed");

        let count = count_receipt_links_for_line(&pool, TEST_TENANT, line_id)
            .await
            .expect("count failed");
        assert_eq!(count, 2);
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_ingest_stores_correct_fields() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (vendor_id, po_id, line_id) = setup_fixtures(&pool).await;
        let receipt_id = Uuid::new_v4();
        let req = sample_req(vendor_id, po_id, line_id, receipt_id);

        ingest_receipt_link(&pool, &req)
            .await
            .expect("ingest failed");

        let row: (Uuid, f64, String, i64, String, String) = sqlx::query_as(
            "SELECT vendor_id, quantity_received::FLOAT8, unit_of_measure, \
             unit_price_minor, currency, received_by \
             FROM po_receipt_links WHERE po_line_id = $1 AND receipt_id = $2",
        )
        .bind(line_id)
        .bind(receipt_id)
        .fetch_one(&pool)
        .await
        .expect("fetch failed");

        assert_eq!(row.0, vendor_id);
        assert!((row.1 - 5.0).abs() < 1e-6);
        assert_eq!(row.2, "each");
        assert_eq!(row.3, 1000);
        assert_eq!(row.4, "USD");
        assert_eq!(row.5, "system:inventory-consumer");
        cleanup(&pool).await;
    }
}
