//! Integrated tests for asset category CRUD (bd-1g18).
//!
//! Covers:
//! 1. Category create — happy path, fields set correctly
//! 2. Duplicate category code rejected for same tenant
//! 3. Category update — mutable fields change
//! 4. Category deactivate — soft delete, excluded from list
//! 5. Tenant isolation — tenant B cannot see tenant A's categories

use fixed_assets::domain::assets::{
    AssetError, CategoryRepo, CreateCategoryRequest, UpdateCategoryRequest,
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
    format!("cat-test-{}", Uuid::new_v4().simple())
}

fn make_category(tenant_id: &str, code: &str) -> CreateCategoryRequest {
    CreateCategoryRequest {
        tenant_id: tenant_id.to_string(),
        code: code.to_string(),
        name: format!("Category {}", code),
        description: None,
        default_method: None,
        default_useful_life_months: Some(60),
        default_salvage_pct_bp: Some(500),
        asset_account_ref: "1500".to_string(),
        depreciation_expense_ref: "6100".to_string(),
        accum_depreciation_ref: "1510".to_string(),
        gain_loss_account_ref: Some("7000".to_string()),
    }
}

// ============================================================================
// 1. Create — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_category_create() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let cat = CategoryRepo::create(&pool, &make_category(&tid, "FURN"))
        .await
        .unwrap();

    assert_eq!(cat.tenant_id, tid);
    assert_eq!(cat.code, "FURN");
    assert_eq!(cat.name, "Category FURN");
    assert_eq!(cat.default_useful_life_months, 60);
    assert_eq!(cat.default_salvage_pct_bp, 500);
    assert_eq!(cat.asset_account_ref, "1500");
    assert_eq!(cat.gain_loss_account_ref.as_deref(), Some("7000"));
    assert!(cat.is_active);
}

// ============================================================================
// 2. Duplicate code rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_category_duplicate_code_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    CategoryRepo::create(&pool, &make_category(&tid, "EQUIP"))
        .await
        .unwrap();
    let err = CategoryRepo::create(&pool, &make_category(&tid, "EQUIP"))
        .await
        .unwrap_err();

    assert!(
        matches!(err, AssetError::DuplicateCategoryCode(_, _)),
        "expected DuplicateCategoryCode, got: {err}"
    );
}

// ============================================================================
// 3. Update — mutable fields change
// ============================================================================

#[tokio::test]
#[serial]
async fn test_category_update() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let cat = CategoryRepo::create(&pool, &make_category(&tid, "VEHC"))
        .await
        .unwrap();

    let updated = CategoryRepo::update(
        &pool,
        cat.id,
        &UpdateCategoryRequest {
            tenant_id: tid.clone(),
            name: Some("Vehicles Updated".to_string()),
            description: Some("Company vehicles".to_string()),
            default_method: None,
            default_useful_life_months: Some(84),
            default_salvage_pct_bp: None,
            asset_account_ref: None,
            depreciation_expense_ref: None,
            accum_depreciation_ref: None,
            gain_loss_account_ref: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.name, "Vehicles Updated");
    assert_eq!(updated.default_useful_life_months, 84);
    assert_eq!(updated.description.as_deref(), Some("Company vehicles"));
    // unchanged fields
    assert_eq!(updated.code, "VEHC");
    assert_eq!(updated.asset_account_ref, "1500");
}

// ============================================================================
// 4. Deactivate — excluded from list, still findable
// ============================================================================

#[tokio::test]
#[serial]
async fn test_category_deactivate() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let cat1 = CategoryRepo::create(&pool, &make_category(&tid, "ACAT"))
        .await
        .unwrap();
    let cat2 = CategoryRepo::create(&pool, &make_category(&tid, "BCAT"))
        .await
        .unwrap();

    let deactivated = CategoryRepo::deactivate(&pool, cat1.id, &tid)
        .await
        .unwrap();
    assert!(!deactivated.is_active);

    // list returns only active categories
    let active = CategoryRepo::list(&pool, &tid).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, cat2.id);

    // find_by_id still returns deactivated category
    let found = CategoryRepo::find_by_id(&pool, cat1.id, &tid)
        .await
        .unwrap();
    assert!(found.is_some());
    assert!(!found.unwrap().is_active);
}

// ============================================================================
// 5. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_category_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let cat_a = CategoryRepo::create(&pool, &make_category(&tid_a, "ISO"))
        .await
        .unwrap();

    // Tenant B has no categories
    let b_cats = CategoryRepo::list(&pool, &tid_b).await.unwrap();
    assert!(b_cats.is_empty(), "tenant B should not see tenant A's categories");

    // Tenant B cannot find tenant A's category by id
    let found = CategoryRepo::find_by_id(&pool, cat_a.id, &tid_b)
        .await
        .unwrap();
    assert!(found.is_none());
}
