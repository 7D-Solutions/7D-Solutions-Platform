use sqlx::PgPool;
use uuid::Uuid;

use crate::db::party_repo;
use crate::domain::party::models::{Party, PartyError, PartyView, SearchQuery};

pub async fn get_party(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Option<PartyView>, PartyError> {
    party_repo::get_party(pool, app_id, party_id).await
}

pub async fn list_parties(
    pool: &PgPool,
    app_id: &str,
    include_inactive: bool,
    page: i64,
    page_size: i64,
) -> Result<(Vec<Party>, i64), PartyError> {
    party_repo::list_parties(pool, app_id, include_inactive, page, page_size).await
}

pub async fn search_parties(
    pool: &PgPool,
    app_id: &str,
    query: &SearchQuery,
) -> Result<(Vec<Party>, i64), PartyError> {
    party_repo::search_parties(pool, app_id, query).await
}
