use bom_rs::domain::bom_service::{self, BomError};
use bom_rs::domain::models::*;
use chrono::{Duration, Utc};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://bom_user:bom_pass@localhost:5450/bom_db".to_string());

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

// ============================================================================
// Create BOM + revision + lines + effectivity
// ============================================================================

#[tokio::test]
#[serial]
async fn create_bom_with_revision_and_lines() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let part_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    // Create BOM header
    let header = bom_service::create_bom(
        &pool,
        &tenant,
        &CreateBomRequest {
            part_id,
            description: Some("Assembly A".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("create_bom");

    assert_eq!(header.tenant_id, tenant);
    assert_eq!(header.part_id, part_id);

    // Create revision
    let rev = bom_service::create_revision(
        &pool,
        &tenant,
        header.id,
        &CreateRevisionRequest {
            revision_label: "Rev A".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("create_revision");

    assert_eq!(rev.status, "draft");

    // Add lines
    let comp1 = Uuid::new_v4();
    let comp2 = Uuid::new_v4();

    let line1 = bom_service::add_line(
        &pool,
        &tenant,
        rev.id,
        &AddLineRequest {
            component_item_id: comp1,
            quantity: 2.0,
            uom: Some("EA".to_string()),
            scrap_factor: Some(0.05),
            find_number: Some(10),
        },
        &corr,
        None,
    )
    .await
    .expect("add_line 1");

    assert_eq!(line1.component_item_id, comp1);
    assert!((line1.quantity - 2.0).abs() < f64::EPSILON);

    let _line2 = bom_service::add_line(
        &pool,
        &tenant,
        rev.id,
        &AddLineRequest {
            component_item_id: comp2,
            quantity: 5.5,
            uom: None,
            scrap_factor: None,
            find_number: Some(20),
        },
        &corr,
        None,
    )
    .await
    .expect("add_line 2");

    // List lines
    let lines = bom_service::list_lines(&pool, &tenant, rev.id)
        .await
        .expect("list_lines");
    assert_eq!(lines.len(), 2);

    // Set effectivity
    let now = Utc::now();
    let effective_rev = bom_service::set_effectivity(
        &pool,
        &tenant,
        rev.id,
        &SetEffectivityRequest {
            effective_from: now,
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .expect("set_effectivity");

    assert_eq!(effective_rev.status, "effective");
    assert!(effective_rev.effective_from.is_some());
}

// Explosion and where-used tests: see bom_queries_integration.rs

// ---- Tenant isolation ----

#[tokio::test]
#[serial]
async fn tenant_isolation_cross_tenant_denied() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Create BOM in tenant A
    let bom = bom_service::create_bom(
        &pool,
        &tenant_a,
        &CreateBomRequest {
            part_id: Uuid::new_v4(),
            description: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Tenant B cannot see it
    let result = bom_service::get_bom(&pool, &tenant_b, bom.id).await;
    assert!(result.is_err());
    match result {
        Err(BomError::Guard(bom_rs::domain::guards::GuardError::NotFound(_))) => {}
        other => panic!("Expected NotFound, got {:?}", other),
    }

    // Tenant B cannot create revisions on it
    let result = bom_service::create_revision(
        &pool,
        &tenant_b,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Sneaky".to_string(),
        },
        &corr,
        None,
    )
    .await;
    assert!(result.is_err());
}

// ============================================================================
// Events emitted via outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn events_emitted_to_outbox() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let bom = bom_service::create_bom(
        &pool,
        &tenant,
        &CreateBomRequest {
            part_id: Uuid::new_v4(),
            description: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let rev = bom_service::create_revision(
        &pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "R1".to_string(),
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    bom_service::add_line(
        &pool,
        &tenant,
        rev.id,
        &AddLineRequest {
            component_item_id: Uuid::new_v4(),
            quantity: 1.0,
            uom: None,
            scrap_factor: None,
            find_number: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    bom_service::set_effectivity(
        &pool,
        &tenant,
        rev.id,
        &SetEffectivityRequest {
            effective_from: Utc::now(),
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Verify events in outbox
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bom_outbox WHERE tenant_id = $1")
        .bind(&tenant)
        .fetch_one(&pool)
        .await
        .unwrap();

    // 4 events: bom.created, bom.revision_created, bom.line_added, bom.effectivity_set
    assert_eq!(count.0, 4, "Expected 4 outbox events, got {}", count.0);

    // Verify event types
    let event_types: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM bom_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let types: Vec<&str> = event_types.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "bom.created",
            "bom.revision_created",
            "bom.line_added",
            "bom.effectivity_set"
        ]
    );

    // Verify envelope has required fields
    let payload: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM bom_outbox WHERE tenant_id = $1 AND event_type = 'bom.created'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(payload.0["source_module"], "bom");
    assert_eq!(payload.0["replay_safe"], true);
    assert!(payload.0["event_id"].is_string());
    assert!(payload.0["tenant_id"].is_string());
    assert!(payload.0["correlation_id"].is_string());
    assert_eq!(payload.0["mutation_class"], "DATA_MUTATION");
}

// ============================================================================
// Effectivity overlap prevention
// ============================================================================

#[tokio::test]
#[serial]
async fn effectivity_overlap_supersedes_previous() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let now = Utc::now();

    let bom = bom_service::create_bom(
        &pool,
        &tenant,
        &CreateBomRequest {
            part_id: Uuid::new_v4(),
            description: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Rev 1: effective from now to +30 days
    let rev1 = bom_service::create_revision(
        &pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Rev-1".to_string(),
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    bom_service::set_effectivity(
        &pool,
        &tenant,
        rev1.id,
        &SetEffectivityRequest {
            effective_from: now,
            effective_to: Some(now + Duration::days(30)),
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Rev 2: overlapping range — should supersede Rev 1
    let rev2 = bom_service::create_revision(
        &pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Rev-2".to_string(),
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    bom_service::set_effectivity(
        &pool,
        &tenant,
        rev2.id,
        &SetEffectivityRequest {
            effective_from: now + Duration::days(10),
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Rev 1 should now be superseded
    let rev1_updated = bom_service::get_revision(&pool, &tenant, rev1.id)
        .await
        .unwrap();
    assert_eq!(
        rev1_updated.status, "superseded",
        "Rev 1 should be superseded"
    );

    // Rev 2 should be effective
    let rev2_updated = bom_service::get_revision(&pool, &tenant, rev2.id)
        .await
        .unwrap();
    assert_eq!(rev2_updated.status, "effective");
}

// ============================================================================
// Draft revision: can't add lines to effective revision
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_add_lines_to_effective_revision() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let bom = bom_service::create_bom(
        &pool,
        &tenant,
        &CreateBomRequest {
            part_id: Uuid::new_v4(),
            description: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let rev = bom_service::create_revision(
        &pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "R1".to_string(),
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Make it effective
    bom_service::set_effectivity(
        &pool,
        &tenant,
        rev.id,
        &SetEffectivityRequest {
            effective_from: Utc::now(),
            effective_to: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Try to add a line — should fail
    let result = bom_service::add_line(
        &pool,
        &tenant,
        rev.id,
        &AddLineRequest {
            component_item_id: Uuid::new_v4(),
            quantity: 1.0,
            uom: None,
            scrap_factor: None,
            find_number: None,
        },
        &corr,
        None,
    )
    .await;

    assert!(result.is_err(), "Should not be able to add lines to effective revision");
}
