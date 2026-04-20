use sqlx::PgPool;
use uuid::Uuid;

use super::authority::AuthorityRow;

/// Read the current authority record for a (app_id, provider, entity_type) triple.
pub async fn get_authority(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
) -> Result<Option<AuthorityRow>, sqlx::Error> {
    sqlx::query_as::<_, AuthorityRow>(
        r#"
        SELECT id, app_id, provider, entity_type, authoritative_side,
               authority_version, last_flipped_by, last_flipped_at,
               created_at, updated_at
        FROM integrations_sync_authority
        WHERE app_id = $1 AND provider = $2 AND entity_type = $3
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .fetch_optional(pool)
    .await
}

/// Insert the authority record if it does not exist; return the current row either way.
/// Does NOT bump version or change side on conflict — use bump_version for that.
pub async fn ensure_authority(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    default_side: &str,
) -> Result<AuthorityRow, sqlx::Error> {
    sqlx::query_as::<_, AuthorityRow>(
        r#"
        INSERT INTO integrations_sync_authority (app_id, provider, entity_type, authoritative_side)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (app_id, provider, entity_type) DO UPDATE
            SET updated_at = integrations_sync_authority.updated_at
        RETURNING id, app_id, provider, entity_type, authoritative_side,
                  authority_version, last_flipped_by, last_flipped_at,
                  created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(default_side)
    .fetch_one(&mut **tx)
    .await
}

/// Increment authority_version and switch authoritative_side.
/// Caller is responsible for holding an advisory lock before calling this
/// to prevent concurrent flips from producing out-of-order versions.
pub async fn bump_version(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    new_side: &str,
    flipped_by: &str,
) -> Result<AuthorityRow, sqlx::Error> {
    sqlx::query_as::<_, AuthorityRow>(
        r#"
        UPDATE integrations_sync_authority
        SET authoritative_side  = $2,
            authority_version   = authority_version + 1,
            last_flipped_by     = $3,
            last_flipped_at     = NOW(),
            updated_at          = NOW()
        WHERE id = $1
        RETURNING id, app_id, provider, entity_type, authoritative_side,
                  authority_version, last_flipped_by, last_flipped_at,
                  created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(new_side)
    .bind(flipped_by)
    .fetch_one(&mut **tx)
    .await
}
