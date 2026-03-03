//! Repository for RMA receipt records.
//!
//! Pure data access — no business logic. All queries filter by tenant_id.

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::rma::{RmaReceipt, RmaReceiptItem};

pub struct RmaRepo;

impl RmaRepo {
    // ── Receipt queries ──────────────────────────────────────

    pub async fn insert_receipt_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        rma_id: &str,
        customer_id: Uuid,
        condition_notes: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<RmaReceipt, sqlx::Error> {
        sqlx::query_as::<_, RmaReceipt>(
            r#"
            INSERT INTO rma_receipts (tenant_id, rma_id, customer_id, condition_notes, idempotency_key)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(rma_id)
        .bind(customer_id)
        .bind(condition_notes)
        .bind(idempotency_key)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn find_by_idempotency_key_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        idempotency_key: &str,
    ) -> Result<Option<RmaReceipt>, sqlx::Error> {
        sqlx::query_as::<_, RmaReceipt>(
            r#"
            SELECT * FROM rma_receipts
            WHERE tenant_id = $1 AND idempotency_key = $2
            "#,
        )
        .bind(tenant_id)
        .bind(idempotency_key)
        .fetch_optional(&mut **tx)
        .await
    }

    pub async fn get_receipt(
        pool: &PgPool,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<RmaReceipt>, sqlx::Error> {
        sqlx::query_as::<_, RmaReceipt>(
            "SELECT * FROM rma_receipts WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn get_for_update_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<RmaReceipt>, sqlx::Error> {
        sqlx::query_as::<_, RmaReceipt>(
            "SELECT * FROM rma_receipts WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await
    }

    pub async fn update_disposition_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        tenant_id: Uuid,
        disposition_status: &str,
    ) -> Result<RmaReceipt, sqlx::Error> {
        sqlx::query_as::<_, RmaReceipt>(
            r#"
            UPDATE rma_receipts
            SET disposition_status = $3, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(disposition_status)
        .fetch_one(&mut **tx)
        .await
    }

    // ── Item queries ─────────────────────────────────────────

    pub async fn insert_item_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        rma_receipt_id: Uuid,
        sku: &str,
        qty: i64,
        condition_notes: Option<&str>,
    ) -> Result<RmaReceiptItem, sqlx::Error> {
        sqlx::query_as::<_, RmaReceiptItem>(
            r#"
            INSERT INTO rma_receipt_items (tenant_id, rma_receipt_id, sku, qty, condition_notes)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(rma_receipt_id)
        .bind(sku)
        .bind(qty)
        .bind(condition_notes)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn get_items(
        pool: &PgPool,
        rma_receipt_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<RmaReceiptItem>, sqlx::Error> {
        sqlx::query_as::<_, RmaReceiptItem>(
            r#"
            SELECT * FROM rma_receipt_items
            WHERE rma_receipt_id = $1 AND tenant_id = $2
            ORDER BY created_at
            "#,
        )
        .bind(rma_receipt_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    }
}
