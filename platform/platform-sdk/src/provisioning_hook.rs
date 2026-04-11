//! Tenant provisioning hook — opt-in callback for module-specific setup.
//!
//! When a tenant completes the orchestrator's 7-step provisioning sequence,
//! the control-plane publishes a `tenant.provisioned` event. Modules that
//! register a provisioning hook via [`ModuleBuilder::on_tenant_provisioned`]
//! subscribe to that subject and run their own setup logic — seed data,
//! default configuration, webhook registration, etc.
//!
//! Infrastructure (database creation, schema migrations) is handled by the
//! provisioning orchestrator before this hook fires. The hook is **opt-in**:
//! modules without one are completely unaffected.
//!
//! # Failure semantics
//!
//! Hook failures are retried (3 attempts, exponential backoff, matching the
//! standard consumer retry policy). If all attempts are exhausted, the error
//! is logged and the event is skipped — the tenant remains active. Hooks
//! must be idempotent: the orchestrator may re-deliver the event on retry.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use event_bus::EventBus;
use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use futures::StreamExt;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::Instrument;
use uuid::Uuid;

use crate::consumer::ConsumerError;
use crate::context::{ModuleContext, TenantPoolResolver};
use crate::startup::StartupError;

// ============================================================================
// Public types
// ============================================================================

/// NATS subject for the tenant.provisioned event.
///
/// Published by the provisioning outbox relay after the tenant's 7-step
/// sequence completes and the tenant is marked `active`.
pub const TENANT_PROVISIONED_SUBJECT: &str = "tenant.provisioned";

/// Event payload delivered to the [`ModuleBuilder::on_tenant_provisioned`] hook.
///
/// Contains the tenant ID whose provisioning just completed. The module's
/// database has been created and migrated before this fires — use this for
/// module-specific setup (seed data, default configuration, etc.).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TenantProvisionedEvent {
    /// The newly provisioned tenant.
    pub tenant_id: Uuid,
}

/// Type-erased async handler for the provisioning hook.
///
/// Register one via [`ModuleBuilder::on_tenant_provisioned`].
pub type ProvisioningHandler = Arc<
    dyn Fn(
            ModuleContext,
            TenantProvisionedEvent,
        ) -> Pin<Box<dyn Future<Output = Result<(), ConsumerError>> + Send>>
        + Send
        + Sync,
>;

// ============================================================================
// Wiring
// ============================================================================

/// Subscribe to `tenant.provisioned` and run the hook for each event.
///
/// Deserializes the raw JSON payload published by the provisioning outbox
/// relay (not an `EventEnvelope` wrapper) into a [`TenantProvisionedEvent`].
/// Malformed payloads are routed to the DLQ; the hook is retried on failure.
///
/// Shares the caller's shutdown receiver so the hook task drains alongside
/// other consumers when the module receives a shutdown signal.
pub async fn wire_provisioning_hook(
    handler: ProvisioningHandler,
    bus: &Arc<dyn EventBus>,
    ctx: &ModuleContext,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<JoinHandle<()>, StartupError> {
    let mut stream = bus.subscribe(TENANT_PROVISIONED_SUBJECT).await.map_err(|e| {
        StartupError::Config(format!(
            "failed to subscribe to '{}': {e}",
            TENANT_PROVISIONED_SUBJECT
        ))
    })?;

    tracing::info!(
        subject = TENANT_PROVISIONED_SUBJECT,
        "provisioning hook subscribed"
    );

    let ctx = ctx.clone();
    let mut rx = shutdown_rx;
    let retry_config = RetryConfig::default();
    let bus_clone = Arc::clone(bus);

    let handle = tokio::spawn(async move {
        tracing::info!(subject = TENANT_PROVISIONED_SUBJECT, "provisioning hook listening");

        loop {
            tokio::select! {
                biased;
                _ = rx.changed() => {
                    if *rx.borrow() {
                        break;
                    }
                }
                msg = stream.next() => {
                    let Some(msg) = msg else { break; };

                    let event: TenantProvisionedEvent =
                        match serde_json::from_slice(&msg.payload) {
                            Ok(e) => e,
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "provisioning hook: failed to parse event — routing to DLQ"
                                );
                                publish_to_dlq(
                                    &bus_clone,
                                    TENANT_PROVISIONED_SUBJECT,
                                    &msg.payload,
                                    &e.to_string(),
                                )
                                .await;
                                continue;
                            }
                        };

                    let tenant_id = event.tenant_id;
                    let span = tracing::info_span!(
                        "provisioning_hook",
                        tenant_id = %tenant_id,
                    );

                    let result = retry_with_backoff(
                        || {
                            let h = handler.clone();
                            let c = ctx.clone();
                            let ev = event.clone();
                            async move { h(c, ev).await.map_err(|e| e.to_string()) }
                        },
                        &retry_config,
                        "provisioning_hook",
                    )
                    .instrument(span)
                    .await;

                    if let Err(e) = result {
                        tracing::error!(
                            tenant_id = %tenant_id,
                            error = %e,
                            "provisioning hook exhausted retries"
                        );
                    }
                }
            }
        }

        tracing::info!("provisioning hook stopped");
    });

    Ok(handle)
}

/// Auto-wire a [`TenantPoolResolver`] to register each new tenant on provisioning.
///
/// Subscribes to `tenant.provisioned` and calls [`TenantPoolResolver::pool_for`]
/// for each event. This warms the resolver's pool cache so the tenant's database
/// is ready before the first request arrives.
///
/// Modules that register a `TenantPoolResolver` via the builder no longer need a
/// manual `on_tenant_provisioned` callback for this common pattern — the SDK calls
/// this automatically. The manual callback remains available for module-specific
/// setup beyond pool warming (seed data, default configuration, etc.).
///
/// Failures are retried (3 attempts, exponential backoff). Exhausted retries are
/// logged and skipped — the tenant remains active and the cache will populate on
/// first request.
pub async fn wire_pool_resolver_auto_register(
    resolver: Arc<dyn TenantPoolResolver>,
    bus: &Arc<dyn EventBus>,
    ctx: &ModuleContext,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<JoinHandle<()>, StartupError> {
    let handler: ProvisioningHandler = Arc::new(move |_ctx, event| {
        let r = Arc::clone(&resolver);
        Box::pin(async move {
            let tenant_id = event.tenant_id;
            match r.pool_for(tenant_id).await {
                Ok(_) => {
                    tracing::info!(
                        tenant_id = %tenant_id,
                        "SDK auto-registered tenant pool on provisioning"
                    );
                    Ok(())
                }
                Err(e) => Err(ConsumerError::Processing(format!(
                    "auto-register pool_for({tenant_id}) failed: {e}"
                ))),
            }
        })
    });

    wire_provisioning_hook(handler, bus, ctx, shutdown_rx).await
}

// ============================================================================
// DLQ helper (local copy — avoids coupling to consumer internals)
// ============================================================================

async fn publish_to_dlq(bus: &Arc<dyn EventBus>, subject: &str, raw: &[u8], error: &str) {
    let dlq_subject = format!("dlq.{subject}");
    let envelope = serde_json::json!({
        "original_subject": subject,
        "error": error,
        "failed_at": chrono::Utc::now().to_rfc3339(),
        "raw_payload": raw,
    });
    match serde_json::to_vec(&envelope) {
        Ok(bytes) => {
            if let Err(e) = bus.publish(&dlq_subject, bytes).await {
                tracing::warn!(
                    subject = %subject,
                    dlq = %dlq_subject,
                    error = %e,
                    "failed to publish malformed provisioning event to DLQ"
                );
            } else {
                tracing::warn!(
                    subject = %subject,
                    dlq = %dlq_subject,
                    "malformed provisioning event sent to DLQ"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                subject = %subject,
                error = %e,
                "failed to serialize DLQ envelope for provisioning event — raw payload lost"
            );
        }
    }
}

// ============================================================================
// Test helpers
// ============================================================================

/// Build a synthetic provisioning payload for use in integration tests.
///
/// Returns a raw JSON byte vector matching the format published by the
/// provisioning outbox relay. Pass this to `bus.publish("tenant.provisioned", ...)`
/// to exercise the hook without running the full provisioning orchestrator.
pub fn test_payload(tenant_id: Uuid) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "tenant_id": tenant_id.to_string(),
    }))
    .expect("payload serialization is infallible")
}
