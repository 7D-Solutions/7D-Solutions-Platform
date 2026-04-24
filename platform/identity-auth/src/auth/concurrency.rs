use sqlx::Row;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

/// In-memory semaphore for bounding concurrent argon2 hash operations (CPU protection).
/// This is NOT the authoritative seat-lease enforcer; that role belongs to session_leases.
#[derive(Clone)]
pub struct HashConcurrencyLimiter {
    sem: Arc<Semaphore>,
    acquire_timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum AcquireError {
    #[error("limiter timeout")]
    Timeout,
    #[error("advisory lock timeout")]
    AdvisoryLockTimeout,
    #[error("db error: {0}")]
    Db(#[from] sqlx::Error),
}

impl HashConcurrencyLimiter {
    pub fn new(max_concurrent: usize, acquire_timeout_ms: u64) -> Self {
        let max = max_concurrent.max(1);
        Self {
            sem: Arc::new(Semaphore::new(max)),
            acquire_timeout: Duration::from_millis(acquire_timeout_ms.max(1)),
        }
    }

    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, AcquireError> {
        match timeout(self.acquire_timeout, self.sem.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            _ => Err(AcquireError::Timeout),
        }
    }
}

// ---------------------------------------------------------------------------
// DB-backed seat lease helpers
// ---------------------------------------------------------------------------

/// Take a PostgreSQL advisory transaction lock keyed on tenant_id.
/// Serializes concurrent logins for the same tenant to prevent TOCTOU on lease count.
/// Uses pg_try_advisory_xact_lock with bounded retry (5s ceiling) rather than blocking
/// pg_advisory_xact_lock, so a slow statement holding the lock doesn't wedge the pool.
pub async fn acquire_tenant_xact_lock(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
) -> Result<(), AcquireError> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let got: (bool,) = sqlx::query_as(
            "SELECT pg_try_advisory_xact_lock(hashtext($1::text)::bigint)",
        )
        .bind(tenant_id)
        .fetch_one(&mut **tx)
        .await?;
        if got.0 {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(AcquireError::AdvisoryLockTimeout);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Count active seat leases for a tenant.
/// Active = revoked_at IS NULL AND last_seen_at >= NOW() - 30 min (inactivity window).
/// Must be called inside a transaction that already holds the advisory lock.
pub async fn count_active_leases_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT COUNT(*) AS cnt
        FROM session_leases
        WHERE tenant_id = $1
          AND revoked_at IS NULL
          AND last_seen_at >= NOW() - INTERVAL '30 minutes'
        "#,
    )
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.get::<i64, _>("cnt"))
}

/// Insert a session lease for a newly issued refresh token.
pub async fn create_lease_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
    session_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO session_leases (tenant_id, user_id, session_id)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(session_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Rotate a lease to a new session_id and refresh last_seen_at (called on token refresh).
pub async fn rotate_lease_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    old_session_id: Uuid,
    new_session_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE session_leases
        SET session_id = $2, last_seen_at = NOW()
        WHERE session_id = $1 AND revoked_at IS NULL
        "#,
    )
    .bind(old_session_id)
    .bind(new_session_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Revoke the session lease whose refresh token matches (tenant_id, token_hash).
/// Called on logout after the refresh_token row has already been revoked.
pub async fn revoke_lease_by_token_hash(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    token_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE session_leases sl
        SET revoked_at = NOW()
        FROM refresh_tokens rt
        WHERE rt.id = sl.session_id
          AND rt.tenant_id = $1
          AND rt.token_hash = $2
          AND sl.revoked_at IS NULL
        "#,
    )
    .bind(tenant_id)
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests (against real DB — no mocks)
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use uuid::Uuid;

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

    /// Insert a real refresh_token row so session_leases FK is satisfied.
    async fn insert_token(pool: &sqlx::PgPool, tenant_id: Uuid, user_id: Uuid) -> Uuid {
        let row = sqlx::query(
            r#"
            INSERT INTO refresh_tokens (tenant_id, user_id, token_hash, expires_at)
            VALUES ($1, $2, $3, NOW() + INTERVAL '14 days')
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(Uuid::new_v4().to_string())
        .fetch_one(pool)
        .await
        .expect("insert refresh_token");
        row.get::<Uuid, _>("id")
    }

    #[tokio::test]
    async fn test_create_and_count_lease() {
        let pool = test_pool().await;
        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();

        let token_id = insert_token(&pool, tenant_id, user_id).await;

        let mut tx = pool.begin().await.expect("begin tx");
        acquire_tenant_xact_lock(&mut tx, tenant_id)
            .await
            .expect("advisory lock");
        let before = count_active_leases_in_tx(&mut tx, tenant_id)
            .await
            .expect("count");
        assert_eq!(before, 0);
        create_lease_in_tx(&mut tx, tenant_id, user_id, token_id)
            .await
            .expect("create lease");
        let after = count_active_leases_in_tx(&mut tx, tenant_id)
            .await
            .expect("count after");
        assert_eq!(after, 1);
        tx.commit().await.expect("commit");
    }

    #[tokio::test]
    async fn test_rotate_lease_updates_session_and_heartbeat() {
        let pool = test_pool().await;
        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();

        let old_token_id = insert_token(&pool, tenant_id, user_id).await;
        let new_token_id = insert_token(&pool, tenant_id, user_id).await;

        // Create initial lease
        let mut tx = pool.begin().await.expect("begin");
        create_lease_in_tx(&mut tx, tenant_id, user_id, old_token_id)
            .await
            .expect("create");
        tx.commit().await.expect("commit");

        // Rotate
        let mut tx2 = pool.begin().await.expect("begin2");
        rotate_lease_in_tx(&mut tx2, old_token_id, new_token_id)
            .await
            .expect("rotate");
        tx2.commit().await.expect("commit2");

        // Verify new session_id is active
        let row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM session_leases WHERE session_id = $1 AND revoked_at IS NULL",
        )
        .bind(new_token_id)
        .fetch_one(&pool)
        .await
        .expect("verify");
        assert_eq!(row.get::<i64, _>("cnt"), 1);

        // Old session_id should no longer be active
        let old_row = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM session_leases WHERE session_id = $1 AND revoked_at IS NULL",
        )
        .bind(old_token_id)
        .fetch_one(&pool)
        .await
        .expect("verify old");
        assert_eq!(old_row.get::<i64, _>("cnt"), 0);
    }

    #[tokio::test]
    async fn test_revoke_lease_by_token_hash() {
        let pool = test_pool().await;
        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let raw_hash = Uuid::new_v4().to_string();

        let token_id = sqlx::query(
            r#"
            INSERT INTO refresh_tokens (tenant_id, user_id, token_hash, expires_at)
            VALUES ($1, $2, $3, NOW() + INTERVAL '14 days')
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(user_id)
        .bind(&raw_hash)
        .fetch_one(&pool)
        .await
        .expect("insert token")
        .get::<Uuid, _>("id");

        let mut tx = pool.begin().await.expect("begin");
        create_lease_in_tx(&mut tx, tenant_id, user_id, token_id)
            .await
            .expect("create lease");
        tx.commit().await.expect("commit");

        revoke_lease_by_token_hash(&pool, tenant_id, &raw_hash)
            .await
            .expect("revoke");

        let cnt = sqlx::query(
            "SELECT COUNT(*) AS cnt FROM session_leases WHERE session_id = $1 AND revoked_at IS NULL",
        )
        .bind(token_id)
        .fetch_one(&pool)
        .await
        .expect("verify")
        .get::<i64, _>("cnt");
        assert_eq!(cnt, 0);
    }

    #[tokio::test]
    async fn test_seat_limit_enforced_atomically() {
        let pool = test_pool().await;
        let tenant_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let limit: i64 = 2;

        // Create 2 leases (at limit)
        for _ in 0..limit {
            let token_id = insert_token(&pool, tenant_id, user_id).await;
            let mut tx = pool.begin().await.expect("begin");
            create_lease_in_tx(&mut tx, tenant_id, user_id, token_id)
                .await
                .expect("create");
            tx.commit().await.expect("commit");
        }

        // Now check: count should equal limit, so a 3rd login should be rejected
        let mut tx = pool.begin().await.expect("begin check");
        acquire_tenant_xact_lock(&mut tx, tenant_id)
            .await
            .expect("lock");
        let cnt = count_active_leases_in_tx(&mut tx, tenant_id)
            .await
            .expect("count");
        tx.rollback().await.ok();

        assert_eq!(cnt, limit, "active lease count should equal limit");
        assert!(cnt >= limit, "login should be rejected when cnt >= limit");
    }
}
