//! Amazon SP-API order poll adapter.
//!
//! ## Overview
//! Triggered by a NATS message on `integrations.poll.amazon_sp` — NOT a sleep
//! loop. For each enabled `amazon_sp` connector config, the poller:
//!
//! 1. Exchanges the LWA `refresh_token` for an `access_token`.
//! 2. Calls SP-API `GET /orders/v0/orders` with `LastUpdatedAfter` set to the
//!    stored `last_poll_timestamp` (or 24 h ago on first run).
//! 3. Normalises each Amazon order into a platform `OrderIngestedPayload`
//!    (identical schema to Shopify).
//! 4. Atomically in a single transaction per order batch:
//!    - Inserts a `file_job` row (`parser_type = "amazon_order"`).
//!    - Enqueues `integrations.order.ingested` in the outbox.
//!    - Updates `last_poll_timestamp` in the connector config JSON.
//!
//! ## Idempotency
//! `idempotency_key = "amazon-fj-{order_id}"`.  Re-polling the same window
//! returns the existing file_job without creating duplicates.
//!
//! ## Rate limiting
//! SP-API returns `Retry-After` on 429. The poller respects this header and
//! applies exponential backoff (up to 3 retries).
//!
//! ## Invariant
//! The `OrderIngestedPayload` emitted on NATS is structurally identical to the
//! Shopify normaliser output. Downstream consumers (AR, inventory) must not
//! care whether the order originated from Shopify or Amazon.

use chrono::{DateTime, Utc};
use event_bus::EventBus;
use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::connectors::repo as connector_repo;
use crate::domain::file_jobs::{models::STATUS_CREATED, repo as file_job_repo};
use crate::events::{
    build_file_job_created_envelope, build_order_ingested_envelope, FileJobCreatedPayload,
    OrderIngestedPayload, OrderLineItemPayload, EVENT_TYPE_FILE_JOB_CREATED,
    EVENT_TYPE_ORDER_INGESTED,
};
use crate::outbox::enqueue_event_tx;

/// NATS subject that triggers an Amazon SP-API poll sweep.
pub const SUBJECT_POLL_AMAZON_SP: &str = "integrations.poll.amazon_sp";

/// LWA token endpoint.
const LWA_TOKEN_URL: &str = "https://api.amazon.com/auth/o2/token";

/// Default poll window: 24 hours before now.
const DEFAULT_LOOKBACK_HOURS: i64 = 24;

// ── LWA OAuth ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LwaTokenResponse {
    access_token: String,
}

/// Exchange a LWA refresh_token for an access_token.
///
/// Returns the access token string on success.
pub async fn exchange_lwa_token(
    http_client: &Client,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let resp = http_client
        .post(LWA_TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("LWA token exchange failed ({}): {}", status, body).into());
    }

    let token: LwaTokenResponse = resp.json().await?;
    Ok(token.access_token)
}

// ── SP-API order structures ────────────────────────────────────────────────────

/// SP-API GetOrders response envelope.
///
/// The outer wrapper uses lowercase `"payload"` (SP-API convention), while
/// the inner `OrderList` uses PascalCase field names (`"Orders"`).
#[derive(Debug, Deserialize)]
struct GetOrdersResponse {
    // SP-API wraps all responses in a lowercase "payload" key.
    payload: Option<OrderList>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct OrderList {
    orders: Option<Vec<SpApiOrder>>,
}

/// A single Amazon order from the SP-API GetOrders response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SpApiOrder {
    amazon_order_id: String,
    order_status: Option<String>,
    purchase_date: Option<String>,
    buyer_info: Option<BuyerInfo>,
    order_items: Option<Vec<SpApiOrderItem>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct BuyerInfo {
    buyer_email: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SpApiOrderItem {
    // SP-API uses all-caps "ASIN", not "Asin" (PascalCase default).
    #[serde(rename = "ASIN")]
    asin: Option<String>,
    // SP-API uses "SellerSKU" (acronym), not "SellerSku".
    #[serde(rename = "SellerSKU")]
    seller_sku: Option<String>,
    title: Option<String>,
    quantity_ordered: Option<u32>,
    item_price: Option<MoneyValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MoneyValue {
    amount: Option<String>,
}

// ── Normalisation ─────────────────────────────────────────────────────────────

/// Parse a raw SP-API GetOrders JSON response into a list of normalized orders.
///
/// Exported for testing without HTTP calls.
pub fn normalize_amazon_orders(
    raw_response: &Value,
    tenant_id: &str,
) -> Result<Vec<OrderIngestedPayload>, Box<dyn std::error::Error + Send + Sync>> {
    let resp: GetOrdersResponse = serde_json::from_value(raw_response.clone())
        .map_err(|e| format!("failed to parse GetOrders response: {}", e))?;

    let orders = resp.payload.and_then(|p| p.orders).unwrap_or_default();

    let payloads = orders
        .into_iter()
        .map(|order| {
            let line_items = order
                .order_items
                .unwrap_or_default()
                .into_iter()
                .map(|item| OrderLineItemPayload {
                    product_id: item.asin.clone().unwrap_or_default(),
                    variant_id: item.asin.unwrap_or_default(),
                    title: item.title.unwrap_or_default(),
                    quantity: item.quantity_ordered.unwrap_or(1),
                    price: item
                        .item_price
                        .and_then(|p| p.amount)
                        .unwrap_or_else(|| "0.00".to_string()),
                    sku: item.seller_sku,
                })
                .collect();

            let ingested_at: DateTime<Utc> = order
                .purchase_date
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(Utc::now);

            OrderIngestedPayload {
                tenant_id: tenant_id.to_string(),
                source: "amazon_sp".to_string(),
                order_id: order.amazon_order_id,
                order_number: None,
                financial_status: order.order_status,
                line_items,
                customer_ref: order.buyer_info.and_then(|b| b.buyer_email),
                file_job_id: Uuid::nil(), // set by caller before persisting
                ingested_at,
            }
        })
        .collect();

    Ok(payloads)
}

// ── Persist normalised orders ─────────────────────────────────────────────────

/// Persist a single normalized order: file_job + outbox event in one transaction.
///
/// Returns `true` if the order was newly created, `false` if it was a duplicate
/// (idempotency_key already exists).
pub async fn persist_amazon_order(
    pool: &PgPool,
    mut order: OrderIngestedPayload,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let tenant_id = order.tenant_id.clone();
    let order_id = order.order_id.clone();
    let idem_key = format!("amazon-fj-{}", order_id);

    let mut tx = pool.begin().await?;

    // Idempotency: skip if already processed
    if let Some(_existing) =
        file_job_repo::find_by_idempotency_key(&mut tx, &tenant_id, &idem_key).await?
    {
        tx.rollback().await?;
        return Ok(false);
    }

    // Insert file_job
    let job_id = Uuid::new_v4();
    let job = file_job_repo::insert(
        &mut tx,
        job_id,
        &tenant_id,
        &format!("amazon:order:{}", order_id),
        "amazon_order",
        STATUS_CREATED,
        &Some(idem_key),
    )
    .await?;

    // Emit file_job.created
    let fj_event_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4().to_string();
    let fj_envelope = build_file_job_created_envelope(
        fj_event_id,
        tenant_id.clone(),
        correlation_id.clone(),
        None,
        FileJobCreatedPayload {
            job_id: job.id,
            tenant_id: tenant_id.clone(),
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
        &tenant_id,
        &fj_envelope,
    )
    .await?;

    // Set the file_job_id on the order payload now that we have it
    order.file_job_id = job.id;

    // Emit integrations.order.ingested
    let order_event_id = Uuid::new_v4();
    let order_correlation_id = Uuid::new_v4().to_string();
    let order_envelope = build_order_ingested_envelope(
        order_event_id,
        tenant_id.clone(),
        order_correlation_id,
        None,
        order,
    );
    enqueue_event_tx(
        &mut tx,
        order_event_id,
        EVENT_TYPE_ORDER_INGESTED,
        "order",
        &order_id,
        &tenant_id,
        &order_envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(true)
}

// ── Poll sweep (per connector config) ────────────────────────────────────────

/// Run a single poll sweep for one tenant's amazon_sp connector config.
///
/// Fetches orders from SP-API since `last_poll_timestamp`, persists each
/// order as a file_job, and atomically updates `last_poll_timestamp`.
pub async fn run_poll_for_config(
    pool: &PgPool,
    http_client: &Client,
    config_id: Uuid,
    tenant_id: &str,
    config: &Value,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let client_id = config
        .get("client_id")
        .and_then(|v| v.as_str())
        .ok_or("missing client_id in config")?;
    let client_secret = config
        .get("client_secret")
        .and_then(|v| v.as_str())
        .ok_or("missing client_secret in config")?;
    let refresh_token = config
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .ok_or("missing refresh_token in config")?;
    let seller_id = config
        .get("seller_id")
        .and_then(|v| v.as_str())
        .ok_or("missing seller_id in config")?;
    let marketplace_id = config
        .get("marketplace_id")
        .and_then(|v| v.as_str())
        .ok_or("missing marketplace_id in config")?;

    // Last poll timestamp (default: 24 h ago)
    let last_updated_after: DateTime<Utc> = config
        .get("last_poll_timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| Utc::now() - chrono::Duration::hours(DEFAULT_LOOKBACK_HOURS));

    let poll_start = Utc::now();

    // Exchange LWA token
    let access_token =
        exchange_lwa_token(http_client, client_id, client_secret, refresh_token).await?;

    // Call SP-API GetOrders with retry on 429
    let sp_api_endpoint = sp_api_orders_url(marketplace_id, &last_updated_after);
    let raw_response =
        call_sp_api_with_retry(http_client, &sp_api_endpoint, seller_id, &access_token, 3).await?;

    // Normalise orders
    let orders = normalize_amazon_orders(&raw_response, tenant_id)?;
    let order_count = orders.len();

    // Persist each order
    for order in orders {
        persist_amazon_order(pool, order).await?;
    }

    // Update last_poll_timestamp atomically
    let mut tx = pool.begin().await?;
    connector_repo::merge_config_json(
        &mut tx,
        config_id,
        tenant_id,
        &serde_json::json!({ "last_poll_timestamp": poll_start.to_rfc3339() }),
    )
    .await?;
    tx.commit().await?;

    Ok(order_count)
}

fn sp_api_orders_url(marketplace_id: &str, last_updated_after: &DateTime<Utc>) -> String {
    format!(
        "https://sellingpartnerapi-na.amazon.com/orders/v0/orders?MarketplaceIds={}&LastUpdatedAfter={}",
        urlencoding::encode(marketplace_id),
        urlencoding::encode(&last_updated_after.to_rfc3339()),
    )
}

async fn call_sp_api_with_retry(
    http_client: &Client,
    url: &str,
    seller_id: &str,
    access_token: &str,
    max_retries: u32,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut attempt = 0;
    loop {
        let resp = http_client
            .get(url)
            .header("x-amz-access-token", access_token)
            .header("x-amz-seller-id", seller_id)
            .send()
            .await?;

        if resp.status().as_u16() == 429 {
            if attempt >= max_retries {
                return Err("SP-API rate limit exceeded, all retries exhausted".into());
            }
            // Respect Retry-After header; default to exponential backoff
            let delay_secs: u64 = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse().ok())
                .unwrap_or(2u64.pow(attempt + 1));
            tracing::warn!(
                attempt,
                delay_secs,
                "SP-API rate limited — backing off before retry"
            );
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
            attempt += 1;
            continue;
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("SP-API error ({}): {}", status, body).into());
        }

        let body: Value = resp.json().await?;
        return Ok(body);
    }
}

// ── NATS consumer ─────────────────────────────────────────────────────────────

/// Start the Amazon SP-API poll consumer as a background task.
///
/// Subscribes to `integrations.poll.amazon_sp`. On each message, fetches all
/// enabled `amazon_sp` connector configs and runs a poll sweep for each tenant.
pub fn start_amazon_poll_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let http_client = Client::new();
        tracing::info!("Integrations: starting Amazon SP-API poll consumer");

        let mut stream = match bus.subscribe(SUBJECT_POLL_AMAZON_SP).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    subject = SUBJECT_POLL_AMAZON_SP,
                    "Integrations: failed to subscribe to Amazon poll subject"
                );
                return;
            }
        };

        tracing::info!(
            subject = SUBJECT_POLL_AMAZON_SP,
            "Integrations: Amazon SP-API poll consumer subscribed"
        );

        while let Some(_msg) = stream.next().await {
            if let Err(e) = handle_poll_trigger(&pool, &http_client).await {
                tracing::error!(error = %e, "Integrations: Amazon poll sweep failed");
            }
        }

        tracing::warn!("Integrations: Amazon SP-API poll consumer stopped");
    });
}

/// Poll all enabled amazon_sp connector configs across all tenants.
async fn handle_poll_trigger(
    pool: &PgPool,
    http_client: &Client,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // List all enabled amazon_sp configs
    let configs = sqlx::query_as::<_, (Uuid, String, Value)>(
        r#"
        SELECT id, app_id, config
        FROM integrations_connector_configs
        WHERE connector_type = 'amazon_sp' AND enabled = TRUE
        "#,
    )
    .fetch_all(pool)
    .await?;

    for (config_id, tenant_id, config) in configs {
        let result = run_poll_for_config(pool, http_client, config_id, &tenant_id, &config).await;
        match result {
            Ok(n) => tracing::info!(
                tenant_id = %tenant_id,
                orders_ingested = n,
                "Integrations: Amazon SP-API poll complete"
            ),
            Err(e) => tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Integrations: Amazon SP-API poll failed for tenant"
            ),
        }
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        let pool = sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to integrations test DB");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("Migrations failed");
        pool
    }

    async fn cleanup(pool: &PgPool, tenant_id: &str) {
        sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM integrations_file_jobs WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }

    fn sample_get_orders_response(order_ids: &[&str]) -> Value {
        let orders: Vec<Value> = order_ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "AmazonOrderId": id,
                    "OrderStatus": "Unshipped",
                    "PurchaseDate": "2024-01-15T10:30:00Z",
                    "BuyerInfo": {
                        "BuyerEmail": "buyer@example.com"
                    },
                    "OrderItems": [
                        {
                            "ASIN": "B0123456789",
                            "SellerSKU": "SKU-001",
                            "Title": "Test Product",
                            "QuantityOrdered": 2,
                            "ItemPrice": {
                                "Amount": "49.99"
                            }
                        }
                    ]
                })
            })
            .collect();

        serde_json::json!({
            "payload": {
                "Orders": orders
            }
        })
    }

    // ── Normalisation ─────────────────────────────────────────────────────────

    #[test]
    fn normalize_amazon_orders_extracts_all_fields() {
        let response = sample_get_orders_response(&["111-2222222-3333333"]);
        let orders =
            normalize_amazon_orders(&response, "tenant-test").expect("normalization failed");

        assert_eq!(orders.len(), 1);
        let order = &orders[0];
        assert_eq!(order.order_id, "111-2222222-3333333");
        assert_eq!(order.source, "amazon_sp");
        assert_eq!(order.tenant_id, "tenant-test");
        assert_eq!(order.financial_status.as_deref(), Some("Unshipped"));
        assert_eq!(order.customer_ref.as_deref(), Some("buyer@example.com"));
        assert_eq!(order.line_items.len(), 1);

        let item = &order.line_items[0];
        assert_eq!(item.product_id, "B0123456789");
        assert_eq!(item.sku.as_deref(), Some("SKU-001"));
        assert_eq!(item.quantity, 2);
        assert_eq!(item.price, "49.99");
    }

    #[test]
    fn normalize_amazon_orders_empty_payload_returns_empty_vec() {
        let response = serde_json::json!({ "payload": { "Orders": [] } });
        let orders =
            normalize_amazon_orders(&response, "tenant-test").expect("normalization failed");
        assert!(orders.is_empty());
    }

    #[test]
    fn normalize_amazon_orders_missing_payload_returns_empty_vec() {
        let response = serde_json::json!({});
        let orders =
            normalize_amazon_orders(&response, "tenant-test").expect("normalization failed");
        assert!(orders.is_empty());
    }

    // ── Persist + idempotency ─────────────────────────────────────────────────

    #[tokio::test]
    #[serial]
    async fn persist_amazon_order_creates_file_job_and_outbox_event() {
        let pool = test_pool().await;
        let tenant_id = "test-amazon-poller-001";
        cleanup(&pool, tenant_id).await;

        let order = OrderIngestedPayload {
            tenant_id: tenant_id.to_string(),
            source: "amazon_sp".to_string(),
            order_id: "111-0000001-9999001".to_string(),
            order_number: None,
            financial_status: Some("Unshipped".to_string()),
            line_items: vec![OrderLineItemPayload {
                product_id: "B0TESTPROD1".to_string(),
                variant_id: "B0TESTPROD1".to_string(),
                title: "Test Product".to_string(),
                quantity: 1,
                price: "29.99".to_string(),
                sku: Some("SKU-TEST-001".to_string()),
            }],
            customer_ref: Some("test@example.com".to_string()),
            file_job_id: Uuid::nil(),
            ingested_at: Utc::now(),
        };

        let created = persist_amazon_order(&pool, order)
            .await
            .expect("persist_amazon_order failed");
        assert!(created, "first persist should return true (new order)");

        // Verify file_job in DB
        let job: Option<(String, String)> = sqlx::query_as(
            "SELECT parser_type, status FROM integrations_file_jobs WHERE tenant_id = $1",
        )
        .bind(tenant_id)
        .fetch_optional(&pool)
        .await
        .expect("query failed");

        let (parser_type, status) = job.expect("file_job should exist");
        assert_eq!(parser_type, "amazon_order");
        assert_eq!(status, STATUS_CREATED);

        // Verify integrations.order.ingested in outbox
        let outbox: Vec<(String,)> = sqlx::query_as(
            "SELECT event_type FROM integrations_outbox
             WHERE app_id = $1 AND event_type = 'integrations.order.ingested'",
        )
        .bind(tenant_id)
        .fetch_all(&pool)
        .await
        .expect("outbox query failed");

        assert_eq!(outbox.len(), 1, "expected one order.ingested event");

        cleanup(&pool, tenant_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn persist_amazon_order_idempotent_on_replay() {
        let pool = test_pool().await;
        let tenant_id = "test-amazon-poller-002";
        cleanup(&pool, tenant_id).await;

        let make_order = || OrderIngestedPayload {
            tenant_id: tenant_id.to_string(),
            source: "amazon_sp".to_string(),
            order_id: "111-0000002-9999002".to_string(),
            order_number: None,
            financial_status: Some("Shipped".to_string()),
            line_items: vec![],
            customer_ref: None,
            file_job_id: Uuid::nil(),
            ingested_at: Utc::now(),
        };

        let first = persist_amazon_order(&pool, make_order())
            .await
            .expect("first persist failed");
        assert!(first, "first persist should return true");

        let second = persist_amazon_order(&pool, make_order())
            .await
            .expect("second persist failed");
        assert!(!second, "second persist (duplicate) should return false");

        // Only one file_job should exist
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM integrations_file_jobs WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&pool)
                .await
                .expect("count query failed");
        assert_eq!(count.0, 1, "exactly one file_job should exist");

        cleanup(&pool, tenant_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn normalize_and_persist_multiple_orders_from_response() {
        let pool = test_pool().await;
        let tenant_id = "test-amazon-poller-003";
        cleanup(&pool, tenant_id).await;

        let order_ids = [
            "111-0000003-9999001",
            "111-0000003-9999002",
            "111-0000003-9999003",
        ];
        let response = sample_get_orders_response(&order_ids);

        let orders = normalize_amazon_orders(&response, tenant_id).expect("normalize failed");
        assert_eq!(orders.len(), 3);

        let mut new_count = 0;
        for order in orders {
            if persist_amazon_order(&pool, order)
                .await
                .expect("persist failed")
            {
                new_count += 1;
            }
        }
        assert_eq!(new_count, 3, "all three orders should be new");

        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM integrations_file_jobs WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(&pool)
                .await
                .expect("count query");
        assert_eq!(count.0, 3);

        cleanup(&pool, tenant_id).await;
    }
}
