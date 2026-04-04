use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::db::{contact_repo, party_repo};
use crate::domain::party::PartyError;

pub async fn guard_party_exists(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    party_repo::guard_party_exists(pool, app_id, party_id).await
}

pub async fn guard_party_exists_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    party_repo::guard_party_exists_tx(tx, app_id, party_id).await
}

pub async fn clear_primary_for_role(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    party_id: Uuid,
    role: Option<&str>,
) -> Result<(), PartyError> {
    contact_repo::clear_primary_for_role_tx(tx, app_id, party_id, role).await
}
