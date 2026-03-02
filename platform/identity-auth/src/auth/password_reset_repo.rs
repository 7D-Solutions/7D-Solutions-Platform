use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

/// Insert a new password reset token record. Only the SHA-256 hash of the raw token is stored.
pub async fn insert_reset_token(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<Uuid, sqlx::Error> {
    let row = sqlx::query(
        r#"
        INSERT INTO password_reset_tokens (user_id, token_hash, expires_at)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;
    Ok(row.get("id"))
}

/// Atomically claim a reset token: marks it used and returns the associated user_id.
/// Returns None if the token does not exist, is already used, or has expired.
/// The single UPDATE + RETURNING prevents concurrent double-claims under real Postgres.
pub async fn claim_reset_token(
    pool: &sqlx::PgPool,
    token_hash: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        UPDATE password_reset_tokens
        SET used_at = NOW()
        WHERE token_hash = $1
          AND used_at IS NULL
          AND expires_at > NOW()
        RETURNING user_id
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.get::<Uuid, _>("user_id")))
}

// ---------------------------------------------------------------------------
// Tests (against real DB — no mocks)
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use sqlx::postgres::PgPoolOptions;

    async fn test_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://auth_user:auth_pass@localhost:5433/auth_db".into());
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("connect to test DB");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    #[tokio::test]
    async fn test_claim_is_single_use() {
        let pool = test_pool().await;
        let user_id = Uuid::new_v4();
        let hash = format!("hash-single-use-{}", Uuid::new_v4());
        let expires_at = Utc::now() + Duration::hours(1);

        insert_reset_token(&pool, user_id, &hash, expires_at)
            .await
            .expect("insert");

        // First claim succeeds
        let first = claim_reset_token(&pool, &hash).await.expect("first claim");
        assert_eq!(first, Some(user_id), "first claim must return the user_id");

        // Second claim returns None — token is already used
        let second = claim_reset_token(&pool, &hash).await.expect("second claim");
        assert_eq!(
            second, None,
            "second claim must return None (token already used)"
        );
    }

    #[tokio::test]
    async fn test_expired_token_returns_none() {
        let pool = test_pool().await;
        let user_id = Uuid::new_v4();
        let hash = format!("hash-expired-{}", Uuid::new_v4());
        // expires_at in the past
        let expires_at = Utc::now() - Duration::minutes(1);

        insert_reset_token(&pool, user_id, &hash, expires_at)
            .await
            .expect("insert");

        let result = claim_reset_token(&pool, &hash)
            .await
            .expect("claim expired");
        assert_eq!(result, None, "expired token must not be claimable");
    }

    #[tokio::test]
    async fn test_concurrent_claim_exactly_one_wins() {
        let pool = test_pool().await;
        let user_id = Uuid::new_v4();
        let hash = format!("hash-concurrent-{}", Uuid::new_v4());
        let expires_at = Utc::now() + Duration::hours(1);

        insert_reset_token(&pool, user_id, &hash, expires_at)
            .await
            .expect("insert");

        // Spawn two tasks racing to claim the same token
        let pool1 = pool.clone();
        let pool2 = pool.clone();
        let hash1 = hash.clone();
        let hash2 = hash.clone();

        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { claim_reset_token(&pool1, &hash1).await }),
            tokio::spawn(async move { claim_reset_token(&pool2, &hash2).await }),
        );

        let r1 = r1.expect("task 1 panicked").expect("db error task 1");
        let r2 = r2.expect("task 2 panicked").expect("db error task 2");

        let wins = [r1, r2].iter().filter(|r| r.is_some()).count();
        assert_eq!(wins, 1, "exactly one concurrent claim must win; got {wins}");
    }
}
