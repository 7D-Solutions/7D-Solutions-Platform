//! Integration tests for Phase E: Maintenance ↔ Production integration (bd-1kw8s)
//!
//! Tests:
//! 1. Workcenter projection — upsert from production events
//! 2. Downtime bridge — production downtime started → maintenance record
//! 3. Downtime bridge — production downtime ended → maintenance record updated
//! 4. Dedup — duplicate events are safely skipped

use chrono::Utc;
use maintenance_rs::consumers::production_downtime_bridge::{
    process_downtime_ended, process_downtime_started, DowntimeEndedPayload, DowntimeStartedPayload,
};
use maintenance_rs::consumers::production_workcenter_bridge::{
    list_workcenter_projections, upsert_workcenter_projection,
};
use maintenance_rs::domain::downtime::{DowntimeRepo, ListDowntimeQuery};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://maintenance_user:maintenance_pass@localhost:5452/maintenance_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to maintenance test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run maintenance migrations");

    pool
}

fn unique_tenant(prefix: &str) -> String {
    format!("{}-{}", prefix, Uuid::new_v4().simple())
}

// ============================================================================
// 1. Workcenter projection — create event → projection upserted
// ============================================================================

#[tokio::test]
#[serial]
async fn workcenter_projection_created_from_production_event() {
    let pool = setup_db().await;
    let tid = unique_tenant("wc-proj");
    let wc_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    upsert_workcenter_projection(
        &pool,
        event_id,
        wc_id,
        &tid,
        "WC-001",
        "Assembly Line 1",
        true,
    )
    .await
    .expect("upsert projection");

    let projections = list_workcenter_projections(&pool, &tid).await.unwrap();
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].workcenter_id, wc_id);
    assert_eq!(projections[0].code, "WC-001");
    assert_eq!(projections[0].name, "Assembly Line 1");
    assert!(projections[0].is_active);
}

// ============================================================================
// 2. Workcenter projection — update overwrites name
// ============================================================================

#[tokio::test]
#[serial]
async fn workcenter_projection_updated_from_production_event() {
    let pool = setup_db().await;
    let tid = unique_tenant("wc-upd");
    let wc_id = Uuid::new_v4();

    upsert_workcenter_projection(
        &pool,
        Uuid::new_v4(),
        wc_id,
        &tid,
        "WC-UPD",
        "Original Name",
        true,
    )
    .await
    .unwrap();

    // Second event updates the name
    upsert_workcenter_projection(
        &pool,
        Uuid::new_v4(),
        wc_id,
        &tid,
        "WC-UPD",
        "Updated Name",
        true,
    )
    .await
    .unwrap();

    let projections = list_workcenter_projections(&pool, &tid).await.unwrap();
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].name, "Updated Name");
}

// ============================================================================
// 3. Workcenter projection — deactivate sets is_active=false
// ============================================================================

#[tokio::test]
#[serial]
async fn workcenter_projection_deactivated_from_production_event() {
    let pool = setup_db().await;
    let tid = unique_tenant("wc-deact");
    let wc_id = Uuid::new_v4();

    upsert_workcenter_projection(
        &pool,
        Uuid::new_v4(),
        wc_id,
        &tid,
        "WC-DEACT",
        "Deactivated WC",
        true,
    )
    .await
    .unwrap();

    // Deactivation event
    upsert_workcenter_projection(
        &pool,
        Uuid::new_v4(),
        wc_id,
        &tid,
        "WC-DEACT",
        "Deactivated WC",
        false,
    )
    .await
    .unwrap();

    let projections = list_workcenter_projections(&pool, &tid).await.unwrap();
    assert_eq!(projections.len(), 1);
    assert!(!projections[0].is_active);
}

// ============================================================================
// 4. Workcenter projection — dedup: same event_id is safely skipped
// ============================================================================

#[tokio::test]
#[serial]
async fn workcenter_projection_dedup_skips_duplicate() {
    let pool = setup_db().await;
    let tid = unique_tenant("wc-dup");
    let wc_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    upsert_workcenter_projection(&pool, event_id, wc_id, &tid, "WC-DUP", "First", true)
        .await
        .unwrap();

    // Same event_id again — should be skipped, name stays "First"
    upsert_workcenter_projection(
        &pool,
        event_id,
        wc_id,
        &tid,
        "WC-DUP",
        "Should Not Update",
        true,
    )
    .await
    .unwrap();

    let projections = list_workcenter_projections(&pool, &tid).await.unwrap();
    assert_eq!(projections.len(), 1);
    assert_eq!(projections[0].name, "First");
}

// ============================================================================
// 5. Downtime bridge — started event creates maintenance downtime record
// ============================================================================

#[tokio::test]
#[serial]
async fn downtime_bridge_started_creates_maintenance_record() {
    let pool = setup_db().await;
    let tid = unique_tenant("dt-bridge");
    let event_id = Uuid::new_v4();
    let prod_downtime_id = Uuid::new_v4();
    let wc_id = Uuid::new_v4();

    let payload = DowntimeStartedPayload {
        downtime_id: prod_downtime_id,
        tenant_id: tid.clone(),
        workcenter_id: wc_id,
        reason: "Machine overheating".to_string(),
        reason_code: Some("OVERHEAT".to_string()),
        started_at: Utc::now(),
        started_by: Some("operator-1".to_string()),
    };

    let result = process_downtime_started(&pool, event_id, &payload).await;
    assert!(result.is_ok());
    let maint_dt_id = result.unwrap().expect("should return downtime ID");

    // Verify maintenance downtime record exists
    let found = DowntimeRepo::find_by_id(&pool, maint_dt_id, &tid)
        .await
        .unwrap()
        .expect("should find downtime record");

    assert_eq!(found.workcenter_id, Some(wc_id));
    assert_eq!(found.reason, "Machine overheating");
    assert_eq!(found.reason_code.as_deref(), Some("OVERHEAT"));
    assert!(found.end_time.is_none());
}

// ============================================================================
// 6. Downtime bridge — ended event updates maintenance downtime with end_time
// ============================================================================

#[tokio::test]
#[serial]
async fn downtime_bridge_ended_updates_maintenance_record() {
    let pool = setup_db().await;
    let tid = unique_tenant("dt-end");
    let prod_downtime_id = Uuid::new_v4();
    let wc_id = Uuid::new_v4();
    let started_at = Utc::now() - chrono::Duration::hours(2);
    let ended_at = Utc::now();

    // First: create via started event
    let started_payload = DowntimeStartedPayload {
        downtime_id: prod_downtime_id,
        tenant_id: tid.clone(),
        workcenter_id: wc_id,
        reason: "Belt replacement".to_string(),
        reason_code: None,
        started_at,
        started_by: None,
    };
    process_downtime_started(&pool, Uuid::new_v4(), &started_payload)
        .await
        .unwrap();

    // Then: end it
    let ended_payload = DowntimeEndedPayload {
        downtime_id: prod_downtime_id,
        tenant_id: tid.clone(),
        workcenter_id: wc_id,
        started_at,
        ended_at,
        ended_by: Some("supervisor".to_string()),
    };
    process_downtime_ended(&pool, Uuid::new_v4(), &ended_payload)
        .await
        .unwrap();

    // Verify the maintenance record now has end_time
    let records = DowntimeRepo::list(
        &pool,
        &ListDowntimeQuery {
            tenant_id: tid.clone(),
            asset_id: None,
            from: None,
            to: None,
            limit: Some(10),
            offset: None,
        },
    )
    .await
    .unwrap();

    let found = records
        .iter()
        .find(|r| r.workcenter_id == Some(wc_id))
        .expect("should find downtime for workcenter");

    assert!(found.end_time.is_some(), "end_time should be set");
}

// ============================================================================
// 7. Downtime bridge — dedup: same event_id is skipped
// ============================================================================

#[tokio::test]
#[serial]
async fn downtime_bridge_dedup_skips_duplicate_started() {
    let pool = setup_db().await;
    let tid = unique_tenant("dt-dup");
    let event_id = Uuid::new_v4();
    let prod_downtime_id = Uuid::new_v4();

    let payload = DowntimeStartedPayload {
        downtime_id: prod_downtime_id,
        tenant_id: tid.clone(),
        workcenter_id: Uuid::new_v4(),
        reason: "First".to_string(),
        reason_code: None,
        started_at: Utc::now(),
        started_by: None,
    };

    // First call creates
    let first = process_downtime_started(&pool, event_id, &payload)
        .await
        .unwrap();
    assert!(first.is_some());

    // Second call with same event_id skips
    let second = process_downtime_started(&pool, event_id, &payload)
        .await
        .unwrap();
    assert!(second.is_none());
}

// ============================================================================
// 8. Tenant isolation — projections scoped to tenant
// ============================================================================

#[tokio::test]
#[serial]
async fn workcenter_projection_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant("wc-iso-a");
    let tid_b = unique_tenant("wc-iso-b");

    upsert_workcenter_projection(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &tid_a,
        "WC-A",
        "Tenant A WC",
        true,
    )
    .await
    .unwrap();

    upsert_workcenter_projection(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &tid_b,
        "WC-B",
        "Tenant B WC",
        true,
    )
    .await
    .unwrap();

    let proj_a = list_workcenter_projections(&pool, &tid_a).await.unwrap();
    let proj_b = list_workcenter_projections(&pool, &tid_b).await.unwrap();

    assert_eq!(proj_a.len(), 1);
    assert_eq!(proj_a[0].code, "WC-A");
    assert_eq!(proj_b.len(), 1);
    assert_eq!(proj_b[0].code, "WC-B");
}
