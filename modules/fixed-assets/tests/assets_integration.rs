//! Integrated tests for asset register CRUD (bd-1g18).
//!
//! Covers:
//! 1. Asset create — happy path, fields set correctly
//! 2. Duplicate asset tag rejected for same tenant
//! 3. Asset create with unknown category rejected
//! 4. Asset update — mutable descriptive fields
//! 5. Asset list (all / by status filter)
//! 6. Tenant isolation — tenant B cannot see tenant A's assets

use chrono::NaiveDate;
use fixed_assets::domain::assets::{
    AssetError, AssetRepo, CategoryRepo, CreateAssetRequest, CreateCategoryRequest,
    UpdateAssetRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to fixed-assets test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run fixed-assets migrations");

    pool
}

fn unique_tenant() -> String {
    format!("asset-test-{}", Uuid::new_v4().simple())
}

async fn create_test_category(pool: &sqlx::PgPool, tenant_id: &str) -> Uuid {
    let code = format!("CAT-{}", &Uuid::new_v4().to_string()[..8]);
    CategoryRepo::create(
        pool,
        &CreateCategoryRequest {
            tenant_id: tenant_id.to_string(),
            code,
            name: "Test Category".to_string(),
            description: None,
            default_method: None,
            default_useful_life_months: Some(60),
            default_salvage_pct_bp: Some(0),
            asset_account_ref: "1500".to_string(),
            depreciation_expense_ref: "6100".to_string(),
            accum_depreciation_ref: "1510".to_string(),
            gain_loss_account_ref: None,
        },
    )
    .await
    .unwrap()
    .id
}

fn make_asset(tenant_id: &str, category_id: Uuid, tag: &str) -> CreateAssetRequest {
    CreateAssetRequest {
        tenant_id: tenant_id.to_string(),
        category_id,
        asset_tag: tag.to_string(),
        name: format!("Asset {}", tag),
        description: None,
        acquisition_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        in_service_date: Some(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()),
        acquisition_cost_minor: 100_000,
        currency: None,
        depreciation_method: None,
        useful_life_months: None,
        salvage_value_minor: None,
        location: None,
        department: None,
        responsible_person: None,
        serial_number: None,
        vendor: None,
        purchase_order_ref: None,
        notes: None,
    }
}

// ============================================================================
// 1. Create — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_asset_create() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let cat_id = create_test_category(&pool, &tid).await;

    let asset = AssetRepo::create(&pool, &make_asset(&tid, cat_id, "FA-001"))
        .await
        .unwrap();

    assert_eq!(asset.tenant_id, tid);
    assert_eq!(asset.asset_tag, "FA-001");
    assert_eq!(asset.status, "draft");
    assert_eq!(asset.acquisition_cost_minor, 100_000);
    assert_eq!(asset.net_book_value_minor, 100_000);
    assert_eq!(asset.category_id, cat_id);
}

// ============================================================================
// 2. Duplicate asset tag rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_asset_duplicate_tag_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let cat_id = create_test_category(&pool, &tid).await;

    AssetRepo::create(&pool, &make_asset(&tid, cat_id, "DUP-001"))
        .await
        .unwrap();
    let err = AssetRepo::create(&pool, &make_asset(&tid, cat_id, "DUP-001"))
        .await
        .unwrap_err();

    assert!(
        matches!(err, AssetError::DuplicateTag(_, _)),
        "expected DuplicateTag, got: {err}"
    );
}

// ============================================================================
// 3. Category not found — rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_asset_unknown_category_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let fake_cat_id = Uuid::new_v4();

    let err = AssetRepo::create(&pool, &make_asset(&tid, fake_cat_id, "UNK-001"))
        .await
        .unwrap_err();

    assert!(
        matches!(err, AssetError::CategoryNotFound(_)),
        "expected CategoryNotFound, got: {err}"
    );
}

// ============================================================================
// 4. Update — mutable descriptive fields
// ============================================================================

#[tokio::test]
#[serial]
async fn test_asset_update() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let cat_id = create_test_category(&pool, &tid).await;
    let asset = AssetRepo::create(&pool, &make_asset(&tid, cat_id, "UPD-001"))
        .await
        .unwrap();

    let updated = AssetRepo::update(
        &pool,
        asset.id,
        &UpdateAssetRequest {
            tenant_id: tid.clone(),
            name: Some("Updated Name".to_string()),
            description: Some("New desc".to_string()),
            location: Some("Building A".to_string()),
            department: Some("IT".to_string()),
            responsible_person: Some("Alice".to_string()),
            notes: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.name, "Updated Name");
    assert_eq!(updated.location.as_deref(), Some("Building A"));
    assert_eq!(updated.department.as_deref(), Some("IT"));
    assert_eq!(updated.responsible_person.as_deref(), Some("Alice"));
    // immutable fields unchanged
    assert_eq!(updated.acquisition_cost_minor, 100_000);
    assert_eq!(updated.asset_tag, "UPD-001");
}

// ============================================================================
// 5. List — all and by status filter
// ============================================================================

#[tokio::test]
#[serial]
async fn test_asset_list_with_status_filter() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let cat_id = create_test_category(&pool, &tid).await;

    let a1 = AssetRepo::create(&pool, &make_asset(&tid, cat_id, "LST-001"))
        .await
        .unwrap();
    AssetRepo::create(&pool, &make_asset(&tid, cat_id, "LST-002"))
        .await
        .unwrap();

    // Deactivate a1 (sets status = 'disposed')
    AssetRepo::deactivate(&pool, a1.id, &tid).await.unwrap();

    let all = AssetRepo::list(&pool, &tid, None).await.unwrap();
    assert_eq!(all.len(), 2);

    let disposed = AssetRepo::list(&pool, &tid, Some("disposed"))
        .await
        .unwrap();
    assert_eq!(disposed.len(), 1);
    assert_eq!(disposed[0].id, a1.id);

    let drafts = AssetRepo::list(&pool, &tid, Some("draft")).await.unwrap();
    assert_eq!(drafts.len(), 1);
}

// ============================================================================
// 6. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_asset_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let cat_a = create_test_category(&pool, &tid_a).await;
    let asset_a = AssetRepo::create(&pool, &make_asset(&tid_a, cat_a, "ISO-A-001"))
        .await
        .unwrap();

    // Tenant B cannot find tenant A's asset by id
    let result = AssetRepo::find_by_id(&pool, asset_a.id, &tid_b)
        .await
        .unwrap();
    assert!(result.is_none());

    // Tenant B list is empty
    let b_assets = AssetRepo::list(&pool, &tid_b, None).await.unwrap();
    assert!(b_assets.is_empty());
}
