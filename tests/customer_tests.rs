mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

const APP_ID: &str = "test-app";

/// TEST 1: Create customer with valid data
#[tokio::test]
#[serial]
async fn test_create_customer_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let email = common::unique_email();
    let external_id = common::unique_external_id();
    let body = serde_json::json!({
        "email": email,
        "name": "John Doe",
        "external_customer_id": external_id,
        "metadata": {"source": "test"}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/customers")
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
    assert!(json["id"].is_number(), "Response should contain customer id");
    assert_eq!(json["email"], email);
    assert_eq!(json["name"], "John Doe");
    assert_eq!(json["status"], "active");
    assert_eq!(json["external_customer_id"], external_id);

    // Verify customer was created in database
    let customer_id = json["id"].as_i64().unwrap() as i32;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_customers WHERE id = $1 AND email = $2"
    )
    .bind(customer_id)
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Customer should exist in database");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 2: Create customer with duplicate email
#[tokio::test]
#[serial]
async fn test_create_customer_duplicate_email() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed existing customer
    let (customer_id, email, _) = common::seed_customer(&pool, APP_ID).await;

    // Attempt to create duplicate
    let body = serde_json::json!({
        "email": email,
        "name": "Duplicate Customer",
        "external_customer_id": common::unique_external_id()
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/customers")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return conflict error
    assert!(
        response.status() == StatusCode::CONFLICT || response.status() == StatusCode::BAD_REQUEST,
        "Should return conflict or bad request for duplicate email"
    );

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 3: Create customer with missing email
#[tokio::test]
#[serial]
async fn test_create_customer_missing_email() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let body = serde_json::json!({
        "name": "No Email Customer"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/customers")
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

    common::teardown_pool(pool).await;
}

/// TEST 4: Get customer by ID
#[tokio::test]
#[serial]
async fn test_get_customer_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, email, external_id) = common::seed_customer(&pool, APP_ID).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/customers/{}", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 200 status
    assert_eq!(response.status(), StatusCode::OK);

    // Assert response body
    let json = common::body_json(response).await;
    assert_eq!(json["id"], customer_id);
    assert_eq!(json["email"], email);
    assert_eq!(json["external_customer_id"], external_id);
    assert_eq!(json["status"], "active");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 5: Get customer with invalid ID
#[tokio::test]
#[serial]
async fn test_get_customer_not_found() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/customers/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    common::teardown_pool(pool).await;
}

/// TEST 6: List customers with pagination
#[tokio::test]
#[serial]
async fn test_list_customers_with_pagination() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed multiple customers
    let (customer1_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let (customer2_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let (customer3_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // List with limit
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/customers?limit=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert!(json.as_array().unwrap().len() >= 2, "Should have at least 2 customers");

    common::cleanup_customers(&pool, &[customer1_id, customer2_id, customer3_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 7: Update customer
#[tokio::test]
#[serial]
async fn test_update_customer_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let new_email = common::unique_email();
    let body = serde_json::json!({
        "email": new_email,
        "name": "Updated Name",
        "metadata": {"updated": true}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&format!("/api/ar/customers/{}", customer_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], customer_id);
    assert_eq!(json["email"], new_email);
    assert_eq!(json["name"], "Updated Name");

    // Verify update in database
    let updated_name: String = sqlx::query_scalar(
        "SELECT name FROM ar_customers WHERE id = $1"
    )
    .bind(customer_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(updated_name, "Updated Name");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 8: List customers by external_customer_id
#[tokio::test]
#[serial]
async fn test_list_customers_by_external_id() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, external_id) = common::seed_customer(&pool, APP_ID).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/customers?external_customer_id={}", external_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 1, "Should find exactly 1 customer");
    assert_eq!(json[0]["id"], customer_id);
    assert_eq!(json[0]["external_customer_id"], external_id);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}
