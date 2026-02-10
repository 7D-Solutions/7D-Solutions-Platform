use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Test helper to create a test database pool
async fn setup_test_db() -> PgPool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5433/ar_test".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database")
}

#[tokio::test]
async fn test_idempotency_key_storage() {
    let db = setup_test_db().await;

    // Clean up previous test data
    sqlx::query("DELETE FROM billing_idempotency_keys WHERE app_id = 'test_app'")
        .execute(&db)
        .await
        .ok();

    // Insert idempotency key
    let app_id = "test_app";
    let idempotency_key = "test-key-001";
    let request_hash = "abc123";
    let response_body = serde_json::json!({"status": "success"});
    let status_code = 201;
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);

    let result = sqlx::query(
        r#"
        INSERT INTO billing_idempotency_keys
            (app_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(idempotency_key)
    .bind(request_hash)
    .bind(&response_body)
    .bind(status_code)
    .bind(expires_at.naive_utc())
    .fetch_one(&db)
    .await;

    assert!(result.is_ok(), "Failed to insert idempotency key");

    // Verify key can be retrieved
    let retrieved: Option<(String, i32)> = sqlx::query_as(
        r#"
        SELECT idempotency_key, status_code
        FROM billing_idempotency_keys
        WHERE app_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(app_id)
    .bind(idempotency_key)
    .fetch_optional(&db)
    .await
    .expect("Failed to retrieve idempotency key");

    assert!(retrieved.is_some());
    let (key, code) = retrieved.unwrap();
    assert_eq!(key, idempotency_key);
    assert_eq!(code, status_code);

    // Clean up
    sqlx::query("DELETE FROM billing_idempotency_keys WHERE app_id = 'test_app'")
        .execute(&db)
        .await
        .ok();
}

#[tokio::test]
async fn test_event_logging() {
    let db = setup_test_db().await;

    // Clean up previous test data
    sqlx::query("DELETE FROM billing_events WHERE app_id = 'test_app'")
        .execute(&db)
        .await
        .ok();

    // Insert event
    let app_id = "test_app";
    let event_type = "customer.created";
    let source = "api";
    let entity_type = Some("customer");
    let entity_id = Some("123");
    let payload = Some(serde_json::json!({
        "email": "test@example.com",
        "name": "Test User"
    }));

    let result = sqlx::query(
        r#"
        INSERT INTO billing_events
            (app_id, event_type, source, entity_type, entity_id, payload)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(event_type)
    .bind(source)
    .bind(entity_type)
    .bind(entity_id)
    .bind(&payload)
    .fetch_one(&db)
    .await;

    assert!(result.is_ok(), "Failed to insert event");

    // Verify event can be retrieved
    let retrieved: Option<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT event_type, source, entity_id
        FROM billing_events
        WHERE app_id = $1 AND event_type = $2
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(app_id)
    .bind(event_type)
    .fetch_optional(&db)
    .await
    .expect("Failed to retrieve event");

    assert!(retrieved.is_some());
    let (evt_type, evt_source, evt_entity_id) = retrieved.unwrap();
    assert_eq!(evt_type, event_type);
    assert_eq!(evt_source, source);
    assert_eq!(evt_entity_id, "123");

    // Test filtering by entity_id
    let count: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM billing_events
        WHERE app_id = $1 AND entity_id = $2
        "#,
    )
    .bind(app_id)
    .bind("123")
    .fetch_one(&db)
    .await
    .expect("Failed to count events");

    assert_eq!(count.0, 1);

    // Clean up
    sqlx::query("DELETE FROM billing_events WHERE app_id = 'test_app'")
        .execute(&db)
        .await
        .ok();
}

#[tokio::test]
async fn test_duplicate_idempotency_key() {
    let db = setup_test_db().await;

    // Clean up previous test data
    sqlx::query("DELETE FROM billing_idempotency_keys WHERE app_id = 'test_app'")
        .execute(&db)
        .await
        .ok();

    let app_id = "test_app";
    let idempotency_key = "duplicate-test-key";
    let request_hash = "hash123";
    let response_body = serde_json::json!({"status": "success"});
    let status_code = 201;
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);

    // Insert first time
    let result1 = sqlx::query(
        r#"
        INSERT INTO billing_idempotency_keys
            (app_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(idempotency_key)
    .bind(request_hash)
    .bind(&response_body)
    .bind(status_code)
    .bind(expires_at.naive_utc())
    .fetch_one(&db)
    .await;

    assert!(result1.is_ok(), "First insert should succeed");

    // Try to insert duplicate (should fail due to unique constraint)
    let result2 = sqlx::query(
        r#"
        INSERT INTO billing_idempotency_keys
            (app_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(idempotency_key)
    .bind(request_hash)
    .bind(&response_body)
    .bind(status_code)
    .bind(expires_at.naive_utc())
    .fetch_one(&db)
    .await;

    assert!(result2.is_err(), "Duplicate insert should fail");

    // Clean up
    sqlx::query("DELETE FROM billing_idempotency_keys WHERE app_id = 'test_app'")
        .execute(&db)
        .await
        .ok();
}
