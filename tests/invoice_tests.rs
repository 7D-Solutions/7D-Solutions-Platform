mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

const APP_ID: &str = "test-app";

// ============================================================================
// INVOICE TESTS
// ============================================================================

/// TEST 1: Create invoice with valid data
#[tokio::test]
#[serial]
async fn test_create_invoice_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let body = serde_json::json!({
        "ar_customer_id": customer_id,
        "amount_cents": 10000,
        "currency": "usd",
        "due_at": "2026-03-15T00:00:00",
        "metadata": {"invoice_type": "subscription", "description": "Monthly subscription invoice"}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/invoices")
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
    assert!(json["id"].is_number(), "Response should contain invoice id");
    assert_eq!(json["ar_customer_id"], customer_id);
    assert_eq!(json["amount_cents"], 10000);
    assert_eq!(json["status"], "draft");

    // Verify invoice was created in database
    let invoice_id = json["id"].as_i64().unwrap() as i32;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoices WHERE id = $1 AND ar_customer_id = $2"
    )
    .bind(invoice_id)
    .bind(customer_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "Invoice should exist in database");

    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 2: Create invoice with missing customer
#[tokio::test]
#[serial]
async fn test_create_invoice_missing_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let body = serde_json::json!({
        "ar_customer_id": 999999,
        "amount_cents": 10000,
        "currency": "usd"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/invoices")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return error for non-existent customer
    assert!(
        response.status() == StatusCode::NOT_FOUND ||
        response.status() == StatusCode::BAD_REQUEST ||
        response.status() == StatusCode::UNPROCESSABLE_ENTITY,
        "Should return error for non-existent customer, got {}",
        response.status()
    );

    common::teardown_pool(pool).await;
}

/// TEST 3: List invoices by customer
#[tokio::test]
#[serial]
async fn test_list_invoices_by_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Create two invoices for the customer
    let invoice1_id = seed_invoice(&pool, APP_ID, customer_id, 5000, "draft").await;
    let invoice2_id = seed_invoice(&pool, APP_ID, customer_id, 8000, "open").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/invoices?customer_id={}", customer_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    let invoices = json.as_array().expect("Response should be an array");
    assert_eq!(invoices.len(), 2, "Should return 2 invoices");

    // Cleanup
    sqlx::query("DELETE FROM ar_invoices WHERE id = ANY($1)")
        .bind(&[invoice1_id, invoice2_id])
        .execute(&pool)
        .await
        .unwrap();
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 4: Get single invoice
#[tokio::test]
#[serial]
async fn test_get_invoice_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let invoice_id = seed_invoice(&pool, APP_ID, customer_id, 12000, "draft").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/invoices/{}", invoice_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], invoice_id);
    assert_eq!(json["ar_customer_id"], customer_id);
    assert_eq!(json["amount_cents"], 12000);

    // Cleanup
    sqlx::query("DELETE FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .execute(&pool)
        .await
        .unwrap();
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 5: Get invoice - not found
#[tokio::test]
#[serial]
async fn test_get_invoice_not_found() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/invoices/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    common::teardown_pool(pool).await;
}

/// TEST 6: Update invoice
#[tokio::test]
#[serial]
async fn test_update_invoice_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let invoice_id = seed_invoice(&pool, APP_ID, customer_id, 10000, "draft").await;

    let body = serde_json::json!({
        "amount_cents": 15000,
        "metadata": {"description": "Updated invoice description"}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&format!("/api/ar/invoices/{}", invoice_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], invoice_id);
    assert_eq!(json["amount_cents"], 15000);

    // Cleanup
    sqlx::query("DELETE FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .execute(&pool)
        .await
        .unwrap();
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 7: Finalize invoice
#[tokio::test]
#[serial]
async fn test_finalize_invoice_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let invoice_id = seed_invoice(&pool, APP_ID, customer_id, 20000, "draft").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/invoices/{}/finalize", invoice_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&serde_json::json!({})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], invoice_id);
    assert_eq!(json["status"], "open");

    // Verify status changed in database
    let status: String = sqlx::query_scalar(
        "SELECT status FROM ar_invoices WHERE id = $1"
    )
    .bind(invoice_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "open");

    // Cleanup
    sqlx::query("DELETE FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .execute(&pool)
        .await
        .unwrap();
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 8: Finalize invoice - already finalized
#[tokio::test]
#[serial]
async fn test_finalize_invoice_already_finalized() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let invoice_id = seed_invoice(&pool, APP_ID, customer_id, 10000, "open").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/invoices/{}/finalize", invoice_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&serde_json::json!({})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return error since invoice is already finalized (open)
    assert!(
        response.status() == StatusCode::BAD_REQUEST ||
        response.status() == StatusCode::CONFLICT,
        "Should return error for already finalized invoice, got {}",
        response.status()
    );

    // Cleanup
    sqlx::query("DELETE FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .execute(&pool)
        .await
        .unwrap();
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Create a test invoice for testing.
/// Returns invoice_id.
async fn seed_invoice(
    pool: &sqlx::PgPool,
    app_id: &str,
    customer_id: i32,
    amount_cents: i32,
    status: &str,
) -> i32 {
    use uuid::Uuid;
    let tilled_invoice_id = format!("inv_{}", Uuid::new_v4());

    let invoice_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id,
            status, amount_cents, currency,
            created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5, 'usd',
            NOW(), NOW()
        )
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(&tilled_invoice_id)
    .bind(customer_id)
    .bind(status)
    .bind(amount_cents)
    .fetch_one(pool)
    .await
    .expect("Failed to seed test invoice");

    invoice_id
}
