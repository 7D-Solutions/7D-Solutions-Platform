use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Extension, Router,
};
/// E2E integration tests for the per-user in-app inbox.
///
/// Covers: creation, idempotent dedup, pagination, read/unread,
/// dismiss/undismiss, tenant boundary isolation, and outbox event emission.
use chrono::Utc;
use notifications_rs::http::inbox;
use notifications_rs::inbox::{
    create_inbox_message, dismiss_message, get_message, list_messages, mark_read, mark_unread,
    undismiss_message, InboxListParams,
};
use notifications_rs::scheduled::insert_pending;
use security::{ActorType, VerifiedClaims};
use serial_test::serial;
use sqlx::PgPool;
use tower::ServiceExt;
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

/// Helper: create a scheduled_notification to serve as the FK target.
async fn seed_notification(pool: &PgPool, tenant_id: &str) -> Uuid {
    let recipient = format!("{}:user-{}", tenant_id, Uuid::new_v4());
    let due = Utc::now() + chrono::Duration::hours(24);
    insert_pending(
        pool,
        &recipient,
        "in_app",
        "test_template",
        serde_json::json!({"test": true}),
        due,
    )
    .await
    .expect("seed notification")
}

// ── Basic CRUD ──────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn create_inbox_message_returns_new_row() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let notif_id = seed_notification(&pool, &tenant).await;

    let msg = create_inbox_message(
        &pool,
        &tenant,
        &user,
        notif_id,
        "Test notification",
        Some("Body text"),
        Some("alert"),
    )
    .await
    .expect("create inbox message");

    assert!(msg.is_some(), "should return a new inbox message");
    let msg = msg.unwrap();
    assert_eq!(msg.tenant_id, tenant);
    assert_eq!(msg.user_id, user);
    assert_eq!(msg.notification_id, notif_id);
    assert_eq!(msg.title, "Test notification");
    assert_eq!(msg.body.as_deref(), Some("Body text"));
    assert_eq!(msg.category.as_deref(), Some("alert"));
    assert!(!msg.is_read);
    assert!(!msg.is_dismissed);
}

// ── Idempotent dedup ────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn duplicate_insert_returns_none() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let notif_id = seed_notification(&pool, &tenant).await;

    let first = create_inbox_message(&pool, &tenant, &user, notif_id, "Title", None, None)
        .await
        .expect("first insert");
    assert!(first.is_some());

    let second = create_inbox_message(&pool, &tenant, &user, notif_id, "Title v2", None, None)
        .await
        .expect("second insert");
    assert!(
        second.is_none(),
        "duplicate (notification_id, user_id) must return None"
    );
}

// ── Pagination ──────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn list_messages_pagination_works() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    // Create 5 inbox messages
    for _ in 0..5 {
        let notif_id = seed_notification(&pool, &tenant).await;
        create_inbox_message(&pool, &tenant, &user, notif_id, "Page test", None, None)
            .await
            .expect("create for pagination");
    }

    let params = InboxListParams {
        tenant_id: tenant.clone(),
        user_id: user.clone(),
        unread_only: false,
        include_dismissed: false,
        category: None,
        limit: 3,
        offset: 0,
    };
    let (page1, total) = list_messages(&pool, &params).await.expect("page 1");
    assert_eq!(page1.len(), 3, "page 1 should have 3 items");
    assert_eq!(total, 5, "total should be 5");

    let params2 = InboxListParams {
        offset: 3,
        ..params
    };
    let (page2, _) = list_messages(&pool, &params2).await.expect("page 2");
    assert_eq!(page2.len(), 2, "page 2 should have 2 items");
}

// ── Read / unread ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn mark_read_then_unread() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let notif_id = seed_notification(&pool, &tenant).await;

    let msg = create_inbox_message(&pool, &tenant, &user, notif_id, "Read test", None, None)
        .await
        .expect("create")
        .unwrap();
    assert!(!msg.is_read);

    let read = mark_read(&pool, &tenant, &user, msg.id)
        .await
        .expect("mark read")
        .unwrap();
    assert!(read.is_read);
    assert!(read.read_at.is_some());

    let unread = mark_unread(&pool, &tenant, &user, msg.id)
        .await
        .expect("mark unread")
        .unwrap();
    assert!(!unread.is_read);
    assert!(unread.read_at.is_none());
}

// ── Dismiss / undismiss ─────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn dismiss_then_undismiss() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let notif_id = seed_notification(&pool, &tenant).await;

    let msg = create_inbox_message(&pool, &tenant, &user, notif_id, "Dismiss test", None, None)
        .await
        .expect("create")
        .unwrap();
    assert!(!msg.is_dismissed);

    let dismissed = dismiss_message(&pool, &tenant, &user, msg.id)
        .await
        .expect("dismiss")
        .unwrap();
    assert!(dismissed.is_dismissed);
    assert!(dismissed.dismissed_at.is_some());

    // Dismissed items hidden by default
    let params = InboxListParams {
        tenant_id: tenant.clone(),
        user_id: user.clone(),
        unread_only: false,
        include_dismissed: false,
        category: None,
        limit: 100,
        offset: 0,
    };
    let (visible, _) = list_messages(&pool, &params).await.expect("list hidden");
    assert!(
        !visible.iter().any(|m| m.id == msg.id),
        "dismissed item should be hidden by default"
    );

    // include_dismissed shows it
    let params_incl = InboxListParams {
        include_dismissed: true,
        ..params.clone()
    };
    let (all, _) = list_messages(&pool, &params_incl).await.expect("list all");
    assert!(
        all.iter().any(|m| m.id == msg.id),
        "dismissed item should appear when include_dismissed=true"
    );

    let undismissed = undismiss_message(&pool, &tenant, &user, msg.id)
        .await
        .expect("undismiss")
        .unwrap();
    assert!(!undismissed.is_dismissed);
    assert!(undismissed.dismissed_at.is_none());
}

// ── Unread-only filter ──────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn unread_only_filter() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let notif1 = seed_notification(&pool, &tenant).await;
    let notif2 = seed_notification(&pool, &tenant).await;

    let msg1 = create_inbox_message(&pool, &tenant, &user, notif1, "Msg 1", None, None)
        .await
        .expect("msg 1")
        .unwrap();
    let _msg2 = create_inbox_message(&pool, &tenant, &user, notif2, "Msg 2", None, None)
        .await
        .expect("msg 2")
        .unwrap();

    // Mark msg1 as read
    mark_read(&pool, &tenant, &user, msg1.id)
        .await
        .expect("read");

    let params = InboxListParams {
        tenant_id: tenant.clone(),
        user_id: user.clone(),
        unread_only: true,
        include_dismissed: false,
        category: None,
        limit: 100,
        offset: 0,
    };
    let (unread, total) = list_messages(&pool, &params).await.expect("unread only");
    assert_eq!(total, 1, "only 1 unread message");
    assert_eq!(unread.len(), 1);
    assert_eq!(unread[0].title, "Msg 2");
}

#[tokio::test]
#[serial]
async fn list_messages_cover_all_filter_templates() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let unread_plain_notif = seed_notification(&pool, &tenant).await;
    let read_plain_notif = seed_notification(&pool, &tenant).await;
    let dismissed_plain_notif = seed_notification(&pool, &tenant).await;
    let unread_categorized_notif = seed_notification(&pool, &tenant).await;

    let unread_plain = create_inbox_message(
        &pool,
        &tenant,
        &user,
        unread_plain_notif,
        "Unread plain",
        None,
        None,
    )
    .await
    .expect("create unread plain")
    .unwrap();
    let read_plain = create_inbox_message(
        &pool,
        &tenant,
        &user,
        read_plain_notif,
        "Read plain",
        None,
        None,
    )
    .await
    .expect("create read plain")
    .unwrap();
    let dismissed_plain = create_inbox_message(
        &pool,
        &tenant,
        &user,
        dismissed_plain_notif,
        "Dismissed plain",
        None,
        None,
    )
    .await
    .expect("create dismissed plain")
    .unwrap();
    let unread_categorized = create_inbox_message(
        &pool,
        &tenant,
        &user,
        unread_categorized_notif,
        "Unread alert",
        None,
        Some("alert"),
    )
    .await
    .expect("create unread categorized")
    .unwrap();

    mark_read(&pool, &tenant, &user, read_plain.id)
        .await
        .expect("mark read");
    dismiss_message(&pool, &tenant, &user, dismissed_plain.id)
        .await
        .expect("dismiss");

    let cases = [
        (
            false,
            false,
            None,
            vec![unread_categorized.id, read_plain.id, unread_plain.id],
            "plain unread/default",
        ),
        (
            true,
            false,
            None,
            vec![unread_categorized.id, unread_plain.id],
            "plain unread-only/default",
        ),
        (
            false,
            true,
            None,
            vec![
                unread_categorized.id,
                dismissed_plain.id,
                read_plain.id,
                unread_plain.id,
            ],
            "plain include-dismissed",
        ),
        (
            true,
            true,
            None,
            vec![unread_categorized.id, dismissed_plain.id, unread_plain.id],
            "plain unread-only include-dismissed",
        ),
        (
            false,
            false,
            Some("alert"),
            vec![unread_categorized.id],
            "categorized default",
        ),
        (
            true,
            false,
            Some("alert"),
            vec![unread_categorized.id],
            "categorized unread-only/default",
        ),
        (
            false,
            true,
            Some("alert"),
            vec![unread_categorized.id],
            "categorized include-dismissed",
        ),
        (
            true,
            true,
            Some("alert"),
            vec![unread_categorized.id],
            "categorized unread-only include-dismissed",
        ),
    ];

    for (unread_only, include_dismissed, category, expected_ids, label) in cases {
        let params = InboxListParams {
            tenant_id: tenant.clone(),
            user_id: user.clone(),
            unread_only,
            include_dismissed,
            category: category.map(str::to_string),
            limit: 20,
            offset: 0,
        };

        let (rows, total) = list_messages(&pool, &params)
            .await
            .unwrap_or_else(|e| panic!("{}: list_messages failed: {e}", label));

        let got_ids: Vec<Uuid> = rows.into_iter().map(|m| m.id).collect();
        assert_eq!(
            total as usize,
            expected_ids.len(),
            "{}: unexpected total",
            label
        );
        assert_eq!(got_ids, expected_ids, "{}: unexpected row set", label);
    }
}

#[tokio::test]
#[serial]
async fn list_my_inbox_uses_authenticated_user() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4();
    let user_me = Uuid::new_v4();
    let user_other = Uuid::new_v4();

    let notif_me = seed_notification(&pool, &tenant.to_string()).await;
    let notif_other = seed_notification(&pool, &tenant.to_string()).await;

    let mine = create_inbox_message(
        &pool,
        &tenant.to_string(),
        &user_me.to_string(),
        notif_me,
        "Mine",
        None,
        None,
    )
    .await
    .expect("create mine")
    .unwrap();
    let other = create_inbox_message(
        &pool,
        &tenant.to_string(),
        &user_other.to_string(),
        notif_other,
        "Other",
        None,
        None,
    )
    .await
    .expect("create other")
    .unwrap();

    let claims = VerifiedClaims {
        user_id: user_me,
        tenant_id: tenant,
        app_id: None,
        roles: vec![],
        perms: vec!["notifications.read".to_string()],
        actor_type: ActorType::User,
        issued_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "1.0.0".to_string(),
    };

    let app = Router::new()
        .route("/api/inbox/mine", axum::routing::get(inbox::list_my_inbox))
        .layer(Extension(claims))
        .with_state(pool);

    let request = Request::builder()
        .uri("/api/inbox/mine?page_size=50&offset=0")
        .body(Body::empty())
        .expect("request");

    let response = app.oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json body");

    let items = payload["data"].as_array().expect("data array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_str().unwrap(), mine.id.to_string());
    assert_ne!(items[0]["id"].as_str().unwrap(), other.id.to_string());
    assert_eq!(payload["pagination"]["total_items"].as_i64(), Some(1));
}

// ── Tenant boundary isolation ───────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_cannot_see_other_tenant_inbox() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();

    let notif_a = seed_notification(&pool, &tenant_a).await;
    let notif_b = seed_notification(&pool, &tenant_b).await;

    let msg_a = create_inbox_message(&pool, &tenant_a, &user, notif_a, "A's msg", None, None)
        .await
        .expect("msg A")
        .unwrap();
    let msg_b = create_inbox_message(&pool, &tenant_b, &user, notif_b, "B's msg", None, None)
        .await
        .expect("msg B")
        .unwrap();

    // Tenant A list
    let params_a = InboxListParams {
        tenant_id: tenant_a.clone(),
        user_id: user.clone(),
        unread_only: false,
        include_dismissed: true,
        category: None,
        limit: 100,
        offset: 0,
    };
    let (items_a, _) = list_messages(&pool, &params_a).await.expect("list A");
    let ids_a: Vec<_> = items_a.iter().map(|m| m.id).collect();
    assert!(ids_a.contains(&msg_a.id));
    assert!(!ids_a.contains(&msg_b.id));

    // Tenant A cannot fetch tenant B's message
    let cross = get_message(&pool, &tenant_a, &user, msg_b.id)
        .await
        .expect("cross fetch");
    assert!(cross.is_none(), "tenant A must not see tenant B's message");

    // Tenant A cannot mark-read tenant B's message
    let cross_read = mark_read(&pool, &tenant_a, &user, msg_b.id)
        .await
        .expect("cross read");
    assert!(
        cross_read.is_none(),
        "tenant A must not modify tenant B's message"
    );
}

// ── Outbox event emitted on create ──────────────────────────────────

#[tokio::test]
#[serial]
async fn outbox_event_emitted_on_create() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let notif_id = seed_notification(&pool, &tenant).await;

    let msg = create_inbox_message(&pool, &tenant, &user, notif_id, "Event test", None, None)
        .await
        .expect("create")
        .unwrap();

    // Check outbox for the inbox.message_created event
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.inbox.message_created' \
         AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count outbox");

    assert!(
        count >= 1,
        "should have at least 1 outbox event for inbox message creation"
    );

    // Verify the event payload references the correct inbox message
    let (payload,): (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM events_outbox \
         WHERE subject = 'notifications.events.inbox.message_created' \
         AND tenant_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("fetch outbox payload");

    let inbox_id_str = payload["payload"]["inbox_message_id"]
        .as_str()
        .expect("inbox_message_id in payload");
    let parsed_id: Uuid = inbox_id_str.parse().expect("valid uuid");
    assert_eq!(parsed_id, msg.id);
}

// ── Outbox events emitted on state changes ──────────────────────────

#[tokio::test]
#[serial]
async fn outbox_events_emitted_on_read_and_dismiss() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let notif_id = seed_notification(&pool, &tenant).await;

    let msg = create_inbox_message(&pool, &tenant, &user, notif_id, "Events test", None, None)
        .await
        .expect("create")
        .unwrap();

    mark_read(&pool, &tenant, &user, msg.id)
        .await
        .expect("read");
    dismiss_message(&pool, &tenant, &user, msg.id)
        .await
        .expect("dismiss");

    let (read_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.inbox.message_read' \
         AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count read events");
    assert!(read_count >= 1, "should have read event in outbox");

    let (dismiss_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.inbox.message_dismissed' \
         AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count dismiss events");
    assert!(dismiss_count >= 1, "should have dismiss event in outbox");
}

// ── Get message detail ──────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn get_message_returns_details() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let notif_id = seed_notification(&pool, &tenant).await;

    let created = create_inbox_message(
        &pool,
        &tenant,
        &user,
        notif_id,
        "Detail test",
        Some("Detailed body"),
        Some("info"),
    )
    .await
    .expect("create")
    .unwrap();

    let fetched = get_message(&pool, &tenant, &user, created.id)
        .await
        .expect("get")
        .expect("should exist");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.title, "Detail test");
    assert_eq!(fetched.body.as_deref(), Some("Detailed body"));
    assert_eq!(fetched.category.as_deref(), Some("info"));
}
