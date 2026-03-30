mod lifecycle;
mod service;

pub use lifecycle::*;
pub use service::*;

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::eco_models::*;
use crate::domain::guards::GuardError;

use crate::domain::bom_service::BomError;

pub(crate) async fn get_eco(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
) -> Result<Eco, BomError> {
    sqlx::query_as::<_, Eco>("SELECT * FROM ecos WHERE id = $1 AND tenant_id = $2")
        .bind(eco_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| GuardError::NotFound("ECO not found".to_string()).into())
}

pub(crate) async fn insert_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    eco_id: Uuid,
    tenant_id: &str,
    action: &str,
    actor: &str,
    detail: Option<serde_json::Value>,
) -> Result<(), BomError> {
    sqlx::query(
        r#"
        INSERT INTO eco_audit (eco_id, tenant_id, action, actor, detail)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(eco_id)
    .bind(tenant_id)
    .bind(action)
    .bind(actor)
    .bind(detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
