//! Repository for shipping document request records.
//!
//! Pure data access — no business logic. All queries filter by tenant_id.

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::shipping_docs::ShippingDocRequest;

pub struct ShippingDocRepo;

impl ShippingDocRepo {
    pub async fn insert_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        shipment_id: Uuid,
        doc_type: &str,
        payload_ref: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<ShippingDocRequest, sqlx::Error> {
        sqlx::query_as::<_, ShippingDocRequest>(
            r#"
            INSERT INTO sr_shipping_doc_requests (tenant_id, shipment_id, doc_type, payload_ref, idempotency_key)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(shipment_id)
        .bind(doc_type)
        .bind(payload_ref)
        .bind(idempotency_key)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn find_by_idempotency_key_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        idempotency_key: &str,
    ) -> Result<Option<ShippingDocRequest>, sqlx::Error> {
        sqlx::query_as::<_, ShippingDocRequest>(
            r#"
            SELECT * FROM sr_shipping_doc_requests
            WHERE tenant_id = $1 AND idempotency_key = $2
            "#,
        )
        .bind(tenant_id)
        .bind(idempotency_key)
        .fetch_optional(&mut **tx)
        .await
    }

    pub async fn get(
        pool: &PgPool,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<ShippingDocRequest>, sqlx::Error> {
        sqlx::query_as::<_, ShippingDocRequest>(
            "SELECT * FROM sr_shipping_doc_requests WHERE id = $1 AND tenant_id = $2",
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
    ) -> Result<Option<ShippingDocRequest>, sqlx::Error> {
        sqlx::query_as::<_, ShippingDocRequest>(
            "SELECT * FROM sr_shipping_doc_requests WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await
    }

    pub async fn update_status_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        tenant_id: Uuid,
        status: &str,
    ) -> Result<ShippingDocRequest, sqlx::Error> {
        sqlx::query_as::<_, ShippingDocRequest>(
            r#"
            UPDATE sr_shipping_doc_requests
            SET status = $3, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(status)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn list_by_shipment(
        pool: &PgPool,
        shipment_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<ShippingDocRequest>, sqlx::Error> {
        sqlx::query_as::<_, ShippingDocRequest>(
            r#"
            SELECT * FROM sr_shipping_doc_requests
            WHERE shipment_id = $1 AND tenant_id = $2
            ORDER BY created_at
            "#,
        )
        .bind(shipment_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    }
}
