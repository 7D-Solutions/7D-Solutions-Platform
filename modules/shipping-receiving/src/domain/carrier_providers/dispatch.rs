//! Carrier request dispatch consumer.
//!
//! Subscribes to `sr.carrier_request.created` events. For each event:
//!   1. Guard: skip if not in `pending` status (idempotency)
//!   2. Transition `pending` → `submitted`
//!   3. Fetch carrier credentials from the Integrations module (best-effort)
//!   4. Dispatch to the registered `CarrierProvider`
//!   5. Transition `submitted` → `completed` (with response) or `submitted` → `failed`
//!
//! State machine invariants:
//! - `pending` → `submitted` before any external call
//! - `submitted` → `completed` | `failed` after provider response
//! - `completed` is terminal — never retry
//! - `failed` → `submitted` is valid (retry via re-publishing the event)

use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::carrier_requests::{
    CarrierRequestService, CarrierRequestType, TransitionCarrierRequest,
};
use super::{credentials, get_provider};

/// NATS subject for carrier request created events (emitted by SR's own outbox).
pub const SUBJECT_CARRIER_REQUEST_CREATED: &str = "sr.carrier_request.created";

// ── Event payload ─────────────────────────────────────────────

/// Anti-corruption layer: mirrors the sr.carrier_request.created outbox payload.
#[derive(Debug, Deserialize)]
struct CarrierRequestCreatedPayload {
    carrier_request_id: Uuid,
    tenant_id: Uuid,
    request_type: String,
    carrier_code: String,
}

// ── Public API ────────────────────────────────────────────────

/// Start the carrier dispatch consumer as a background task.
pub fn start_carrier_dispatch_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let http_client = Client::new();
        tracing::info!("SR: starting carrier dispatch consumer");

        let mut stream = match bus.subscribe(SUBJECT_CARRIER_REQUEST_CREATED).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    subject = SUBJECT_CARRIER_REQUEST_CREATED,
                    "SR: failed to subscribe to carrier dispatch subject"
                );
                return;
            }
        };

        tracing::info!(
            subject = SUBJECT_CARRIER_REQUEST_CREATED,
            "SR: carrier dispatch consumer subscribed"
        );

        while let Some(msg) = stream.next().await {
            if let Err(e) = handle_message(&pool, &http_client, &msg).await {
                tracing::error!(error = %e, "SR: carrier dispatch message handling failed");
            }
        }

        tracing::warn!("SR: carrier dispatch consumer stopped");
    });
}

/// Process a single `sr.carrier_request.created` event.
///
/// Exported for integration testing without NATS — call this directly with
/// the carrier request fields to verify the full dispatch pipeline.
pub async fn dispatch_carrier_request(
    pool: &PgPool,
    http_client: &Client,
    carrier_request_id: Uuid,
    tenant_id: Uuid,
    request_type: &str,
    carrier_code: &str,
    payload: &Value,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Guard: idempotency — skip if already past pending
    let current =
        CarrierRequestService::find_by_id(pool, carrier_request_id, tenant_id)
            .await?
            .ok_or_else(|| {
                format!("carrier request {} not found for tenant {}", carrier_request_id, tenant_id)
            })?;

    if current.status != "pending" {
        tracing::info!(
            carrier_request_id = %carrier_request_id,
            status = %current.status,
            "SR: carrier request not in pending state, skipping dispatch"
        );
        return Ok(());
    }

    // Step 1: pending → submitted
    CarrierRequestService::transition_status(
        pool,
        carrier_request_id,
        tenant_id,
        &TransitionCarrierRequest {
            status: "submitted".to_string(),
            response: None,
        },
    )
    .await?;

    // Step 2: resolve provider
    let provider = match get_provider(carrier_code) {
        Some(p) => p,
        None => {
            tracing::warn!(carrier_code, "SR: no provider registered for carrier_code");
            CarrierRequestService::transition_status(
                pool,
                carrier_request_id,
                tenant_id,
                &TransitionCarrierRequest {
                    status: "failed".to_string(),
                    response: Some(serde_json::json!({
                        "error": format!("no provider registered for carrier_code={}", carrier_code)
                    })),
                },
            )
            .await?;
            return Ok(());
        }
    };

    // Step 3: fetch credentials (best-effort — stub doesn't need them)
    let config =
        fetch_credentials_or_empty(http_client, &tenant_id.to_string(), carrier_code).await;

    // Step 4: dispatch to provider
    let result = call_provider(&*provider, request_type, payload, &config).await;

    // Step 5: submitted → completed | failed
    match result {
        Ok(response) => {
            CarrierRequestService::transition_status(
                pool,
                carrier_request_id,
                tenant_id,
                &TransitionCarrierRequest {
                    status: "completed".to_string(),
                    response: Some(response),
                },
            )
            .await?;
        }
        Err(e) => {
            tracing::warn!(
                carrier_request_id = %carrier_request_id,
                error = %e,
                "SR: carrier provider dispatch failed"
            );
            CarrierRequestService::transition_status(
                pool,
                carrier_request_id,
                tenant_id,
                &TransitionCarrierRequest {
                    status: "failed".to_string(),
                    response: Some(serde_json::json!({ "error": e.to_string() })),
                },
            )
            .await?;
        }
    }

    Ok(())
}

// ── Internal helpers ──────────────────────────────────────────

async fn handle_message(
    pool: &PgPool,
    http_client: &Client,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let p: CarrierRequestCreatedPayload = serde_json::from_slice(&msg.payload)?;

    tracing::info!(
        carrier_request_id = %p.carrier_request_id,
        carrier_code = %p.carrier_code,
        request_type = %p.request_type,
        "SR: received carrier_request.created event"
    );

    // Load full request to get the payload stored at creation time
    let req = CarrierRequestService::find_by_id(pool, p.carrier_request_id, p.tenant_id)
        .await?
        .ok_or_else(|| {
            format!(
                "carrier request {} not found for tenant {}",
                p.carrier_request_id, p.tenant_id
            )
        })?;

    dispatch_carrier_request(
        pool,
        http_client,
        p.carrier_request_id,
        p.tenant_id,
        &p.request_type,
        &p.carrier_code,
        &req.payload,
    )
    .await
}

async fn fetch_credentials_or_empty(
    http_client: &Client,
    app_id: &str,
    connector_type: &str,
) -> Value {
    match credentials::get_carrier_credentials(http_client, app_id, connector_type).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::debug!(
                error = %e,
                connector_type,
                "SR: could not fetch carrier credentials, using empty config"
            );
            serde_json::Value::Object(Default::default())
        }
    }
}

async fn call_provider(
    provider: &dyn super::CarrierProvider,
    request_type: &str,
    payload: &Value,
    config: &Value,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let req_type = CarrierRequestType::from_str_value(request_type)
        .map_err(|e| format!("invalid request_type '{}': {}", request_type, e))?;

    match req_type {
        CarrierRequestType::Rate => {
            let rates = provider.get_rates(payload, config).await?;
            Ok(serde_json::to_value(rates)?)
        }
        CarrierRequestType::Label => {
            let label = provider.create_label(payload, config).await?;
            Ok(serde_json::to_value(label)?)
        }
        CarrierRequestType::Track => {
            let tracking_number = payload
                .get("tracking_number")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = provider.track(tracking_number, config).await?;
            Ok(serde_json::to_value(result)?)
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::carrier_requests::{CarrierRequestService, CreateCarrierRequest};
    use serial_test::serial;

    const TEST_TENANT: &str = "00000000-0000-0000-0000-000000000099";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db"
                .to_string()
        })
    }

    fn test_nats_url() -> String {
        // Default embeds the dev token so the connection succeeds without setting env vars.
        std::env::var("NATS_URL").unwrap_or_else(|_| {
            let token = std::env::var("NATS_AUTH_TOKEN")
                .unwrap_or_else(|_| "dev-nats-token".to_string());
            format!("nats://{}@localhost:4222", token)
        })
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to SR test DB")
    }

    async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
        sqlx::query("DELETE FROM sr_carrier_requests WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM sr_events_outbox WHERE tenant_id = $1")
            .bind(tenant_id.to_string())
            .execute(pool)
            .await
            .ok();
    }

    async fn create_pending_request(
        pool: &PgPool,
        tenant_id: Uuid,
        request_type: &str,
        idem_key: &str,
    ) -> crate::domain::carrier_requests::CarrierRequest {
        let shipment_id = Uuid::new_v4();
        let req = CreateCarrierRequest {
            shipment_id,
            request_type: request_type.to_string(),
            carrier_code: "stub".to_string(),
            payload: serde_json::json!({"origin": "STUB_ORIGIN"}),
            idempotency_key: Some(idem_key.to_string()),
        };
        CarrierRequestService::create(pool, tenant_id, &req)
            .await
            .expect("create_pending_request failed")
    }

    #[tokio::test]
    #[serial]
    async fn dispatch_rate_request_transitions_to_completed() {
        let pool = test_pool().await;
        let tenant_id: Uuid = TEST_TENANT.parse().expect("valid test tenant UUID");
        cleanup(&pool, tenant_id).await;

        let carrier_req =
            create_pending_request(&pool, tenant_id, "rate", "test-dispatch-rate-001").await;
        assert_eq!(carrier_req.status, "pending");

        let http_client = Client::new();
        dispatch_carrier_request(
            &pool,
            &http_client,
            carrier_req.id,
            tenant_id,
            "rate",
            "stub",
            &carrier_req.payload,
        )
        .await
        .expect("dispatch failed");

        let updated = CarrierRequestService::find_by_id(&pool, carrier_req.id, tenant_id)
            .await
            .expect("find failed")
            .expect("not found");
        assert_eq!(updated.status, "completed");
        assert!(updated.response.is_some(), "response should be set on completed request");

        cleanup(&pool, tenant_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn dispatch_label_request_transitions_to_completed() {
        let pool = test_pool().await;
        let tenant_id: Uuid = TEST_TENANT.parse().expect("valid test tenant UUID");
        cleanup(&pool, tenant_id).await;

        let carrier_req =
            create_pending_request(&pool, tenant_id, "label", "test-dispatch-label-001").await;

        let http_client = Client::new();
        dispatch_carrier_request(
            &pool,
            &http_client,
            carrier_req.id,
            tenant_id,
            "label",
            "stub",
            &carrier_req.payload,
        )
        .await
        .expect("dispatch failed");

        let updated = CarrierRequestService::find_by_id(&pool, carrier_req.id, tenant_id)
            .await
            .expect("find failed")
            .expect("not found");
        assert_eq!(updated.status, "completed");

        cleanup(&pool, tenant_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn dispatch_skips_non_pending_request() {
        let pool = test_pool().await;
        let tenant_id: Uuid = TEST_TENANT.parse().expect("valid test tenant UUID");
        cleanup(&pool, tenant_id).await;

        let carrier_req =
            create_pending_request(&pool, tenant_id, "rate", "test-dispatch-skip-001").await;

        let http_client = Client::new();
        // First dispatch: pending → submitted → completed
        dispatch_carrier_request(
            &pool,
            &http_client,
            carrier_req.id,
            tenant_id,
            "rate",
            "stub",
            &carrier_req.payload,
        )
        .await
        .expect("first dispatch failed");

        // Second dispatch: should skip because status is completed (not pending)
        dispatch_carrier_request(
            &pool,
            &http_client,
            carrier_req.id,
            tenant_id,
            "rate",
            "stub",
            &carrier_req.payload,
        )
        .await
        .expect("second dispatch (idempotent skip) failed");

        cleanup(&pool, tenant_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn dispatch_via_nats_transitions_to_completed() {
        let pool = test_pool().await;
        let tenant_id: Uuid = TEST_TENANT.parse().expect("valid test tenant UUID");
        cleanup(&pool, tenant_id).await;

        let nats_client = event_bus::connect_nats(&test_nats_url())
            .await
            .expect("Failed to connect to NATS — is NATS running?");
        let bus: Arc<dyn EventBus> =
            Arc::new(event_bus::NatsBus::new(nats_client));

        // Create carrier request in pending state
        let carrier_req =
            create_pending_request(&pool, tenant_id, "label", "test-dispatch-nats-001").await;
        assert_eq!(carrier_req.status, "pending");

        // Start dispatch consumer (subscribes first)
        start_carrier_dispatch_consumer(bus.clone(), pool.clone());

        // Small delay to ensure subscription is established before publish
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Publish sr.carrier_request.created event (mirrors outbox payload)
        let event_bytes = serde_json::to_vec(&serde_json::json!({
            "carrier_request_id": carrier_req.id,
            "tenant_id": tenant_id,
            "shipment_id": carrier_req.shipment_id,
            "request_type": "label",
            "carrier_code": "stub",
            "status": "pending",
        }))
        .expect("serialize event");
        bus.publish(SUBJECT_CARRIER_REQUEST_CREATED, event_bytes)
            .await
            .expect("publish failed");

        // Poll until completed (max 5 seconds)
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let updated =
                CarrierRequestService::find_by_id(&pool, carrier_req.id, tenant_id)
                    .await
                    .expect("find failed")
                    .expect("not found");
            if updated.status == "completed" {
                assert!(
                    updated.response.is_some(),
                    "completed request must have a response payload"
                );
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "Timed out waiting for carrier request {} to reach completed status (current: {})",
                    carrier_req.id, updated.status
                );
            }
        }

        cleanup(&pool, tenant_id).await;
    }
}
