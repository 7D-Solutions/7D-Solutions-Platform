mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

const APP_ID: &str = "test-app";

/// TEST 1: Add payment method with valid data
#[tokio::test]
#[serial]
async fn test_add_payment_method_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Create customer first
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let tilled_pm_id = format!("pm_{}", uuid::Uuid::new_v4());
    let body = serde_json::json!({
        "ar_customer_id": customer_id,
        "tilled_payment_method_id": tilled_pm_id
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/payment-methods")
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
    assert!(json["id"].is_number(), "Response should contain payment method id");
    assert_eq!(json["ar_customer_id"], customer_id);
    assert_eq!(json["tilled_payment_method_id"], tilled_pm_id);
    assert_eq!(json["status"], "active");
    assert_eq!(json["type"], "card");

    // Verify payment method was created in database
    let pm_id = json["id"].as_i64().unwrap() as i32;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_payment_methods WHERE id = $1 AND ar_customer_id = $2"
    )
    .bind(pm_id)
    .bind(customer_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Payment method should exist in database");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 2: Add payment method with invalid customer
#[tokio::test]
#[serial]
async fn test_add_payment_method_invalid_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let tilled_pm_id = format!("pm_{}", uuid::Uuid::new_v4());
    let body = serde_json::json!({
        "ar_customer_id": 999999, // Non-existent customer
        "tilled_payment_method_id": tilled_pm_id
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/payment-methods")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return error (404 or 400)
    assert!(
        response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::BAD_REQUEST,
        "Should return error for invalid customer"
    );

    common::teardown_pool(pool).await;
}

/// TEST 3: List payment methods by customer
#[tokio::test]
#[serial]
async fn test_list_payment_methods_by_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Create customer and payment methods
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let pm1_id = common::seed_payment_method(&pool, APP_ID, customer_id, true).await;
    let _pm2_id = common::seed_payment_method(&pool, APP_ID, customer_id, false).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/payment-methods?customer_id={}", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 2, "Should have 2 payment methods");

    // Verify default payment method is listed first
    assert_eq!(json[0]["id"], pm1_id);
    assert_eq!(json[0]["is_default"], true);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 4: Get payment method by ID
#[tokio::test]
#[serial]
async fn test_get_payment_method_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let pm_id = common::seed_payment_method(&pool, APP_ID, customer_id, true).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/payment-methods/{}", pm_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 200 status
    assert_eq!(response.status(), StatusCode::OK);

    // Assert response body
    let json = common::body_json(response).await;
    assert_eq!(json["id"], pm_id);
    assert_eq!(json["ar_customer_id"], customer_id);
    assert_eq!(json["status"], "active");
    assert_eq!(json["type"], "card");
    assert_eq!(json["brand"], "visa");
    assert_eq!(json["last4"], "4242");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 5: Get payment method with invalid ID
#[tokio::test]
#[serial]
async fn test_get_payment_method_not_found() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/payment-methods/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    common::teardown_pool(pool).await;
}

/// TEST 6: Update payment method metadata
#[tokio::test]
#[serial]
async fn test_update_payment_method_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let pm_id = common::seed_payment_method(&pool, APP_ID, customer_id, false).await;

    let body = serde_json::json!({
        "metadata": {"nickname": "Primary Card", "updated": true}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&format!("/api/ar/payment-methods/{}", pm_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], pm_id);
    assert!(json["metadata"].is_object());
    assert_eq!(json["metadata"]["nickname"], "Primary Card");

    // Verify update in database
    let metadata: serde_json::Value = sqlx::query_scalar(
        "SELECT metadata FROM ar_payment_methods WHERE id = $1"
    )
    .bind(pm_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(metadata["nickname"], "Primary Card");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 7: Delete payment method (soft delete)
#[tokio::test]
#[serial]
async fn test_delete_payment_method_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let pm_id = common::seed_payment_method(&pool, APP_ID, customer_id, false).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&format!("/api/ar/payment-methods/{}", pm_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 204 No Content
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify soft delete in database (deleted_at should be set)
    let deleted_at: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT deleted_at FROM ar_payment_methods WHERE id = $1"
    )
    .bind(pm_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(deleted_at.is_some(), "Payment method should be soft deleted");

    // Verify is_default is cleared
    let is_default: bool = sqlx::query_scalar(
        "SELECT is_default FROM ar_payment_methods WHERE id = $1"
    )
    .bind(pm_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(is_default, false, "Default flag should be cleared");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 8: Delete payment method with invalid ID
#[tokio::test]
#[serial]
async fn test_delete_payment_method_not_found() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/ar/payment-methods/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    common::teardown_pool(pool).await;
}

/// TEST 9: Set default payment method
#[tokio::test]
#[serial]
async fn test_set_default_payment_method_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let pm1_id = common::seed_payment_method(&pool, APP_ID, customer_id, true).await;
    let pm2_id = common::seed_payment_method(&pool, APP_ID, customer_id, false).await;

    // Set pm2 as default
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/payment-methods/{}/set-default", pm2_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], pm2_id);
    assert_eq!(json["is_default"], true);

    // Verify pm1 is no longer default
    let pm1_is_default: bool = sqlx::query_scalar(
        "SELECT is_default FROM ar_payment_methods WHERE id = $1"
    )
    .bind(pm1_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pm1_is_default, false, "Previous default should be cleared");

    // Verify pm2 is now default
    let pm2_is_default: bool = sqlx::query_scalar(
        "SELECT is_default FROM ar_payment_methods WHERE id = $1"
    )
    .bind(pm2_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pm2_is_default, true, "New payment method should be default");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 10: Set default payment method when already default
#[tokio::test]
#[serial]
async fn test_set_default_payment_method_already_default() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let pm_id = common::seed_payment_method(&pool, APP_ID, customer_id, true).await;

    // Try to set already-default payment method as default again
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/payment-methods/{}/set-default", pm_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should still return success (idempotent operation)
    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], pm_id);
    assert_eq!(json["is_default"], true);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 11: List payment methods with pagination
#[tokio::test]
#[serial]
async fn test_list_payment_methods_with_pagination() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    common::seed_payment_method(&pool, APP_ID, customer_id, true).await;
    common::seed_payment_method(&pool, APP_ID, customer_id, false).await;
    common::seed_payment_method(&pool, APP_ID, customer_id, false).await;

    // List with limit
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/payment-methods?customer_id={}&limit=2", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 2, "Should have 2 payment methods with limit");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 12: List payment methods filtered by status
#[tokio::test]
#[serial]
async fn test_list_payment_methods_by_status() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let pm_id = common::seed_payment_method(&pool, APP_ID, customer_id, true).await;

    // Create another payment method with pending status
    let tilled_pm_id = format!("pm_{}", uuid::Uuid::new_v4());
    sqlx::query(
        r#"INSERT INTO ar_payment_methods (
            app_id, ar_customer_id, tilled_payment_method_id,
            status, type, is_default, created_at, updated_at
        ) VALUES ($1, $2, $3, 'pending', 'card', FALSE, NOW(), NOW())"#,
    )
    .bind(APP_ID)
    .bind(customer_id)
    .bind(&tilled_pm_id)
    .execute(&pool)
    .await
    .unwrap();

    // List only active payment methods
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/payment-methods?customer_id={}&status=active", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 1, "Should have 1 active payment method");
    assert_eq!(json[0]["id"], pm_id);
    assert_eq!(json[0]["status"], "active");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}
