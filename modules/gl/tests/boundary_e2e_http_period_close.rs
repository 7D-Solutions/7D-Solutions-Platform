//! HTTP Boundary E2E Tests: Period Close Workflow (Phase 13: bd-37m)
//!
//! Tests verify HTTP endpoints for period close lifecycle:
//! - POST /api/gl/periods/{id}/validate-close
//! - POST /api/gl/periods/{id}/close
//! - GET /api/gl/periods/{id}/close-status
//!
//! All tests use serial execution with singleton pool + connection caps.

mod common;

use chrono::NaiveDate;
use common::{cleanup_test_tenant, get_test_pool, setup_test_account, setup_test_period};
use reqwest::StatusCode;
use serde_json::json;
use serial_test::serial;
use std::time::Duration;
use uuid::Uuid;

/// HTTP client with retries for flakiness
async fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

/// Base URL for GL service (matches Phase 12 pattern)
const BASE_URL: &str = "http://localhost:8090";

// ============================================================
// VALIDATE CLOSE ENDPOINT TESTS
// ============================================================

/// Test validate-close succeeds on open period
#[tokio::test]
#[serial]
async fn test_http_validate_close_success() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_http_val_success";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create open period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
    )
    .await;

    // HTTP POST to validate-close
    let client = http_client().await;
    let url = format!("{}/api/gl/periods/{}/validate-close", BASE_URL, period_id);
    let body = json!({ "tenant_id": tenant_id });

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap();

    // Should return 200 OK
    assert_eq!(response.status(), StatusCode::OK);

    // Parse response
    let json: serde_json::Value = response.json().await.unwrap();

    // Verify response structure
    assert_eq!(json["period_id"], period_id.to_string());
    assert_eq!(json["tenant_id"], tenant_id);
    assert_eq!(json["can_close"], true);
    assert!(json["validation_report"]["issues"].as_array().unwrap().is_empty());
    assert!(json["validated_at"].is_string());
}

/// Test validate-close fails when period already closed
#[tokio::test]
#[serial]
async fn test_http_validate_close_already_closed() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_http_val_closed";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 2, 28).unwrap(),
    )
    .await;

    // Close the period manually
    sqlx::query(
        "UPDATE accounting_periods SET closed_at = NOW(), close_hash = 'test_hash' WHERE id = $1",
    )
    .bind(period_id)
    .execute(&pool)
    .await
    .unwrap();

    // HTTP POST to validate-close
    let client = http_client().await;
    let url = format!("{}/api/gl/periods/{}/validate-close", BASE_URL, period_id);
    let body = json!({ "tenant_id": tenant_id });

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap();

    // Should return 200 OK (validation always succeeds, but can_close=false)
    assert_eq!(response.status(), StatusCode::OK);

    // Parse response
    let json: serde_json::Value = response.json().await.unwrap();

    // Verify response
    assert_eq!(json["can_close"], false);
    assert!(!json["validation_report"]["issues"].as_array().unwrap().is_empty());

    // Verify has PERIOD_ALREADY_CLOSED error
    let issues = json["validation_report"]["issues"].as_array().unwrap();
    let has_already_closed = issues
        .iter()
        .any(|issue| issue["code"] == "PERIOD_ALREADY_CLOSED");
    assert!(has_already_closed);
}

// ============================================================
// CLOSE PERIOD ENDPOINT TESTS
// ============================================================

/// Test successful close returns close status with hash
#[tokio::test]
#[serial]
async fn test_http_close_period_success() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_http_close_success";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create open period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(),
    )
    .await;

    // HTTP POST to close
    let client = http_client().await;
    let url = format!("{}/api/gl/periods/{}/close", BASE_URL, period_id);
    let body = json!({
        "tenant_id": tenant_id,
        "closed_by": "http_test_user",
        "close_reason": "HTTP E2E test close"
    });

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap();

    // Should return 200 OK
    assert_eq!(response.status(), StatusCode::OK);

    // Parse response
    let json: serde_json::Value = response.json().await.unwrap();

    // Verify response
    assert_eq!(json["success"], true);
    assert!(json["close_status"].is_object());
    assert!(json["validation_report"].is_null());

    // Verify close status structure
    let close_status = &json["close_status"];
    assert_eq!(close_status["state"], "CLOSED");
    assert_eq!(close_status["closed_by"], "http_test_user");
    assert_eq!(close_status["close_reason"], "HTTP E2E test close");
    assert!(close_status["close_hash"].is_string());
    assert_eq!(close_status["close_hash"].as_str().unwrap().len(), 64); // SHA-256 hex

    // Verify period is actually closed in database
    let closed_at = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT closed_at FROM accounting_periods WHERE id = $1",
    )
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(closed_at.is_some());
}

/// Test idempotent close - calling close twice returns same hash
#[tokio::test]
#[serial]
async fn test_http_close_period_idempotent() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_http_close_idem";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create open period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 4, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 4, 30).unwrap(),
    )
    .await;

    let client = http_client().await;
    let url = format!("{}/api/gl/periods/{}/close", BASE_URL, period_id);
    let body = json!({
        "tenant_id": tenant_id,
        "closed_by": "user1",
        "close_reason": "First close"
    });

    // First close
    let response1 = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap();

    assert_eq!(response1.status(), StatusCode::OK);
    let json1: serde_json::Value = response1.json().await.unwrap();
    assert_eq!(json1["success"], true);

    let hash1 = json1["close_status"]["close_hash"].as_str().unwrap();

    // Second close (different user, different reason - should be ignored)
    let body2 = json!({
        "tenant_id": tenant_id,
        "closed_by": "user2",
        "close_reason": "Second close"
    });

    let response2 = client
        .post(&url)
        .json(&body2)
        .send()
        .await
        .unwrap();

    assert_eq!(response2.status(), StatusCode::OK);
    let json2: serde_json::Value = response2.json().await.unwrap();
    assert_eq!(json2["success"], true);

    let hash2 = json2["close_status"]["close_hash"].as_str().unwrap();

    // Verify idempotency
    assert_eq!(hash1, hash2);
    assert_eq!(json2["close_status"]["closed_by"], "user1"); // Original user
    assert_eq!(json2["close_status"]["close_reason"], "First close"); // Original reason
}

/// Test close fails when validation detects unbalanced entries
#[tokio::test]
#[serial]
async fn test_http_close_period_validation_failure() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_http_close_unbalanced";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 5, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 5, 31).unwrap(),
    )
    .await;

    // Create account
    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;

    // Create unbalanced journal entry
    let entry_id = Uuid::new_v4();
    let entry_date = NaiveDate::from_ymd_opt(2025, 5, 15).unwrap();
    let posted_at = entry_date.and_hms_opt(12, 0, 0).unwrap().and_utc();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description, created_at)
        VALUES ($1, $2, 'test', gen_random_uuid(), 'test', $3, 'USD', 'Unbalanced', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(posted_at)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES (gen_random_uuid(), $1, 1, '1000', 100000, 0, 'Unbalanced')
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .unwrap();

    // HTTP POST to close - should fail validation
    let client = http_client().await;
    let url = format!("{}/api/gl/periods/{}/close", BASE_URL, period_id);
    let body = json!({
        "tenant_id": tenant_id,
        "closed_by": "admin"
    });

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap();

    // Should return 200 OK (but success=false)
    assert_eq!(response.status(), StatusCode::OK);

    // Parse response
    let json: serde_json::Value = response.json().await.unwrap();

    // Verify failure
    assert_eq!(json["success"], false);
    assert!(json["close_status"].is_null());
    assert!(json["validation_report"].is_object());

    // Verify has UNBALANCED_ENTRIES error
    let issues = json["validation_report"]["issues"].as_array().unwrap();
    let has_unbalanced = issues
        .iter()
        .any(|issue| issue["code"] == "UNBALANCED_ENTRIES");
    assert!(has_unbalanced);

    // Verify period is NOT closed in database
    let closed_at = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT closed_at FROM accounting_periods WHERE id = $1",
    )
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(closed_at.is_none());
}

// ============================================================
// CLOSE STATUS ENDPOINT TESTS
// ============================================================

/// Test close-status returns Open for open period
#[tokio::test]
#[serial]
async fn test_http_close_status_open() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_http_status_open";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create open period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 6, 30).unwrap(),
    )
    .await;

    // HTTP GET close-status
    let client = http_client().await;
    let url = format!(
        "{}/api/gl/periods/{}/close-status?tenant_id={}",
        BASE_URL, period_id, tenant_id
    );

    let response = client.get(&url).send().await.unwrap();

    // Should return 200 OK
    assert_eq!(response.status(), StatusCode::OK);

    // Parse response
    let json: serde_json::Value = response.json().await.unwrap();

    // Verify response
    assert_eq!(json["period_id"], period_id.to_string());
    assert_eq!(json["tenant_id"], tenant_id);
    assert_eq!(json["period_start"], "2025-06-01");
    assert_eq!(json["period_end"], "2025-06-30");
    assert_eq!(json["close_status"]["state"], "OPEN");
}

/// Test close-status reflects sealed snapshot after close
#[tokio::test]
#[serial]
async fn test_http_close_status_closed_with_hash() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_http_status_closed";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 7, 31).unwrap(),
    )
    .await;

    // Close the period via HTTP
    let client = http_client().await;
    let close_url = format!("{}/api/gl/periods/{}/close", BASE_URL, period_id);
    let close_body = json!({
        "tenant_id": tenant_id,
        "closed_by": "status_test_user",
        "close_reason": "Status test"
    });

    let close_response = client
        .post(&close_url)
        .json(&close_body)
        .send()
        .await
        .unwrap();

    assert_eq!(close_response.status(), StatusCode::OK);

    let close_json: serde_json::Value = close_response.json().await.unwrap();
    let expected_hash = close_json["close_status"]["close_hash"].as_str().unwrap();

    // Now query close-status
    let status_url = format!(
        "{}/api/gl/periods/{}/close-status?tenant_id={}",
        BASE_URL, period_id, tenant_id
    );

    let status_response = client.get(&status_url).send().await.unwrap();

    assert_eq!(status_response.status(), StatusCode::OK);

    // Parse status response
    let status_json: serde_json::Value = status_response.json().await.unwrap();

    // Verify CLOSED state with hash
    assert_eq!(status_json["close_status"]["state"], "CLOSED");
    assert_eq!(status_json["close_status"]["closed_by"], "status_test_user");
    assert_eq!(status_json["close_status"]["close_reason"], "Status test");
    assert_eq!(status_json["close_status"]["close_hash"], expected_hash);
    assert!(status_json["close_status"]["closed_at"].is_string());
}

/// Test close-status returns 404 for non-existent period
#[tokio::test]
#[serial]
async fn test_http_close_status_not_found() {
    let tenant_id = "tenant_http_status_notfound";
    let fake_period_id = Uuid::new_v4();

    // HTTP GET close-status for non-existent period
    let client = http_client().await;
    let url = format!(
        "{}/api/gl/periods/{}/close-status?tenant_id={}",
        BASE_URL, fake_period_id, tenant_id
    );

    let response = client.get(&url).send().await.unwrap();

    // Should return 404 NOT_FOUND
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Parse error response
    let json: serde_json::Value = response.json().await.unwrap();
    assert!(json["error"].is_string());
    assert!(json["error"].as_str().unwrap().contains("not found"));
}

// ============================================================
// PERFORMANCE GUARD
// ============================================================

/// Test all endpoints complete in reasonable time (< 1s per suite target)
#[tokio::test]
#[serial]
async fn test_http_period_close_performance_guard() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_http_perf";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create open period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 8, 31).unwrap(),
    )
    .await;

    let client = http_client().await;

    // Test validate-close performance
    let start = std::time::Instant::now();
    let validate_url = format!("{}/api/gl/periods/{}/validate-close", BASE_URL, period_id);
    let validate_body = json!({ "tenant_id": tenant_id });

    client
        .post(&validate_url)
        .json(&validate_body)
        .send()
        .await
        .unwrap();

    let validate_duration = start.elapsed();
    assert!(
        validate_duration < Duration::from_millis(500),
        "Validate took {:?}",
        validate_duration
    );

    // Test close performance
    let start = std::time::Instant::now();
    let close_url = format!("{}/api/gl/periods/{}/close", BASE_URL, period_id);
    let close_body = json!({
        "tenant_id": tenant_id,
        "closed_by": "perf_test"
    });

    client
        .post(&close_url)
        .json(&close_body)
        .send()
        .await
        .unwrap();

    let close_duration = start.elapsed();
    assert!(
        close_duration < Duration::from_millis(500),
        "Close took {:?}",
        close_duration
    );

    // Test close-status performance
    let start = std::time::Instant::now();
    let status_url = format!(
        "{}/api/gl/periods/{}/close-status?tenant_id={}",
        BASE_URL, period_id, tenant_id
    );

    client.get(&status_url).send().await.unwrap();

    let status_duration = start.elapsed();
    assert!(
        status_duration < Duration::from_millis(500),
        "Close-status took {:?}",
        status_duration
    );
}
