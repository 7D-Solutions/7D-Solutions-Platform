//! Integration tests for item change history (bd-1nj81).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Change capture E2E: create revision + activate → verify change history
//! 2. Policy change tracking: update policy → verify before/after diff
//! 3. Tenant isolation: tenant_A history invisible to tenant_B
//! 4. Idempotency: same change twice → no duplicate rows
//! 5. Outbox event: verify audit event in outbox with correct envelope
//! 6. Ordering: multiple changes → chronological order preserved

use chrono::Utc;
use inventory_rs::domain::{
    history::change_history::{list_change_history, record_change, RecordChangeRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    revisions::{
        activate_revision, create_revision, update_revision_policy, ActivateRevisionRequest,
        CreateRevisionRequest, UpdateRevisionPolicyRequest,
    },
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=require"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");

    pool
}

fn make_item(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Test Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn make_create_rev(tenant_id: &str, item_id: Uuid, idem: &str) -> CreateRevisionRequest {
    CreateRevisionRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        name: "Widget Rev".to_string(),
        description: Some("Updated specifications".to_string()),
        uom: "ea".to_string(),
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        traceability_level: "none".to_string(),
        inspection_required: false,
        shelf_life_days: None,
        shelf_life_enforced: false,
        change_reason: "Spec update".to_string(),
        idempotency_key: idem.to_string(),
        correlation_id: Some("corr-test".to_string()),
        causation_id: None,
        actor_id: Some("user-alice".to_string()),
    }
}

fn make_activate(
    tenant_id: &str,
    idem: &str,
    from: chrono::DateTime<Utc>,
    to: Option<chrono::DateTime<Utc>>,
) -> ActivateRevisionRequest {
    ActivateRevisionRequest {
        tenant_id: tenant_id.to_string(),
        effective_from: from,
        effective_to: to,
        idempotency_key: idem.to_string(),
        correlation_id: Some("corr-test".to_string()),
        causation_id: None,
        actor_id: Some("user-alice".to_string()),
    }
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_change_history WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_revisions WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// 1. Change capture E2E: create revision + activate → verify history
// ============================================================================

#[tokio::test]
#[serial]
async fn change_history_e2e_create_and_activate() {
    let pool = setup_db().await;
    let tenant = format!("test-ch-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-CH-001"))
        .await
        .expect("create item");

    // Create revision
    let idem_create = format!("idem-{}", Uuid::new_v4());
    let mut req = make_create_rev(&tenant, item.id, &idem_create);
    req.actor_id = Some("user-alice".to_string());
    let (rev, _) = create_revision(&pool, &req).await.expect("create revision");

    // Activate revision
    let now = Utc::now();
    let idem_activate = format!("idem-{}", Uuid::new_v4());
    let mut act_req = make_activate(&tenant, &idem_activate, now, None);
    act_req.actor_id = Some("user-bob".to_string());
    activate_revision(&pool, item.id, rev.id, &act_req)
        .await
        .expect("activate revision");

    // Query change history
    let history = list_change_history(&pool, &tenant, item.id)
        .await
        .expect("list history");

    assert_eq!(history.len(), 2, "should have 2 change history entries");

    // First entry: revision_created
    let create_entry = &history[0];
    assert_eq!(create_entry.change_type, "revision_created");
    assert_eq!(create_entry.actor_id, "user-alice");
    assert_eq!(create_entry.revision_id, Some(rev.id));
    assert_eq!(create_entry.item_id, item.id);
    assert!(create_entry.reason.is_some());

    // Verify diff contains expected fields
    let diff = &create_entry.diff;
    assert_eq!(diff["name"]["after"], "Widget Rev");
    assert_eq!(diff["uom"]["after"], "ea");

    // Second entry: revision_activated
    let activate_entry = &history[1];
    assert_eq!(activate_entry.change_type, "revision_activated");
    assert_eq!(activate_entry.actor_id, "user-bob");
    assert_eq!(activate_entry.revision_id, Some(rev.id));

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 2. Policy change tracking: before/after values captured
// ============================================================================

#[tokio::test]
#[serial]
async fn change_history_policy_change_before_after() {
    let pool = setup_db().await;
    let tenant = format!("test-ch-pol-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-CH-POL"))
        .await
        .expect("create item");

    let idem_create = format!("idem-{}", Uuid::new_v4());
    let (rev, _) = create_revision(&pool, &make_create_rev(&tenant, item.id, &idem_create))
        .await
        .expect("create revision");

    // Update policy: change traceability from "none" to "lot"
    let idem_policy = format!("idem-pol-{}", Uuid::new_v4());
    let policy_req = UpdateRevisionPolicyRequest {
        tenant_id: tenant.clone(),
        traceability_level: "lot".to_string(),
        inspection_required: true,
        shelf_life_days: Some(180),
        shelf_life_enforced: true,
        idempotency_key: idem_policy,
        correlation_id: Some("corr-policy".to_string()),
        causation_id: None,
        actor_id: Some("user-charlie".to_string()),
    };
    update_revision_policy(&pool, item.id, rev.id, &policy_req)
        .await
        .expect("update policy");

    let history = list_change_history(&pool, &tenant, item.id)
        .await
        .expect("list history");

    // Should have 2 entries: revision_created + policy_updated
    assert_eq!(history.len(), 2);

    let policy_entry = &history[1];
    assert_eq!(policy_entry.change_type, "policy_updated");
    assert_eq!(policy_entry.actor_id, "user-charlie");

    // Verify before/after diff
    let diff = &policy_entry.diff;
    assert_eq!(diff["traceability_level"]["before"], "none");
    assert_eq!(diff["traceability_level"]["after"], "lot");
    assert_eq!(diff["inspection_required"]["before"], false);
    assert_eq!(diff["inspection_required"]["after"], true);
    assert!(diff["shelf_life_days"]["before"].is_null());
    assert_eq!(diff["shelf_life_days"]["after"], 180);
    assert_eq!(diff["shelf_life_enforced"]["before"], false);
    assert_eq!(diff["shelf_life_enforced"]["after"], true);

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 3. Tenant isolation: tenant_A history invisible to tenant_B
// ============================================================================

#[tokio::test]
#[serial]
async fn change_history_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = format!("test-ch-a-{}", Uuid::new_v4());
    let tenant_b = format!("test-ch-b-{}", Uuid::new_v4());

    let item_a = ItemRepo::create(&pool, &make_item(&tenant_a, "SKU-CH-ISO"))
        .await
        .expect("create item A");
    let item_b = ItemRepo::create(&pool, &make_item(&tenant_b, "SKU-CH-ISO"))
        .await
        .expect("create item B");

    // Create revision under tenant_A
    let idem_a = format!("idem-{}", Uuid::new_v4());
    create_revision(&pool, &make_create_rev(&tenant_a, item_a.id, &idem_a))
        .await
        .expect("create rev A");

    // Tenant B queries tenant_A's item → zero results
    let history_b = list_change_history(&pool, &tenant_b, item_a.id)
        .await
        .expect("list history as tenant B");
    assert!(
        history_b.is_empty(),
        "tenant B must not see tenant A's history"
    );

    // Tenant B queries their own item → zero results (no revisions created)
    let history_b_own = list_change_history(&pool, &tenant_b, item_b.id)
        .await
        .expect("list B's own history");
    assert!(history_b_own.is_empty());

    // Tenant A can see their own history
    let history_a = list_change_history(&pool, &tenant_a, item_a.id)
        .await
        .expect("list A's history");
    assert_eq!(history_a.len(), 1, "tenant A should have 1 history entry");

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}

// ============================================================================
// 4. Idempotency: same change twice → no duplicate rows
// ============================================================================

#[tokio::test]
#[serial]
async fn change_history_idempotent_no_duplicates() {
    let pool = setup_db().await;
    let tenant = format!("test-ch-idem-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-CH-IDEM"))
        .await
        .expect("create item");

    let idem_key = format!("idem-{}", Uuid::new_v4());
    let change_req = RecordChangeRequest {
        tenant_id: tenant.clone(),
        item_id: item.id,
        revision_id: None,
        change_type: "revision_created".to_string(),
        actor_id: "user-alice".to_string(),
        diff: serde_json::json!({"name": {"after": "Widget"}}),
        reason: Some("First change".to_string()),
        idempotency_key: idem_key.clone(),
        correlation_id: Some("corr-idem".to_string()),
        causation_id: None,
    };

    // First call
    let (entry1, replay1) = record_change(&pool, &change_req)
        .await
        .expect("first record");
    assert!(!replay1, "first call must not be a replay");

    // Second call (same idempotency key)
    let (entry2, replay2) = record_change(&pool, &change_req)
        .await
        .expect("second record");
    assert!(replay2, "second call must be a replay");
    assert_eq!(entry1.id, entry2.id, "must return the same entry");

    // Verify only one row exists
    let history = list_change_history(&pool, &tenant, item.id)
        .await
        .expect("list history");
    assert_eq!(
        history.len(),
        1,
        "idempotent retry must not create duplicates"
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 5. Outbox event: verify audit event with correct envelope
// ============================================================================

#[tokio::test]
#[serial]
async fn change_history_outbox_event_emitted() {
    let pool = setup_db().await;
    let tenant = format!("test-ch-outbox-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-CH-OUTBOX"))
        .await
        .expect("create item");

    // Create revision (which records change history + outbox event)
    let idem = format!("idem-{}", Uuid::new_v4());
    let mut req = make_create_rev(&tenant, item.id, &idem);
    req.actor_id = Some("user-alice".to_string());
    create_revision(&pool, &req).await.expect("create revision");

    // Query outbox for the change_recorded event
    let outbox_rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT event_type, payload
        FROM inv_outbox
        WHERE tenant_id = $1
          AND event_type = 'inventory.item_change_recorded'
        ORDER BY created_at ASC
        "#,
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .expect("outbox query");

    assert_eq!(
        outbox_rows.len(),
        1,
        "should have exactly 1 change_recorded outbox event"
    );

    let (event_type, payload) = &outbox_rows[0];
    assert_eq!(event_type, "inventory.item_change_recorded");

    // Verify envelope payload has the correct fields
    assert_eq!(payload["event_type"], "inventory.item_change_recorded");
    assert_eq!(payload["source_module"], "inventory");
    assert_eq!(payload["tenant_id"], tenant);

    // Verify nested payload data (EventEnvelope stores payload under "payload" key)
    let inner = &payload["payload"];
    assert_eq!(inner["change_type"], "revision_created");
    assert_eq!(inner["actor_id"], "user-alice");
    assert_eq!(inner["item_id"], item.id.to_string());

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 6. Ordering: multiple changes → chronological order
// ============================================================================

#[tokio::test]
#[serial]
async fn change_history_chronological_ordering() {
    let pool = setup_db().await;
    let tenant = format!("test-ch-order-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-CH-ORDER"))
        .await
        .expect("create item");

    // Create revision (generates change #1: revision_created)
    let idem1 = format!("idem-{}", Uuid::new_v4());
    let mut req = make_create_rev(&tenant, item.id, &idem1);
    req.actor_id = Some("user-alice".to_string());
    let (rev, _) = create_revision(&pool, &req).await.expect("create revision");

    // Update policy (generates change #2: policy_updated)
    let idem_pol = format!("idem-pol-{}", Uuid::new_v4());
    let policy_req = UpdateRevisionPolicyRequest {
        tenant_id: tenant.clone(),
        traceability_level: "lot".to_string(),
        inspection_required: true,
        shelf_life_days: Some(90),
        shelf_life_enforced: true,
        idempotency_key: idem_pol,
        correlation_id: Some("corr-order".to_string()),
        causation_id: None,
        actor_id: Some("user-bob".to_string()),
    };
    update_revision_policy(&pool, item.id, rev.id, &policy_req)
        .await
        .expect("update policy");

    // Activate revision (generates change #3: revision_activated)
    let now = Utc::now();
    let idem_act = format!("idem-act-{}", Uuid::new_v4());
    let mut act_req = make_activate(&tenant, &idem_act, now, None);
    act_req.actor_id = Some("user-charlie".to_string());
    activate_revision(&pool, item.id, rev.id, &act_req)
        .await
        .expect("activate revision");

    // Verify ordering
    let history = list_change_history(&pool, &tenant, item.id)
        .await
        .expect("list history");

    assert_eq!(history.len(), 3, "should have 3 change history entries");

    // Changes must be in chronological order
    assert_eq!(history[0].change_type, "revision_created");
    assert_eq!(history[0].actor_id, "user-alice");

    assert_eq!(history[1].change_type, "policy_updated");
    assert_eq!(history[1].actor_id, "user-bob");

    assert_eq!(history[2].change_type, "revision_activated");
    assert_eq!(history[2].actor_id, "user-charlie");

    // Timestamps must be non-decreasing
    assert!(history[0].created_at <= history[1].created_at);
    assert!(history[1].created_at <= history[2].created_at);

    cleanup(&pool, &tenant).await;
}
