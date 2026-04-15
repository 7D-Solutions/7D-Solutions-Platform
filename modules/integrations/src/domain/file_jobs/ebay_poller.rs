//! eBay Fulfillment API order poll adapter.
//!
//! ## Overview
//! Triggered by a NATS message on `integrations.poll.ebay` — NOT a sleep loop.
//! For each enabled `ebay` connector config, the poller:
//!
//! 1. Exchanges `client_id` + `client_secret` for an OAuth2 `access_token`
//!    (client-credentials grant).
//! 2. Calls eBay Fulfillment API `GET /sell/fulfillment/v1/order` with
//!    `filter=lastmodifieddate:[{last_poll_timestamp}...]` and cursor-based
//!    pagination via the `next` field in the response.
//! 3. Normalises each eBay order into a platform `OrderIngestedPayload`
//!    (identical schema to Shopify / Amazon).
//! 4. Atomically in a single transaction per order:
//!    - Inserts a `file_job` row (`parser_type = "ebay_order"`).
//!    - Enqueues `integrations.order.ingested` in the outbox.
//!    - Updates `last_poll_timestamp` in the connector config JSON.
//!
//! ## Idempotency
//! `idempotency_key = "ebay-fj-{order_id}"`.  Re-polling the same window
//! returns the existing file_job without creating duplicates.
//!
//! ## Invariant
//! The `OrderIngestedPayload` emitted on NATS is structurally identical to the
//! Shopify and Amazon normaliser output. Downstream consumers (AR, inventory)
//! must not care which marketplace the order originated from.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
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

/// NATS subject that triggers an eBay poll sweep.
pub const SUBJECT_POLL_EBAY: &str = "integrations.poll.ebay";

/// Default poll window: 24 hours before now.
const DEFAULT_LOOKBACK_HOURS: i64 = 24;

/// eBay OAuth2 token endpoint — sandbox.
const EBAY_TOKEN_URL_SANDBOX: &str = "https://api.sandbox.ebay.com/identity/v1/oauth2/token";

/// eBay OAuth2 token endpoint — production.
const EBAY_TOKEN_URL_PRODUCTION: &str = "https://api.ebay.com/identity/v1/oauth2/token";

/// eBay Fulfillment API orders endpoint — sandbox.
const EBAY_ORDERS_URL_SANDBOX: &str = "https://api.sandbox.ebay.com/sell/fulfillment/v1/order";

/// eBay Fulfillment API orders endpoint — production.
const EBAY_ORDERS_URL_PRODUCTION: &str = "https://api.ebay.com/sell/fulfillment/v1/order";

// ── OAuth2 token exchange ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct EbayTokenResponse {
    access_token: String,
}

/// Exchange client-credentials for an eBay OAuth2 access token.
///
/// Uses HTTP Basic auth with `client_id:client_secret` encoded in base64,
/// as required by the eBay OAuth2 specification.
pub async fn exchange_ebay_token(
    http_client: &Client,
    client_id: &str,
    client_secret: &str,
    environment: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let token_url = if environment.eq_ignore_ascii_case("SANDBOX") {
        EBAY_TOKEN_URL_SANDBOX
    } else {
        EBAY_TOKEN_URL_PRODUCTION
    };

    let credentials = BASE64.encode(format!("{}:{}", client_id, client_secret));
    let scope = "https://api.ebay.com/oauth/api_scope/sell.fulfillment.readonly";

    let resp = http_client
        .post(token_url)
        .header("Authorization", format!("Basic {}", credentials))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[("grant_type", "client_credentials"), ("scope", scope)])
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("eBay token exchange failed ({}): {}", status, body).into());
    }

    let token: EbayTokenResponse = resp.json().await?;
    Ok(token.access_token)
}

// ── eBay API order structures ─────────────────────────────────────────────────

/// eBay Fulfillment API GetOrders response envelope.
#[derive(Debug, Deserialize)]
struct EbayOrdersResponse {
    orders: Option<Vec<EbayOrder>>,
    /// Cursor token for the next page; absent when on the last page.
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EbayOrder {
    order_id: String,
    order_fulfillment_status: Option<String>,
    creation_date: Option<String>,
    buyer: Option<EbayBuyer>,
    line_items: Option<Vec<EbayLineItem>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EbayBuyer {
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EbayLineItem {
    legacy_item_id: Option<String>,
    #[serde(rename = "legacyVariationId")]
    legacy_variation_id: Option<String>,
    title: Option<String>,
    sku: Option<String>,
    quantity: Option<u32>,
    line_item_cost: Option<EbayMoney>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EbayMoney {
    value: Option<String>,
}

// ── Normalisation ─────────────────────────────────────────────────────────────

/// Parse a raw eBay GetOrders JSON response into a list of normalized orders.
///
/// Exported for testing without HTTP calls.
pub fn normalize_ebay_orders(
    raw_response: &Value,
    tenant_id: &str,
) -> Result<Vec<OrderIngestedPayload>, Box<dyn std::error::Error + Send + Sync>> {
    let resp: EbayOrdersResponse = serde_json::from_value(raw_response.clone())
        .map_err(|e| format!("failed to parse eBay GetOrders response: {}", e))?;

    let orders = resp.orders.unwrap_or_default();

    let payloads = orders
        .into_iter()
        .map(|order| {
            let line_items = order
                .line_items
                .unwrap_or_default()
                .into_iter()
                .map(|item| {
                    let product_id = item.legacy_item_id.clone().unwrap_or_default();
                    let variant_id = item
                        .legacy_variation_id
                        .clone()
                        .unwrap_or_else(|| product_id.clone());
                    OrderLineItemPayload {
                        product_id,
                        variant_id,
                        title: item.title.unwrap_or_default(),
                        quantity: item.quantity.unwrap_or(1),
                        price: item
                            .line_item_cost
                            .and_then(|m| m.value)
                            .unwrap_or_else(|| "0.00".to_string()),
                        sku: item.sku,
                    }
                })
                .collect();

            let ingested_at: DateTime<Utc> = order
                .creation_date
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(Utc::now);

            OrderIngestedPayload {
                tenant_id: tenant_id.to_string(),
                source: "ebay".to_string(),
                order_id: order.order_id,
                order_number: None,
                financial_status: order.order_fulfillment_status,
                line_items,
                customer_ref: order.buyer.and_then(|b| b.username),
                file_job_id: Uuid::nil(), // set by caller before persisting
                ingested_at,
            }
        })
        .collect();

    Ok(payloads)
}

/// Extract the cursor for the next page from a raw response, if present.
pub fn next_page_cursor(raw_response: &Value) -> Option<String> {
    raw_response
        .get("next")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

// ── Persist normalised orders ─────────────────────────────────────────────────

/// Persist a single normalized eBay order: file_job + outbox event in one transaction.
///
/// Returns `true` if the order was newly created, `false` if it was a duplicate.
pub async fn persist_ebay_order(
    pool: &PgPool,
    mut order: OrderIngestedPayload,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let tenant_id = order.tenant_id.clone();
    let order_id = order.order_id.clone();
    let idem_key = format!("ebay-fj-{}", order_id);

    let mut tx = pool.begin().await?;

    // Idempotency: skip if already processed
    if file_job_repo::find_by_idempotency_key(&mut tx, &tenant_id, &idem_key)
        .await?
        .is_some()
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
        &format!("ebay:order:{}", order_id),
        "ebay_order",
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

/// Run a single poll sweep for one tenant's eBay connector config.
///
/// Fetches orders from the eBay Fulfillment API modified since
/// `last_poll_timestamp`, persists each order as a file_job, and
/// atomically updates `last_poll_timestamp`.
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
    let environment = config
        .get("environment")
        .and_then(|v| v.as_str())
        .unwrap_or("SANDBOX");

    let last_modified_after: DateTime<Utc> = config
        .get("last_poll_timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| Utc::now() - chrono::Duration::hours(DEFAULT_LOOKBACK_HOURS));

    let poll_start = Utc::now();

    let access_token =
        exchange_ebay_token(http_client, client_id, client_secret, environment).await?;

    let orders_base_url = if environment.eq_ignore_ascii_case("SANDBOX") {
        EBAY_ORDERS_URL_SANDBOX
    } else {
        EBAY_ORDERS_URL_PRODUCTION
    };

    let filter = format!("lastmodifieddate:[{}...]", last_modified_after.to_rfc3339());

    let mut total_ingested = 0;
    let mut cursor: Option<String> = None;

    loop {
        let raw_response = fetch_orders_page(
            http_client,
            orders_base_url,
            &access_token,
            &filter,
            cursor.as_deref(),
        )
        .await?;

        let orders = normalize_ebay_orders(&raw_response, tenant_id)?;

        for order in orders {
            if persist_ebay_order(pool, order).await? {
                total_ingested += 1;
            }
        }

        cursor = next_page_cursor(&raw_response);
        if cursor.is_none() {
            break;
        }
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

    Ok(total_ingested)
}

async fn fetch_orders_page(
    http_client: &Client,
    base_url: &str,
    access_token: &str,
    filter: &str,
    cursor: Option<&str>,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let mut request = http_client
        .get(base_url)
        .bearer_auth(access_token)
        .query(&[("filter", filter)]);

    if let Some(c) = cursor {
        request = request.query(&[("offset", c)]);
    }

    let resp = request.send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("eBay Fulfillment API error ({}): {}", status, body).into());
    }

    let body: Value = resp.json().await?;
    Ok(body)
}

// ── NATS consumer ─────────────────────────────────────────────────────────────

/// Start the eBay poll consumer as a background task.
///
/// Subscribes to `integrations.poll.ebay`. On each message, fetches all
/// enabled `ebay` connector configs and runs a poll sweep for each tenant.
pub fn start_ebay_poll_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let http_client = Client::new();
        tracing::info!("Integrations: starting eBay poll consumer");

        let mut stream = match bus.subscribe(SUBJECT_POLL_EBAY).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    subject = SUBJECT_POLL_EBAY,
                    "Integrations: failed to subscribe to eBay poll subject"
                );
                return;
            }
        };

        tracing::info!(
            subject = SUBJECT_POLL_EBAY,
            "Integrations: eBay poll consumer subscribed"
        );

        while let Some(_msg) = stream.next().await {
            if let Err(e) = handle_poll_trigger(&pool, &http_client).await {
                tracing::error!(error = %e, "Integrations: eBay poll sweep failed");
            }
        }

        tracing::warn!("Integrations: eBay poll consumer stopped");
    });
}

/// Poll all enabled eBay connector configs across all tenants.
async fn handle_poll_trigger(
    pool: &PgPool,
    http_client: &Client,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let configs = sqlx::query_as::<_, (Uuid, String, Value)>(
        r#"
        SELECT id, app_id, config
        FROM integrations_connector_configs
        WHERE connector_type = 'ebay' AND enabled = TRUE
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
                "Integrations: eBay poll complete"
            ),
            Err(e) => tracing::error!(
                tenant_id = %tenant_id,
                error = %e,
                "Integrations: eBay poll failed for tenant"
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

    fn sample_orders_response(order_ids: &[&str]) -> Value {
        let orders: Vec<Value> = order_ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "orderId": id,
                    "orderFulfillmentStatus": "NOT_STARTED",
                    "creationDate": "2024-03-10T14:00:00.000Z",
                    "buyer": {
                        "username": "buyer_test_001"
                    },
                    "lineItems": [
                        {
                            "legacyItemId": "123456789012",
                            "legacyVariationId": "987654321",
                            "title": "Test Widget",
                            "sku": "TW-001",
                            "quantity": 2,
                            "lineItemCost": {
                                "value": "39.99",
                                "currency": "USD"
                            }
                        }
                    ]
                })
            })
            .collect();

        serde_json::json!({ "orders": orders })
    }

    // ── Normalisation ─────────────────────────────────────────────────────────

    #[test]
    fn normalize_ebay_orders_extracts_all_fields() {
        let response = sample_orders_response(&["12-34567-89012"]);
        let orders =
            normalize_ebay_orders(&response, "tenant-ebay-test").expect("normalization failed");

        assert_eq!(orders.len(), 1);
        let order = &orders[0];
        assert_eq!(order.order_id, "12-34567-89012");
        assert_eq!(order.source, "ebay");
        assert_eq!(order.tenant_id, "tenant-ebay-test");
        assert_eq!(order.financial_status.as_deref(), Some("NOT_STARTED"));
        assert_eq!(order.customer_ref.as_deref(), Some("buyer_test_001"));
        assert_eq!(order.line_items.len(), 1);

        let item = &order.line_items[0];
        assert_eq!(item.product_id, "123456789012");
        assert_eq!(item.variant_id, "987654321");
        assert_eq!(item.title, "Test Widget");
        assert_eq!(item.sku.as_deref(), Some("TW-001"));
        assert_eq!(item.quantity, 2);
        assert_eq!(item.price, "39.99");
    }

    #[test]
    fn normalize_ebay_orders_empty_response_returns_empty_vec() {
        let response = serde_json::json!({ "orders": [] });
        let orders =
            normalize_ebay_orders(&response, "tenant-ebay-test").expect("normalization failed");
        assert!(orders.is_empty());
    }

    #[test]
    fn normalize_ebay_orders_missing_orders_field_returns_empty_vec() {
        let response = serde_json::json!({});
        let orders =
            normalize_ebay_orders(&response, "tenant-ebay-test").expect("normalization failed");
        assert!(orders.is_empty());
    }

    #[test]
    fn normalize_ebay_orders_missing_variation_id_falls_back_to_item_id() {
        let response = serde_json::json!({
            "orders": [{
                "orderId": "99-11111-22222",
                "lineItems": [{
                    "legacyItemId": "111222333444",
                    "title": "No Variation Item",
                    "quantity": 1,
                    "lineItemCost": { "value": "9.99" }
                }]
            }]
        });
        let orders = normalize_ebay_orders(&response, "t").expect("normalization failed");
        assert_eq!(orders.len(), 1);
        let item = &orders[0].line_items[0];
        assert_eq!(item.product_id, "111222333444");
        assert_eq!(
            item.variant_id, "111222333444",
            "variant_id should fall back to product_id"
        );
    }

    #[test]
    fn next_page_cursor_present_returns_some() {
        let response = serde_json::json!({ "orders": [], "next": "cursor_abc123" });
        assert_eq!(
            next_page_cursor(&response),
            Some("cursor_abc123".to_string())
        );
    }

    #[test]
    fn next_page_cursor_absent_returns_none() {
        let response = serde_json::json!({ "orders": [] });
        assert!(next_page_cursor(&response).is_none());
    }

    #[test]
    fn next_page_cursor_empty_string_returns_none() {
        let response = serde_json::json!({ "orders": [], "next": "" });
        assert!(next_page_cursor(&response).is_none());
    }

    // ── Persist + idempotency ─────────────────────────────────────────────────

    #[tokio::test]
    #[serial]
    async fn persist_ebay_order_creates_file_job_and_outbox_event() {
        let pool = test_pool().await;
        let tenant_id = "test-ebay-poller-001";
        cleanup(&pool, tenant_id).await;

        let order = OrderIngestedPayload {
            tenant_id: tenant_id.to_string(),
            source: "ebay".to_string(),
            order_id: "12-00000001-99990001".to_string(),
            order_number: None,
            financial_status: Some("NOT_STARTED".to_string()),
            line_items: vec![OrderLineItemPayload {
                product_id: "100000000001".to_string(),
                variant_id: "200000000001".to_string(),
                title: "Test eBay Item".to_string(),
                quantity: 1,
                price: "49.99".to_string(),
                sku: Some("EBAY-SKU-001".to_string()),
            }],
            customer_ref: Some("ebay_buyer_001".to_string()),
            file_job_id: Uuid::nil(),
            ingested_at: Utc::now(),
        };

        let created = persist_ebay_order(&pool, order)
            .await
            .expect("persist_ebay_order failed");
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
        assert_eq!(parser_type, "ebay_order");
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
    async fn persist_ebay_order_idempotent_on_replay() {
        let pool = test_pool().await;
        let tenant_id = "test-ebay-poller-002";
        cleanup(&pool, tenant_id).await;

        let make_order = || OrderIngestedPayload {
            tenant_id: tenant_id.to_string(),
            source: "ebay".to_string(),
            order_id: "12-00000002-99990002".to_string(),
            order_number: None,
            financial_status: Some("IN_PROGRESS".to_string()),
            line_items: vec![],
            customer_ref: None,
            file_job_id: Uuid::nil(),
            ingested_at: Utc::now(),
        };

        let first = persist_ebay_order(&pool, make_order())
            .await
            .expect("first persist failed");
        assert!(first, "first persist should return true");

        let second = persist_ebay_order(&pool, make_order())
            .await
            .expect("second persist failed");
        assert!(!second, "second persist (duplicate) should return false");

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
        let tenant_id = "test-ebay-poller-003";
        cleanup(&pool, tenant_id).await;

        let order_ids = [
            "12-00000003-99990001",
            "12-00000003-99990002",
            "12-00000003-99990003",
        ];
        let response = sample_orders_response(&order_ids);

        let orders = normalize_ebay_orders(&response, tenant_id).expect("normalize failed");
        assert_eq!(orders.len(), 3);

        let mut new_count = 0;
        for order in orders {
            if persist_ebay_order(&pool, order)
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
