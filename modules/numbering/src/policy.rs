//! Policy repository — CRUD for per-tenant, per-entity numbering policies.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// Stored policy row from the `numbering_policies` table.
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct PolicyRow {
    pub tenant_id: Uuid,
    pub entity: String,
    pub pattern: String,
    pub prefix: String,
    pub padding: i32,
    pub version: i32,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

/// Fetch a policy for a given tenant + entity. Returns None if no policy exists.
pub async fn get_policy(
    pool: &PgPool,
    tenant_id: Uuid,
    entity: &str,
) -> Result<Option<PolicyRow>, sqlx::Error> {
    sqlx::query_as::<_, PolicyRow>(
        "SELECT tenant_id, entity, pattern, prefix, padding, version, created_at, updated_at \
         FROM numbering_policies WHERE tenant_id = $1 AND entity = $2",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_optional(pool)
    .await
}

/// Upsert a policy inside an existing transaction (Guard → Mutation).
///
/// On conflict, bumps the version and updates the timestamp.
/// Returns the resulting policy row.
pub async fn upsert_policy_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    entity: &str,
    pattern: &str,
    prefix: &str,
    padding: i32,
) -> Result<PolicyRow, sqlx::Error> {
    sqlx::query_as::<_, PolicyRow>(
        r#"
        INSERT INTO numbering_policies (tenant_id, entity, pattern, prefix, padding)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, entity)
        DO UPDATE SET pattern    = EXCLUDED.pattern,
                      prefix     = EXCLUDED.prefix,
                      padding    = EXCLUDED.padding,
                      version    = numbering_policies.version + 1,
                      updated_at = NOW()
        RETURNING tenant_id, entity, pattern, prefix, padding, version, created_at, updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(entity)
    .bind(pattern)
    .bind(prefix)
    .bind(padding)
    .fetch_one(&mut **tx)
    .await
}
