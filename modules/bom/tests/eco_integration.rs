use bom_rs::domain::bom_service;
use bom_rs::domain::eco_models::*;
use bom_rs::domain::eco_service;
use bom_rs::domain::models::*;
use chrono::{Duration, Utc};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://bom_user:bom_pass@localhost:5450/bom_db?sslmode=require".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to BOM test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run BOM migrations");

    pool
}

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

/// Helper: create a BOM with two revisions (Rev-A effective, Rev-B draft) and a line on each.
async fn setup_bom_with_revisions(
    pool: &sqlx::PgPool,
    tenant: &str,
) -> (BomHeader, BomRevision, BomRevision) {
    let corr = Uuid::new_v4().to_string();
    let part_id = Uuid::new_v4();

    let header = bom_service::create_bom(
        pool,
        tenant,
        &CreateBomRequest {
            part_id,
            description: Some("ECO Test Assembly".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("create_bom");

    // Rev A: effective
    let rev_a = bom_service::create_revision(
        pool,
        tenant,
        header.id,
        &CreateRevisionRequest {
            revision_label: "Rev-A".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("create rev-a");

    bom_service::add_line(
        pool,
        tenant,
        rev_a.id,
        &AddLineRequest {
            component_item_id: Uuid::new_v4(),
            quantity: 2.0,
            uom: Some("EA".to_string()),
            scrap_factor: None,
            find_number: Some(10),
        },
        &corr,
        None,
    )
    .await
    .expect("add line to rev-a");

    let now = Utc::now() - Duration::days(30);
    let rev_a = bom_service::set_effectivity(
        pool,
        tenant,
        rev_a.id,
        &SetEffectivityRequest {
            effective_from: now,
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .expect("set effectivity rev-a");

    assert_eq!(rev_a.status, "effective");

    // Rev B: draft (the new revision)
    let rev_b = bom_service::create_revision(
        pool,
        tenant,
        header.id,
        &CreateRevisionRequest {
            revision_label: "Rev-B".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("create rev-b");

    bom_service::add_line(
        pool,
        tenant,
        rev_b.id,
        &AddLineRequest {
            component_item_id: Uuid::new_v4(),
            quantity: 3.0,
            uom: Some("EA".to_string()),
            scrap_factor: Some(0.02),
            find_number: Some(10),
        },
        &corr,
        None,
    )
    .await
    .expect("add line to rev-b");

    (header, rev_a, rev_b)
}

// ============================================================================
// Full ECO lifecycle: create -> submit -> approve -> apply -> verify
// ============================================================================

#[tokio::test]
#[serial]
async fn eco_full_lifecycle_applies_bom_revision_supersession() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (header, rev_a, rev_b) = setup_bom_with_revisions(&pool, &tenant).await;

    // Create ECO
    let eco = eco_service::create_eco(
        &pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: Some("ECO-001".to_string()),
            title: "Update assembly components".to_string(),
            description: Some("Replace old component with new spec".to_string()),
            created_by: "engineer-1".to_string(),
        },
        None,
        None,
        &corr,
        None,
    )
    .await
    .expect("create eco");

    assert_eq!(eco.status, "draft");
    assert_eq!(eco.eco_number, "ECO-001");

    // Link BOM revisions (before=Rev-A, after=Rev-B)
    let bom_link = eco_service::link_bom_revision(
        &pool,
        &tenant,
        eco.id,
        &LinkBomRevisionRequest {
            bom_id: header.id,
            before_revision_id: rev_a.id,
            after_revision_id: rev_b.id,
        },
    )
    .await
    .expect("link bom revision");

    assert_eq!(bom_link.before_revision_id, rev_a.id);
    assert_eq!(bom_link.after_revision_id, rev_b.id);

    // Link a doc revision (evidence)
    let doc_id = Uuid::new_v4();
    let doc_rev_id = Uuid::new_v4();
    let doc_link = eco_service::link_doc_revision(
        &pool,
        &tenant,
        eco.id,
        &LinkDocRevisionRequest {
            doc_id,
            doc_revision_id: doc_rev_id,
        },
    )
    .await
    .expect("link doc revision");

    assert_eq!(doc_link.doc_id, doc_id);

    // Submit ECO
    let eco = eco_service::submit_eco(
        &pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "engineer-1".to_string(),
            comment: Some("Ready for review".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("submit eco");

    assert_eq!(eco.status, "submitted");

    // Approve ECO
    let eco = eco_service::approve_eco(
        &pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "manager-1".to_string(),
            comment: Some("Approved".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("approve eco");

    assert_eq!(eco.status, "approved");
    assert!(eco.approved_by.is_some());

    // Apply ECO — this drives BOM revision supersession
    let effective_from = Utc::now();
    let eco = eco_service::apply_eco(
        &pool,
        &tenant,
        eco.id,
        &ApplyEcoRequest {
            actor: "engineer-1".to_string(),
            effective_from,
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .expect("apply eco");

    assert_eq!(eco.status, "applied");
    assert!(eco.applied_at.is_some());

    // Verify: Rev-A is now superseded
    let rev_a_after = bom_service::get_revision(&pool, &tenant, rev_a.id)
        .await
        .expect("get rev-a");
    assert_eq!(
        rev_a_after.status, "superseded",
        "Rev-A should be superseded after ECO apply"
    );

    // Verify: Rev-B is now effective with correct effectivity
    let rev_b_after = bom_service::get_revision(&pool, &tenant, rev_b.id)
        .await
        .expect("get rev-b");
    assert_eq!(
        rev_b_after.status, "effective",
        "Rev-B should be effective after ECO apply"
    );
    assert!(rev_b_after.effective_from.is_some());

    // Verify: audit trail has complete history
    let audit = eco_service::get_audit_trail(&pool, &tenant, eco.id)
        .await
        .expect("audit trail");

    let actions: Vec<&str> = audit.iter().map(|a| a.action.as_str()).collect();
    assert_eq!(
        actions,
        vec!["created", "submitted", "approved", "applied"],
        "Audit trail should show full lifecycle"
    );

    // Verify: doc revision links are queryable
    let doc_links = eco_service::list_doc_revision_links(&pool, &tenant, eco.id)
        .await
        .expect("doc links");
    assert_eq!(doc_links.len(), 1);
    assert_eq!(doc_links[0].doc_revision_id, doc_rev_id);

    // Verify: ECO history for part
    let history = eco_service::eco_history_for_part(&pool, &tenant, header.part_id)
        .await
        .expect("eco history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].eco_number, "ECO-001");
}

// ============================================================================
// Query: BOM revision effective on date X
// ============================================================================

#[tokio::test]
#[serial]
async fn query_bom_revision_effective_on_date() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (header, rev_a, rev_b) = setup_bom_with_revisions(&pool, &tenant).await;

    // Create and apply ECO to supersede Rev-A with Rev-B
    let eco = eco_service::create_eco(
        &pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: Some("ECO-DATE-TEST".to_string()),
            title: "Date effectivity test".to_string(),
            description: None,
            created_by: "eng-1".to_string(),
        },
        None,
        None,
        &corr,
        None,
    )
    .await
    .unwrap();

    eco_service::link_bom_revision(
        &pool,
        &tenant,
        eco.id,
        &LinkBomRevisionRequest {
            bom_id: header.id,
            before_revision_id: rev_a.id,
            after_revision_id: rev_b.id,
        },
    )
    .await
    .unwrap();

    eco_service::submit_eco(
        &pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "eng-1".to_string(),
            comment: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    eco_service::approve_eco(
        &pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "mgr-1".to_string(),
            comment: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Apply with effectivity starting now
    let now = Utc::now();
    eco_service::apply_eco(
        &pool,
        &tenant,
        eco.id,
        &ApplyEcoRequest {
            actor: "eng-1".to_string(),
            effective_from: now,
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Query explosion with current date — should use Rev-B lines
    let explosion = bom_service::explode(
        &pool,
        &tenant,
        header.id,
        &ExplosionQuery {
            date: Some(now + Duration::seconds(1)),
            max_depth: None,
        },
    )
    .await
    .expect("explosion at current date");

    assert!(!explosion.is_empty(), "Should have explosion rows at current date");
    assert_eq!(
        explosion[0].revision_label, "Rev-B",
        "Current date should resolve to Rev-B"
    );
}

// ============================================================================
// Guard: only approved ECO can be applied
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_apply_draft_eco() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let eco = eco_service::create_eco(
        &pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: Some("ECO-GUARD".to_string()),
            title: "Guard test".to_string(),
            description: None,
            created_by: "eng-1".to_string(),
        },
        None,
        None,
        &corr,
        None,
    )
    .await
    .unwrap();

    let result = eco_service::apply_eco(
        &pool,
        &tenant,
        eco.id,
        &ApplyEcoRequest {
            actor: "eng-1".to_string(),
            effective_from: Utc::now(),
            effective_to: None,
        },
        &corr,
        None,
    )
    .await;

    assert!(result.is_err(), "Should not be able to apply a draft ECO");
}

// ============================================================================
// Guard: cannot bypass ECO for BOM revision supersession
// ============================================================================

#[tokio::test]
#[serial]
async fn eco_events_emitted_to_outbox() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (header, rev_a, rev_b) = setup_bom_with_revisions(&pool, &tenant).await;

    // Count outbox events before ECO
    let before_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bom_outbox WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();

    // Full ECO lifecycle
    let eco = eco_service::create_eco(
        &pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: Some("ECO-EVENTS".to_string()),
            title: "Event test".to_string(),
            description: None,
            created_by: "eng-1".to_string(),
        },
        None,
        None,
        &corr,
        None,
    )
    .await
    .unwrap();

    eco_service::link_bom_revision(
        &pool,
        &tenant,
        eco.id,
        &LinkBomRevisionRequest {
            bom_id: header.id,
            before_revision_id: rev_a.id,
            after_revision_id: rev_b.id,
        },
    )
    .await
    .unwrap();

    eco_service::submit_eco(
        &pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "eng-1".to_string(),
            comment: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    eco_service::approve_eco(
        &pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "mgr-1".to_string(),
            comment: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    eco_service::apply_eco(
        &pool,
        &tenant,
        eco.id,
        &ApplyEcoRequest {
            actor: "eng-1".to_string(),
            effective_from: Utc::now(),
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Count outbox events after ECO
    let after_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bom_outbox WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();

    // ECO lifecycle emits: eco.created + eco.submitted + eco.approved
    //   + (bom.revision_superseded + bom.revision_released + eco.applied) per link
    // = 3 + 3 = 6 ECO-related events
    let eco_events = after_count.0 - before_count.0;
    assert!(
        eco_events >= 6,
        "Expected at least 6 ECO events, got {}",
        eco_events
    );

    // Verify specific event types are present
    let eco_event_types: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT event_type FROM bom_outbox
        WHERE tenant_id = $1 AND event_type LIKE 'eco.%'
        ORDER BY created_at
        "#,
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let types: Vec<&str> = eco_event_types.iter().map(|r| r.0.as_str()).collect();
    assert!(types.contains(&"eco.created"));
    assert!(types.contains(&"eco.submitted"));
    assert!(types.contains(&"eco.approved"));
    assert!(types.contains(&"eco.applied"));

    // Verify BOM revision events
    let rev_events: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT event_type FROM bom_outbox
        WHERE tenant_id = $1 AND event_type LIKE 'bom.revision_%'
        ORDER BY created_at
        "#,
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let rev_types: Vec<&str> = rev_events.iter().map(|r| r.0.as_str()).collect();
    assert!(rev_types.contains(&"bom.revision_superseded"));
    assert!(rev_types.contains(&"bom.revision_released"));
}

// ============================================================================
// ECO rejection
// ============================================================================

#[tokio::test]
#[serial]
async fn eco_rejection_preserves_bom_state() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (_header, rev_a, _rev_b) = setup_bom_with_revisions(&pool, &tenant).await;

    let eco = eco_service::create_eco(
        &pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: Some("ECO-REJECT".to_string()),
            title: "Reject test".to_string(),
            description: None,
            created_by: "eng-1".to_string(),
        },
        None,
        None,
        &corr,
        None,
    )
    .await
    .unwrap();

    eco_service::submit_eco(
        &pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "eng-1".to_string(),
            comment: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let eco = eco_service::reject_eco(
        &pool,
        &tenant,
        eco.id,
        &EcoActionRequest {
            actor: "mgr-1".to_string(),
            comment: Some("Needs more analysis".to_string()),
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    assert_eq!(eco.status, "rejected");

    // Verify Rev-A is still effective (unchanged)
    let rev_a_check = bom_service::get_revision(&pool, &tenant, rev_a.id)
        .await
        .unwrap();
    assert_eq!(rev_a_check.status, "effective");
}
