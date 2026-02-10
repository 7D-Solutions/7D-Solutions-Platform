mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

const APP_ID: &str = "test-app";

// ============================================================================
// CHARGE TESTS
// ============================================================================

/// TEST 1: Create charge for customer
#[tokio::test]
#[serial]
async fn test_create_charge_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let body = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": 5000,
        "currency": "usd",
        "charge_type": "one_time",
        "reason": "Product purchase",
        "reference_id": common::unique_reference_id(),
        "metadata": {"product": "widget"}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/charges")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 201 status
    assert_eq!(response.status(), StatusCode::CREATED);

    // Assert response body
    let json = common::body_json(response).await;
    assert!(json["id"].is_number(), "Response should contain charge id");
    assert_eq!(json["ar_customer_id"], customer_id);
    assert_eq!(json["amount_cents"], 5000);
    assert_eq!(json["currency"], "usd");
    assert_eq!(json["charge_type"], "one_time");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 2: Create charge with invalid amount (negative)
#[tokio::test]
#[serial]
async fn test_create_charge_invalid_amount() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let body = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": -100,
        "currency": "usd",
        "reason": "Invalid charge",
        "reference_id": common::unique_reference_id()
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/charges")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return validation error
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = common::body_json(response).await;
    assert!(json["error"].is_string(), "Should have error message");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 3: Create charge with zero amount
#[tokio::test]
#[serial]
async fn test_create_charge_zero_amount() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let body = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": 0,
        "currency": "usd",
        "reason": "Zero charge",
        "reference_id": common::unique_reference_id()
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/charges")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return validation error
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 4: Get charge by ID
#[tokio::test]
#[serial]
async fn test_get_charge_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 2500, "succeeded").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/charges/{}", charge_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 200 status
    assert_eq!(response.status(), StatusCode::OK);

    // Assert response body
    let json = common::body_json(response).await;
    assert_eq!(json["id"], charge_id);
    assert_eq!(json["ar_customer_id"], customer_id);
    assert_eq!(json["amount_cents"], 2500);
    assert_eq!(json["status"], "succeeded");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 5: List charges for customer
#[tokio::test]
#[serial]
async fn test_list_charges_by_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge1_id = common::seed_charge(&pool, APP_ID, customer_id, 1000, "succeeded").await;
    let charge2_id = common::seed_charge(&pool, APP_ID, customer_id, 2000, "pending").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/charges?customer_id={}", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 2, "Should have 2 charges");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

// ============================================================================
// REFUND TESTS
// ============================================================================

/// TEST 6: Create refund for full charge amount
#[tokio::test]
#[serial]
async fn test_create_refund_full_amount() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;

    let body = serde_json::json!({
        "charge_id": charge_id,
        "amount_cents": 5000,
        "currency": "usd",
        "reason": "Customer request",
        "reference_id": common::unique_reference_id(),
        "metadata": {"approved_by": "support"}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/refunds")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 201 status
    assert_eq!(response.status(), StatusCode::CREATED);

    // Assert response body
    let json = common::body_json(response).await;
    assert!(json["id"].is_number(), "Response should contain refund id");
    assert_eq!(json["charge_id"], charge_id);
    assert_eq!(json["amount_cents"], 5000);
    assert_eq!(json["currency"], "usd");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 7: Create refund for partial charge amount
#[tokio::test]
#[serial]
async fn test_create_refund_partial_amount() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;

    let body = serde_json::json!({
        "charge_id": charge_id,
        "amount_cents": 2000,
        "currency": "usd",
        "reason": "Partial refund",
        "reference_id": common::unique_reference_id()
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/refunds")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 201 status
    assert_eq!(response.status(), StatusCode::CREATED);

    // Assert response body
    let json = common::body_json(response).await;
    assert_eq!(json["charge_id"], charge_id);
    assert_eq!(json["amount_cents"], 2000);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 8: Create refund exceeding charge amount
#[tokio::test]
#[serial]
async fn test_create_refund_exceeds_charge() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;

    let body = serde_json::json!({
        "charge_id": charge_id,
        "amount_cents": 10000,
        "currency": "usd",
        "reason": "Invalid refund",
        "reference_id": common::unique_reference_id()
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/refunds")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return validation error
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let json = common::body_json(response).await;
    assert!(json["error"].is_string(), "Should have error message");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 9: Get refund by ID
#[tokio::test]
#[serial]
async fn test_get_refund_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;

    // Create refund in database
    let refund_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_refunds (
            app_id, ar_customer_id, charge_id, status, amount_cents, currency,
            reference_id, created_at, updated_at
        ) VALUES ($1, $2, $3, 'succeeded', 2500, 'usd', $4, NOW(), NOW())
        RETURNING id"#,
    )
    .bind(APP_ID)
    .bind(customer_id)
    .bind(charge_id)
    .bind(common::unique_reference_id())
    .fetch_one(&pool)
    .await
    .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/refunds/{}", refund_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], refund_id);
    assert_eq!(json["charge_id"], charge_id);
    assert_eq!(json["amount_cents"], 2500);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 10: List refunds for customer
#[tokio::test]
#[serial]
async fn test_list_refunds_by_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;

    // Create multiple refunds
    for _ in 0..3 {
        sqlx::query(
            r#"INSERT INTO ar_refunds (
                app_id, ar_customer_id, charge_id, status, amount_cents, currency,
                reference_id, created_at, updated_at
            ) VALUES ($1, $2, $3, 'succeeded', 1000, 'usd', $4, NOW(), NOW())"#,
        )
        .bind(APP_ID)
        .bind(customer_id)
        .bind(charge_id)
        .bind(common::unique_reference_id())
        .execute(&pool)
        .await
        .unwrap();
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/refunds?customer_id={}", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 3, "Should have 3 refunds");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 11: List refunds by charge ID
#[tokio::test]
#[serial]
async fn test_list_refunds_by_charge() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;

    // Create refund
    sqlx::query(
        r#"INSERT INTO ar_refunds (
            app_id, ar_customer_id, charge_id, status, amount_cents, currency,
            reference_id, created_at, updated_at
        ) VALUES ($1, $2, $3, 'succeeded', 2500, 'usd', $4, NOW(), NOW())"#,
    )
    .bind(APP_ID)
    .bind(customer_id)
    .bind(charge_id)
    .bind(common::unique_reference_id())
    .execute(&pool)
    .await
    .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/refunds?charge_id={}", charge_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 1, "Should have 1 refund");
    assert_eq!(json[0]["charge_id"], charge_id);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

// ============================================================================
// CHARGE CAPTURE TESTS
// ============================================================================

/// TEST 12: Capture charge successfully
#[tokio::test]
#[serial]
async fn test_capture_charge_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "authorized").await;

    let body = serde_json::json!({
        "amount_cents": 5000
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/charges/{}/capture", charge_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 200 status
    assert_eq!(response.status(), StatusCode::OK);

    // Assert response body
    let json = common::body_json(response).await;
    assert_eq!(json["id"], charge_id);
    assert_eq!(json["status"], "succeeded");
    assert_eq!(json["amount_cents"], 5000);

    // Verify charge status updated in database
    let status: String = sqlx::query_scalar(
        "SELECT status FROM ar_charges WHERE id = $1"
    )
    .bind(charge_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "succeeded", "Charge should be marked as succeeded");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 13: Capture already captured charge
#[tokio::test]
#[serial]
async fn test_capture_charge_already_captured() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;

    let body = serde_json::json!({
        "amount_cents": 5000
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/charges/{}/capture", charge_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return error (already captured)
    assert!(
        response.status() == StatusCode::BAD_REQUEST || response.status() == StatusCode::CONFLICT,
        "Should return error for already captured charge"
    );

    let json = common::body_json(response).await;
    assert!(json["error"].is_string(), "Should have error message");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 14: Capture non-existent charge
#[tokio::test]
#[serial]
async fn test_capture_charge_not_found() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let body = serde_json::json!({
        "amount_cents": 5000
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/charges/999999/capture")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    common::teardown_pool(pool).await;
}

/// TEST 15: Capture charge with partial amount (supports partial capture)
#[tokio::test]
#[serial]
async fn test_capture_charge_partial_amount() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "authorized").await;

    // Capture partial amount (3000 out of 5000 authorized)
    let body = serde_json::json!({
        "amount_cents": 3000
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/charges/{}/capture", charge_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Partial capture succeeds
    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], charge_id);
    assert_eq!(json["status"], "succeeded");
    // Verify partial amount was captured as requested
    assert_eq!(json["amount_cents"], 3000);

    // Verify in database
    let amount: i32 = sqlx::query_scalar(
        "SELECT amount_cents FROM ar_charges WHERE id = $1"
    )
    .bind(charge_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(amount, 3000, "Database should reflect partial captured amount");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}
