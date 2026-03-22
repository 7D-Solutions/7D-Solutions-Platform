use bom_rs::domain::bom_service::{self, BomError};
use bom_rs::domain::models::*;
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

#[tokio::test]
#[serial]
async fn get_bom_by_part_id_returns_existing() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let part_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    let created = bom_service::create_bom(
        &pool,
        &tenant,
        &CreateBomRequest {
            part_id,
            description: Some("Widget BOM".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("create_bom");

    let found = bom_service::get_bom_by_part_id(&pool, &tenant, part_id)
        .await
        .expect("get_bom_by_part_id");

    assert_eq!(found.id, created.id);
    assert_eq!(found.part_id, part_id);
    assert_eq!(found.tenant_id, tenant);
}

#[tokio::test]
#[serial]
async fn get_bom_by_part_id_returns_404_for_nonexistent() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let nonexistent_part = Uuid::new_v4();

    let result = bom_service::get_bom_by_part_id(&pool, &tenant, nonexistent_part).await;
    assert!(result.is_err());
    match result {
        Err(BomError::Guard(bom_rs::domain::guards::GuardError::NotFound(_))) => {}
        other => panic!("Expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
#[serial]
async fn get_bom_by_part_id_cross_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let part_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    // Create BOM in tenant A
    bom_service::create_bom(
        &pool,
        &tenant_a,
        &CreateBomRequest {
            part_id,
            description: Some("Tenant A BOM".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("create_bom in tenant A");

    // Tenant B should NOT see it
    let result = bom_service::get_bom_by_part_id(&pool, &tenant_b, part_id).await;
    assert!(result.is_err());
    match result {
        Err(BomError::Guard(bom_rs::domain::guards::GuardError::NotFound(_))) => {}
        other => panic!("Expected NotFound for cross-tenant, got {:?}", other),
    }

    // Tenant A should see it
    let found = bom_service::get_bom_by_part_id(&pool, &tenant_a, part_id)
        .await
        .expect("tenant A should find own BOM");
    assert_eq!(found.part_id, part_id);
    assert_eq!(found.tenant_id, tenant_a);
}
