//! Integration tests for asset tracking and downtime events (bd-127te).
//!
//! Covers all 6 required test categories:
//! 1. Asset CRUD E2E
//! 2. Downtime event E2E
//! 3. Asset-downtime relationship test
//! 4. Tenant isolation test
//! 5. Idempotency test
//! 6. Outbox event test

use chrono::{Duration, Utc};
use maintenance_rs::domain::assets::{
    AssetError, AssetRepo, CreateAssetRequest, ListAssetsQuery, UpdateAssetRequest,
};
use maintenance_rs::domain::downtime::{
    CreateDowntimeRequest, DowntimeError, DowntimeRepo, ListDowntimeQuery,
};
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

fn base_asset_req(tid: &str, tag: &str) -> CreateAssetRequest {
    CreateAssetRequest {
        tenant_id: tid.to_string(),
        asset_tag: tag.to_string(),
        name: format!("Test Asset {}", tag),
        description: None,
        asset_type: "equipment".into(),
        location: Some("Building A".into()),
        department: None,
        responsible_person: None,
        serial_number: Some(format!("SN-{}", tag)),
        fixed_asset_ref: None,
        metadata: None,
        maintenance_schedule: Some(serde_json::json!({"interval_days": 90})),
        idempotency_key: None,
    }
}

// ============================================================================
// 1. Asset CRUD E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn test_asset_crud_e2e() {
    let pool = setup_db().await;
    let tid = unique_tenant("asset-crud");

    // Create
    let asset = AssetRepo::create(&pool, &base_asset_req(&tid, "CRUD-001"))
        .await
        .unwrap();
    assert_eq!(asset.tenant_id, tid);
    assert_eq!(asset.asset_tag, "CRUD-001");
    assert_eq!(asset.name, "Test Asset CRUD-001");
    assert_eq!(asset.serial_number.as_deref(), Some("SN-CRUD-001"));
    assert_eq!(asset.location.as_deref(), Some("Building A"));
    assert_eq!(asset.status.as_str(), "active");
    assert!(asset.maintenance_schedule.is_some());

    // Read
    let found = AssetRepo::find_by_id(&pool, asset.id, &tid)
        .await
        .unwrap()
        .expect("asset should be found");
    assert_eq!(found.id, asset.id);

    // Update status
    let updated = AssetRepo::update(
        &pool,
        asset.id,
        &tid,
        &UpdateAssetRequest {
            name: None,
            description: None,
            asset_type: None,
            location: Some("Building B".into()),
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            status: Some("inactive".into()),
            metadata: None,
            maintenance_schedule: None,
            out_of_service: None,
            out_of_service_reason: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.status.as_str(), "inactive");
    assert_eq!(updated.location.as_deref(), Some("Building B"));

    // List
    let list = AssetRepo::list(
        &pool,
        &ListAssetsQuery {
            tenant_id: tid.clone(),
            asset_type: None,
            status: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(list.items.iter().any(|a| a.id == asset.id));
}

// ============================================================================
// 2. Downtime event E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn test_downtime_event_e2e() {
    let pool = setup_db().await;
    let tid = unique_tenant("dt-e2e");

    let asset = AssetRepo::create(&pool, &base_asset_req(&tid, "DT-001"))
        .await
        .unwrap();

    let start = Utc::now() - Duration::hours(2);
    let end = Utc::now() - Duration::hours(1);

    let dt = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid.clone(),
            asset_id: Some(asset.id),
            start_time: start,
            end_time: Some(end),
            reason: "Bearing replacement".into(),
            impact_classification: "major".into(),
            idempotency_key: None,
            notes: Some("Scheduled maintenance".into()),
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(dt.tenant_id, tid);
    assert_eq!(dt.asset_id, Some(asset.id));
    assert_eq!(dt.reason, "Bearing replacement");
    assert_eq!(dt.impact_classification, "major");
    assert!(dt.end_time.is_some());
    assert_eq!(dt.notes.as_deref(), Some("Scheduled maintenance"));

    // Read back
    let found = DowntimeRepo::find_by_id(&pool, dt.id, &tid)
        .await
        .unwrap()
        .expect("downtime should be found");
    assert_eq!(found.id, dt.id);
    assert_eq!(found.reason, "Bearing replacement");

    // Validate end_time > start_time invariant
    let err = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid.clone(),
            asset_id: Some(asset.id),
            start_time: end,
            end_time: Some(start), // end before start
            reason: "Bad times".into(),
            impact_classification: "minor".into(),
            idempotency_key: None,
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await;
    assert!(matches!(err, Err(DowntimeError::Validation(_))));
}

// ============================================================================
// 3. Asset-downtime relationship test
// ============================================================================

#[tokio::test]
#[serial]
async fn test_asset_downtime_relationship() {
    let pool = setup_db().await;
    let tid = unique_tenant("dt-rel");

    let asset = AssetRepo::create(&pool, &base_asset_req(&tid, "REL-001"))
        .await
        .unwrap();

    let now = Utc::now();

    // Create multiple downtime events at different times
    let dt1 = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid.clone(),
            asset_id: Some(asset.id),
            start_time: now - Duration::hours(6),
            end_time: Some(now - Duration::hours(5)),
            reason: "Oil change".into(),
            impact_classification: "none".into(),
            idempotency_key: None,
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await
    .unwrap();

    let dt2 = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid.clone(),
            asset_id: Some(asset.id),
            start_time: now - Duration::hours(3),
            end_time: Some(now - Duration::hours(2)),
            reason: "Belt replacement".into(),
            impact_classification: "minor".into(),
            idempotency_key: None,
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await
    .unwrap();

    let dt3 = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid.clone(),
            asset_id: Some(asset.id),
            start_time: now - Duration::hours(1),
            end_time: None, // ongoing
            reason: "Motor failure".into(),
            impact_classification: "critical".into(),
            idempotency_key: None,
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await
    .unwrap();

    // Query downtime history for asset
    let history = DowntimeRepo::list_for_asset(&pool, asset.id, &tid)
        .await
        .unwrap();
    assert_eq!(history.len(), 3);

    // Verify ordering: most recent first (start_time DESC)
    assert_eq!(history[0].id, dt3.id);
    assert_eq!(history[1].id, dt2.id);
    assert_eq!(history[2].id, dt1.id);

    // Also test via list query with asset_id filter
    let listed = DowntimeRepo::list(
        &pool,
        &ListDowntimeQuery {
            tenant_id: tid.clone(),
            asset_id: Some(asset.id),
            limit: None,
            offset: None,
            from: None,
            to: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.len(), 3);
}

// ============================================================================
// 4. Tenant isolation test
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant("iso-a");
    let tid_b = unique_tenant("iso-b");

    // Create assets under each tenant
    let asset_a = AssetRepo::create(&pool, &base_asset_req(&tid_a, "ISO-A"))
        .await
        .unwrap();
    let asset_b = AssetRepo::create(&pool, &base_asset_req(&tid_b, "ISO-B"))
        .await
        .unwrap();

    // Create downtime under each tenant
    let now = Utc::now();
    DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid_a.clone(),
            asset_id: Some(asset_a.id),
            start_time: now - Duration::hours(1),
            end_time: Some(now),
            reason: "Tenant A downtime".into(),
            impact_classification: "minor".into(),
            idempotency_key: None,
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await
    .unwrap();

    DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid_b.clone(),
            asset_id: Some(asset_b.id),
            start_time: now - Duration::hours(1),
            end_time: Some(now),
            reason: "Tenant B downtime".into(),
            impact_classification: "major".into(),
            idempotency_key: None,
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await
    .unwrap();

    // Tenant A cannot see tenant B's asset
    let cross_asset = AssetRepo::find_by_id(&pool, asset_b.id, &tid_a)
        .await
        .unwrap();
    assert!(
        cross_asset.is_none(),
        "Tenant A must not see Tenant B's asset"
    );

    // Tenant B cannot see tenant A's asset
    let cross_asset_rev = AssetRepo::find_by_id(&pool, asset_a.id, &tid_b)
        .await
        .unwrap();
    assert!(
        cross_asset_rev.is_none(),
        "Tenant B must not see Tenant A's asset"
    );

    // Asset list isolation
    let a_assets = AssetRepo::list(
        &pool,
        &ListAssetsQuery {
            tenant_id: tid_a.clone(),
            asset_type: None,
            status: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    for a in &a_assets.items {
        assert_eq!(
            a.tenant_id, tid_a,
            "Asset list must only contain tenant A's assets"
        );
    }

    // Downtime isolation: tenant A sees zero results when querying tenant B's asset
    let cross_dt = DowntimeRepo::list_for_asset(&pool, asset_b.id, &tid_a)
        .await
        .unwrap();
    assert_eq!(
        cross_dt.len(),
        0,
        "Tenant A must not see Tenant B's downtime"
    );

    let cross_dt_rev = DowntimeRepo::list_for_asset(&pool, asset_a.id, &tid_b)
        .await
        .unwrap();
    assert_eq!(
        cross_dt_rev.len(),
        0,
        "Tenant B must not see Tenant A's downtime"
    );

    // Full downtime list is scoped to tenant
    let a_dt = DowntimeRepo::list(
        &pool,
        &ListDowntimeQuery {
            tenant_id: tid_a.clone(),
            asset_id: None,
            limit: None,
            offset: None,
            from: None,
            to: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(a_dt.len(), 1);
    assert_eq!(a_dt[0].reason, "Tenant A downtime");

    let b_dt = DowntimeRepo::list(
        &pool,
        &ListDowntimeQuery {
            tenant_id: tid_b.clone(),
            asset_id: None,
            limit: None,
            offset: None,
            from: None,
            to: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(b_dt.len(), 1);
    assert_eq!(b_dt[0].reason, "Tenant B downtime");
}

// ============================================================================
// 5. Idempotency test
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotency() {
    let pool = setup_db().await;
    let tid = unique_tenant("idem");

    // Create asset with idempotency key
    let ikey = format!("idem-asset-{}", Uuid::new_v4());
    let mut req = base_asset_req(&tid, "IDEM-001");
    req.idempotency_key = Some(ikey.clone());

    let asset1 = AssetRepo::create(&pool, &req).await.unwrap();

    // Second attempt with same idempotency key returns the existing asset
    let result = AssetRepo::create(&pool, &req).await;
    match result {
        Err(AssetError::IdempotentDuplicate(existing)) => {
            assert_eq!(existing.id, asset1.id, "should return the same asset");
        }
        _ => panic!("expected IdempotentDuplicate, got {:?}", result),
    }

    // Create asset for downtime tests
    let asset = AssetRepo::create(&pool, &base_asset_req(&tid, "IDEM-DT"))
        .await
        .unwrap();

    // Create downtime with idempotency key
    let dt_ikey = format!("idem-dt-{}", Uuid::new_v4());
    let now = Utc::now();

    let dt1 = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid.clone(),
            asset_id: Some(asset.id),
            start_time: now - Duration::hours(1),
            end_time: Some(now),
            reason: "First submission".into(),
            impact_classification: "minor".into(),
            idempotency_key: Some(dt_ikey.clone()),
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await
    .unwrap();

    // Submit same downtime event again with same idempotency key — no duplicate
    let result = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid.clone(),
            asset_id: Some(asset.id),
            start_time: now - Duration::hours(1),
            end_time: Some(now),
            reason: "Duplicate submission".into(),
            impact_classification: "minor".into(),
            idempotency_key: Some(dt_ikey.clone()),
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await;
    match result {
        Err(DowntimeError::IdempotentDuplicate(existing)) => {
            assert_eq!(existing.id, dt1.id, "should return the same downtime event");
            assert_eq!(
                existing.reason, "First submission",
                "should return original data, not duplicate"
            );
        }
        _ => panic!("expected IdempotentDuplicate, got {:?}", result),
    }

    // Verify only one downtime event exists for this asset
    let events = DowntimeRepo::list_for_asset(&pool, asset.id, &tid)
        .await
        .unwrap();
    assert_eq!(events.len(), 1, "idempotency should prevent duplicate");
}

// ============================================================================
// 6. Outbox event test
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbox_events() {
    let pool = setup_db().await;
    let tid = unique_tenant("outbox");

    // Create asset — should produce outbox event
    let asset = AssetRepo::create(&pool, &base_asset_req(&tid, "OBX-001"))
        .await
        .unwrap();

    let asset_event: Option<(String, String)> = sqlx::query_as(
        "SELECT event_type, aggregate_type FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(asset.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();
    let (event_type, agg_type) = asset_event.expect("asset creation should emit outbox event");
    assert_eq!(event_type, "maintenance.asset.created");
    assert_eq!(agg_type, "asset");

    // Verify envelope contains tenant_id
    let payload: (serde_json::Value,) =
        sqlx::query_as("SELECT payload FROM events_outbox WHERE aggregate_id = $1")
            .bind(asset.id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    let envelope = &payload.0;
    assert_eq!(
        envelope.get("tenant_id").and_then(|v| v.as_str()),
        Some(tid.as_str()),
        "envelope must include tenant_id"
    );

    // Update asset — should produce outbox event
    AssetRepo::update(
        &pool,
        asset.id,
        &tid,
        &UpdateAssetRequest {
            name: None,
            description: None,
            asset_type: None,
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            status: Some("inactive".into()),
            metadata: None,
            maintenance_schedule: None,
            out_of_service: None,
            out_of_service_reason: None,
        },
    )
    .await
    .unwrap();

    let update_events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(asset.id.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(update_events.len(), 2);
    assert_eq!(update_events[0].0, "maintenance.asset.created");
    assert_eq!(update_events[1].0, "maintenance.asset.updated");

    // Create downtime event — should produce outbox event
    let now = Utc::now();
    let dt = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: tid.clone(),
            asset_id: Some(asset.id),
            start_time: now - Duration::hours(1),
            end_time: Some(now),
            reason: "Outbox test downtime".into(),
            impact_classification: "critical".into(),
            idempotency_key: None,
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await
    .unwrap();

    let dt_event: Option<(String, String)> = sqlx::query_as(
        "SELECT event_type, aggregate_type FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(dt.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();
    let (dt_type, dt_agg) = dt_event.expect("downtime creation should emit outbox event");
    assert_eq!(dt_type, "maintenance.downtime.recorded");
    assert_eq!(dt_agg, "downtime_event");

    // Verify downtime envelope contains tenant_id
    let dt_payload: (serde_json::Value,) =
        sqlx::query_as("SELECT payload FROM events_outbox WHERE aggregate_id = $1")
            .bind(dt.id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    let dt_envelope = &dt_payload.0;
    assert_eq!(
        dt_envelope.get("tenant_id").and_then(|v| v.as_str()),
        Some(tid.as_str()),
        "downtime envelope must include tenant_id"
    );
}
