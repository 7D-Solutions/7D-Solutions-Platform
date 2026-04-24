use auth_rs::auth::concurrency::{acquire_tenant_xact_lock, AcquireError};
use auth_rs::db::create_pool;
use serial_test::serial;
use std::borrow::Cow;
use uuid::Uuid;

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://auth_user:auth_pass@localhost:5433/auth_db".into())
}

/// advisory_lock_times_out_after_ceiling: tx1 holds the lock; tx2 should get
/// AdvisoryLockTimeout after the 5-second retry ceiling.
#[tokio::test]
#[serial]
async fn advisory_lock_times_out_after_ceiling() {
    let pool = create_pool(&db_url()).await.expect("connect to test DB");
    let tenant_a = Uuid::new_v4();

    let mut tx1 = pool.begin().await.expect("begin tx1");
    acquire_tenant_xact_lock(&mut tx1, tenant_a)
        .await
        .expect("tx1 should acquire lock");

    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let hold = tokio::spawn(async move {
        release_rx.await.ok();
        tx1.rollback().await.ok();
    });

    let mut tx2 = pool.begin().await.expect("begin tx2");
    let result = acquire_tenant_xact_lock(&mut tx2, tenant_a).await;
    tx2.rollback().await.ok();

    let _ = release_tx.send(());
    hold.await.ok();

    assert!(
        matches!(result, Err(AcquireError::AdvisoryLockTimeout)),
        "expected AdvisoryLockTimeout, got: {:?}",
        result
    );
}

/// statement_timeout_cancels_slow_statement: create_pool sets statement_timeout=10s;
/// a 15s pg_sleep should be cancelled with SQLSTATE 57014.
#[tokio::test]
#[serial]
async fn statement_timeout_cancels_slow_statement() {
    let pool = create_pool(&db_url()).await.expect("connect to test DB");
    let start = std::time::Instant::now();
    let result = sqlx::query("SELECT pg_sleep(15)").execute(&pool).await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "expected error from statement_timeout");
    assert!(
        elapsed.as_secs() < 11,
        "statement should be cancelled within 11s, took {:?}",
        elapsed
    );

    match result.unwrap_err() {
        sqlx::Error::Database(db_err) => {
            assert_eq!(
                db_err.code(),
                Some(Cow::Borrowed("57014")),
                "expected SQLSTATE 57014 (query_canceled), got: {:?}",
                db_err.code()
            );
        }
        other => panic!("expected Database error, got: {:?}", other),
    }
}

/// statement_timeout_releases_advisory_lock: after statement_timeout cancels a statement
/// inside tx1, rolling back tx1 releases the advisory lock so tx2 can acquire it.
#[tokio::test]
#[serial]
async fn statement_timeout_releases_advisory_lock() {
    let pool = create_pool(&db_url()).await.expect("connect to test DB");
    let tenant_b = Uuid::new_v4();

    let mut tx1 = pool.begin().await.expect("begin tx1");
    acquire_tenant_xact_lock(&mut tx1, tenant_b)
        .await
        .expect("tx1 should acquire lock");

    // This statement is cancelled by statement_timeout at ~10s; tx1 enters aborted state
    // but still holds the advisory lock until rollback.
    let _ = sqlx::query("SELECT pg_sleep(15)")
        .execute(&mut *tx1)
        .await;

    // Explicit rollback releases the advisory lock (Drop alone is insufficient here since
    // the aborted transaction may not rollback synchronously).
    tx1.rollback().await.ok();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut tx2 = pool.begin().await.expect("begin tx2");
    let result = acquire_tenant_xact_lock(&mut tx2, tenant_b).await;
    tx2.rollback().await.ok();

    assert!(
        result.is_ok(),
        "tx2 should acquire lock after tx1 rollback, got: {:?}",
        result
    );
}
