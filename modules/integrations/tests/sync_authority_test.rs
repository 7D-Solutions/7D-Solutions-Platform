use std::time::Duration;

use integrations_rs::domain::sync::authority_repo;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

static TEST_POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn init_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

async fn setup_db() -> sqlx::PgPool {
    TEST_POOL.get_or_init(init_pool).await.clone()
}

fn unique_app() -> String {
    format!("sync-auth-test-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_sync_authority WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
#[serial]
async fn test_ensure_authority_creates_row_at_version_one() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let mut tx = pool.begin().await.expect("begin tx");
    let row =
        authority_repo::ensure_authority(&mut tx, &app_id, "quickbooks", "invoice", "platform")
            .await
            .expect("ensure_authority");
    tx.commit().await.expect("commit");

    assert_eq!(row.app_id, app_id);
    assert_eq!(row.provider, "quickbooks");
    assert_eq!(row.entity_type, "invoice");
    assert_eq!(row.authoritative_side, "platform");
    assert_eq!(row.authority_version, 1, "new row must start at version 1");
    assert!(row.last_flipped_by.is_none());
    assert!(row.last_flipped_at.is_none());

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_get_authority_returns_row() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let mut tx = pool.begin().await.expect("begin tx");
    authority_repo::ensure_authority(&mut tx, &app_id, "quickbooks", "customer", "external")
        .await
        .expect("ensure_authority");
    tx.commit().await.expect("commit");

    let fetched = authority_repo::get_authority(&pool, &app_id, "quickbooks", "customer")
        .await
        .expect("get_authority")
        .expect("row must exist");

    assert_eq!(fetched.authoritative_side, "external");
    assert_eq!(fetched.authority_version, 1);

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_ensure_authority_is_idempotent() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let mut tx = pool.begin().await.expect("begin first");
    let first =
        authority_repo::ensure_authority(&mut tx, &app_id, "quickbooks", "bill", "platform")
            .await
            .expect("first ensure");
    tx.commit().await.expect("commit first");

    let mut tx2 = pool.begin().await.expect("begin second");
    let second =
        authority_repo::ensure_authority(&mut tx2, &app_id, "quickbooks", "bill", "platform")
            .await
            .expect("second ensure");
    tx2.commit().await.expect("commit second");

    assert_eq!(first.id, second.id, "same row on second ensure");
    assert_eq!(
        second.authority_version, 1,
        "version must not change on no-op ensure"
    );

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_bump_version_increments_and_switches_side() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let mut tx = pool.begin().await.expect("begin");
    let row =
        authority_repo::ensure_authority(&mut tx, &app_id, "quickbooks", "payment", "platform")
            .await
            .expect("ensure");
    tx.commit().await.expect("commit");

    assert_eq!(row.authority_version, 1);

    let mut tx2 = pool.begin().await.expect("begin bump");
    let flipped = authority_repo::bump_version(&mut tx2, row.id, "external", "test-agent")
        .await
        .expect("bump_version");
    tx2.commit().await.expect("commit bump");

    assert_eq!(
        flipped.authority_version, 2,
        "version must increment to 2 after flip"
    );
    assert_eq!(flipped.authoritative_side, "external");
    assert_eq!(flipped.last_flipped_by.as_deref(), Some("test-agent"));
    assert!(flipped.last_flipped_at.is_some());

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_authority_unique_per_app_provider_entity_type() {
    let pool = setup_db().await;
    let app_id_a = unique_app();
    let app_id_b = unique_app();

    cleanup(&pool, &app_id_a).await;
    cleanup(&pool, &app_id_b).await;

    // Same provider+entity_type is allowed for different tenants
    let mut tx = pool.begin().await.expect("begin a");
    authority_repo::ensure_authority(&mut tx, &app_id_a, "quickbooks", "invoice", "platform")
        .await
        .expect("tenant A");
    tx.commit().await.expect("commit a");

    let mut tx2 = pool.begin().await.expect("begin b");
    authority_repo::ensure_authority(&mut tx2, &app_id_b, "quickbooks", "invoice", "platform")
        .await
        .expect("tenant B — separate row allowed");
    tx2.commit().await.expect("commit b");

    let row_a = authority_repo::get_authority(&pool, &app_id_a, "quickbooks", "invoice")
        .await
        .expect("get a")
        .expect("a exists");
    let row_b = authority_repo::get_authority(&pool, &app_id_b, "quickbooks", "invoice")
        .await
        .expect("get b")
        .expect("b exists");

    assert_ne!(row_a.id, row_b.id, "different tenants get separate rows");

    cleanup(&pool, &app_id_a).await;
    cleanup(&pool, &app_id_b).await;
}

#[tokio::test]
#[serial]
async fn test_authority_version_monotonic_across_multiple_flips() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let mut tx = pool.begin().await.expect("begin");
    let row =
        authority_repo::ensure_authority(&mut tx, &app_id, "quickbooks", "vendor", "platform")
            .await
            .expect("ensure");
    tx.commit().await.expect("commit");

    let mut current_id = row.id;
    let mut current_version = row.authority_version;

    for i in 0..3 {
        let new_side = if i % 2 == 0 { "external" } else { "platform" };
        let mut tx = pool.begin().await.expect("begin flip");
        let flipped = authority_repo::bump_version(&mut tx, current_id, new_side, "loop-test")
            .await
            .expect("bump");
        tx.commit().await.expect("commit flip");

        assert_eq!(
            flipped.authority_version,
            current_version + 1,
            "version must increment by exactly 1 per flip"
        );
        current_id = flipped.id;
        current_version = flipped.authority_version;
    }

    assert_eq!(
        current_version, 4,
        "after 3 flips from v1, version must be 4"
    );

    cleanup(&pool, &app_id).await;
}
