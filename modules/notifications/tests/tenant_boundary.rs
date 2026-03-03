/// Integration tests: tenant boundary isolation for DLQ operations.
///
/// These tests verify that DLQ queries scoped by tenant_id correctly
/// prevent cross-tenant data leakage — a tenant can only see and act
/// on their own dead-lettered notifications.
use chrono::Utc;
use notifications_rs::scheduled::{
    dispatch_once, insert_pending, LoggingSender, NotificationSender, RetryPolicy,
};
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

const DEFAULT_DB_URL: &str =
    "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db";

async fn get_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to notifications test DB");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

/// Insert a notification for a specific tenant and dispatch it to dead_lettered.
async fn force_dlq_for_tenant(pool: &PgPool, tenant_id: &str) -> Uuid {
    let recipient_ref = format!("{}:user-{}", tenant_id, Uuid::new_v4());
    let due = Utc::now() - chrono::Duration::seconds(1);
    let id = insert_pending(
        pool,
        &recipient_ref,
        "email",
        "nonexistent_template",
        serde_json::json!({"tenant": tenant_id}),
        due,
    )
    .await
    .expect("insert pending for tenant DLQ");

    let sender: Arc<dyn NotificationSender> = Arc::new(LoggingSender);
    dispatch_once(pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch to DLQ");

    // Verify it's dead_lettered
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "dead_lettered", "should be dead_lettered");
    id
}

// ── DLQ list: tenant A can only see their own items ──────────────────

#[tokio::test]
#[serial]
async fn tenant_a_cannot_see_tenant_b_dlq_items() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();

    let id_a = force_dlq_for_tenant(&pool, &tenant_a).await;
    let id_b = force_dlq_for_tenant(&pool, &tenant_b).await;

    // Tenant A's DLQ list
    let rows_a: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM scheduled_notifications \
         WHERE status = 'dead_lettered' AND tenant_id = $1",
    )
    .bind(&tenant_a)
    .fetch_all(&pool)
    .await
    .expect("tenant A DLQ list");

    let ids_a: Vec<Uuid> = rows_a.iter().map(|r| r.0).collect();
    assert!(ids_a.contains(&id_a), "tenant A should see their own item");
    assert!(
        !ids_a.contains(&id_b),
        "tenant A must NOT see tenant B's item"
    );

    // Tenant B's DLQ list
    let rows_b: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM scheduled_notifications \
         WHERE status = 'dead_lettered' AND tenant_id = $1",
    )
    .bind(&tenant_b)
    .fetch_all(&pool)
    .await
    .expect("tenant B DLQ list");

    let ids_b: Vec<Uuid> = rows_b.iter().map(|r| r.0).collect();
    assert!(ids_b.contains(&id_b), "tenant B should see their own item");
    assert!(
        !ids_b.contains(&id_a),
        "tenant B must NOT see tenant A's item"
    );
}

// ── DLQ detail: tenant A can't fetch tenant B's item ─────────────────

#[tokio::test]
#[serial]
async fn tenant_a_cannot_fetch_tenant_b_dlq_detail() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();

    let _id_a = force_dlq_for_tenant(&pool, &tenant_a).await;
    let id_b = force_dlq_for_tenant(&pool, &tenant_b).await;

    // Tenant A tries to fetch tenant B's item by ID
    let cross_fetch: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM scheduled_notifications \
         WHERE id = $1 AND status = 'dead_lettered' AND tenant_id = $2",
    )
    .bind(id_b)
    .bind(&tenant_a)
    .fetch_optional(&pool)
    .await
    .expect("cross-tenant fetch");

    assert!(
        cross_fetch.is_none(),
        "tenant A must NOT be able to fetch tenant B's item"
    );
}

// ── DLQ replay: tenant A can't replay tenant B's item ────────────────

#[tokio::test]
#[serial]
async fn tenant_a_cannot_replay_tenant_b_dlq_item() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();

    let _id_a = force_dlq_for_tenant(&pool, &tenant_a).await;
    let id_b = force_dlq_for_tenant(&pool, &tenant_b).await;

    // Tenant A tries to replay tenant B's item (guard query with tenant_id)
    let guarded: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM scheduled_notifications \
         WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(id_b)
    .bind(&tenant_a)
    .fetch_optional(&pool)
    .await
    .expect("cross-tenant replay guard");

    assert!(
        guarded.is_none(),
        "guard must reject: tenant A cannot replay tenant B's notification"
    );

    // Verify tenant B's item is still dead_lettered (not modified)
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id_b)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(
        status, "dead_lettered",
        "tenant B's item must remain dead_lettered"
    );
}

// ── DLQ abandon: tenant A can't abandon tenant B's item ──────────────

#[tokio::test]
#[serial]
async fn tenant_a_cannot_abandon_tenant_b_dlq_item() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();

    let _id_a = force_dlq_for_tenant(&pool, &tenant_a).await;
    let id_b = force_dlq_for_tenant(&pool, &tenant_b).await;

    // Tenant A tries to abandon tenant B's item (guard query with tenant_id)
    let guarded: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM scheduled_notifications \
         WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(id_b)
    .bind(&tenant_a)
    .fetch_optional(&pool)
    .await
    .expect("cross-tenant abandon guard");

    assert!(
        guarded.is_none(),
        "guard must reject: tenant A cannot abandon tenant B's notification"
    );

    // Verify tenant B's item is still dead_lettered
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id_b)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(
        status, "dead_lettered",
        "tenant B's item must remain dead_lettered"
    );
}

// ── DLQ count: tenant-scoped count is correct ────────────────────────

#[tokio::test]
#[serial]
async fn tenant_scoped_dlq_count_is_accurate() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();

    // Create 2 items for tenant A, 1 for tenant B
    let _id_a1 = force_dlq_for_tenant(&pool, &tenant_a).await;
    let _id_a2 = force_dlq_for_tenant(&pool, &tenant_a).await;
    let _id_b1 = force_dlq_for_tenant(&pool, &tenant_b).await;

    let (count_a,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM scheduled_notifications \
         WHERE status = 'dead_lettered' AND tenant_id = $1",
    )
    .bind(&tenant_a)
    .fetch_one(&pool)
    .await
    .expect("count tenant A");

    let (count_b,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM scheduled_notifications \
         WHERE status = 'dead_lettered' AND tenant_id = $1",
    )
    .bind(&tenant_b)
    .fetch_one(&pool)
    .await
    .expect("count tenant B");

    assert_eq!(count_a, 2, "tenant A should have exactly 2 DLQ items");
    assert_eq!(count_b, 1, "tenant B should have exactly 1 DLQ item");
}

// ── Tenant-scoped replay: own item succeeds ──────────────────────────

#[tokio::test]
#[serial]
async fn tenant_can_replay_own_dlq_item() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4().to_string();

    let id_a = force_dlq_for_tenant(&pool, &tenant_a).await;

    // Tenant A replays their own item (full guard+mutation like the handler)
    let guarded: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM scheduled_notifications \
         WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(id_a)
    .bind(&tenant_a)
    .fetch_optional(&pool)
    .await
    .expect("own tenant replay guard");

    assert!(guarded.is_some(), "guard must pass for own tenant's item");
    assert_eq!(guarded.unwrap().0, "dead_lettered");

    // Perform the mutation
    sqlx::query(
        "UPDATE scheduled_notifications \
         SET status = 'pending', deliver_at = NOW(), retry_count = 0, \
             replay_generation = replay_generation + 1, \
             last_error = NULL, dead_lettered_at = NULL, failed_at = NULL \
         WHERE id = $1",
    )
    .bind(id_a)
    .execute(&pool)
    .await
    .expect("replay mutation");

    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id_a)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "pending", "item should be reset to pending");
}
