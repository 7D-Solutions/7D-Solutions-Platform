/// E2E integration tests for broadcast / role-based announcements.
///
/// Covers: tenant broadcast, role-based broadcast, idempotency,
/// tenant isolation, outbox events, and empty audience handling.
use notifications_rs::broadcast::{
    create_broadcast_and_fan_out, get_broadcast, list_recipients, AudienceType, CreateBroadcast,
};
use serial_test::serial;
use sqlx::PgPool;
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

/// Helper: generate N user IDs for a tenant.
fn make_users(n: usize) -> Vec<String> {
    (0..n).map(|_| Uuid::new_v4().to_string()).collect()
}

// ── 1. Tenant broadcast E2E ──────────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_broadcast_creates_individual_recipients() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let users = make_users(5);
    let idem_key = Uuid::new_v4().to_string();

    let req = CreateBroadcast {
        tenant_id: tenant.clone(),
        idempotency_key: idem_key,
        audience_type: AudienceType::AllTenant,
        audience_filter: None,
        title: "System maintenance tonight".to_string(),
        body: Some("Downtime expected 2-4 AM UTC".to_string()),
        channel: "in_app".to_string(),
    };

    let result = create_broadcast_and_fan_out(&pool, &req, &users)
        .await
        .expect("create broadcast");

    assert!(!result.was_duplicate, "should not be a duplicate");
    assert_eq!(result.recipients_created, 5, "should create 5 recipients");
    assert_eq!(result.broadcast.tenant_id, tenant);
    assert_eq!(result.broadcast.audience_type, "all_tenant");
    assert_eq!(result.broadcast.title, "System maintenance tonight");
    assert_eq!(
        result.broadcast.body.as_deref(),
        Some("Downtime expected 2-4 AM UTC")
    );
    assert_eq!(result.broadcast.status, "fan_out_complete");
    assert_eq!(result.broadcast.recipient_count, 5);

    // Verify individual recipient records exist
    let recipients = list_recipients(&pool, &tenant, result.broadcast.id)
        .await
        .expect("list recipients");
    assert_eq!(recipients.len(), 5, "should have 5 recipient records");

    let recipient_user_ids: Vec<&str> = recipients.iter().map(|r| r.user_id.as_str()).collect();
    for user in &users {
        assert!(
            recipient_user_ids.contains(&user.as_str()),
            "user {} should be in recipient list",
            user
        );
    }
}

// ── 2. Role-based broadcast E2E ──────────────────────────────────────

#[tokio::test]
#[serial]
async fn role_based_broadcast_targets_only_matching_users() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let idem_key = Uuid::new_v4().to_string();

    // Simulate role resolution: only 2 of 5 users have the "inspector" role.
    // The caller is responsible for resolving roles to user IDs — the broadcast
    // module just records the audience_type and fan-out targets.
    let inspectors = make_users(2);

    let req = CreateBroadcast {
        tenant_id: tenant.clone(),
        idempotency_key: idem_key,
        audience_type: AudienceType::Role,
        audience_filter: Some("inspector".to_string()),
        title: "New inspection protocol".to_string(),
        body: Some("Review the updated inspection checklist".to_string()),
        channel: "in_app".to_string(),
    };

    let result = create_broadcast_and_fan_out(&pool, &req, &inspectors)
        .await
        .expect("create role broadcast");

    assert!(!result.was_duplicate);
    assert_eq!(result.recipients_created, 2, "only 2 inspectors");
    assert_eq!(result.broadcast.audience_type, "role");
    assert_eq!(
        result.broadcast.audience_filter.as_deref(),
        Some("inspector")
    );
    assert_eq!(result.broadcast.recipient_count, 2);

    // Verify only inspector users received it
    let recipients = list_recipients(&pool, &tenant, result.broadcast.id)
        .await
        .expect("list recipients");
    assert_eq!(recipients.len(), 2);
    for r in &recipients {
        assert!(
            inspectors.contains(&r.user_id),
            "recipient should be an inspector"
        );
    }
}

// ── 3. Idempotency test ──────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn idempotent_broadcast_no_duplicate_fanout() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let users = make_users(3);
    let idem_key = Uuid::new_v4().to_string();

    let req = CreateBroadcast {
        tenant_id: tenant.clone(),
        idempotency_key: idem_key.clone(),
        audience_type: AudienceType::AllTenant,
        audience_filter: None,
        title: "Idempotency test".to_string(),
        body: None,
        channel: "in_app".to_string(),
    };

    // First call: creates broadcast and fan-out
    let r1 = create_broadcast_and_fan_out(&pool, &req, &users)
        .await
        .expect("first broadcast");
    assert!(!r1.was_duplicate);
    assert_eq!(r1.recipients_created, 3);

    // Second call with same idempotency_key: should return duplicate
    let r2 = create_broadcast_and_fan_out(&pool, &req, &users)
        .await
        .expect("duplicate broadcast");
    assert!(
        r2.was_duplicate,
        "second call should be flagged as duplicate"
    );
    assert_eq!(r2.recipients_created, 0, "no new recipients on duplicate");

    // Verify only 3 total recipients exist (not 6)
    let recipients = list_recipients(&pool, &tenant, r1.broadcast.id)
        .await
        .expect("list recipients");
    assert_eq!(
        recipients.len(),
        3,
        "should still have exactly 3 recipients after duplicate attempt"
    );

    // Verify broadcast IDs match
    assert_eq!(
        r1.broadcast.id, r2.broadcast.id,
        "duplicate should return the same broadcast"
    );
}

// ── 4. Tenant isolation test ─────────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_isolation_broadcast() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();
    let users_a = make_users(3);
    let users_b = make_users(2);

    // Create broadcast under tenant_a
    let req_a = CreateBroadcast {
        tenant_id: tenant_a.clone(),
        idempotency_key: Uuid::new_v4().to_string(),
        audience_type: AudienceType::AllTenant,
        audience_filter: None,
        title: "Tenant A announcement".to_string(),
        body: None,
        channel: "in_app".to_string(),
    };
    let result_a = create_broadcast_and_fan_out(&pool, &req_a, &users_a)
        .await
        .expect("tenant A broadcast");

    // Create broadcast under tenant_b
    let req_b = CreateBroadcast {
        tenant_id: tenant_b.clone(),
        idempotency_key: Uuid::new_v4().to_string(),
        audience_type: AudienceType::AllTenant,
        audience_filter: None,
        title: "Tenant B announcement".to_string(),
        body: None,
        channel: "in_app".to_string(),
    };
    let result_b = create_broadcast_and_fan_out(&pool, &req_b, &users_b)
        .await
        .expect("tenant B broadcast");

    // Tenant B cannot see tenant A's broadcast
    let cross_fetch = get_broadcast(&pool, &tenant_b, result_a.broadcast.id)
        .await
        .expect("cross-tenant fetch");
    assert!(
        cross_fetch.is_none(),
        "tenant B must not see tenant A's broadcast"
    );

    // Tenant A cannot see tenant B's broadcast
    let cross_fetch_2 = get_broadcast(&pool, &tenant_a, result_b.broadcast.id)
        .await
        .expect("cross-tenant fetch 2");
    assert!(
        cross_fetch_2.is_none(),
        "tenant A must not see tenant B's broadcast"
    );

    // Tenant B cannot list tenant A's recipients
    let cross_recipients = list_recipients(&pool, &tenant_b, result_a.broadcast.id)
        .await
        .expect("cross-tenant recipients");
    assert!(
        cross_recipients.is_empty(),
        "tenant B must not see tenant A's recipients"
    );

    // Each tenant sees only their own recipients
    let recipients_a = list_recipients(&pool, &tenant_a, result_a.broadcast.id)
        .await
        .expect("tenant A recipients");
    assert_eq!(recipients_a.len(), 3, "tenant A should have 3 recipients");

    let recipients_b = list_recipients(&pool, &tenant_b, result_b.broadcast.id)
        .await
        .expect("tenant B recipients");
    assert_eq!(recipients_b.len(), 2, "tenant B should have 2 recipients");
}

// ── 5. Outbox event test ─────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn broadcast_creates_outbox_events() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let users = make_users(3);
    let idem_key = Uuid::new_v4().to_string();

    // Count outbox events before
    let (before_created,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.broadcast.created' \
         AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count before created");

    let (before_delivered,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.broadcast.delivered' \
         AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count before delivered");

    let req = CreateBroadcast {
        tenant_id: tenant.clone(),
        idempotency_key: idem_key,
        audience_type: AudienceType::AllTenant,
        audience_filter: None,
        title: "Outbox event test".to_string(),
        body: None,
        channel: "in_app".to_string(),
    };

    create_broadcast_and_fan_out(&pool, &req, &users)
        .await
        .expect("create broadcast");

    // Verify broadcast.created event
    let (after_created,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.broadcast.created' \
         AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count after created");

    assert_eq!(
        after_created,
        before_created + 1,
        "exactly 1 broadcast.created outbox event"
    );

    // Verify individual delivery events (1 per recipient)
    let (after_delivered,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.broadcast.delivered' \
         AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count after delivered");

    assert_eq!(
        after_delivered,
        before_delivered + 3,
        "should have 3 individual delivery outbox events (1 per user)"
    );

    // Verify broadcast.created event payload
    let (payload,): (serde_json::Value,) = sqlx::query_as(
        r#"
        SELECT payload FROM events_outbox
        WHERE subject = 'notifications.events.broadcast.created'
        AND tenant_id = $1
        ORDER BY created_at DESC LIMIT 1
        "#,
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("fetch broadcast.created payload");

    let inner = payload.get("payload").unwrap_or(&payload);
    assert_eq!(
        inner.get("audience_type").and_then(|v| v.as_str()),
        Some("all_tenant")
    );
    assert_eq!(
        inner.get("recipient_count").and_then(|v| v.as_i64()),
        Some(3)
    );
    assert_eq!(
        inner.get("title").and_then(|v| v.as_str()),
        Some("Outbox event test")
    );
}

// ── 6. Empty audience test ───────────────────────────────────────────

#[tokio::test]
#[serial]
async fn empty_audience_creates_broadcast_with_zero_recipients() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let idem_key = Uuid::new_v4().to_string();

    let req = CreateBroadcast {
        tenant_id: tenant.clone(),
        idempotency_key: idem_key,
        audience_type: AudienceType::Role,
        audience_filter: Some("nonexistent_role".to_string()),
        title: "Empty audience broadcast".to_string(),
        body: Some("Nobody will see this".to_string()),
        channel: "in_app".to_string(),
    };

    // Empty user_ids slice — no matching users for this role
    let result = create_broadcast_and_fan_out(&pool, &req, &[])
        .await
        .expect("empty audience broadcast");

    assert!(!result.was_duplicate);
    assert_eq!(
        result.recipients_created, 0,
        "no recipients for empty audience"
    );
    assert_eq!(result.broadcast.recipient_count, 0);
    assert_eq!(result.broadcast.status, "fan_out_complete");
    assert_eq!(
        result.broadcast.audience_filter.as_deref(),
        Some("nonexistent_role")
    );

    // Audit record exists (the broadcast itself serves as the audit record)
    let fetched = get_broadcast(&pool, &tenant, result.broadcast.id)
        .await
        .expect("fetch broadcast")
        .expect("broadcast should exist as audit record");
    assert_eq!(fetched.id, result.broadcast.id);
    assert_eq!(fetched.recipient_count, 0);

    // No recipient records
    let recipients = list_recipients(&pool, &tenant, result.broadcast.id)
        .await
        .expect("list recipients");
    assert!(recipients.is_empty(), "no recipients for empty audience");

    // Outbox event still created (broadcast.created for audit trail)
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.broadcast.created' \
         AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count outbox");
    assert!(
        count >= 1,
        "broadcast.created event should exist even for empty audience"
    );
}
