//! Integration tests for the authority flip service.
//!
//! Tests run against a real Postgres instance. No mocks, no stubs.
//! Requires DATABASE_URL pointing to the integrations test database.

use std::time::Duration;

use integrations_rs::domain::sync::authority_repo;
use integrations_rs::domain::sync::authority_service::flip_authority;
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
        .max_connections(8)
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
    format!("flip-svc-test-{}", Uuid::new_v4().simple())
}

/// Insert a minimal OAuth connection so flip_authority can resolve connector_id.
/// Uses dummy encrypted tokens — tests never make real OAuth calls.
async fn seed_connection(pool: &sqlx::PgPool, app_id: &str, provider: &str) -> Uuid {
    let realm_id = format!("test-realm-{}", Uuid::new_v4().simple());
    sqlx::query(
        r#"
        INSERT INTO integrations_oauth_connections (
            app_id, provider, realm_id,
            access_token, refresh_token,
            access_token_expires_at, refresh_token_expires_at,
            scopes_granted, connection_status
        )
        VALUES ($1, $2, $3,
                '\x74657374'::bytea, '\x74657374'::bytea,
                NOW() + INTERVAL '1 hour', NOW() + INTERVAL '30 days',
                'com.intuit.quickbooks.accounting', 'connected')
        ON CONFLICT (app_id, provider) DO UPDATE SET realm_id = EXCLUDED.realm_id
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(realm_id)
    .execute(pool)
    .await
    .expect("seed connection");
    // Fetch the row id.
    let row: (Uuid,) = sqlx::query_as(
        "SELECT id FROM integrations_oauth_connections WHERE app_id = $1 AND provider = $2",
    )
    .bind(app_id)
    .bind(provider)
    .fetch_one(pool)
    .await
    .expect("fetch connection id");
    row.0
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_sync_authority WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_oauth_connections WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Basic flip
// ============================================================================

#[tokio::test]
#[serial]
async fn test_flip_creates_row_and_emits_event() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;
    seed_connection(&pool, &app_id, "quickbooks").await;

    let result = flip_authority(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        "external",
        "user-1",
        "corr-1".to_string(),
    )
    .await
    .expect("first flip should succeed");

    assert_eq!(result.row.authoritative_side, "external");
    assert_eq!(
        result.row.authority_version, 2,
        "new row starts at 1, flip bumps to 2"
    );
    assert_eq!(result.previous_side, "platform");
    assert_eq!(result.row.last_flipped_by.as_deref(), Some("user-1"));
    assert!(result.row.last_flipped_at.is_some());

    // Verify the outbox event was enqueued.
    let event_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1 AND event_type = 'sync.authority.changed'",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("count events");
    assert_eq!(
        event_count.0, 1,
        "exactly one authority.changed event must be enqueued"
    );

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_flip_fails_without_oauth_connection() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;
    // Deliberately do NOT seed a connection.

    let err = flip_authority(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        "external",
        "user-1",
        "corr-2".to_string(),
    )
    .await
    .expect_err("flip without connection must fail");

    assert!(
        matches!(
            err,
            integrations_rs::domain::sync::FlipError::ConnectionNotFound(_, _)
        ),
        "expected ConnectionNotFound, got {:?}",
        err
    );
    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_flip_rejects_invalid_side() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;
    seed_connection(&pool, &app_id, "quickbooks").await;

    let err = flip_authority(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        "neither",
        "user-1",
        "corr-3".to_string(),
    )
    .await
    .expect_err("invalid side must fail");

    assert!(
        matches!(
            err,
            integrations_rs::domain::sync::FlipError::InvalidSide(_)
        ),
        "expected InvalidSide, got {:?}",
        err
    );
    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Version monotonicity
// ============================================================================

#[tokio::test]
#[serial]
async fn test_sequential_flips_version_is_monotonic() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;
    seed_connection(&pool, &app_id, "quickbooks").await;

    let sides = ["external", "platform", "external", "platform"];
    let mut expected_version = 2i64; // first flip on new row: v1 → v2

    for (i, side) in sides.iter().enumerate() {
        let result = flip_authority(
            &pool,
            &app_id,
            "quickbooks",
            "customer",
            side,
            "test",
            format!("corr-{}", i),
        )
        .await
        .unwrap_or_else(|e| panic!("flip {} failed: {:?}", i, e));

        assert_eq!(
            result.row.authority_version, expected_version,
            "flip {}: expected version {}, got {}",
            i, expected_version, result.row.authority_version
        );
        expected_version += 1;
    }

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Outbox quiesce
// ============================================================================

#[tokio::test]
#[serial]
async fn test_flip_quiesces_pending_push_outbox_rows() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;
    seed_connection(&pool, &app_id, "quickbooks").await;

    // Seed two pending push_attempt outbox rows for entity_type=invoice.
    let seeded_ids: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
    for &eid in &seeded_ids {
        sqlx::query(
            r#"
            INSERT INTO integrations_outbox
                (event_id, event_type, aggregate_type, aggregate_id, app_id, payload, schema_version)
            VALUES ($1, 'sync.push.attempt', 'sync_push_attempt', $2, $3,
                    '{"entity_type":"invoice"}'::jsonb, '1.0.0')
            "#,
        )
        .bind(eid)
        .bind(eid.to_string())
        .bind(&app_id)
        .execute(&pool)
        .await
        .expect("seed push outbox row");
    }

    // Seed one row for a different entity_type — must NOT be quiesced.
    let other_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO integrations_outbox
            (event_id, event_type, aggregate_type, aggregate_id, app_id, payload, schema_version)
        VALUES ($1, 'sync.push.attempt', 'sync_push_attempt', $2, $3,
                '{"entity_type":"customer"}'::jsonb, '1.0.0')
        "#,
    )
    .bind(other_id)
    .bind(other_id.to_string())
    .bind(&app_id)
    .execute(&pool)
    .await
    .expect("seed other push outbox row");

    // Flip authority for invoice.
    flip_authority(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        "external",
        "user-1",
        "corr-quiesce".to_string(),
    )
    .await
    .expect("flip");

    // Both invoice rows must now have failure_reason = authority_superseded.
    for &eid in &seeded_ids {
        let row: (Option<String>,) =
            sqlx::query_as("SELECT failure_reason FROM integrations_outbox WHERE event_id = $1")
                .bind(eid)
                .fetch_one(&pool)
                .await
                .expect("fetch row");
        assert_eq!(
            row.0.as_deref(),
            Some("authority_superseded"),
            "invoice push row {} must be quiesced",
            eid
        );
    }

    // The customer row must be untouched.
    let other_row: (Option<String>,) =
        sqlx::query_as("SELECT failure_reason FROM integrations_outbox WHERE event_id = $1")
            .bind(other_id)
            .fetch_one(&pool)
            .await
            .expect("fetch other row");
    assert_eq!(
        other_row.0, None,
        "customer push row must not be quiesced by invoice flip"
    );

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Concurrent flip serialization
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_concurrent_flips_serialize_and_version_is_monotonic() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;
    seed_connection(&pool, &app_id, "quickbooks").await;

    let concurrency = 8usize;
    let pool = std::sync::Arc::new(pool);
    let app_id = std::sync::Arc::new(app_id);

    let handles: Vec<_> = (0..concurrency)
        .map(|i| {
            let pool = pool.clone();
            let app_id = app_id.clone();
            tokio::spawn(async move {
                let side = if i % 2 == 0 { "external" } else { "platform" };
                flip_authority(
                    &pool,
                    &app_id,
                    "quickbooks",
                    "invoice",
                    side,
                    "concurrent-test",
                    format!("corr-concurrent-{}", i),
                )
                .await
            })
        })
        .collect();

    let mut versions: Vec<i64> = Vec::new();
    for h in handles {
        match h.await.expect("task panicked") {
            Ok(r) => versions.push(r.row.authority_version),
            Err(e) => panic!("flip error: {:?}", e),
        }
    }

    // All versions must be unique (no two flips produced the same version).
    let mut sorted = versions.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        versions.len(),
        "concurrent flips must produce unique, monotonic versions; got {:?}",
        versions
    );

    // All versions must be > 1 (the initial row is at v1; every flip increments).
    assert!(
        versions.iter().all(|&v| v >= 2),
        "all flip versions must be >= 2"
    );

    // The final row in the DB must have version equal to the maximum we observed.
    let final_version: (i64,) = sqlx::query_as(
        "SELECT authority_version FROM integrations_sync_authority WHERE app_id = $1 AND provider = 'quickbooks' AND entity_type = 'invoice'",
    )
    .bind(app_id.as_str())
    .fetch_one(&*pool)
    .await
    .expect("fetch final version");
    assert_eq!(
        final_version.0,
        *sorted.iter().max().unwrap(),
        "final DB version must equal max observed version"
    );

    // Verify exactly one authority.changed event per flip.
    let event_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1 AND event_type = 'sync.authority.changed'",
    )
    .bind(app_id.as_str())
    .fetch_one(&*pool)
    .await
    .expect("count events");
    assert_eq!(
        event_count.0, concurrency as i64,
        "must have exactly one authority.changed event per flip"
    );

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Repo-level advisory lock test (confirms bump_version requires explicit lock)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_bump_version_without_advisory_lock_still_increments() {
    // This confirms the repo primitive works in isolation; the service layer adds the lock.
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let mut tx = pool.begin().await.expect("begin");
    let row =
        authority_repo::ensure_authority(&mut tx, &app_id, "quickbooks", "payment", "platform")
            .await
            .expect("ensure");
    tx.commit().await.expect("commit");

    let mut tx2 = pool.begin().await.expect("begin bump");
    let bumped = authority_repo::bump_version(&mut tx2, row.id, "external", "test")
        .await
        .expect("bump");
    tx2.commit().await.expect("commit bump");

    assert_eq!(bumped.authority_version, 2);
    assert_eq!(bumped.authoritative_side, "external");

    cleanup(&pool, &app_id).await;
}
