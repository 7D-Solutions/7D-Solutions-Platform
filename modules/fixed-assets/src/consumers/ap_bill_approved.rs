//! Fixed Assets consumer for ap.vendor_bill_approved events.
//!
//! Ingests AP bill approval events and creates asset records for any bill line
//! whose `gl_account_code` maps to an active `fa_category.asset_account_ref`.
//! Non-capex lines (no matching category) are skipped without error.
//!
//! ## Idempotency
//! Delegates to `capitalize_from_ap_line` which uses
//! `INSERT … UNIQUE (tenant_id, bill_id, line_id)` to guarantee exactly-once
//! asset creation under replay.
//!
//! ## No AP DB writes
//! This consumer reads only the fixed-assets DB. It never writes to AP.
//!
//! ## NATS Subject
//! Subscribes to `ap.events.ap.vendor_bill_approved` — the subject the AP
//! outbox publisher constructs as `ap.events.{event_type}`.

use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::capitalize::service::{capitalize_from_ap_line, CapitalizeFromApLineRequest};

// ============================================================================
// Local payload mirror (anti-corruption layer)
// Mirrors ap::events::bill::VendorBillApprovedPayload +
//         ap::events::vendor_bill_approved::ApprovedGlLine
// ============================================================================

/// Mirror of ap::events::vendor_bill_approved::ApprovedGlLine.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ApprovedGlLine {
    pub line_id: Uuid,
    pub gl_account_code: String,
    pub amount_minor: i64,
    pub po_line_id: Option<Uuid>,
}

/// Mirror of ap::events::bill::VendorBillApprovedPayload.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VendorBillApprovedPayload {
    pub bill_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    pub vendor_invoice_ref: String,
    pub approved_amount_minor: i64,
    pub currency: String,
    pub due_date: DateTime<Utc>,
    pub approved_by: String,
    pub approved_at: DateTime<Utc>,
    pub fx_rate_id: Option<Uuid>,
    pub gl_lines: Vec<ApprovedGlLine>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process a single ap.vendor_bill_approved event payload.
///
/// Iterates each GL line; for each capex line (category found for gl_account_code)
/// creates an asset and records the linkage. Non-capex lines are skipped.
/// Idempotent: safe to replay the same event multiple times.
pub async fn handle_bill_approved(
    pool: &PgPool,
    _event_id: Uuid,
    payload: &VendorBillApprovedPayload,
) -> Result<(), sqlx::Error> {
    let acquisition_date = payload.approved_at.date_naive();

    for line in &payload.gl_lines {
        let req = CapitalizeFromApLineRequest {
            tenant_id: payload.tenant_id.clone(),
            bill_id: payload.bill_id,
            line_id: line.line_id,
            gl_account_code: line.gl_account_code.clone(),
            amount_minor: line.amount_minor,
            currency: payload.currency.clone(),
            acquisition_date,
            vendor_invoice_ref: payload.vendor_invoice_ref.clone(),
        };

        match capitalize_from_ap_line(pool, &req).await? {
            Some(result) => {
                tracing::info!(
                    bill_id = %payload.bill_id,
                    line_id = %line.line_id,
                    asset_id = %result.asset_id,
                    gl_account_code = %line.gl_account_code,
                    "FA consumer: capitalized AP bill line"
                );
            }
            None => {
                tracing::debug!(
                    bill_id = %payload.bill_id,
                    line_id = %line.line_id,
                    gl_account_code = %line.gl_account_code,
                    "FA consumer: skipped line (non-capex or already processed)"
                );
            }
        }
    }

    Ok(())
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the Fixed Assets AP bill approved consumer task.
///
/// Subscribes to `ap.events.ap.vendor_bill_approved` and creates assets for
/// capex bill lines via `handle_bill_approved`. Idempotent on redelivery.
pub async fn start_ap_bill_approved_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "ap.events.ap.vendor_bill_approved";
        tracing::info!(subject, "FA: starting AP bill approved consumer");

        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "FA: failed to subscribe");
                return;
            }
        };

        tracing::info!(subject, "FA: subscribed to AP bill approved events");

        while let Some(msg) = stream.next().await {
            let pool_ref = pool.clone();
            if let Err(e) = process_bill_approved_message(&pool_ref, &msg).await {
                tracing::error!(error = %e, "FA: failed to process ap.vendor_bill_approved");
            }
        }

        tracing::warn!("FA: ap.vendor_bill_approved consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_bill_approved_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let envelope: EventEnvelope<VendorBillApprovedPayload> =
        serde_json::from_slice(&msg.payload)
            .map_err(|e| format!("Failed to parse ap.vendor_bill_approved envelope: {}", e))?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        bill_id = %envelope.payload.bill_id,
        gl_lines_count = envelope.payload.gl_lines.len(),
        "FA: processing ap.vendor_bill_approved"
    );

    handle_bill_approved(pool, envelope.event_id, &envelope.payload)
        .await
        .map_err(|e| format!("handle_bill_approved failed: {}", e).into())
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-ap-consumer";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db?sslmode=require"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to FA test DB")
    }

    async fn setup_capex_category(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO fa_categories
                (id, tenant_id, code, name,
                 default_method, default_useful_life_months, default_salvage_pct_bp,
                 asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
                 is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, 'straight_line', 60, 0, '1500', '6100', '1510',
                    TRUE, NOW(), NOW())
            "#,
        )
        .bind(id)
        .bind(TEST_TENANT)
        .bind(format!("IT-{}", &id.to_string()[..8]))
        .bind(format!("IT Equipment-{}", &id.to_string()[..8]))
        .execute(pool)
        .await
        .expect("insert category");
        id
    }

    async fn cleanup(pool: &PgPool) {
        for q in [
            "DELETE FROM fa_ap_capitalizations WHERE tenant_id = $1",
            "DELETE FROM fa_events_outbox WHERE tenant_id = $1",
            "DELETE FROM fa_assets WHERE tenant_id = $1",
            "DELETE FROM fa_categories WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(pool).await.ok();
        }
    }

    fn sample_payload(bill_id: Uuid, lines: Vec<ApprovedGlLine>) -> VendorBillApprovedPayload {
        VendorBillApprovedPayload {
            bill_id,
            tenant_id: TEST_TENANT.to_string(),
            vendor_id: Uuid::new_v4(),
            vendor_invoice_ref: format!("INV-{}", &bill_id.to_string()[..8]),
            approved_amount_minor: lines.iter().map(|l| l.amount_minor).sum(),
            currency: "USD".to_string(),
            due_date: Utc::now(),
            approved_by: "approver-1".to_string(),
            approved_at: Utc::now(),
            fx_rate_id: None,
            gl_lines: lines,
        }
    }

    fn capex_line(amount: i64) -> ApprovedGlLine {
        ApprovedGlLine {
            line_id: Uuid::new_v4(),
            gl_account_code: "1500".to_string(),
            amount_minor: amount,
            po_line_id: None,
        }
    }

    fn expense_line(amount: i64) -> ApprovedGlLine {
        ApprovedGlLine {
            line_id: Uuid::new_v4(),
            gl_account_code: "6200".to_string(), // expense — no category maps to this
            amount_minor: amount,
            po_line_id: None,
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_capex_line_creates_asset() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat = setup_capex_category(&pool).await;

        let bill_id = Uuid::new_v4();
        let line = capex_line(250_000);
        let line_id = line.line_id;
        let payload = sample_payload(bill_id, vec![line]);

        handle_bill_approved(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("handle_bill_approved failed");

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM fa_assets WHERE tenant_id = $1")
                .bind(TEST_TENANT)
                .fetch_one(&pool)
                .await
                .expect("asset count");
        assert_eq!(count, 1, "one asset must be created for the capex line");

        let (link_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM fa_ap_capitalizations \
             WHERE bill_id = $1 AND line_id = $2",
        )
        .bind(bill_id)
        .bind(line_id)
        .fetch_one(&pool)
        .await
        .expect("linkage count");
        assert_eq!(link_count, 1, "linkage must be stored");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_expense_line_skipped() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat = setup_capex_category(&pool).await;

        let payload = sample_payload(Uuid::new_v4(), vec![expense_line(50_000)]);

        handle_bill_approved(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("handle failed");

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM fa_assets WHERE tenant_id = $1")
                .bind(TEST_TENANT)
                .fetch_one(&pool)
                .await
                .expect("asset count");
        assert_eq!(count, 0, "no asset for expense line");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_mixed_lines_only_capex_capitalized() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat = setup_capex_category(&pool).await;

        let bill_id = Uuid::new_v4();
        let capex = capex_line(100_000);
        let expense = expense_line(30_000);
        let payload = sample_payload(bill_id, vec![capex, expense]);

        handle_bill_approved(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("handle failed");

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM fa_assets WHERE tenant_id = $1")
                .bind(TEST_TENANT)
                .fetch_one(&pool)
                .await
                .expect("asset count");
        assert_eq!(count, 1, "only capex line produces an asset");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_on_replay() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat = setup_capex_category(&pool).await;

        let bill_id = Uuid::new_v4();
        let payload = sample_payload(bill_id, vec![capex_line(75_000)]);

        handle_bill_approved(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("first handle failed");

        handle_bill_approved(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("second handle (replay) failed");

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM fa_assets WHERE tenant_id = $1")
                .bind(TEST_TENANT)
                .fetch_one(&pool)
                .await
                .expect("asset count");
        assert_eq!(count, 1, "replay must not create duplicate assets");

        let (link_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM fa_ap_capitalizations WHERE tenant_id = $1")
                .bind(TEST_TENANT)
                .fetch_one(&pool)
                .await
                .expect("link count");
        assert_eq!(
            link_count, 1,
            "replay must not create duplicate linkage rows"
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_multi_capex_lines_all_capitalized() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat = setup_capex_category(&pool).await;

        let bill_id = Uuid::new_v4();
        let lines = vec![capex_line(100_000), capex_line(200_000)];
        let payload = sample_payload(bill_id, lines);

        handle_bill_approved(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("handle failed");

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM fa_assets WHERE tenant_id = $1")
                .bind(TEST_TENANT)
                .fetch_one(&pool)
                .await
                .expect("asset count");
        assert_eq!(count, 2, "two capex lines produce two assets");

        cleanup(&pool).await;
    }
}
