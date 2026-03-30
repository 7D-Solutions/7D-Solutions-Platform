//! Genealogy edge queries: children_of / parents_of.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// A genealogy edge returned by query functions.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct GenealogyEdge {
    pub id: Uuid,
    pub tenant_id: String,
    pub operation_id: Uuid,
    pub parent_lot_id: Uuid,
    pub child_lot_id: Uuid,
    pub transformation: String,
    pub quantity: i64,
    pub unit: String,
    pub occurred_at: DateTime<Utc>,
    pub actor_id: Option<Uuid>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Return all children of a lot (forward trace).
pub async fn children_of(
    pool: &PgPool,
    tenant_id: &str,
    lot_id: Uuid,
) -> Result<Vec<GenealogyEdge>, sqlx::Error> {
    sqlx::query_as::<_, GenealogyEdge>(
        r#"
        SELECT id, tenant_id, operation_id, parent_lot_id, child_lot_id,
               transformation, quantity, unit, occurred_at, actor_id, notes,
               created_at
        FROM inv_lot_genealogy
        WHERE tenant_id = $1 AND parent_lot_id = $2
        ORDER BY created_at ASC
        "#,
    )
    .bind(tenant_id)
    .bind(lot_id)
    .fetch_all(pool)
    .await
}

/// Return all parents of a lot (reverse trace).
pub async fn parents_of(
    pool: &PgPool,
    tenant_id: &str,
    lot_id: Uuid,
) -> Result<Vec<GenealogyEdge>, sqlx::Error> {
    sqlx::query_as::<_, GenealogyEdge>(
        r#"
        SELECT id, tenant_id, operation_id, parent_lot_id, child_lot_id,
               transformation, quantity, unit, occurred_at, actor_id, notes,
               created_at
        FROM inv_lot_genealogy
        WHERE tenant_id = $1 AND child_lot_id = $2
        ORDER BY created_at ASC
        "#,
    )
    .bind(tenant_id)
    .bind(lot_id)
    .fetch_all(pool)
    .await
}
