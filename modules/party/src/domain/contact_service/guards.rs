use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::party::PartyError;

pub async fn guard_party_exists(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(pool)
            .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }
    Ok(())
}

pub async fn guard_party_exists_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(&mut **tx)
            .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }
    Ok(())
}

pub async fn clear_primary_for_role(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    party_id: Uuid,
    role: Option<&str>,
) -> Result<(), PartyError> {
    if let Some(role) = role {
        sqlx::query(
            r#"
            UPDATE party_contacts
            SET is_primary = false
            WHERE party_id = $1 AND app_id = $2 AND role = $3
              AND is_primary = true AND deactivated_at IS NULL
            "#,
        )
        .bind(party_id)
        .bind(app_id)
        .bind(role)
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query(
            r#"
            UPDATE party_contacts
            SET is_primary = false
            WHERE party_id = $1 AND app_id = $2
              AND is_primary = true AND deactivated_at IS NULL
            "#,
        )
        .bind(party_id)
        .bind(app_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
