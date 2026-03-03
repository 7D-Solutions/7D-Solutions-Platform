//! Integration tests for item revision management (bd-scult).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Happy path: create revision + activate + query at time T
//! 2. Idempotency: duplicate key returns stored result
//! 3. Auto-increment: revision numbers increment per item
//! 4. Supersede: activating a new revision closes the predecessor
//! 5. Overlap rejection: DB exclusion constraint prevents overlaps
//! 6. Tenant isolation: revisions scoped per tenant
//! 7. Concurrent creation: two revisions created in quick succession

use chrono::{Duration, Utc};
use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    revisions::{
        activate_revision, create_revision, list_revisions, revision_at, update_revision_policy,
        ActivateRevisionRequest, CreateRevisionRequest, RevisionError, UpdateRevisionPolicyRequest,
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
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");

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
        actor_id: None,
    }
}

fn make_update_policy(
    tenant_id: &str,
    idem: &str,
    traceability_level: &str,
    inspection_required: bool,
    shelf_life_days: Option<i32>,
    shelf_life_enforced: bool,
) -> UpdateRevisionPolicyRequest {
    UpdateRevisionPolicyRequest {
        tenant_id: tenant_id.to_string(),
        traceability_level: traceability_level.to_string(),
        inspection_required,
        shelf_life_days,
        shelf_life_enforced,
        idempotency_key: idem.to_string(),
        correlation_id: Some("corr-policy".to_string()),
        causation_id: None,
        actor_id: None,
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
        actor_id: None,
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
// 1. Happy path: create → activate → query at time T
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_create_activate_query_happy_path() {
    let pool = setup_db().await;
    let tenant = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-REV-001"))
        .await
        .expect("create item");

    // Create revision
    let idem_create = format!("idem-{}", Uuid::new_v4());
    let req = make_create_rev(&tenant, item.id, &idem_create);
    let (rev, is_replay) = create_revision(&pool, &req).await.expect("create revision");
    assert!(!is_replay);
    assert_eq!(rev.revision_number, 1);
    assert!(
        rev.effective_from.is_none(),
        "draft revision has no effective_from"
    );

    // Verify outbox event was written
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_revision_created'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox query");
    assert_eq!(outbox_count, 1);

    // Activate revision
    let now = Utc::now();
    let idem_activate = format!("idem-{}", Uuid::new_v4());
    let act_req = make_activate(&tenant, &idem_activate, now, None);
    let (activated, is_replay) = activate_revision(&pool, item.id, rev.id, &act_req)
        .await
        .expect("activate revision");
    assert!(!is_replay);
    assert_eq!(activated.effective_from, Some(now));
    assert!(activated.effective_to.is_none());
    assert!(activated.activated_at.is_some());

    // Verify activation outbox event
    let act_outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_revision_activated'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("activation outbox query");
    assert_eq!(act_outbox_count, 1);

    // Query at time T (now) should return this revision
    let found = revision_at(&pool, &tenant, item.id, now)
        .await
        .expect("revision_at")
        .expect("should find a revision at now");
    assert_eq!(found.id, rev.id);
    assert_eq!(found.revision_number, 1);

    // Query at time T (far past) should return nothing
    let far_past = now - Duration::days(365);
    let not_found = revision_at(&pool, &tenant, item.id, far_past)
        .await
        .expect("revision_at past");
    assert!(not_found.is_none());

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 2. Idempotency: duplicate create returns stored result
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_create_idempotent_replay() {
    let pool = setup_db().await;
    let tenant = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-REV-IDEM"))
        .await
        .expect("create item");

    let idem = format!("idem-{}", Uuid::new_v4());
    let req = make_create_rev(&tenant, item.id, &idem);

    // First call
    let (rev1, replay1) = create_revision(&pool, &req).await.expect("first create");
    assert!(!replay1);

    // Second call (same key, same body)
    let (rev2, replay2) = create_revision(&pool, &req).await.expect("second create");
    assert!(replay2, "second call must be replay");
    assert_eq!(rev1.id, rev2.id);
    assert_eq!(rev1.revision_number, rev2.revision_number);

    // Only one revision row in DB
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM item_revisions WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant)
    .bind(item.id)
    .fetch_one(&pool)
    .await
    .expect("count query");
    assert_eq!(count, 1);

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 3. Auto-increment: revision numbers increment per item
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_numbers_auto_increment() {
    let pool = setup_db().await;
    let tenant = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-REV-INCR"))
        .await
        .expect("create item");

    // Create three revisions
    for i in 1..=3 {
        let idem = format!("idem-incr-{}-{}", i, Uuid::new_v4());
        let mut req = make_create_rev(&tenant, item.id, &idem);
        req.name = format!("Widget v{}", i);
        req.change_reason = format!("Reason {}", i);

        let (rev, _) = create_revision(&pool, &req).await.expect("create revision");
        assert_eq!(rev.revision_number, i as i32);
    }

    // List should return all 3 in order
    let revs = list_revisions(&pool, &tenant, item.id)
        .await
        .expect("list revisions");
    assert_eq!(revs.len(), 3);
    assert_eq!(revs[0].revision_number, 1);
    assert_eq!(revs[1].revision_number, 2);
    assert_eq!(revs[2].revision_number, 3);

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 4. Supersede: activating new revision auto-closes predecessor
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_activation_supersedes_predecessor() {
    let pool = setup_db().await;
    let tenant = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-REV-SUP"))
        .await
        .expect("create item");

    // Create and activate rev 1
    let idem1 = format!("idem-{}", Uuid::new_v4());
    let (rev1, _) = create_revision(&pool, &make_create_rev(&tenant, item.id, &idem1))
        .await
        .expect("create rev1");

    let t1 = Utc::now() - Duration::hours(2);
    let act1_idem = format!("idem-{}", Uuid::new_v4());
    let (activated1, _) = activate_revision(
        &pool,
        item.id,
        rev1.id,
        &make_activate(&tenant, &act1_idem, t1, None),
    )
    .await
    .expect("activate rev1");
    assert!(
        activated1.effective_to.is_none(),
        "rev1 should be open-ended"
    );

    // Create and activate rev 2 — should auto-close rev 1
    let idem2 = format!("idem-{}", Uuid::new_v4());
    let mut req2 = make_create_rev(&tenant, item.id, &idem2);
    req2.name = "Widget v2".to_string();
    req2.change_reason = "Second update".to_string();
    let (rev2, _) = create_revision(&pool, &req2).await.expect("create rev2");

    let t2 = Utc::now();
    let act2_idem = format!("idem-{}", Uuid::new_v4());
    let (activated2, _) = activate_revision(
        &pool,
        item.id,
        rev2.id,
        &make_activate(&tenant, &act2_idem, t2, None),
    )
    .await
    .expect("activate rev2");
    assert!(
        activated2.effective_to.is_none(),
        "rev2 should be open-ended"
    );

    // Rev 1 should now be closed (effective_to = rev2.effective_from)
    let rev1_updated: (Option<chrono::DateTime<Utc>>,) =
        sqlx::query_as("SELECT effective_to FROM item_revisions WHERE id = $1")
            .bind(rev1.id)
            .fetch_one(&pool)
            .await
            .expect("fetch rev1");
    assert_eq!(
        rev1_updated.0,
        Some(t2),
        "rev1.effective_to should equal rev2.effective_from"
    );

    // Query at t1+1h should return rev1
    let mid_t1 = t1 + Duration::hours(1);
    let found_at_mid = revision_at(&pool, &tenant, item.id, mid_t1)
        .await
        .expect("revision_at mid")
        .expect("should find rev1");
    assert_eq!(found_at_mid.id, rev1.id);

    // Query at t2 should return rev2
    let found_at_t2 = revision_at(&pool, &tenant, item.id, t2)
        .await
        .expect("revision_at t2")
        .expect("should find rev2");
    assert_eq!(found_at_t2.id, rev2.id);

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 5. Already-activated revision rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_double_activation_rejected() {
    let pool = setup_db().await;
    let tenant = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-REV-DBL"))
        .await
        .expect("create item");

    let idem = format!("idem-{}", Uuid::new_v4());
    let (rev, _) = create_revision(&pool, &make_create_rev(&tenant, item.id, &idem))
        .await
        .expect("create rev");

    let now = Utc::now();
    let act1_idem = format!("idem-{}", Uuid::new_v4());
    activate_revision(
        &pool,
        item.id,
        rev.id,
        &make_activate(&tenant, &act1_idem, now, None),
    )
    .await
    .expect("first activation");

    // Second activation with different key should fail
    let act2_idem = format!("idem-{}", Uuid::new_v4());
    let err = activate_revision(
        &pool,
        item.id,
        rev.id,
        &make_activate(&tenant, &act2_idem, now, None),
    )
    .await
    .expect_err("double activation must fail");
    assert!(
        matches!(err, RevisionError::AlreadyActivated),
        "expected AlreadyActivated, got: {:?}",
        err
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 6. Tenant isolation: revision scoped per tenant
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = format!("test-a-{}", Uuid::new_v4());
    let tenant_b = format!("test-b-{}", Uuid::new_v4());

    let item_a = ItemRepo::create(&pool, &make_item(&tenant_a, "SKU-ISO-001"))
        .await
        .expect("create item A");
    let item_b = ItemRepo::create(&pool, &make_item(&tenant_b, "SKU-ISO-001"))
        .await
        .expect("create item B");

    // Create revision for tenant A
    let idem_a = format!("idem-{}", Uuid::new_v4());
    let (rev_a, _) = create_revision(&pool, &make_create_rev(&tenant_a, item_a.id, &idem_a))
        .await
        .expect("create rev A");

    // Activate for tenant A
    let now = Utc::now();
    let act_idem_a = format!("idem-{}", Uuid::new_v4());
    activate_revision(
        &pool,
        item_a.id,
        rev_a.id,
        &make_activate(&tenant_a, &act_idem_a, now, None),
    )
    .await
    .expect("activate rev A");

    // Tenant B should see no revisions for their item
    let revs_b = list_revisions(&pool, &tenant_b, item_b.id)
        .await
        .expect("list revs B");
    assert!(revs_b.is_empty(), "tenant B should have no revisions");

    // Tenant B cannot see tenant A's revision
    let found = revision_at(&pool, &tenant_b, item_a.id, now)
        .await
        .expect("revision_at cross-tenant");
    assert!(found.is_none(), "cross-tenant query must return nothing");

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}

// ============================================================================
// 7. Concurrent creation: two revisions in quick succession
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_concurrent_creation() {
    let pool = setup_db().await;
    let tenant = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-REV-CONC"))
        .await
        .expect("create item");

    // Create two revisions concurrently
    let idem1 = format!("idem-conc-{}", Uuid::new_v4());
    let idem2 = format!("idem-conc-{}", Uuid::new_v4());

    let mut req1 = make_create_rev(&tenant, item.id, &idem1);
    req1.name = "Widget Concurrent A".to_string();
    req1.change_reason = "Concurrent A".to_string();

    let mut req2 = make_create_rev(&tenant, item.id, &idem2);
    req2.name = "Widget Concurrent B".to_string();
    req2.change_reason = "Concurrent B".to_string();

    let pool2 = pool.clone();
    let (r1, r2) = tokio::join!(
        create_revision(&pool, &req1),
        create_revision(&pool2, &req2),
    );

    // Both should succeed (one may retry on unique violation for revision_number)
    // but at least one must succeed
    let results: Vec<_> = [r1, r2].into_iter().filter_map(|r| r.ok()).collect();
    assert!(
        !results.is_empty(),
        "at least one concurrent creation must succeed"
    );

    // If both succeeded, they must have different revision numbers
    if results.len() == 2 {
        let nums: Vec<i32> = results.iter().map(|(rev, _)| rev.revision_number).collect();
        assert_ne!(nums[0], nums[1], "revision numbers must be unique");
    }

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 8. Guard: inactive item rejected for revision creation
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_guard_rejects_inactive_item() {
    let pool = setup_db().await;
    let tenant = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-REV-INACT"))
        .await
        .expect("create item");

    ItemRepo::deactivate(&pool, item.id, &tenant)
        .await
        .expect("deactivate");

    let idem = format!("idem-{}", Uuid::new_v4());
    let err = create_revision(&pool, &make_create_rev(&tenant, item.id, &idem))
        .await
        .expect_err("inactive item must fail");

    assert!(
        matches!(err, RevisionError::ItemInactive),
        "expected ItemInactive, got: {:?}",
        err
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 9. Effective window query: bounded revision found correctly
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_bounded_effective_window() {
    let pool = setup_db().await;
    let tenant = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-REV-BOUND"))
        .await
        .expect("create item");

    // Create and activate a revision with a bounded window
    let idem = format!("idem-{}", Uuid::new_v4());
    let (rev, _) = create_revision(&pool, &make_create_rev(&tenant, item.id, &idem))
        .await
        .expect("create rev");

    let start = Utc::now() - Duration::hours(5);
    let end = Utc::now() - Duration::hours(1);
    let act_idem = format!("idem-{}", Uuid::new_v4());
    activate_revision(
        &pool,
        item.id,
        rev.id,
        &make_activate(&tenant, &act_idem, start, Some(end)),
    )
    .await
    .expect("activate with bounded window");

    // Query within window — should find it
    let mid = start + Duration::hours(2);
    let found = revision_at(&pool, &tenant, item.id, mid)
        .await
        .expect("query mid")
        .expect("should find within window");
    assert_eq!(found.id, rev.id);

    // Query after window — should not find it
    let after = end + Duration::hours(1);
    let not_found = revision_at(&pool, &tenant, item.id, after)
        .await
        .expect("query after");
    assert!(not_found.is_none(), "should not find after window closes");

    // Query before window — should not find it
    let before = start - Duration::hours(1);
    let not_found_before = revision_at(&pool, &tenant, item.id, before)
        .await
        .expect("query before");
    assert!(
        not_found_before.is_none(),
        "should not find before window opens"
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 10. Policy flags resolve by effective time
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_policy_flags_resolve_at_time_t() {
    let pool = setup_db().await;
    let tenant = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item(&tenant, "SKU-REV-POL-TIME"))
        .await
        .expect("create item");

    let (rev1, _) = create_revision(
        &pool,
        &make_create_rev(&tenant, item.id, &format!("idem-r1-{}", Uuid::new_v4())),
    )
    .await
    .expect("create rev1");

    let (rev1, _) = update_revision_policy(
        &pool,
        item.id,
        rev1.id,
        &make_update_policy(
            &tenant,
            &format!("idem-pol-r1-{}", Uuid::new_v4()),
            "lot",
            true,
            Some(180),
            true,
        ),
    )
    .await
    .expect("update rev1 policy");

    let t1 = Utc::now() - Duration::hours(3);
    activate_revision(
        &pool,
        item.id,
        rev1.id,
        &make_activate(
            &tenant,
            &format!("idem-act-r1-{}", Uuid::new_v4()),
            t1,
            None,
        ),
    )
    .await
    .expect("activate rev1");

    let mut req2 = make_create_rev(&tenant, item.id, &format!("idem-r2-{}", Uuid::new_v4()));
    req2.name = "Widget Rev 2".to_string();
    let (rev2, _) = create_revision(&pool, &req2).await.expect("create rev2");

    let (rev2, _) = update_revision_policy(
        &pool,
        item.id,
        rev2.id,
        &make_update_policy(
            &tenant,
            &format!("idem-pol-r2-{}", Uuid::new_v4()),
            "serial",
            true,
            Some(365),
            true,
        ),
    )
    .await
    .expect("update rev2 policy");

    let t2 = Utc::now() - Duration::hours(1);
    activate_revision(
        &pool,
        item.id,
        rev2.id,
        &make_activate(
            &tenant,
            &format!("idem-act-r2-{}", Uuid::new_v4()),
            t2,
            None,
        ),
    )
    .await
    .expect("activate rev2");

    let mid = t1 + Duration::minutes(30);
    let at_mid = revision_at(&pool, &tenant, item.id, mid)
        .await
        .expect("query mid")
        .expect("rev present");
    assert_eq!(at_mid.traceability_level, "lot");
    assert!(at_mid.inspection_required);
    assert_eq!(at_mid.shelf_life_days, Some(180));
    assert!(at_mid.shelf_life_enforced);

    let at_t2 = revision_at(&pool, &tenant, item.id, t2)
        .await
        .expect("query t2")
        .expect("rev present at t2");
    assert_eq!(at_t2.traceability_level, "serial");
    assert!(at_t2.inspection_required);
    assert_eq!(at_t2.shelf_life_days, Some(365));
    assert!(at_t2.shelf_life_enforced);

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// 11. Policy update idempotency is tenant-scoped
// ============================================================================

#[tokio::test]
#[serial]
async fn revision_policy_update_idempotent_and_tenant_scoped() {
    let pool = setup_db().await;
    let tenant_a = format!("test-a-{}", Uuid::new_v4());
    let tenant_b = format!("test-b-{}", Uuid::new_v4());

    let item_a = ItemRepo::create(&pool, &make_item(&tenant_a, "SKU-POL-ISO-1"))
        .await
        .expect("create item A");
    let item_b = ItemRepo::create(&pool, &make_item(&tenant_b, "SKU-POL-ISO-1"))
        .await
        .expect("create item B");

    let (rev_a, _) = create_revision(
        &pool,
        &make_create_rev(
            &tenant_a,
            item_a.id,
            &format!("idem-a-create-{}", Uuid::new_v4()),
        ),
    )
    .await
    .expect("create rev A");
    let (rev_b, _) = create_revision(
        &pool,
        &make_create_rev(
            &tenant_b,
            item_b.id,
            &format!("idem-b-create-{}", Uuid::new_v4()),
        ),
    )
    .await
    .expect("create rev B");

    let shared_idem = format!("idem-shared-{}", Uuid::new_v4());
    let req_a = make_update_policy(&tenant_a, &shared_idem, "lot", true, Some(120), true);
    let req_b = make_update_policy(&tenant_b, &shared_idem, "serial", false, None, false);

    let (a1, replay_a1) = update_revision_policy(&pool, item_a.id, rev_a.id, &req_a)
        .await
        .expect("update A first");
    assert!(!replay_a1);
    let (a2, replay_a2) = update_revision_policy(&pool, item_a.id, rev_a.id, &req_a)
        .await
        .expect("update A replay");
    assert!(replay_a2);
    assert_eq!(a1.id, a2.id);
    assert_eq!(a2.traceability_level, "lot");

    let (b1, replay_b1) = update_revision_policy(&pool, item_b.id, rev_b.id, &req_b)
        .await
        .expect("update B with same idempotency key");
    assert!(!replay_b1);
    assert_eq!(b1.traceability_level, "serial");

    let mut conflicting =
        make_update_policy(&tenant_a, &shared_idem, "serial", true, Some(120), true);
    conflicting.traceability_level = "batch".to_string();
    let err = update_revision_policy(&pool, item_a.id, rev_a.id, &conflicting)
        .await
        .expect_err("conflicting payload must fail");
    assert!(
        matches!(err, RevisionError::ConflictingIdempotencyKey),
        "expected conflict, got: {:?}",
        err
    );

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}
