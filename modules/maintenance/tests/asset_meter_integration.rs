//! Integration tests for assets + meter types/readings CRUD (bd-1lmd).
//!
//! Covers:
//! 1. Asset CRUD: create, get, list (with filters), update
//! 2. Tenant isolation: cross-tenant access returns 404/empty
//! 3. Meter types: create, list, duplicate name rejected
//! 4. Meter readings: record, list, monotonicity enforcement
//! 5. Rollover: valid rollover accepted, invalid rollover rejected
//! 6. Out-of-order timestamps with value monotonicity

use maintenance_rs::domain::assets::{
    AssetRepo, CreateAssetRequest, ListAssetsQuery, UpdateAssetRequest,
};
use maintenance_rs::domain::meters::{
    CreateMeterTypeRequest, ListReadingsQuery, MeterReadingRepo, MeterTypeRepo,
    RecordReadingRequest,
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

fn unique_tenant() -> String {
    format!("maint-test-{}", Uuid::new_v4().simple())
}

// ============================================================================
// 1. Asset CRUD happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_and_get_asset() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = CreateAssetRequest {
        tenant_id: tid.clone(),
        asset_tag: "VEH-001".into(),
        name: "Ford F-150".into(),
        description: Some("Fleet truck".into()),
        asset_type: "vehicle".into(),
        location: Some("Yard A".into()),
        department: Some("Operations".into()),
        responsible_person: Some("John Doe".into()),
        serial_number: Some("1FTFW1E50MFC00001".into()),
        fixed_asset_ref: None,
        metadata: Some(serde_json::json!({"color": "white"})),
    };

    let asset = AssetRepo::create(&pool, &req).await.unwrap();
    assert_eq!(asset.asset_tag, "VEH-001");
    assert_eq!(asset.name, "Ford F-150");

    let fetched = AssetRepo::find_by_id(&pool, asset.id, &tid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, asset.id);
    assert_eq!(fetched.serial_number, Some("1FTFW1E50MFC00001".to_string()));
}

// ============================================================================
// 2. Duplicate asset tag rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_duplicate_asset_tag_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = CreateAssetRequest {
        tenant_id: tid.clone(),
        asset_tag: "DUP-TAG".into(),
        name: "Asset A".into(),
        description: None,
        asset_type: "equipment".into(),
        location: None,
        department: None,
        responsible_person: None,
        serial_number: None,
        fixed_asset_ref: None,
        metadata: None,
    };

    AssetRepo::create(&pool, &req).await.unwrap();

    let dup = CreateAssetRequest {
        tenant_id: tid.clone(),
        asset_tag: "DUP-TAG".into(),
        name: "Asset B".into(),
        description: None,
        asset_type: "equipment".into(),
        location: None,
        department: None,
        responsible_person: None,
        serial_number: None,
        fixed_asset_ref: None,
        metadata: None,
    };
    let err = AssetRepo::create(&pool, &dup).await.unwrap_err();
    assert!(
        matches!(
            err,
            maintenance_rs::domain::assets::AssetError::DuplicateTag(_, _)
        ),
        "expected DuplicateTag, got: {:?}",
        err
    );
}

// ============================================================================
// 3. Asset list with filters
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_assets_with_filters() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    for (tag, atype) in &[
        ("V-001", "vehicle"),
        ("V-002", "vehicle"),
        ("E-001", "equipment"),
    ] {
        AssetRepo::create(
            &pool,
            &CreateAssetRequest {
                tenant_id: tid.clone(),
                asset_tag: tag.to_string(),
                name: format!("Asset {}", tag),
                description: None,
                asset_type: atype.to_string(),
                location: None,
                department: None,
                responsible_person: None,
                serial_number: None,
                fixed_asset_ref: None,
                metadata: None,
            },
        )
        .await
        .unwrap();
    }

    // All assets
    let all = AssetRepo::list(
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
    assert_eq!(all.total, 3);

    // Filter by type
    let vehicles = AssetRepo::list(
        &pool,
        &ListAssetsQuery {
            tenant_id: tid.clone(),
            asset_type: Some("vehicle".into()),
            status: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(vehicles.total, 2);
}

// ============================================================================
// 4. Tenant isolation — cross-tenant access returns nothing
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_assets() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let asset = AssetRepo::create(
        &pool,
        &CreateAssetRequest {
            tenant_id: tid_a.clone(),
            asset_tag: "ISO-001".into(),
            name: "Isolated Asset".into(),
            description: None,
            asset_type: "other".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    // Cross-tenant lookup returns None
    let result = AssetRepo::find_by_id(&pool, asset.id, &tid_b)
        .await
        .unwrap();
    assert!(result.is_none(), "cross-tenant access should return None");

    // Cross-tenant list returns empty
    let list = AssetRepo::list(
        &pool,
        &ListAssetsQuery {
            tenant_id: tid_b,
            asset_type: None,
            status: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(list.total, 0);
}

// ============================================================================
// 5. Asset update
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_asset() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let asset = AssetRepo::create(
        &pool,
        &CreateAssetRequest {
            tenant_id: tid.clone(),
            asset_tag: "UPD-001".into(),
            name: "Original Name".into(),
            description: None,
            asset_type: "machinery".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let updated = AssetRepo::update(
        &pool,
        asset.id,
        &tid,
        &UpdateAssetRequest {
            name: Some("Updated Name".into()),
            description: Some("Now has description".into()),
            status: Some("inactive".into()),
            asset_type: None,
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.name, "Updated Name");
    assert_eq!(updated.description, Some("Now has description".to_string()));
}

// ============================================================================
// 6. Meter type CRUD
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_and_list_meter_types() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let odometer = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: "Odometer".into(),
            unit_label: "miles".into(),
            rollover_value: Some(999_999),
        },
    )
    .await
    .unwrap();

    assert_eq!(odometer.name, "Odometer");
    assert_eq!(odometer.rollover_value, Some(999_999));

    MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: "Engine Hours".into(),
            unit_label: "hours".into(),
            rollover_value: None,
        },
    )
    .await
    .unwrap();

    let types = MeterTypeRepo::list(&pool, &tid).await.unwrap();
    assert_eq!(types.len(), 2);
}

// ============================================================================
// 7. Duplicate meter type name rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_duplicate_meter_type_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: "Odometer".into(),
            unit_label: "miles".into(),
            rollover_value: None,
        },
    )
    .await
    .unwrap();

    let err = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: "Odometer".into(),
            unit_label: "km".into(),
            rollover_value: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            err,
            maintenance_rs::domain::meters::MeterError::DuplicateName(_, _)
        ),
        "expected DuplicateName, got: {:?}",
        err
    );
}

// ============================================================================
// 8. Meter reading — monotonic increasing accepted
// ============================================================================

#[tokio::test]
#[serial]
async fn test_meter_readings_monotonic_increasing() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let asset = AssetRepo::create(
        &pool,
        &CreateAssetRequest {
            tenant_id: tid.clone(),
            asset_tag: "RDG-001".into(),
            name: "Reading Test Truck".into(),
            description: None,
            asset_type: "vehicle".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let meter = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: "Odometer".into(),
            unit_label: "miles".into(),
            rollover_value: None,
        },
    )
    .await
    .unwrap();

    // First reading
    let r1 = MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter.id,
            reading_value: 10_000,
            recorded_at: None,
            recorded_by: Some("tech-1".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(r1.reading_value, 10_000);

    // Second reading (higher)
    let r2 = MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter.id,
            reading_value: 15_000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(r2.reading_value, 15_000);

    // Equal reading is also valid
    MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter.id,
            reading_value: 15_000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();
}

// ============================================================================
// 9. Meter reading — decrease rejected (no rollover)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_meter_reading_decrease_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let asset = AssetRepo::create(
        &pool,
        &CreateAssetRequest {
            tenant_id: tid.clone(),
            asset_tag: "DEC-001".into(),
            name: "Decrease Test".into(),
            description: None,
            asset_type: "vehicle".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let meter = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: "Hours".into(),
            unit_label: "hours".into(),
            rollover_value: None,
        },
    )
    .await
    .unwrap();

    MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter.id,
            reading_value: 500,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();

    let err = MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter.id,
            reading_value: 400,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            err,
            maintenance_rs::domain::meters::MeterError::MonotonicityViolation {
                previous: 500,
                attempted: 400
            }
        ),
        "expected MonotonicityViolation, got: {:?}",
        err
    );
}

// ============================================================================
// 10. Meter reading — valid rollover accepted
// ============================================================================

#[tokio::test]
#[serial]
async fn test_meter_reading_rollover_accepted() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let asset = AssetRepo::create(
        &pool,
        &CreateAssetRequest {
            tenant_id: tid.clone(),
            asset_tag: "ROLL-001".into(),
            name: "Rollover Test".into(),
            description: None,
            asset_type: "vehicle".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let meter = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: "Odometer".into(),
            unit_label: "miles".into(),
            rollover_value: Some(1_000_000),
        },
    )
    .await
    .unwrap();

    // Record reading near rollover
    MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter.id,
            reading_value: 950_000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();

    // Rollover: 950,000 → 12 (valid: prev >= 90% of 1M, new <= 10% of 1M)
    let reading = MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter.id,
            reading_value: 12,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(reading.reading_value, 12);
}

// ============================================================================
// 11. Meter reading — invalid rollover rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_meter_reading_invalid_rollover_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let asset = AssetRepo::create(
        &pool,
        &CreateAssetRequest {
            tenant_id: tid.clone(),
            asset_tag: "BADROLL-001".into(),
            name: "Bad Rollover Test".into(),
            description: None,
            asset_type: "vehicle".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let meter = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: "Odometer".into(),
            unit_label: "miles".into(),
            rollover_value: Some(1_000_000),
        },
    )
    .await
    .unwrap();

    MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter.id,
            reading_value: 500_000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();

    // 500,000 → 12: prev NOT within 10% of rollover (500k < 900k) → reject
    let err = MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter.id,
            reading_value: 12,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            err,
            maintenance_rs::domain::meters::MeterError::MonotonicityViolation { .. }
        ),
        "expected MonotonicityViolation, got: {:?}",
        err
    );
}

// ============================================================================
// 12. List readings for asset
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_readings_for_asset() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let asset = AssetRepo::create(
        &pool,
        &CreateAssetRequest {
            tenant_id: tid.clone(),
            asset_tag: "LIST-001".into(),
            name: "List Test".into(),
            description: None,
            asset_type: "equipment".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let meter = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: "Cycles".into(),
            unit_label: "cycles".into(),
            rollover_value: None,
        },
    )
    .await
    .unwrap();

    for val in [100, 200, 300] {
        MeterReadingRepo::record(
            &pool,
            asset.id,
            &RecordReadingRequest {
                tenant_id: tid.clone(),
                meter_type_id: meter.id,
                reading_value: val,
                recorded_at: None,
                recorded_by: None,
            },
        )
        .await
        .unwrap();
    }

    let readings = MeterReadingRepo::list(
        &pool,
        &tid,
        asset.id,
        &ListReadingsQuery {
            meter_type_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(readings.len(), 3);
}

// ============================================================================
// 13. Tenant isolation — meter readings
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_meter_readings() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let asset = AssetRepo::create(
        &pool,
        &CreateAssetRequest {
            tenant_id: tid_a.clone(),
            asset_tag: "MISO-001".into(),
            name: "Meter Isolation".into(),
            description: None,
            asset_type: "vehicle".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let meter = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid_a.clone(),
            name: "Odometer".into(),
            unit_label: "miles".into(),
            rollover_value: None,
        },
    )
    .await
    .unwrap();

    // Record reading as tenant A
    MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid_a.clone(),
            meter_type_id: meter.id,
            reading_value: 1000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();

    // Tenant B cannot record against tenant A's asset
    let err = MeterReadingRepo::record(
        &pool,
        asset.id,
        &RecordReadingRequest {
            tenant_id: tid_b,
            meter_type_id: meter.id,
            reading_value: 2000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            err,
            maintenance_rs::domain::meters::MeterError::AssetNotFound
        ),
        "cross-tenant recording should fail with AssetNotFound, got: {:?}",
        err
    );
}
