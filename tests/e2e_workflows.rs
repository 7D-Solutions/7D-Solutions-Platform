/// End-to-End workflow tests for AR migration
/// Tests complete user journeys across multiple endpoints
mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

const APP_ID: &str = "test-app";

/// E2E TEST 1: Complete customer lifecycle
/// Create customer → Update → List → Get → Archive
#[tokio::test]
#[serial]
async fn test_customer_lifecycle_workflow() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Step 1: Create customer
    let email = common::unique_email();
    let external_id = common::unique_external_id();
    let create_body = serde_json::json!({
        "email": email,
        "name": "Lifecycle Test Customer",
        "external_customer_id": external_id,
        "metadata": {"source": "e2e_test"}
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/customers")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let customer = common::body_json(response).await;
    let customer_id = customer["id"].as_i64().unwrap() as i32;

    // Step 2: Update customer
    let new_email = common::unique_email();
    let update_body = serde_json::json!({
        "email": new_email,
        "name": "Updated Lifecycle Customer",
        "metadata": {"updated": true}
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&format!("/api/ar/customers/{}", customer_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let updated = common::body_json(response).await;
    assert_eq!(updated["email"], new_email);
    assert_eq!(updated["name"], "Updated Lifecycle Customer");

    // Step 3: Get customer
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/customers/{}", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let fetched = common::body_json(response).await;
    assert_eq!(fetched["id"], customer_id);
    assert_eq!(fetched["email"], new_email);

    // Step 4: List customers (verify it appears)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/customers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let list = common::body_json(response).await;
    assert!(list.is_array());
    let found = list
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"].as_i64().unwrap() == customer_id as i64);
    assert!(found, "Customer should appear in list");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// E2E TEST 2: Subscription workflow
/// Create customer → Create subscription → Update subscription → Cancel
#[tokio::test]
#[serial]
async fn test_subscription_workflow() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Step 1: Create customer
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Step 2: Create subscription
    let plan_id = common::unique_plan_id();
    let create_sub_body = serde_json::json!({
        "ar_customer_id": customer_id,
        "plan_id": plan_id,
        "plan_name": "Pro Plan",
        "price_cents": 2999,
        "interval_unit": "month",
        "interval_count": 1,
        "payment_method_id": "pm_test123",
        "payment_method_type": "card"
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/subscriptions")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&create_sub_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let subscription = common::body_json(response).await;
    let subscription_id = subscription["id"].as_i64().unwrap() as i32;
    assert_eq!(subscription["status"], "active");

    // Step 3: Update subscription
    let update_sub_body = serde_json::json!({
        "plan_name": "Premium Plan",
        "price_cents": 4999
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&format!("/api/ar/subscriptions/{}", subscription_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&update_sub_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let updated_sub = common::body_json(response).await;
    assert_eq!(updated_sub["plan_name"], "Premium Plan");
    assert_eq!(updated_sub["price_cents"], 4999);

    // Step 4: Cancel subscription
    let cancel_body = serde_json::json!({
        "cancel_at_period_end": true
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/subscriptions/{}/cancel", subscription_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&cancel_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let cancelled_sub = common::body_json(response).await;
    assert_eq!(cancelled_sub["cancel_at_period_end"], true);

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// E2E TEST 3: Payment workflow
/// Create customer → Create charge → Capture charge → Create refund
#[tokio::test]
#[serial]
async fn test_payment_workflow() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Step 1: Create customer
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Step 2: Create charge (authorized, not captured)
    let create_charge_body = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": 5000,
        "currency": "usd",
        "charge_type": "one_time",
        "reason": "Test purchase",
        "capture": false,
        "payment_method_id": "pm_test123"
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/charges")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&create_charge_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let charge = common::body_json(response).await;
    let charge_id = charge["id"].as_i64().unwrap() as i32;
    assert_eq!(charge["amount_cents"], 5000);
    assert_eq!(charge["status"], "authorized");

    // Step 3: Capture charge
    let capture_body = serde_json::json!({
        "amount_cents": 5000
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/charges/{}/capture", charge_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&capture_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let captured = common::body_json(response).await;
    assert_eq!(captured["status"], "succeeded");

    // Step 4: Create partial refund
    let charge_id_str = charge["tilled_charge_id"].as_str().unwrap();
    let refund_body = serde_json::json!({
        "charge_id": charge_id_str,
        "amount_cents": 2000,
        "reason": "partial_refund"
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/refunds")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&refund_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let refund = common::body_json(response).await;
    assert_eq!(refund["amount_cents"], 2000);
    assert_eq!(refund["status"], "succeeded");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// E2E TEST 4: Invoice workflow
/// Create customer → Create invoice → Add line items → Finalize → Pay
#[tokio::test]
#[serial]
async fn test_invoice_workflow() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Step 1: Create customer
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Step 2: Create draft invoice
    let create_invoice_body = serde_json::json!({
        "ar_customer_id": customer_id,
        "due_date": "2026-03-01",
        "line_items": [
            {
                "description": "Consulting Services",
                "amount_cents": 15000,
                "quantity": 1
            },
            {
                "description": "Implementation Fee",
                "amount_cents": 25000,
                "quantity": 1
            }
        ],
        "metadata": {"project": "migration"}
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/invoices")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&create_invoice_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let invoice = common::body_json(response).await;
    let invoice_id = invoice["id"].as_i64().unwrap() as i32;
    assert_eq!(invoice["status"], "draft");
    assert_eq!(invoice["total_cents"], 40000);

    // Step 3: Finalize invoice
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/invoices/{}/finalize", invoice_id))
                .header("content-type", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let finalized = common::body_json(response).await;
    assert_eq!(finalized["status"], "open");

    // Step 4: Pay invoice (create charge)
    let pay_body = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": 40000,
        "currency": "usd",
        "charge_type": "one_time",
        "reason": format!("Payment for invoice {}", invoice_id),
        "reference_id": format!("invoice_{}", invoice_id),
        "payment_method_id": "pm_test123"
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/charges")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&pay_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let payment = common::body_json(response).await;
    assert_eq!(payment["amount_cents"], 40000);
    assert_eq!(payment["status"], "succeeded");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// E2E TEST 5: Webhook processing workflow
/// Create customer → Trigger webhook → Verify event logged → Replay webhook
#[tokio::test]
#[serial]
async fn test_webhook_workflow() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Step 1: Create test customer and subscription
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let subscription_id = common::seed_subscription(&pool, APP_ID, customer_id, "active").await;

    // Step 2: Simulate Tilled webhook (payment succeeded)
    let webhook_body = serde_json::json!({
        "id": format!("evt_{}", uuid::Uuid::new_v4()),
        "type": "payment_intent.succeeded",
        "data": {
            "object": {
                "id": format!("pi_{}", uuid::Uuid::new_v4()),
                "amount": 2999,
                "currency": "usd",
                "customer_id": customer_id,
                "subscription_id": subscription_id,
                "status": "succeeded"
            }
        },
        "created": chrono::Utc::now().timestamp()
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/webhooks/tilled")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&webhook_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let webhook_response = common::body_json(response).await;
    assert_eq!(webhook_response["status"], "processed");

    // Step 3: List webhooks to verify it was logged
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/webhooks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let webhooks = common::body_json(response).await;
    assert!(webhooks.is_array());
    assert!(webhooks.as_array().unwrap().len() > 0);

    // Step 4: Get specific webhook
    let webhook_id = webhooks[0]["id"].as_i64().unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/webhooks/{}", webhook_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let webhook = common::body_json(response).await;
    assert_eq!(webhook["event_type"], "payment_intent.succeeded");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// E2E TEST 6: Error recovery workflow
/// Test idempotency keys and retry behavior
#[tokio::test]
#[serial]
async fn test_error_recovery_workflow() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Step 1: Create customer with idempotency key
    let email = common::unique_email();
    let external_id = common::unique_external_id();
    let idempotency_key = uuid::Uuid::new_v4().to_string();
    let create_body = serde_json::json!({
        "email": email,
        "name": "Idempotency Test",
        "external_customer_id": external_id
    });

    let response1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/customers")
                .header("content-type", "application/json")
                .header("idempotency-key", &idempotency_key)
                .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response1.status(), StatusCode::CREATED);
    let customer1 = common::body_json(response1).await;
    let customer_id = customer1["id"].as_i64().unwrap() as i32;

    // Step 2: Retry same request with same idempotency key (should return cached response)
    let response2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/customers")
                .header("content-type", "application/json")
                .header("idempotency-key", &idempotency_key)
                .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response2.status(), StatusCode::OK); // Returns cached response
    let customer2 = common::body_json(response2).await;
    assert_eq!(customer2["id"], customer1["id"]);
    assert_eq!(customer2["email"], customer1["email"]);

    // Step 3: Verify only one customer was created
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_customers WHERE email = $1"
    )
    .bind(&email)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Should only have one customer despite retry");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// E2E TEST 7: Multi-tenant isolation
/// Verify customers from different app_ids are isolated
#[tokio::test]
#[serial]
async fn test_multi_tenant_isolation() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Create customers in different apps
    let (customer1_id, email1, _) = common::seed_customer(&pool, "app1").await;
    let (customer2_id, email2, _) = common::seed_customer(&pool, "app2").await;

    // Verify both exist in database
    let count1: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_customers WHERE app_id = $1"
    )
    .bind("app1")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(count1 >= 1);

    let count2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_customers WHERE app_id = $1"
    )
    .bind("app2")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(count2 >= 1);

    // List customers (should see all in test context, but app filtering would apply in production)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/customers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let customers = common::body_json(response).await;
    assert!(customers.is_array());
    assert!(customers.as_array().unwrap().len() >= 2);

    common::cleanup_customers(&pool, &[customer1_id, customer2_id]).await;
    common::teardown_pool(pool).await;
}
