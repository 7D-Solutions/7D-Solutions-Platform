mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

const APP_ID: &str = "test-app";

/// TEST 1: Create subscription for customer
#[tokio::test]
#[serial]
async fn test_create_subscription_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let body = serde_json::json!({
        "ar_customer_id": customer_id,
        "payment_method_id": "pm_test123",
        "plan_id": common::unique_plan_id(),
        "plan_name": "Pro Plan",
        "price_cents": 2999,
        "interval_unit": "month",
        "interval_count": 1,
        "metadata": {"trial": false}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/subscriptions")
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
    assert!(json["id"].is_number(), "Response should contain subscription id");
    assert_eq!(json["ar_customer_id"], customer_id);
    assert_eq!(json["plan_name"], "Pro Plan");
    assert_eq!(json["price_cents"], 2999);
    assert_eq!(json["status"], "active");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 2: Create subscription with invalid customer ID
#[tokio::test]
#[serial]
async fn test_create_subscription_invalid_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let body = serde_json::json!({
        "ar_customer_id": 999999,
        "payment_method_id": "pm_test123",
        "plan_id": common::unique_plan_id(),
        "plan_name": "Pro Plan",
        "price_cents": 2999
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/subscriptions")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return not found or bad request
    assert!(
        response.status() == StatusCode::NOT_FOUND || response.status() == StatusCode::BAD_REQUEST,
        "Should return error for invalid customer"
    );

    common::teardown_pool(pool).await;
}

/// TEST 3: Get subscription by ID
#[tokio::test]
#[serial]
async fn test_get_subscription_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let subscription_id = common::seed_subscription(&pool, APP_ID, customer_id, "active").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/subscriptions/{}", subscription_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 200 status
    assert_eq!(response.status(), StatusCode::OK);

    // Assert response body
    let json = common::body_json(response).await;
    assert_eq!(json["id"], subscription_id);
    assert_eq!(json["ar_customer_id"], customer_id);
    assert_eq!(json["status"], "active");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 4: Cancel subscription with cancel_at_period_end
#[tokio::test]
#[serial]
async fn test_cancel_subscription_at_period_end() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let subscription_id = common::seed_subscription(&pool, APP_ID, customer_id, "active").await;

    let body = serde_json::json!({
        "cancel_at_period_end": true
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/subscriptions/{}/cancel", subscription_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], subscription_id);
    assert_eq!(json["cancel_at_period_end"], true);

    // Verify in database
    let cancel_at_period_end: bool = sqlx::query_scalar(
        "SELECT cancel_at_period_end FROM ar_subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(cancel_at_period_end, true);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 5: Cancel subscription immediately
#[tokio::test]
#[serial]
async fn test_cancel_subscription_immediately() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let subscription_id = common::seed_subscription(&pool, APP_ID, customer_id, "active").await;

    let body = serde_json::json!({
        "cancel_at_period_end": false
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/subscriptions/{}/cancel", subscription_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], subscription_id);
    assert_eq!(json["status"], "canceled");

    // Verify in database
    let status: String = sqlx::query_scalar(
        "SELECT status::text FROM ar_subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "canceled");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 6: List subscriptions for customer
#[tokio::test]
#[serial]
async fn test_list_subscriptions_by_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let sub1_id = common::seed_subscription(&pool, APP_ID, customer_id, "active").await;
    let sub2_id = common::seed_subscription(&pool, APP_ID, customer_id, "canceled").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/subscriptions?customer_id={}", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 2, "Should have 2 subscriptions");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 7: List subscriptions by status
#[tokio::test]
#[serial]
async fn test_list_subscriptions_by_status() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let active_sub_id = common::seed_subscription(&pool, APP_ID, customer_id, "active").await;
    let canceled_sub_id = common::seed_subscription(&pool, APP_ID, customer_id, "canceled").await;

    // Query only active subscriptions
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/subscriptions?customer_id={}&status=active", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(json.as_array().unwrap().len(), 1, "Should have 1 active subscription");
    assert_eq!(json[0]["id"], active_sub_id);
    assert_eq!(json[0]["status"], "active");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 8: Update subscription price
#[tokio::test]
#[serial]
async fn test_update_subscription_price() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let subscription_id = common::seed_subscription(&pool, APP_ID, customer_id, "active").await;

    let body = serde_json::json!({
        "price_cents": 3999,
        "plan_name": "Premium Plan"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&format!("/api/ar/subscriptions/{}", subscription_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], subscription_id);
    assert_eq!(json["price_cents"], 3999);
    assert_eq!(json["plan_name"], "Premium Plan");

    // Verify in database
    let price: i32 = sqlx::query_scalar(
        "SELECT price_cents FROM ar_subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(price, 3999);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}
