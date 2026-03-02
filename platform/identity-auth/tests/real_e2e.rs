use auth_rs::db::{rbac, user_lifecycle_audit};
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, Row};
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

#[tokio::test]
async fn timeline_ordering_for_user_lifecycle_events() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let review_id = Uuid::new_v4();

    let role = rbac::create_role(&pool, tenant_id, "qa_manager", "QA Manager", false)
        .await
        .expect("create role");

    let mut tx = pool.begin().await.expect("begin tx");
    sqlx::query(
        r#"INSERT INTO credentials (tenant_id, user_id, email, password_hash)
           VALUES ($1, $2, $3, $4)"#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(format!("timeline-{}@example.com", Uuid::new_v4()))
    .bind("test-hash")
    .execute(&mut *tx)
    .await
    .expect("insert credential");

    let create_ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-register-1".to_string(),
        causation_id: None,
        idempotency_key: format!("register:{tenant_id}:{user_id}"),
    };

    user_lifecycle_audit::append_lifecycle_event_tx(
        &mut tx,
        tenant_id,
        user_id,
        user_lifecycle_audit::LifecycleEventType::UserCreated,
        None,
        None,
        None,
        None,
        json!({"user_id": user_id, "email": "timeline@example.com"}),
        &create_ctx,
    )
    .await
    .expect("append create event");

    tx.commit().await.expect("commit create tx");

    let bind_ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-bind-1".to_string(),
        causation_id: None,
        idempotency_key: format!("bind:{tenant_id}:{user_id}:{}", role.id),
    };

    rbac::bind_user_role_with_audit(
        &pool,
        tenant_id,
        user_id,
        role.id,
        Some(actor_id),
        &bind_ctx,
    )
    .await
    .expect("bind role with audit");

    let revoke_ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-revoke-1".to_string(),
        causation_id: None,
        idempotency_key: format!("revoke:{tenant_id}:{user_id}:{}", role.id),
    };

    rbac::revoke_user_role_with_audit(
        &pool,
        tenant_id,
        user_id,
        role.id,
        Some(actor_id),
        &revoke_ctx,
    )
    .await
    .expect("revoke role with audit");

    let review_ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-review-1".to_string(),
        causation_id: None,
        idempotency_key: format!("review:{tenant_id}:{user_id}:{review_id}"),
    };

    user_lifecycle_audit::record_access_review_decision(
        &pool,
        tenant_id,
        user_id,
        actor_id,
        "approved",
        review_id,
        Some("quarterly review"),
        &review_ctx,
    )
    .await
    .expect("record access review");

    let timeline = user_lifecycle_audit::list_user_lifecycle_timeline(&pool, tenant_id, user_id)
        .await
        .expect("list timeline");

    assert_eq!(timeline.len(), 4, "expected exactly 4 lifecycle events");

    let event_types = timeline
        .iter()
        .map(|e| e.event_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec![
            "user_created",
            "role_assigned",
            "role_revoked",
            "access_review_recorded",
        ],
        "timeline ordering mismatch"
    );

    let outbox_count: i64 = sqlx::query("SELECT COUNT(*) AS c FROM user_lifecycle_events_outbox WHERE tenant_id = $1 AND aggregate_id = $2")
        .bind(tenant_id)
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .expect("count outbox")
        .get("c");
    assert_eq!(
        outbox_count, 4,
        "outbox should contain one record per event"
    );
}

#[tokio::test]
async fn replay_safety_duplicate_idempotency_key_does_not_duplicate_audit_rows() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let reviewer = Uuid::new_v4();
    let review_id = Uuid::new_v4();
    let idem = format!("review-replay:{tenant_id}:{user_id}:{review_id}");

    let ctx = user_lifecycle_audit::LifecycleAuditContext {
        producer: "auth-rs@test".to_string(),
        trace_id: "trace-review-replay".to_string(),
        causation_id: None,
        idempotency_key: idem.clone(),
    };

    let first = user_lifecycle_audit::record_access_review_decision(
        &pool,
        tenant_id,
        user_id,
        reviewer,
        "approved",
        review_id,
        Some("first attempt"),
        &ctx,
    )
    .await
    .expect("first review write");

    let second = user_lifecycle_audit::record_access_review_decision(
        &pool,
        tenant_id,
        user_id,
        reviewer,
        "approved",
        review_id,
        Some("retry attempt"),
        &ctx,
    )
    .await
    .expect("second review write");

    assert!(first.is_some(), "first write should create an event");
    assert!(
        second.is_none(),
        "duplicate idempotency key must be a no-op"
    );

    let audit_count: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM user_lifecycle_audit_events WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idem)
    .fetch_one(&pool)
    .await
    .expect("count audit rows")
    .get("c");

    let outbox_count: i64 = sqlx::query(
        "SELECT COUNT(*) AS c FROM user_lifecycle_events_outbox WHERE tenant_id = $1 AND aggregate_id = $2",
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("count outbox rows")
    .get("c");

    assert_eq!(
        audit_count, 1,
        "duplicate idempotency must not create another audit row"
    );
    assert_eq!(
        outbox_count, 1,
        "duplicate idempotency must not create another outbox row"
    );
}
