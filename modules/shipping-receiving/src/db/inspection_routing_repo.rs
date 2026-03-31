//! Repository for inspection routing records.
//!
//! Pure data access — no business logic. All queries filter by tenant_id.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Row returned from inspection_routings table.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, utoipa::ToSchema)]
pub struct InspectionRoutingRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub shipment_id: Uuid,
    pub shipment_line_id: Uuid,
    pub route_decision: String,
    pub reason: Option<String>,
    pub routed_by: Option<Uuid>,
    pub routed_at: DateTime<Utc>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub struct InspectionRoutingRepo;

impl InspectionRoutingRepo {
    /// Insert a routing record within a transaction.
    /// Returns the created row.
    pub async fn insert_routing_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        shipment_id: Uuid,
        shipment_line_id: Uuid,
        route_decision: &str,
        reason: Option<&str>,
        routed_by: Option<Uuid>,
        routed_at: DateTime<Utc>,
        idempotency_key: Option<&str>,
    ) -> Result<InspectionRoutingRow, sqlx::Error> {
        sqlx::query_as::<_, InspectionRoutingRow>(
            r#"
            INSERT INTO inspection_routings
                (tenant_id, shipment_id, shipment_line_id, route_decision,
                 reason, routed_by, routed_at, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(shipment_id)
        .bind(shipment_line_id)
        .bind(route_decision)
        .bind(reason)
        .bind(routed_by)
        .bind(routed_at)
        .bind(idempotency_key)
        .fetch_one(&mut **tx)
        .await
    }

    /// Find an existing routing by idempotency key within a transaction.
    pub async fn find_by_idempotency_key_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        idempotency_key: &str,
    ) -> Result<Option<InspectionRoutingRow>, sqlx::Error> {
        sqlx::query_as::<_, InspectionRoutingRow>(
            r#"
            SELECT * FROM inspection_routings
            WHERE tenant_id = $1 AND idempotency_key = $2
            "#,
        )
        .bind(tenant_id)
        .bind(idempotency_key)
        .fetch_optional(&mut **tx)
        .await
    }

    /// Check if a line already has a routing decision within a transaction.
    pub async fn find_by_line_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: Uuid,
        shipment_line_id: Uuid,
    ) -> Result<Option<InspectionRoutingRow>, sqlx::Error> {
        sqlx::query_as::<_, InspectionRoutingRow>(
            r#"
            SELECT * FROM inspection_routings
            WHERE tenant_id = $1 AND shipment_line_id = $2
            "#,
        )
        .bind(tenant_id)
        .bind(shipment_line_id)
        .fetch_optional(&mut **tx)
        .await
    }

    /// List all routings for a shipment.
    pub async fn get_routings_for_shipment(
        pool: &PgPool,
        tenant_id: Uuid,
        shipment_id: Uuid,
    ) -> Result<Vec<InspectionRoutingRow>, sqlx::Error> {
        sqlx::query_as::<_, InspectionRoutingRow>(
            r#"
            SELECT * FROM inspection_routings
            WHERE tenant_id = $1 AND shipment_id = $2
            ORDER BY created_at
            "#,
        )
        .bind(tenant_id)
        .bind(shipment_id)
        .fetch_all(pool)
        .await
    }
}
