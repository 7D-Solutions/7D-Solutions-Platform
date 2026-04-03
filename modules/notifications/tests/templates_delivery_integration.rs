/// Integration tests for Phase 67: notification templates, sends, and delivery receipts.
///
/// All tests run against real Postgres — no mocks, no stubs.
use notifications_rs::sends::repo as sends_repo;
use notifications_rs::template_store::{models::CreateTemplate, repo as tpl_repo};
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

fn unique_tenant() -> String {
    Uuid::new_v4().to_string()
}

// ── Template publishing + versioning ──────────────────────────────────

#[tokio::test]
#[serial]
async fn publish_template_creates_version_1() {
    let pool = get_pool().await;
    let tenant = unique_tenant();

    let input = CreateTemplate {
        template_key: "test_invoice".to_string(),
        channel: "email".to_string(),
        subject: "Invoice {{invoice_id}} due".to_string(),
        body: "<p>Pay {{amount}} by {{due_date}}</p>".to_string(),
        required_vars: vec![
            "invoice_id".to_string(),
            "amount".to_string(),
            "due_date".to_string(),
        ],
    };

    let tpl = tpl_repo::publish_template(&pool, &tenant, &input, Some("test-user"))
        .await
        .expect("publish template");

    assert_eq!(tpl.version, 1);
    assert_eq!(tpl.template_key, "test_invoice");
    assert_eq!(tpl.channel, "email");
    assert_eq!(tpl.created_by.as_deref(), Some("test-user"));
}

#[tokio::test]
#[serial]
async fn publish_template_auto_increments_version() {
    let pool = get_pool().await;
    let tenant = unique_tenant();

    let input = CreateTemplate {
        template_key: "versioned_tpl".to_string(),
        channel: "email".to_string(),
        subject: "V1 subject".to_string(),
        body: "V1 body".to_string(),
        required_vars: vec![],
    };

    let v1 = tpl_repo::publish_template(&pool, &tenant, &input, None)
        .await
        .expect("publish v1");
    assert_eq!(v1.version, 1);

    let input_v2 = CreateTemplate {
        template_key: "versioned_tpl".to_string(),
        channel: "email".to_string(),
        subject: "V2 subject".to_string(),
        body: "V2 body".to_string(),
        required_vars: vec![],
    };

    let v2 = tpl_repo::publish_template(&pool, &tenant, &input_v2, None)
        .await
        .expect("publish v2");
    assert_eq!(v2.version, 2);
    assert_eq!(v2.subject, "V2 subject");
}

#[tokio::test]
#[serial]
async fn get_latest_resolves_highest_version() {
    let pool = get_pool().await;
    let tenant = unique_tenant();

    for i in 1..=3 {
        let input = CreateTemplate {
            template_key: "multi_ver".to_string(),
            channel: "email".to_string(),
            subject: format!("Subject v{}", i),
            body: format!("Body v{}", i),
            required_vars: vec![],
        };
        tpl_repo::publish_template(&pool, &tenant, &input, None)
            .await
            .expect("publish");
    }

    let latest = tpl_repo::get_latest(&pool, &tenant, "multi_ver")
        .await
        .expect("get latest")
        .expect("should exist");

    assert_eq!(latest.version, 3);
    assert_eq!(latest.subject, "Subject v3");
}

#[tokio::test]
#[serial]
async fn version_history_returns_all_versions_desc() {
    let pool = get_pool().await;
    let tenant = unique_tenant();

    for _ in 0..3 {
        let input = CreateTemplate {
            template_key: "history_tpl".to_string(),
            channel: "sms".to_string(),
            subject: "s".to_string(),
            body: "b".to_string(),
            required_vars: vec![],
        };
        tpl_repo::publish_template(&pool, &tenant, &input, None)
            .await
            .expect("publish");
    }

    let versions = tpl_repo::list_versions(&pool, &tenant, "history_tpl")
        .await
        .expect("list versions");

    assert_eq!(versions.len(), 3);
    assert_eq!(versions[0].version, 3);
    assert_eq!(versions[2].version, 1);
}

// ── Template rendering ────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn render_template_substitutes_variables() {
    let pool = get_pool().await;
    let tenant = unique_tenant();

    let input = CreateTemplate {
        template_key: "render_test".to_string(),
        channel: "email".to_string(),
        subject: "Hello {{name}}".to_string(),
        body: "<p>Your order {{order_id}} is ready</p>".to_string(),
        required_vars: vec!["name".to_string(), "order_id".to_string()],
    };

    let tpl = tpl_repo::publish_template(&pool, &tenant, &input, None)
        .await
        .expect("publish");

    let payload = serde_json::json!({"name": "Alice", "order_id": "ORD-42"});
    let (subject, body) =
        tpl_repo::render_template(&tpl, &payload).expect("render should succeed");

    assert_eq!(subject, "Hello Alice");
    assert_eq!(body, "<p>Your order ORD-42 is ready</p>");
}

#[tokio::test]
#[serial]
async fn render_template_missing_required_var_fails() {
    let pool = get_pool().await;
    let tenant = unique_tenant();

    let input = CreateTemplate {
        template_key: "missing_var".to_string(),
        channel: "email".to_string(),
        subject: "Hi {{name}}".to_string(),
        body: "Body".to_string(),
        required_vars: vec!["name".to_string(), "age".to_string()],
    };

    let tpl = tpl_repo::publish_template(&pool, &tenant, &input, None)
        .await
        .expect("publish");

    let payload = serde_json::json!({"name": "Bob"});
    let err = tpl_repo::render_template(&tpl, &payload).unwrap_err();
    assert!(err.contains("age"), "error should mention missing var: {}", err);
}

// ── Notification sends + delivery receipts ────────────────────────────

#[tokio::test]
#[serial]
async fn send_creates_record_and_receipts() {
    let pool = get_pool().await;
    let tenant = unique_tenant();

    // Create a template first
    let input = CreateTemplate {
        template_key: "send_test".to_string(),
        channel: "email".to_string(),
        subject: "Test {{x}}".to_string(),
        body: "Body {{x}}".to_string(),
        required_vars: vec!["x".to_string()],
    };
    let tpl = tpl_repo::publish_template(&pool, &tenant, &input, None)
        .await
        .expect("publish template");

    // Insert send
    let send = sends_repo::insert_send(
        &pool,
        &tenant,
        Some("send_test"),
        Some(tpl.version),
        "email",
        &["alice@example.com".to_string(), "bob@example.com".to_string()],
        &serde_json::json!({"x": "hello"}),
        Some("corr-123"),
        Some("cause-456"),
        Some("abc123hash"),
    )
    .await
    .expect("insert send");

    assert_eq!(send.template_key.as_deref(), Some("send_test"));
    assert_eq!(send.template_version, Some(1));
    assert_eq!(send.rendered_hash.as_deref(), Some("abc123hash"));
    assert_eq!(send.correlation_id.as_deref(), Some("corr-123"));

    // Insert receipts
    let r1 = sends_repo::insert_receipt(
        &pool,
        &tenant,
        send.id,
        "alice@example.com",
        "email",
        "succeeded",
        Some("provider-msg-1"),
        None,
        None,
    )
    .await
    .expect("insert receipt 1");

    let r2 = sends_repo::insert_receipt(
        &pool,
        &tenant,
        send.id,
        "bob@example.com",
        "email",
        "failed",
        None,
        Some("transient"),
        Some("timeout"),
    )
    .await
    .expect("insert receipt 2");

    assert_eq!(r1.status, "succeeded");
    assert!(r1.succeeded_at.is_some());
    assert_eq!(r2.status, "failed");
    assert!(r2.failed_at.is_some());
    assert_eq!(r2.error_class.as_deref(), Some("transient"));

    // Get send detail
    let fetched = sends_repo::get_send(&pool, &tenant, send.id)
        .await
        .expect("get send")
        .expect("should exist");
    assert_eq!(fetched.id, send.id);

    // Get receipts for send
    let receipts = sends_repo::get_receipts_for_send(&pool, &tenant, send.id)
        .await
        .expect("get receipts");
    assert_eq!(receipts.len(), 2);
}

#[tokio::test]
#[serial]
async fn query_receipts_by_correlation_id() {
    let pool = get_pool().await;
    let tenant = unique_tenant();
    let corr_id = Uuid::new_v4().to_string();

    let send = sends_repo::insert_send(
        &pool,
        &tenant,
        Some("corr_test"),
        Some(1),
        "email",
        &["user@test.com".to_string()],
        &serde_json::json!({}),
        Some(&corr_id),
        None,
        None,
    )
    .await
    .expect("insert send");

    sends_repo::insert_receipt(
        &pool,
        &tenant,
        send.id,
        "user@test.com",
        "email",
        "succeeded",
        Some("prov-1"),
        None,
        None,
    )
    .await
    .expect("insert receipt");

    let results = sends_repo::query_receipts(
        &pool,
        &tenant,
        Some(&corr_id),
        None,
        None,
        None,
        50,
        0,
    )
    .await
    .expect("query receipts");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].recipient, "user@test.com");
    assert_eq!(results[0].provider_id.as_deref(), Some("prov-1"));
}

// ── Tenant isolation ──────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn cross_tenant_template_access_denied() {
    let pool = get_pool().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let input = CreateTemplate {
        template_key: "private_tpl".to_string(),
        channel: "email".to_string(),
        subject: "Secret".to_string(),
        body: "Classified".to_string(),
        required_vars: vec![],
    };

    tpl_repo::publish_template(&pool, &tenant_a, &input, None)
        .await
        .expect("publish for tenant A");

    // Tenant B should not see tenant A's template
    let result = tpl_repo::get_latest(&pool, &tenant_b, "private_tpl")
        .await
        .expect("query should succeed");
    assert!(result.is_none(), "tenant B must not see tenant A's template");
}

#[tokio::test]
#[serial]
async fn cross_tenant_send_access_denied() {
    let pool = get_pool().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let send = sends_repo::insert_send(
        &pool,
        &tenant_a,
        Some("secret_tpl"),
        Some(1),
        "email",
        &["user@a.com".to_string()],
        &serde_json::json!({}),
        None,
        None,
        None,
    )
    .await
    .expect("insert send for tenant A");

    // Tenant B should not see tenant A's send
    let result = sends_repo::get_send(&pool, &tenant_b, send.id)
        .await
        .expect("query should succeed");
    assert!(result.is_none(), "tenant B must not see tenant A's send");
}

#[tokio::test]
#[serial]
async fn cross_tenant_receipt_query_isolated() {
    let pool = get_pool().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let corr = "shared-corr";

    let send_a = sends_repo::insert_send(
        &pool,
        &tenant_a,
        Some("tpl"),
        Some(1),
        "email",
        &["a@a.com".to_string()],
        &serde_json::json!({}),
        Some(corr),
        None,
        None,
    )
    .await
    .expect("insert send A");

    sends_repo::insert_receipt(
        &pool,
        &tenant_a,
        send_a.id,
        "a@a.com",
        "email",
        "succeeded",
        None,
        None,
        None,
    )
    .await
    .expect("insert receipt A");

    // Tenant B queries same correlation_id — should see nothing
    let results = sends_repo::query_receipts(
        &pool,
        &tenant_b,
        Some(corr),
        None,
        None,
        None,
        50,
        0,
    )
    .await
    .expect("query receipts");

    assert!(
        results.is_empty(),
        "tenant B must not see tenant A's receipts"
    );
}

// ── Send status tracking ──────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn send_status_updates_correctly() {
    let pool = get_pool().await;
    let tenant = unique_tenant();

    let send = sends_repo::insert_send(
        &pool,
        &tenant,
        Some("status_tpl"),
        Some(1),
        "email",
        &["user@test.com".to_string()],
        &serde_json::json!({}),
        None,
        None,
        None,
    )
    .await
    .expect("insert send");

    assert_eq!(send.status, "pending");

    sends_repo::update_send_status(&pool, send.id, "delivered")
        .await
        .expect("update status");

    let updated = sends_repo::get_send(&pool, &tenant, send.id)
        .await
        .expect("get send")
        .expect("should exist");

    assert_eq!(updated.status, "delivered");
}
