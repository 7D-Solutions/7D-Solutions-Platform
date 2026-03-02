//! Integrated tests for connector CRUD + test action dispatch (bd-3lwh).
//!
//! Covers:
//!  1. Register echo connector — happy path
//!  2. Register unknown connector type — error case
//!  3. Register duplicate name — error case
//!  4. List connectors
//!  5. Get connector by id
//!  6. Get connector not found — error case
//!  7. Run test action on echo connector — happy path
//!  8. Run test action on disabled connector — error case
//!  9. Tenant isolation (cross-tenant connector not visible / not triggerable)

use integrations_rs::domain::connectors::{
    service::{get_connector_config, list_connector_configs, register_connector, run_test_action},
    ConnectorError, RegisterConnectorRequest, RunTestActionRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://integrations_user:integrations_pass@localhost:5449/integrations_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

fn unique_tenant() -> String {
    format!("conn-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

fn echo_req(name: &str) -> RegisterConnectorRequest {
    RegisterConnectorRequest {
        connector_type: "echo".to_string(),
        name: name.to_string(),
        config: Some(serde_json::json!({"echo_prefix": "test"})),
    }
}

// ============================================================================
// 1. Register echo connector — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_connector_register_echo() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = echo_req("my-echo-connector");
    let created = register_connector(&pool, &tid, &req, corr())
        .await
        .expect("register_connector failed");

    assert_eq!(created.app_id, tid);
    assert_eq!(created.connector_type, "echo");
    assert_eq!(created.name, "my-echo-connector");
    assert!(created.enabled);
}

// ============================================================================
// 2. Register unknown connector type — error case
// ============================================================================

#[tokio::test]
#[serial]
async fn test_connector_register_unknown_type() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = RegisterConnectorRequest {
        connector_type: "nonexistent-type".to_string(),
        name: "ghost-connector".to_string(),
        config: None,
    };

    let err = register_connector(&pool, &tid, &req, corr()).await;
    assert!(err.is_err(), "unknown connector type should fail");
    match err.unwrap_err() {
        ConnectorError::UnknownType(t) => assert_eq!(t, "nonexistent-type"),
        other => panic!("expected UnknownType, got: {:?}", other),
    }
}

// ============================================================================
// 3. Register duplicate name — error case
// ============================================================================

#[tokio::test]
#[serial]
async fn test_connector_register_duplicate_name() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = echo_req("dup-name-connector");
    register_connector(&pool, &tid, &req, corr())
        .await
        .expect("first register failed");

    let err = register_connector(&pool, &tid, &req, corr()).await;
    assert!(err.is_err(), "duplicate name should fail");
    match err.unwrap_err() {
        ConnectorError::InvalidConfig(msg) => {
            assert!(
                msg.contains("already exists"),
                "expected 'already exists' in: {}",
                msg
            );
        }
        other => panic!("expected InvalidConfig for duplicate, got: {:?}", other),
    }
}

// ============================================================================
// 4. List connectors
// ============================================================================

#[tokio::test]
#[serial]
async fn test_connector_list() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    register_connector(&pool, &tid, &echo_req("echo-a"), corr())
        .await
        .expect("register echo-a");
    register_connector(&pool, &tid, &echo_req("echo-b"), corr())
        .await
        .expect("register echo-b");

    let all = list_connector_configs(&pool, &tid, false)
        .await
        .expect("list_connector_configs failed");

    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|c| c.app_id == tid));

    // enabled_only filter returns both since both are enabled by default
    let enabled = list_connector_configs(&pool, &tid, true)
        .await
        .expect("list_connector_configs (enabled) failed");
    assert_eq!(enabled.len(), 2);
}

// ============================================================================
// 5. Get connector by id
// ============================================================================

#[tokio::test]
#[serial]
async fn test_connector_get() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let created = register_connector(&pool, &tid, &echo_req("get-me"), corr())
        .await
        .expect("register failed");

    let fetched = get_connector_config(&pool, &tid, created.id)
        .await
        .expect("get_connector_config failed");

    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().id, created.id);
}

// ============================================================================
// 6. Get connector not found — error case
// ============================================================================

#[tokio::test]
#[serial]
async fn test_connector_get_not_found() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let result = get_connector_config(&pool, &tid, Uuid::new_v4())
        .await
        .expect("get should not return DB error for missing row");

    assert!(result.is_none(), "expected None for non-existent connector");
}

// ============================================================================
// 7. Run test action on echo connector — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_connector_run_test_action_echo() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = RegisterConnectorRequest {
        connector_type: "echo".to_string(),
        name: "echo-test-connector".to_string(),
        config: Some(serde_json::json!({"echo_prefix": "hello"})),
    };
    let created = register_connector(&pool, &tid, &req, corr())
        .await
        .expect("register failed");

    let idem_key = "test-key-abc-123";
    let action_req = RunTestActionRequest {
        idempotency_key: idem_key.to_string(),
    };

    let result = run_test_action(&pool, &tid, created.id, &action_req)
        .await
        .expect("run_test_action failed");

    assert!(result.success);
    assert_eq!(result.connector_type, "echo");
    assert_eq!(result.idempotency_key, idem_key);

    // Echo connector output contains the prefix and idempotency key
    let msg = result.output["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("hello"),
        "output should contain prefix: {}",
        msg
    );
    assert!(
        msg.contains(idem_key),
        "output should contain idempotency key: {}",
        msg
    );
}

// ============================================================================
// 8. Run test action on disabled connector — error case
// ============================================================================

#[tokio::test]
#[serial]
async fn test_connector_run_test_action_disabled() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let created = register_connector(&pool, &tid, &echo_req("disabled-echo"), corr())
        .await
        .expect("register failed");

    // Deactivate the connector directly via SQL
    sqlx::query(
        "UPDATE integrations_connector_configs SET enabled = FALSE WHERE id = $1 AND app_id = $2",
    )
    .bind(created.id)
    .bind(&tid)
    .execute(&pool)
    .await
    .expect("failed to deactivate connector");

    let action_req = RunTestActionRequest {
        idempotency_key: "key-xyz".to_string(),
    };
    let err = run_test_action(&pool, &tid, created.id, &action_req).await;

    assert!(err.is_err(), "disabled connector must reject test action");
    match err.unwrap_err() {
        ConnectorError::InvalidConfig(msg) => {
            assert!(
                msg.contains("disabled"),
                "expected 'disabled' in error: {}",
                msg
            );
        }
        other => panic!("expected InvalidConfig (disabled), got: {:?}", other),
    }
}

// ============================================================================
// 9. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_connector_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    // Tenant A registers a connector
    let created = register_connector(&pool, &tid_a, &echo_req("tenant-a-echo"), corr())
        .await
        .expect("register for tenant A failed");

    // Tenant B cannot see tenant A's connector by id
    let not_found = get_connector_config(&pool, &tid_b, created.id)
        .await
        .expect("tenant-b get should not DB-error");
    assert!(
        not_found.is_none(),
        "tenant B must not see tenant A's connector"
    );

    // Tenant B's list is empty
    let b_list = list_connector_configs(&pool, &tid_b, false)
        .await
        .expect("tenant-b list failed");
    assert!(
        b_list.iter().all(|c| c.id != created.id),
        "tenant B's list must not include tenant A's connector"
    );

    // Tenant B cannot trigger test action on tenant A's connector
    let action_req = RunTestActionRequest {
        idempotency_key: "iso-key".to_string(),
    };
    let err = run_test_action(&pool, &tid_b, created.id, &action_req).await;
    assert!(
        err.is_err(),
        "tenant B must not trigger tenant A's connector"
    );
    match err.unwrap_err() {
        ConnectorError::NotFound(_) => {}
        other => panic!(
            "expected NotFound for cross-tenant trigger, got: {:?}",
            other
        ),
    }
}
