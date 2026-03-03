//! Repository for carrier integration request records.
//!
//! Pure data access — no business logic. All queries filter by tenant_id.

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::carrier_requests::CarrierRequest;

pub struct CarrierRequestRepo;

impl CarrierRequestRepo {
    pub async fn insert_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        shipment_id: Uuid,
        request_type: &str,
        carrier_code: &str,
        payload: &serde_json::Value,
        idempotency_key: Option<&str>,
    ) -> Result<CarrierRequest, sqlx::Error> {
        sqlx::query_as::<_, CarrierRequest>(
            r#"
            INSERT INTO sr_carrier_requests (tenant_id, shipment_id, request_type, carrier_code, payload, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(shipment_id)
        .bind(request_type)
        .bind(carrier_code)
        .bind(payload)
        .bind(idempotency_key)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn find_by_idempotency_key_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        idempotency_key: &str,
    ) -> Result<Option<CarrierRequest>, sqlx::Error> {
        sqlx::query_as::<_, CarrierRequest>(
            r#"
            SELECT * FROM sr_carrier_requests
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
    ) -> Result<Option<CarrierRequest>, sqlx::Error> {
        sqlx::query_as::<_, CarrierRequest>(
            "SELECT * FROM sr_carrier_requests WHERE id = $1 AND tenant_id = $2",
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
    ) -> Result<Option<CarrierRequest>, sqlx::Error> {
        sqlx::query_as::<_, CarrierRequest>(
            "SELECT * FROM sr_carrier_requests WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
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
        response: Option<&serde_json::Value>,
    ) -> Result<CarrierRequest, sqlx::Error> {
        sqlx::query_as::<_, CarrierRequest>(
            r#"
            UPDATE sr_carrier_requests
            SET status = $3, response = COALESCE($4, response), updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(status)
        .bind(response)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn list_by_shipment(
        pool: &PgPool,
        shipment_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<CarrierRequest>, sqlx::Error> {
        sqlx::query_as::<_, CarrierRequest>(
            r#"
            SELECT * FROM sr_carrier_requests
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
