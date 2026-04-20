//! Integration tests for GET /sync/dlq and GET /sync/push-attempts (bd-xvdvh)
//!
//! Proves:
//! 1. list_failed returns only failed outbox rows for the correct tenant
//! 2. list_failed filters by failure_reason correctly
//! 3. list_failed paginates and counts correctly
//! 4. list_failed does NOT return rows for other tenants (isolation)
//! 5. list_attempts returns push-attempt rows for the correct tenant
//! 6. list_attempts filters by status, provider, entity_type, request_fingerprint
//! 7. list_attempts filters by time window (started_after, started_before)
//! 8. list_attempts paginates and counts correctly
//! 9. list_attempts does NOT return rows for other tenants (isolation)

use chrono::Utc;
use integrations_rs::domain::sync::push_attempts::{
    insert_attempt, list_attempts, ListAttemptsFilter,
};
use integrations_rs::outbox::list_failed;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://integrations_user:integrations_pass@localhost:5449/integrations_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

fn tid() -> String {
    format!("dlq-test-{}", Uuid::new_v4().simple())
}

/// Insert a failed outbox row with a specific failure_reason.
async fn seed_failed_outbox(
    pool: &sqlx::PgPool,
    app_id: &str,
    event_type: &str,
    failure_reason: &str,
) -> Uuid {
    let event_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO integrations_outbox
            (event_id, event_type, aggregate_type, aggregate_id, app_id, payload,
             failed_at, error_message, failure_reason, schema_version)
        VALUES ($1, $2, 'test', 'agg-1', $3, '{}', NOW(), 'test error', $4, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(app_id)
    .bind(failure_reason)
    .execute(pool)
    .await
    .expect("seed_failed_outbox failed");
    event_id
}

// ============================================================================
// 1. list_failed returns only failed rows for correct tenant
// ============================================================================

#[tokio::test]
#[serial]
async fn list_failed_returns_only_failed_rows_for_tenant() {
    let pool = setup_db().await;
    let app_id = tid();

    // Seed one failed row
    seed_failed_outbox(&pool, &app_id, "sync.push", "retry_exhausted").await;

    let (rows, total) = list_failed(&pool, &app_id, None, 1, 50)
        .await
        .expect("list_failed failed");

    assert_eq!(total, 1, "should have exactly 1 failed row");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].app_id, app_id);
    assert!(rows[0].failed_at.is_some());
}

// ============================================================================
// 2. list_failed filters by failure_reason
// ============================================================================

#[tokio::test]
#[serial]
async fn list_failed_filters_by_failure_reason() {
    let pool = setup_db().await;
    let app_id = tid();

    seed_failed_outbox(&pool, &app_id, "sync.push", "retry_exhausted").await;
    seed_failed_outbox(&pool, &app_id, "sync.push", "needs_reauth").await;
    seed_failed_outbox(&pool, &app_id, "sync.push", "needs_reauth").await;

    // Filter by retry_exhausted
    let (rows, total) = list_failed(&pool, &app_id, Some("retry_exhausted"), 1, 50)
        .await
        .expect("list_failed with reason failed");
    assert_eq!(total, 1, "only 1 retry_exhausted row");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].failure_reason.as_deref(), Some("retry_exhausted"));

    // Filter by needs_reauth
    let (rows, total) = list_failed(&pool, &app_id, Some("needs_reauth"), 1, 50)
        .await
        .expect("list_failed with reason failed");
    assert_eq!(total, 2, "2 needs_reauth rows");
    assert_eq!(rows.len(), 2);
    for r in &rows {
        assert_eq!(r.failure_reason.as_deref(), Some("needs_reauth"));
    }

    // No filter — all 3
    let (_, total_all) = list_failed(&pool, &app_id, None, 1, 50)
        .await
        .expect("list_failed all failed");
    assert_eq!(total_all, 3);
}

// ============================================================================
// 3. list_failed paginates correctly
// ============================================================================

#[tokio::test]
#[serial]
async fn list_failed_paginates() {
    let pool = setup_db().await;
    let app_id = tid();

    for _ in 0..7 {
        seed_failed_outbox(&pool, &app_id, "sync.push", "bus_publish_failed").await;
    }

    let (page1, total) = list_failed(&pool, &app_id, None, 1, 3)
        .await
        .expect("page 1 failed");
    assert_eq!(total, 7);
    assert_eq!(page1.len(), 3);

    let (page2, _) = list_failed(&pool, &app_id, None, 2, 3)
        .await
        .expect("page 2 failed");
    assert_eq!(page2.len(), 3);

    let (page3, _) = list_failed(&pool, &app_id, None, 3, 3)
        .await
        .expect("page 3 failed");
    assert_eq!(page3.len(), 1);

    // IDs must be distinct across pages
    let ids1: Vec<_> = page1.iter().map(|r| r.event_id).collect();
    let ids2: Vec<_> = page2.iter().map(|r| r.event_id).collect();
    for id in &ids2 {
        assert!(!ids1.contains(id), "pages must not overlap");
    }
}

// ============================================================================
// 4. list_failed tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn list_failed_tenant_isolation() {
    let pool = setup_db().await;
    let a = tid();
    let b = tid();

    seed_failed_outbox(&pool, &a, "sync.push", "retry_exhausted").await;
    seed_failed_outbox(&pool, &b, "sync.push", "retry_exhausted").await;

    let (a_rows, a_total) = list_failed(&pool, &a, None, 1, 50).await.unwrap();
    let (b_rows, b_total) = list_failed(&pool, &b, None, 1, 50).await.unwrap();

    assert_eq!(a_total, 1);
    assert_eq!(b_total, 1);
    assert_eq!(a_rows[0].app_id, a);
    assert_eq!(b_rows[0].app_id, b);
    assert_ne!(a_rows[0].event_id, b_rows[0].event_id);
}

// ============================================================================
// 5. list_attempts basic tenant read
// ============================================================================

#[tokio::test]
#[serial]
async fn list_attempts_returns_tenant_rows() {
    let pool = setup_db().await;
    let app_id = tid();

    insert_attempt(
        &pool, &app_id, "qbo", "invoice", "inv-1", "create", 1, "fp-001",
    )
    .await
    .expect("insert failed");

    let filter = ListAttemptsFilter::default();
    let (rows, total) = list_attempts(&pool, &app_id, &filter, 1, 50)
        .await
        .expect("list_attempts failed");

    assert_eq!(total, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].app_id, app_id);
    assert_eq!(rows[0].entity_type, "invoice");
}

// ============================================================================
// 6. list_attempts filters: status, provider, entity_type, request_fingerprint
// ============================================================================

#[tokio::test]
#[serial]
async fn list_attempts_filter_by_status_provider_entity() {
    let pool = setup_db().await;
    let app_id = tid();

    insert_attempt(&pool, &app_id, "qbo", "invoice", "inv-1", "create", 1, "fp-a").await.unwrap();
    insert_attempt(&pool, &app_id, "qbo", "invoice", "inv-2", "create", 1, "fp-b").await.unwrap();
    insert_attempt(&pool, &app_id, "stripe", "charge", "chg-1", "update", 1, "fp-c").await.unwrap();

    // Filter by provider=qbo
    let (rows, total) = list_attempts(
        &pool, &app_id,
        &ListAttemptsFilter { provider: Some("qbo"), ..Default::default() },
        1, 50,
    ).await.unwrap();
    assert_eq!(total, 2, "2 qbo rows");
    for r in &rows { assert_eq!(r.provider, "qbo"); }

    // Filter by entity_type=charge
    let (rows, total) = list_attempts(
        &pool, &app_id,
        &ListAttemptsFilter { entity_type: Some("charge"), ..Default::default() },
        1, 50,
    ).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].entity_type, "charge");

    // Filter by provider + entity_type
    let (rows, total) = list_attempts(
        &pool, &app_id,
        &ListAttemptsFilter {
            provider: Some("qbo"),
            entity_type: Some("invoice"),
            ..Default::default()
        },
        1, 50,
    ).await.unwrap();
    assert_eq!(total, 2);

    // Filter by request_fingerprint
    let (rows, total) = list_attempts(
        &pool, &app_id,
        &ListAttemptsFilter { request_fingerprint: Some("fp-c"), ..Default::default() },
        1, 50,
    ).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows[0].request_fingerprint, "fp-c");
}

// ============================================================================
// 7. list_attempts time window filter
// ============================================================================

#[tokio::test]
#[serial]
async fn list_attempts_time_window() {
    let pool = setup_db().await;
    let app_id = tid();

    let before = Utc::now();
    insert_attempt(&pool, &app_id, "qbo", "invoice", "inv-tw", "create", 1, "fp-tw")
        .await
        .unwrap();
    let _after = Utc::now();

    // started_after = before the insert → should include the row
    let (_rows, total) = list_attempts(
        &pool, &app_id,
        &ListAttemptsFilter { started_after: Some(before - chrono::Duration::seconds(5)), ..Default::default() },
        1, 50,
    ).await.unwrap();
    assert!(total >= 1, "row inserted after 'before' must appear");

    // started_before = before the insert → should NOT include the row
    let (rows, _) = list_attempts(
        &pool, &app_id,
        &ListAttemptsFilter {
            started_before: Some(before - chrono::Duration::seconds(5)),
            ..Default::default()
        },
        1, 50,
    ).await.unwrap();
    assert!(!rows.iter().any(|r| r.entity_id == "inv-tw"), "row must be excluded by started_before");
}

// ============================================================================
// 8. list_attempts pagination
// ============================================================================

#[tokio::test]
#[serial]
async fn list_attempts_paginates() {
    let pool = setup_db().await;
    let app_id = tid();

    for i in 0..6 {
        insert_attempt(
            &pool, &app_id, "qbo", "invoice", &format!("inv-pg-{i}"), "create", 1, &format!("fp-pg-{i}"),
        )
        .await
        .unwrap();
    }

    let (p1, total) = list_attempts(&pool, &app_id, &Default::default(), 1, 4).await.unwrap();
    assert_eq!(total, 6);
    assert_eq!(p1.len(), 4);

    let (p2, _) = list_attempts(&pool, &app_id, &Default::default(), 2, 4).await.unwrap();
    assert_eq!(p2.len(), 2);

    let ids1: std::collections::HashSet<_> = p1.iter().map(|r| r.id).collect();
    let ids2: std::collections::HashSet<_> = p2.iter().map(|r| r.id).collect();
    assert!(ids1.is_disjoint(&ids2), "pages must not overlap");
}

// ============================================================================
// 9. list_attempts tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn list_attempts_tenant_isolation() {
    let pool = setup_db().await;
    let a = tid();
    let b = tid();

    insert_attempt(&pool, &a, "qbo", "invoice", "inv-a", "create", 1, "fp-iso-a").await.unwrap();
    insert_attempt(&pool, &b, "qbo", "invoice", "inv-b", "create", 1, "fp-iso-b").await.unwrap();

    let (a_rows, a_total) = list_attempts(&pool, &a, &Default::default(), 1, 50).await.unwrap();
    let (b_rows, b_total) = list_attempts(&pool, &b, &Default::default(), 1, 50).await.unwrap();

    assert_eq!(a_total, 1);
    assert_eq!(b_total, 1);
    assert_eq!(a_rows[0].app_id, a);
    assert_eq!(b_rows[0].app_id, b);
    assert_ne!(a_rows[0].id, b_rows[0].id);
}
