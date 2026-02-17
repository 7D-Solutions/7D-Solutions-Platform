/// E2E smoke test for tenant registry baseline
///
/// Verifies that the tenant registry schema:
/// 1. Stores tenant records with status, environment, and schema versions
/// 2. Tracks provisioning steps with verification results
/// 3. Enforces deterministic provisioning sequence
/// 4. Supports multi-tenant isolation

mod common;

use chrono::Utc;
use common::get_tenant_registry_pool;
use serial_test::serial;
use sqlx::PgPool;
use std::collections::HashMap;
use tenant_registry::standard_provisioning_sequence;
use uuid::Uuid;

/// Helper to run migrations on the tenant registry database
async fn run_tenant_registry_migrations(pool: &PgPool) {
    // Read and execute the migration file
    let migration_sql = include_str!(
        "../../platform/tenant-registry/db/migrations/20260216000001_create_tenant_registry.sql"
    );

    // Drop existing tables if they exist (for test idempotency)
    sqlx::query("DROP TABLE IF EXISTS provisioning_steps CASCADE")
        .execute(pool)
        .await
        .expect("Failed to drop provisioning_steps table");

    sqlx::query("DROP TABLE IF EXISTS tenants CASCADE")
        .execute(pool)
        .await
        .expect("Failed to drop tenants table");

    // Execute the migration
    sqlx::raw_sql(migration_sql)
        .execute(pool)
        .await
        .expect("Failed to run tenant registry migrations");
}

#[tokio::test]
#[serial]
async fn test_tenant_record_insert_and_query() {
    let pool = get_tenant_registry_pool().await;
    run_tenant_registry_migrations(&pool).await;

    let tenant_id = Uuid::new_v4();
    let created_at = Utc::now();

    // Insert a tenant record
    sqlx::query(
        r#"
        INSERT INTO tenants (tenant_id, status, environment, module_schema_versions, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $5)
        "#,
    )
    .bind(tenant_id)
    .bind("provisioning")
    .bind("development")
    .bind(serde_json::json!({}))
    .bind(created_at)
    .execute(&pool)
    .await
    .expect("Failed to insert tenant");

    // Query it back
    let (queried_id, status, environment): (Uuid, String, String) = sqlx::query_as(
        "SELECT tenant_id, status, environment FROM tenants WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query tenant");

    assert_eq!(queried_id, tenant_id);
    assert_eq!(status, "provisioning");
    assert_eq!(environment, "development");
}

#[tokio::test]
#[serial]
async fn test_module_schema_versions_storage() {
    let pool = get_tenant_registry_pool().await;
    run_tenant_registry_migrations(&pool).await;

    let tenant_id = Uuid::new_v4();

    // Create a tenant with module schema versions
    let mut module_versions = HashMap::new();
    module_versions.insert("ar".to_string(), "20260216000001".to_string());
    module_versions.insert("payments".to_string(), "20260215000002".to_string());
    module_versions.insert("gl".to_string(), "20260214000003".to_string());

    sqlx::query(
        r#"
        INSERT INTO tenants (tenant_id, status, environment, module_schema_versions)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(tenant_id)
    .bind("active")
    .bind("production")
    .bind(serde_json::to_value(&module_versions).unwrap())
    .execute(&pool)
    .await
    .expect("Failed to insert tenant");

    // Query and verify module_schema_versions
    let queried_versions: serde_json::Value = sqlx::query_scalar(
        "SELECT module_schema_versions FROM tenants WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query tenant");

    let queried_map: HashMap<String, String> =
        serde_json::from_value(queried_versions).expect("Failed to deserialize versions");

    assert_eq!(queried_map.get("ar"), Some(&"20260216000001".to_string()));
    assert_eq!(
        queried_map.get("payments"),
        Some(&"20260215000002".to_string())
    );
    assert_eq!(queried_map.get("gl"), Some(&"20260214000003".to_string()));
}

#[tokio::test]
#[serial]
async fn test_provisioning_steps_tracking() {
    let pool = get_tenant_registry_pool().await;
    run_tenant_registry_migrations(&pool).await;

    let tenant_id = Uuid::new_v4();

    // Create tenant first
    sqlx::query(
        r#"
        INSERT INTO tenants (tenant_id, status, environment)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(tenant_id)
    .bind("provisioning")
    .bind("development")
    .execute(&pool)
    .await
    .expect("Failed to insert tenant");

    // Insert provisioning steps
    let step_id_1 = Uuid::new_v4();
    let step_id_2 = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO provisioning_steps (step_id, tenant_id, step_name, step_order, status)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(step_id_1)
    .bind(tenant_id)
    .bind("validate_tenant_id")
    .bind(1)
    .bind("completed")
    .execute(&pool)
    .await
    .expect("Failed to insert step 1");

    sqlx::query(
        r#"
        INSERT INTO provisioning_steps (step_id, tenant_id, step_name, step_order, status)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(step_id_2)
    .bind(tenant_id)
    .bind("create_tenant_databases")
    .bind(2)
    .bind("in_progress")
    .execute(&pool)
    .await
    .expect("Failed to insert step 2");

    // Query steps back in order
    let steps: Vec<(String, i32, String)> = sqlx::query_as(
        r#"
        SELECT step_name, step_order, status
        FROM provisioning_steps
        WHERE tenant_id = $1
        ORDER BY step_order
        "#,
    )
    .bind(tenant_id)
    .fetch_all(&pool)
    .await
    .expect("Failed to query steps");

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].0, "validate_tenant_id");
    assert_eq!(steps[0].1, 1);
    assert_eq!(steps[0].2, "completed");
    assert_eq!(steps[1].0, "create_tenant_databases");
    assert_eq!(steps[1].1, 2);
    assert_eq!(steps[1].2, "in_progress");
}

#[tokio::test]
#[serial]
async fn test_provisioning_step_verification_result() {
    let pool = get_tenant_registry_pool().await;
    run_tenant_registry_migrations(&pool).await;

    let tenant_id = Uuid::new_v4();

    // Create tenant
    sqlx::query("INSERT INTO tenants (tenant_id, status, environment) VALUES ($1, $2, $3)")
        .bind(tenant_id)
        .bind("provisioning")
        .bind("staging")
        .execute(&pool)
        .await
        .expect("Failed to insert tenant");

    // Create verification result JSON
    let verification_result = serde_json::json!({
        "checks_passed": ["ar_database_exists", "payments_database_exists"],
        "checks_failed": [],
        "details": {
            "ar_db_size": "10MB",
            "payments_db_size": "8MB"
        }
    });

    // Insert step with verification result
    sqlx::query(
        r#"
        INSERT INTO provisioning_steps (tenant_id, step_name, step_order, status, verification_result)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(tenant_id)
    .bind("create_tenant_databases")
    .bind(2)
    .bind("completed")
    .bind(&verification_result)
    .execute(&pool)
    .await
    .expect("Failed to insert step with verification");

    // Query it back
    let queried_result: serde_json::Value = sqlx::query_scalar(
        "SELECT verification_result FROM provisioning_steps WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("Failed to query verification result");

    // Verify structure
    assert_eq!(
        queried_result["checks_passed"].as_array().unwrap().len(),
        2
    );
    assert_eq!(queried_result["checks_failed"].as_array().unwrap().len(), 0);
    assert_eq!(queried_result["details"]["ar_db_size"], "10MB");
}

#[tokio::test]
#[serial]
async fn test_tenant_status_constraint() {
    let pool = get_tenant_registry_pool().await;
    run_tenant_registry_migrations(&pool).await;

    let tenant_id = Uuid::new_v4();

    // Valid status should succeed
    let result = sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment) VALUES ($1, $2, $3)",
    )
    .bind(tenant_id)
    .bind("active")
    .bind("production")
    .execute(&pool)
    .await;

    assert!(result.is_ok(), "Valid status should insert successfully");

    // Invalid status should fail
    let invalid_tenant_id = Uuid::new_v4();
    let result = sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment) VALUES ($1, $2, $3)",
    )
    .bind(invalid_tenant_id)
    .bind("invalid_status")
    .bind("production")
    .execute(&pool)
    .await;

    assert!(result.is_err(), "Invalid status should fail constraint check");
}

#[tokio::test]
#[serial]
async fn test_environment_constraint() {
    let pool = get_tenant_registry_pool().await;
    run_tenant_registry_migrations(&pool).await;

    let tenant_id = Uuid::new_v4();

    // Valid environment should succeed
    let result = sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment) VALUES ($1, $2, $3)",
    )
    .bind(tenant_id)
    .bind("active")
    .bind("staging")
    .execute(&pool)
    .await;

    assert!(
        result.is_ok(),
        "Valid environment should insert successfully"
    );

    // Invalid environment should fail
    let invalid_tenant_id = Uuid::new_v4();
    let result = sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment) VALUES ($1, $2, $3)",
    )
    .bind(invalid_tenant_id)
    .bind("active")
    .bind("invalid_env")
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "Invalid environment should fail constraint check"
    );
}

#[tokio::test]
#[serial]
async fn test_standard_provisioning_sequence_definition() {
    // Verify the provisioning sequence is deterministic and complete
    let sequence = standard_provisioning_sequence();

    // Should have 7 steps
    assert_eq!(sequence.len(), 7);

    // Steps should be ordered 1-7
    for (i, step) in sequence.iter().enumerate() {
        assert_eq!(step.step_order, (i + 1) as i32);
    }

    // All steps should have verification checks
    for step in &sequence {
        assert!(!step.verification_checks.is_empty());
    }

    // Verify specific step names exist
    let step_names: Vec<&str> = sequence.iter().map(|s| s.step_name).collect();
    assert!(step_names.contains(&"validate_tenant_id"));
    assert!(step_names.contains(&"create_tenant_databases"));
    assert!(step_names.contains(&"run_schema_migrations"));
    assert!(step_names.contains(&"activate_tenant"));
}

#[tokio::test]
#[serial]
async fn test_multi_tenant_isolation() {
    let pool = get_tenant_registry_pool().await;
    run_tenant_registry_migrations(&pool).await;

    let tenant1_id = Uuid::new_v4();
    let tenant2_id = Uuid::new_v4();

    // Create two tenants
    sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment) VALUES ($1, $2, $3)",
    )
    .bind(tenant1_id)
    .bind("active")
    .bind("development")
    .execute(&pool)
    .await
    .expect("Failed to insert tenant 1");

    sqlx::query(
        "INSERT INTO tenants (tenant_id, status, environment) VALUES ($1, $2, $3)",
    )
    .bind(tenant2_id)
    .bind("provisioning")
    .bind("production")
    .execute(&pool)
    .await
    .expect("Failed to insert tenant 2");

    // Add steps for each tenant
    sqlx::query(
        "INSERT INTO provisioning_steps (tenant_id, step_name, step_order, status) VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant1_id)
    .bind("validate_tenant_id")
    .bind(1)
    .bind("completed")
    .execute(&pool)
    .await
    .expect("Failed to insert step for tenant 1");

    sqlx::query(
        "INSERT INTO provisioning_steps (tenant_id, step_name, step_order, status) VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant2_id)
    .bind("create_tenant_databases")
    .bind(2)
    .bind("in_progress")
    .execute(&pool)
    .await
    .expect("Failed to insert step for tenant 2");

    // Query steps for tenant 1
    let steps1: Vec<String> = sqlx::query_scalar(
        "SELECT step_name FROM provisioning_steps WHERE tenant_id = $1 ORDER BY step_order",
    )
    .bind(tenant1_id)
    .fetch_all(&pool)
    .await
    .expect("Failed to query steps for tenant 1");

    // Query steps for tenant 2
    let steps2: Vec<String> = sqlx::query_scalar(
        "SELECT step_name FROM provisioning_steps WHERE tenant_id = $1 ORDER BY step_order",
    )
    .bind(tenant2_id)
    .fetch_all(&pool)
    .await
    .expect("Failed to query steps for tenant 2");

    // Verify isolation
    assert_eq!(steps1.len(), 1);
    assert_eq!(steps1[0], "validate_tenant_id");
    assert_eq!(steps2.len(), 1);
    assert_eq!(steps2[0], "create_tenant_databases");
}

#[tokio::test]
#[serial]
async fn test_unique_step_per_tenant_constraint() {
    let pool = get_tenant_registry_pool().await;
    run_tenant_registry_migrations(&pool).await;

    let tenant_id = Uuid::new_v4();

    // Create tenant
    sqlx::query("INSERT INTO tenants (tenant_id, status, environment) VALUES ($1, $2, $3)")
        .bind(tenant_id)
        .bind("provisioning")
        .bind("development")
        .execute(&pool)
        .await
        .expect("Failed to insert tenant");

    // Insert first step
    let result1 = sqlx::query(
        "INSERT INTO provisioning_steps (tenant_id, step_name, step_order, status) VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind("validate_tenant_id")
    .bind(1)
    .bind("completed")
    .execute(&pool)
    .await;

    assert!(result1.is_ok(), "First insert should succeed");

    // Try to insert duplicate step_name for same tenant (should fail)
    let result2 = sqlx::query(
        "INSERT INTO provisioning_steps (tenant_id, step_name, step_order, status) VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind("validate_tenant_id")
    .bind(1)
    .bind("failed")
    .execute(&pool)
    .await;

    assert!(
        result2.is_err(),
        "Duplicate step_name for same tenant should fail unique constraint"
    );
}
