//! Shopify webhook normalizer — HMAC verification, order parsing, file_job creation.
//!
//! ## Data flow
//! 1. **Verify** HMAC-SHA256 using `webhook_secret` from the connector config.
//! 2. **Parse** Shopify order JSON into a platform-standard [`OrderPayload`].
//! 3. **Persist** atomically in one transaction:
//!    - `integrations_webhook_ingest` record (idempotent via order_id + event_type key).
//!    - `integrations_file_jobs` record (`parser_type = "shopify_order"`).
//!    - `integrations.order.ingested` outbox event carrying the normalised order.
//!
//! ## HMAC verification
//! Shopify signs every webhook body with HMAC-SHA256 and sends the signature
//! base64-encoded in the `X-Shopify-Hmac-SHA256` header. The secret is the
//! `webhook_secret` field of the per-tenant connector config.
//!
//! ## Connector config schema (required keys)
//! - `shop_domain` — the Shopify shop domain.
//! - `webhook_secret` — the signing secret for HMAC verification.
//!
//! ## Idempotency
//! The idempotency key is `"shopify-{order_id}-{event_type}"`. Replayed webhooks
//! return `ShopifyNormalizeResult { is_duplicate: true }` without re-processing.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::file_jobs::models::STATUS_CREATED;
use crate::domain::file_jobs::repo as file_job_repo;
use crate::domain::webhooks::repo as webhook_repo;
use crate::events::{
    build_file_job_created_envelope, build_order_ingested_envelope, FileJobCreatedPayload,
    OrderIngestedPayload, OrderLineItemPayload, EVENT_TYPE_FILE_JOB_CREATED,
    EVENT_TYPE_ORDER_INGESTED,
};
use crate::outbox::enqueue_event_tx;

use super::models::WebhookError;

type HmacSha256 = Hmac<Sha256>;

// ============================================================================
// Public types
// ============================================================================

/// A platform-standard representation of a Shopify order extracted from a webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPayload {
    /// Shopify order ID (numeric string).
    pub order_id: String,
    /// Human-readable order number.
    pub order_number: Option<u64>,
    /// Financial status at ingestion time (e.g. `"paid"`, `"pending"`).
    pub financial_status: Option<String>,
    /// Normalised line items.
    pub line_items: Vec<LineItem>,
    /// Customer email, if present.
    pub customer_ref: Option<String>,
}

/// A single line item extracted from a Shopify order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineItem {
    pub product_id: String,
    pub variant_id: String,
    pub title: String,
    pub quantity: u32,
    pub price: String,
    pub sku: Option<String>,
}

/// Result of a successful normalisation run.
#[derive(Debug, Clone)]
pub struct ShopifyNormalizeResult {
    pub ingest_id: i64,
    pub file_job_id: Uuid,
    pub is_duplicate: bool,
}

// ============================================================================
// Normalizer
// ============================================================================

pub struct ShopifyNormalizer {
    pool: PgPool,
}

impl ShopifyNormalizer {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Process a single Shopify order webhook.
    ///
    /// # Arguments
    /// * `raw_body` — Raw request body bytes (used for HMAC verification).
    /// * `raw_payload` — Parsed JSON body.
    /// * `headers` — Lowercase HTTP headers map.
    /// * `app_id` — Tenant identifier.
    /// * `event_type` — Shopify webhook topic (e.g. `"orders/create"`).
    /// * `connector_config` — The tenant's Shopify connector config containing
    ///   `webhook_secret` used for HMAC verification.
    pub async fn normalize(
        &self,
        raw_body: &[u8],
        raw_payload: &serde_json::Value,
        headers: &std::collections::HashMap<String, String>,
        app_id: &str,
        event_type: &str,
        connector_config: &serde_json::Value,
    ) -> Result<ShopifyNormalizeResult, WebhookError> {
        // ── 1. HMAC verification (before any DB I/O) ─────────────────────────
        verify_shopify_hmac(raw_body, headers, connector_config)?;

        // ── 2. Parse order payload ────────────────────────────────────────────
        let order = parse_shopify_order(raw_payload)?;

        // ── 3. Idempotency key ────────────────────────────────────────────────
        let idempotency_key = format!("shopify-{}-{}", order.order_id, event_type);
        let headers_json = serde_json::to_value(headers)
            .map_err(|e| WebhookError::Serialization(e.to_string()))?;

        let mut tx = self.pool.begin().await?;

        // ── 4. Insert webhook ingest record (idempotent) ─────────────────────
        let ingest_result = webhook_repo::insert_ingest(
            &mut tx,
            app_id,
            "shopify",
            &Some(event_type.to_string()),
            raw_payload,
            &headers_json,
            Utc::now(),
            &Some(idempotency_key.clone()),
        )
        .await?;

        let ingest_id = match ingest_result {
            Some((id,)) => id,
            None => {
                // Duplicate — look up existing ingest id
                let existing = webhook_repo::lookup_existing_ingest(
                    &mut tx,
                    app_id,
                    "shopify",
                    &Some(idempotency_key),
                )
                .await?;
                tx.rollback().await?;
                let id = existing.map(|(id,)| id).unwrap_or(0);
                return Ok(ShopifyNormalizeResult {
                    ingest_id: id,
                    file_job_id: Uuid::nil(),
                    is_duplicate: true,
                });
            }
        };

        // ── 5. Idempotency check for file_job ────────────────────────────────
        let file_job_idem_key = format!("shopify-fj-{}-{}", order.order_id, event_type);
        let existing_job =
            file_job_repo::find_by_idempotency_key(&mut tx, app_id, &file_job_idem_key).await?;

        let file_job = if let Some(job) = existing_job {
            job
        } else {
            // ── 6. Create file_job record ─────────────────────────────────────
            let job_id = Uuid::new_v4();
            let job = file_job_repo::insert(
                &mut tx,
                job_id,
                app_id,
                &format!("shopify:order:{}", order.order_id),
                "shopify_order",
                STATUS_CREATED,
                &Some(file_job_idem_key),
            )
            .await?;

            // Emit file_job.created
            let fj_event_id = Uuid::new_v4();
            let correlation_id = Uuid::new_v4().to_string();
            let fj_envelope = build_file_job_created_envelope(
                fj_event_id,
                app_id.to_string(),
                correlation_id.clone(),
                None,
                FileJobCreatedPayload {
                    job_id: job.id,
                    tenant_id: app_id.to_string(),
                    file_ref: job.file_ref.clone(),
                    parser_type: job.parser_type.clone(),
                    status: job.status.clone(),
                    created_at: job.created_at,
                },
            );
            enqueue_event_tx(
                &mut tx,
                fj_event_id,
                EVENT_TYPE_FILE_JOB_CREATED,
                "file_job",
                &job.id.to_string(),
                app_id,
                &fj_envelope,
            )
            .await?;

            job
        };

        // ── 7. Emit integrations.order.ingested ───────────────────────────────
        let order_event_id = Uuid::new_v4();
        let order_correlation_id = Uuid::new_v4().to_string();
        let order_envelope = build_order_ingested_envelope(
            order_event_id,
            app_id.to_string(),
            order_correlation_id,
            None,
            OrderIngestedPayload {
                tenant_id: app_id.to_string(),
                source: "shopify".to_string(),
                order_id: order.order_id.clone(),
                order_number: order.order_number,
                financial_status: order.financial_status.clone(),
                line_items: order
                    .line_items
                    .iter()
                    .map(|li| OrderLineItemPayload {
                        product_id: li.product_id.clone(),
                        variant_id: li.variant_id.clone(),
                        title: li.title.clone(),
                        quantity: li.quantity,
                        price: li.price.clone(),
                        sku: li.sku.clone(),
                    })
                    .collect(),
                customer_ref: order.customer_ref.clone(),
                file_job_id: file_job.id,
                ingested_at: Utc::now(),
            },
        );
        enqueue_event_tx(
            &mut tx,
            order_event_id,
            EVENT_TYPE_ORDER_INGESTED,
            "order",
            &order.order_id,
            app_id,
            &order_envelope,
        )
        .await?;

        // ── 8. Mark ingest processed + commit ─────────────────────────────────
        webhook_repo::mark_ingest_processed(&mut tx, ingest_id, Utc::now()).await?;
        tx.commit().await?;

        Ok(ShopifyNormalizeResult {
            ingest_id,
            file_job_id: file_job.id,
            is_duplicate: false,
        })
    }
}

// ============================================================================
// HMAC verification
// ============================================================================

/// Verify the `X-Shopify-Hmac-SHA256` header.
///
/// Shopify sends HMAC-SHA256 of the raw body, base64-encoded, using the
/// `webhook_secret` from the connector config as the key.
pub fn verify_shopify_hmac(
    raw_body: &[u8],
    headers: &std::collections::HashMap<String, String>,
    connector_config: &serde_json::Value,
) -> Result<(), WebhookError> {
    let webhook_secret = connector_config
        .get("webhook_secret")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if webhook_secret.is_empty() {
        return Err(WebhookError::SignatureVerification(
            "webhook_secret is missing from connector config".to_string(),
        ));
    }

    let sig_b64 = headers.get("x-shopify-hmac-sha256").ok_or_else(|| {
        WebhookError::SignatureVerification("missing X-Shopify-Hmac-SHA256 header".to_string())
    })?;

    let provided = STANDARD.decode(sig_b64).map_err(|_| {
        WebhookError::SignatureVerification("invalid base64 in X-Shopify-Hmac-SHA256".to_string())
    })?;

    let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes()).map_err(|_| {
        WebhookError::SignatureVerification("invalid webhook_secret key".to_string())
    })?;
    mac.update(raw_body);
    let computed = mac.finalize().into_bytes();

    // Constant-time comparison
    if computed.len() != provided.len()
        || computed
            .iter()
            .zip(provided.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            != 0
    {
        return Err(WebhookError::SignatureVerification(
            "HMAC signature mismatch — payload may be tampered".to_string(),
        ));
    }

    Ok(())
}

// ============================================================================
// Order payload parsing
// ============================================================================

/// Parse a Shopify `orders/create` or `orders/updated` webhook body.
pub fn parse_shopify_order(payload: &serde_json::Value) -> Result<OrderPayload, WebhookError> {
    let order_id = payload
        .get("id")
        .and_then(|v| {
            // Shopify IDs are numbers in JSON
            v.as_u64()
                .map(|n| n.to_string())
                .or_else(|| v.as_str().map(str::to_string))
        })
        .ok_or_else(|| WebhookError::MalformedPayload("missing order id".to_string()))?;

    let order_number = payload.get("order_number").and_then(|v| v.as_u64());

    let financial_status = payload
        .get("financial_status")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let line_items = parse_line_items(payload)?;

    let customer_ref = payload
        .pointer("/customer/email")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok(OrderPayload {
        order_id,
        order_number,
        financial_status,
        line_items,
        customer_ref,
    })
}

fn parse_line_items(payload: &serde_json::Value) -> Result<Vec<LineItem>, WebhookError> {
    let arr = match payload.get("line_items").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Ok(vec![]),
    };

    arr.iter()
        .map(|item| {
            let product_id = item
                .get("product_id")
                .and_then(|v| {
                    v.as_u64()
                        .map(|n| n.to_string())
                        .or_else(|| v.as_str().map(str::to_string))
                })
                .unwrap_or_default();

            let variant_id = item
                .get("variant_id")
                .and_then(|v| {
                    v.as_u64()
                        .map(|n| n.to_string())
                        .or_else(|| v.as_str().map(str::to_string))
                })
                .unwrap_or_default();

            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let quantity = item.get("quantity").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

            let price = item
                .get("price")
                .and_then(|v| v.as_str())
                .unwrap_or("0.00")
                .to_string();

            let sku = item
                .get("sku")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);

            Ok(LineItem {
                product_id,
                variant_id,
                title,
                quantity,
                price,
                sku,
            })
        })
        .collect()
}

// ============================================================================
// Tests — helpers for generating valid HMAC signatures
// ============================================================================

#[cfg(test)]
pub fn shopify_hmac_b64(secret: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC init");
    mac.update(body);
    STANDARD.encode(mac.finalize().into_bytes())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;

    const TEST_APP: &str = "test-shopify-norm";
    const WEBHOOK_SECRET: &str = "test-webhook-secret-xyz";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        let pool = sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to integrations test database");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("Migrations failed");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM integrations_file_jobs WHERE tenant_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
    }

    fn connector_config() -> serde_json::Value {
        json!({
            "shop_domain": "test-store.myshopify.com",
            "api_key": "key123",
            "api_secret": "secret456",
            "webhook_secret": WEBHOOK_SECRET,
        })
    }

    fn sample_order_payload(order_id: u64) -> serde_json::Value {
        json!({
            "id": order_id,
            "order_number": 1001,
            "financial_status": "paid",
            "customer": {
                "email": "alice@example.com"
            },
            "line_items": [
                {
                    "product_id": 111222,
                    "variant_id": 333444,
                    "title": "Widget Pro",
                    "quantity": 2,
                    "price": "29.99",
                    "sku": "WGT-PRO-L"
                },
                {
                    "product_id": 555666,
                    "variant_id": 777888,
                    "title": "Gadget Plus",
                    "quantity": 1,
                    "price": "49.99",
                    "sku": ""
                }
            ]
        })
    }

    fn signed_headers(body: &[u8]) -> std::collections::HashMap<String, String> {
        let mut h = std::collections::HashMap::new();
        h.insert(
            "x-shopify-hmac-sha256".to_string(),
            shopify_hmac_b64(WEBHOOK_SECRET, body),
        );
        h
    }

    // ── Unit: HMAC verification ───────────────────────────────────────────────

    #[test]
    fn hmac_valid_signature_passes() {
        let body = b"{\"id\":1001}";
        let headers = signed_headers(body);
        assert!(verify_shopify_hmac(body, &headers, &connector_config()).is_ok());
    }

    #[test]
    fn hmac_tampered_payload_fails() {
        let body = b"{\"id\":1001}";
        let headers = signed_headers(body);
        // Tamper: different body
        let result = verify_shopify_hmac(b"{\"id\":9999}", &headers, &connector_config());
        assert!(matches!(
            result,
            Err(WebhookError::SignatureVerification(_))
        ));
    }

    #[test]
    fn hmac_missing_header_fails() {
        let result = verify_shopify_hmac(
            b"{}",
            &std::collections::HashMap::new(),
            &connector_config(),
        );
        assert!(matches!(
            result,
            Err(WebhookError::SignatureVerification(_))
        ));
    }

    #[test]
    fn hmac_missing_secret_in_config_fails() {
        let config = json!({});
        let mut headers = std::collections::HashMap::new();
        headers.insert("x-shopify-hmac-sha256".to_string(), "dGVzdA==".to_string());
        let result = verify_shopify_hmac(b"{}", &headers, &config);
        assert!(matches!(
            result,
            Err(WebhookError::SignatureVerification(_))
        ));
    }

    // ── Unit: order parsing ────────────────────────────────────────────────────

    #[test]
    fn parse_order_extracts_correct_fields() {
        let payload = sample_order_payload(5678901234);
        let order = parse_shopify_order(&payload).expect("parse failed");

        assert_eq!(order.order_id, "5678901234");
        assert_eq!(order.order_number, Some(1001));
        assert_eq!(order.financial_status.as_deref(), Some("paid"));
        assert_eq!(order.customer_ref.as_deref(), Some("alice@example.com"));
        assert_eq!(order.line_items.len(), 2);

        let first = &order.line_items[0];
        assert_eq!(first.title, "Widget Pro");
        assert_eq!(first.quantity, 2);
        assert_eq!(first.price, "29.99");
        assert_eq!(first.sku.as_deref(), Some("WGT-PRO-L"));

        // Empty SKU → None
        assert!(order.line_items[1].sku.is_none());
    }

    #[test]
    fn parse_order_missing_id_errors() {
        let payload = json!({ "order_number": 1 });
        assert!(matches!(
            parse_shopify_order(&payload),
            Err(WebhookError::MalformedPayload(_))
        ));
    }

    // ── Integration: end-to-end (real DB) ─────────────────────────────────────

    #[tokio::test]
    #[serial]
    async fn normalize_creates_file_job_and_emits_order_ingested() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let order_id = 9000000001u64;
        let payload = sample_order_payload(order_id);
        let body = serde_json::to_vec(&payload).expect("serialize test payload");
        let headers = signed_headers(&body);

        let normalizer = ShopifyNormalizer::new(pool.clone());
        let result = normalizer
            .normalize(
                &body,
                &payload,
                &headers,
                TEST_APP,
                "orders/create",
                &connector_config(),
            )
            .await
            .expect("normalize failed");

        assert!(!result.is_duplicate);
        assert!(result.ingest_id > 0);
        assert_ne!(result.file_job_id, Uuid::nil());

        // File job was created
        let job: Option<(String, String)> =
            sqlx::query_as("SELECT parser_type, status FROM integrations_file_jobs WHERE id = $1")
                .bind(result.file_job_id)
                .fetch_optional(&pool)
                .await
                .expect("file_job query failed");

        let (parser_type, status) = job.expect("file_job should exist");
        assert_eq!(parser_type, "shopify_order");
        assert_eq!(status, STATUS_CREATED);

        // integrations.order.ingested event in outbox
        let outbox_events: Vec<(String, String)> = sqlx::query_as(
            "SELECT event_type, aggregate_type FROM integrations_outbox
             WHERE app_id = $1 AND event_type = 'integrations.order.ingested'",
        )
        .bind(TEST_APP)
        .fetch_all(&pool)
        .await
        .expect("outbox query failed");

        assert_eq!(outbox_events.len(), 1, "expected one order.ingested event");
        assert_eq!(outbox_events[0].1, "order");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn normalize_idempotent_replay_returns_duplicate() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let order_id = 9000000002u64;
        let payload = sample_order_payload(order_id);
        let body = serde_json::to_vec(&payload).expect("serialize test payload");
        let headers = signed_headers(&body);

        let normalizer = ShopifyNormalizer::new(pool.clone());

        let r1 = normalizer
            .normalize(
                &body,
                &payload,
                &headers,
                TEST_APP,
                "orders/create",
                &connector_config(),
            )
            .await
            .expect("first normalize failed");
        assert!(!r1.is_duplicate);

        let r2 = normalizer
            .normalize(
                &body,
                &payload,
                &headers,
                TEST_APP,
                "orders/create",
                &connector_config(),
            )
            .await
            .expect("second normalize failed");
        assert!(r2.is_duplicate);

        // Only one ingest record
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM integrations_webhook_ingest
             WHERE app_id = $1 AND system = 'shopify'",
        )
        .bind(TEST_APP)
        .fetch_one(&pool)
        .await
        .expect("count query failed");
        assert_eq!(count.0, 1);

        cleanup(&pool).await;
    }

    // ── Edge case: orders/updated topic creates a separate ingest record ───────

    #[tokio::test]
    #[serial]
    async fn normalize_orders_updated_topic_creates_separate_ingest() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let order_id = 9000000003u64;
        let payload = sample_order_payload(order_id);
        let body = serde_json::to_vec(&payload).expect("serialize test payload");
        let headers = signed_headers(&body);

        let normalizer = ShopifyNormalizer::new(pool.clone());

        let r1 = normalizer
            .normalize(
                &body,
                &payload,
                &headers,
                TEST_APP,
                "orders/create",
                &connector_config(),
            )
            .await
            .expect("orders/create normalize failed");
        assert!(!r1.is_duplicate);

        // orders/updated has a different idempotency key — should NOT be a duplicate
        let r2 = normalizer
            .normalize(
                &body,
                &payload,
                &headers,
                TEST_APP,
                "orders/updated",
                &connector_config(),
            )
            .await
            .expect("orders/updated normalize failed");
        assert!(
            !r2.is_duplicate,
            "orders/updated should not be flagged as duplicate of orders/create"
        );
        assert_ne!(r2.file_job_id, Uuid::nil());

        cleanup(&pool).await;
    }

    // ── Edge case: invalid HMAC is rejected inside normalize() ────────────────

    #[tokio::test]
    async fn normalize_rejects_invalid_hmac_via_normalize() {
        let pool = test_pool().await;
        let order_id = 9999000001u64;
        let payload = sample_order_payload(order_id);
        let body = serde_json::to_vec(&payload).expect("serialize test payload");

        let mut bad_headers = std::collections::HashMap::new();
        bad_headers.insert(
            "x-shopify-hmac-sha256".to_string(),
            shopify_hmac_b64(WEBHOOK_SECRET, b"this is not the real body"),
        );

        let normalizer = ShopifyNormalizer::new(pool.clone());
        let result = normalizer
            .normalize(
                &body,
                &payload,
                &bad_headers,
                TEST_APP,
                "orders/create",
                &connector_config(),
            )
            .await;

        assert!(
            matches!(result, Err(WebhookError::SignatureVerification(_))),
            "invalid HMAC should return SignatureVerification error, got: {:?}",
            result
        );
    }

    // ── Edge case: order with empty line_items parses successfully ─────────────

    #[test]
    fn parse_order_empty_line_items_succeeds() {
        let payload = serde_json::json!({
            "id": 1234567890u64,
            "order_number": 42,
            "financial_status": "pending",
            "line_items": []
        });

        let order = parse_shopify_order(&payload).expect("should parse successfully");
        assert_eq!(order.order_id, "1234567890");
        assert!(
            order.line_items.is_empty(),
            "empty line_items should yield empty vec"
        );
    }
}
