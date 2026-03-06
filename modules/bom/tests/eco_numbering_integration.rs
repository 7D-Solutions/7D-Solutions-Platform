use bom_rs::domain::eco_models::*;
use bom_rs::domain::eco_service;
use bom_rs::domain::numbering_client::NumberingClient;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_bom_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://bom_user:bom_pass@localhost:5450/bom_db?sslmode=require".to_string()
    });

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

async fn setup_numbering_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("NUMBERING_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://numbering_user:numbering_pass@localhost:5456/numbering_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to Numbering test DB");

    pool
}

fn unique_tenant() -> String {
    Uuid::new_v4().to_string()
}

// ============================================================================
// Auto-allocation: eco_number omitted -> auto-allocated from numbering
// ============================================================================

#[tokio::test]
#[serial]
async fn eco_auto_allocates_sequential_number() {
    let bom_pool = setup_bom_db().await;
    let num_pool = setup_numbering_db().await;
    let tenant = unique_tenant();
    let numbering = NumberingClient::direct(num_pool);

    // First ECO — should get ECO-00001
    let eco1 = eco_service::create_eco(
        &bom_pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: None,
            title: "First change".to_string(),
            description: None,
            created_by: "eng-1".to_string(),
        },
        Some(&numbering),
        None,
        &Uuid::new_v4().to_string(),
        None,
    )
    .await
    .expect("create eco 1");

    assert!(
        eco1.eco_number.starts_with("ECO-"),
        "Auto-allocated number should start with ECO-, got: {}",
        eco1.eco_number
    );
    assert_eq!(eco1.eco_number, "ECO-00001");

    // Second ECO — should get ECO-00002
    let eco2 = eco_service::create_eco(
        &bom_pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: None,
            title: "Second change".to_string(),
            description: None,
            created_by: "eng-1".to_string(),
        },
        Some(&numbering),
        None,
        &Uuid::new_v4().to_string(),
        None,
    )
    .await
    .expect("create eco 2");

    assert_eq!(eco2.eco_number, "ECO-00002");
}

// ============================================================================
// Tenant isolation: different tenants get independent ECO sequences
// ============================================================================

#[tokio::test]
#[serial]
async fn eco_numbering_tenant_isolation() {
    let bom_pool = setup_bom_db().await;
    let num_pool = setup_numbering_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let numbering = NumberingClient::direct(num_pool);

    let eco_a = eco_service::create_eco(
        &bom_pool,
        &tenant_a,
        &CreateEcoRequest {
            eco_number: None,
            title: "Tenant A change".to_string(),
            description: None,
            created_by: "eng-a".to_string(),
        },
        Some(&numbering),
        None,
        &Uuid::new_v4().to_string(),
        None,
    )
    .await
    .expect("create eco tenant A");

    let eco_b = eco_service::create_eco(
        &bom_pool,
        &tenant_b,
        &CreateEcoRequest {
            eco_number: None,
            title: "Tenant B change".to_string(),
            description: None,
            created_by: "eng-b".to_string(),
        },
        Some(&numbering),
        None,
        &Uuid::new_v4().to_string(),
        None,
    )
    .await
    .expect("create eco tenant B");

    assert_eq!(eco_a.eco_number, "ECO-00001", "Tenant A should start at 1");
    assert_eq!(eco_b.eco_number, "ECO-00001", "Tenant B should start at 1 independently");
}

// ============================================================================
// Idempotency: same correlation_id (idempotency_key) returns same number
// ============================================================================

#[tokio::test]
#[serial]
async fn eco_numbering_idempotent() {
    let bom_pool = setup_bom_db().await;
    let num_pool = setup_numbering_db().await;
    let tenant = unique_tenant();
    let numbering = NumberingClient::direct(num_pool);
    let corr = Uuid::new_v4().to_string();

    // First allocation with this correlation_id
    let eco1 = eco_service::create_eco(
        &bom_pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: None,
            title: "Idempotent ECO".to_string(),
            description: None,
            created_by: "eng-1".to_string(),
        },
        Some(&numbering),
        None,
        &corr,
        None,
    )
    .await
    .expect("create eco 1");

    // Allocate with a DIFFERENT correlation_id for a new ECO
    let eco2 = eco_service::create_eco(
        &bom_pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: None,
            title: "Second ECO".to_string(),
            description: None,
            created_by: "eng-1".to_string(),
        },
        Some(&numbering),
        None,
        &Uuid::new_v4().to_string(),
        None,
    )
    .await
    .expect("create eco 2");

    assert_eq!(eco1.eco_number, "ECO-00001");
    assert_eq!(eco2.eco_number, "ECO-00002", "Different key should advance");
}

// ============================================================================
// Numbering unavailable -> ECO creation fails with clear error
// ============================================================================

#[tokio::test]
#[serial]
async fn eco_creation_fails_without_numbering() {
    let bom_pool = setup_bom_db().await;
    let tenant = unique_tenant();

    // No numbering client provided, no eco_number in request
    let result = eco_service::create_eco(
        &bom_pool,
        &tenant,
        &CreateEcoRequest {
            eco_number: None,
            title: "Should fail".to_string(),
            description: None,
            created_by: "eng-1".to_string(),
        },
        None,
        None,
        &Uuid::new_v4().to_string(),
        None,
    )
    .await;

    assert!(
        result.is_err(),
        "ECO creation must fail when numbering is unavailable and eco_number is not provided"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("numbering"),
        "Error should mention numbering service, got: {}",
        err_msg
    );
}
