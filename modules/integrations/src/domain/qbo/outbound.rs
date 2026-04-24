//! QBO outbound invoice sync.
//!
//! Subscribes to `ar.events.ar.invoice_opened` on the event bus and creates a
//! corresponding invoice in QuickBooks Online.  The cross-system mapping is
//! persisted atomically in `integrations_external_refs` together with an outbox
//! event so the relay picks it up.
//!
//! ## Idempotency
//! Before calling QBO we check whether a `qbo_invoice` external ref already
//! exists for the AR invoice.  If it does, the message is a duplicate and we
//! return immediately.
//!
//! ## Failure modes
//! * **No customer mapping**: emits `integrations.qbo.invoice_sync_failed` to
//!   the outbox and returns `Ok(())` so the consumer does not re-queue the
//!   message. The error event carries enough context for manual remediation.
//! * **QBO call succeeds but ref-insert fails**: logs `ERROR` with the QBO
//!   invoice ID so it can be manually reconciled, then bubbles the DB error so
//!   the message is re-queued.

use chrono::NaiveDateTime;
use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch;
use uuid::Uuid;

use super::cdc::{qbo_base_url as default_qbo_base_url, DbTokenProvider};
use super::client::{QboClient, QboInvoicePayload, QboLineItem};
use crate::domain::external_refs::repo as refs_repo;
use crate::domain::oauth::repo as oauth_repo;
use crate::events::envelope::create_integrations_envelope;
use crate::events::{OrderIngestedPayload, MUTATION_CLASS_DATA_MUTATION};
use crate::outbox::enqueue_event_tx;

// ============================================================================
// Constants
// ============================================================================

pub const NATS_SUBJECT_AR_INVOICE_OPENED: &str = "ar.events.ar.invoice_opened";
pub const NATS_SUBJECT_ORDER_INGESTED: &str = "integrations.order.ingested";
pub const EVENT_TYPE_QBO_INVOICE_CREATED: &str = "integrations.qbo.invoice_created";
pub const EVENT_TYPE_QBO_INVOICE_SYNC_FAILED: &str = "integrations.qbo.invoice_sync_failed";

const ENTITY_TYPE_AR_INVOICE: &str = "ar_invoice";
const ENTITY_TYPE_AR_CUSTOMER: &str = "ar_customer";
const ENTITY_TYPE_MARKETPLACE_ORDER: &str = "marketplace_order";
const ENTITY_TYPE_MARKETPLACE_CUSTOMER: &str = "marketplace_customer";
const SYSTEM_QBO_INVOICE: &str = "qbo_invoice";
const SYSTEM_QBO_CUSTOMER: &str = "qbo";

// ============================================================================
// Inbound event shape (mirrors AR module's InvoiceLifecyclePayload)
// ============================================================================

/// Minimal deserialization struct for the AR invoice opened event envelope.
/// We only extract the fields we need — unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct ArInvoiceEnvelope {
    payload: ArInvoicePayload,
}

#[derive(Debug, Deserialize)]
struct ArInvoicePayload {
    pub invoice_id: String,
    pub customer_id: String,
    pub app_id: String,
    pub amount_cents: i64,
    pub due_at: Option<NaiveDateTime>,
}

// ============================================================================
// Outbound event payloads
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QboInvoiceCreatedPayload {
    pub ar_invoice_id: String,
    pub qbo_invoice_id: String,
    pub app_id: String,
    pub realm_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QboInvoiceSyncErrorPayload {
    pub ar_invoice_id: String,
    pub ar_customer_id: String,
    pub app_id: String,
    pub reason: String,
}

// ============================================================================
// Core processing
// ============================================================================

/// Process a single `ar.events.ar.invoice_opened` NATS message.
///
/// `qbo_base_url` is the QBO REST API base URL, injected so tests can point
/// the client at a local stub server without modifying global state.
pub async fn process_ar_invoice_opened(
    pool: &PgPool,
    msg: &BusMessage,
    qbo_base_url: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let envelope: ArInvoiceEnvelope = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("qbo_outbound: failed to deserialize AR invoice event: {e}"))?;

    let p = &envelope.payload;
    let app_id = p.app_id.as_str();
    let invoice_id = p.invoice_id.as_str();
    let customer_id = p.customer_id.as_str();

    // ── Idempotency guard ─────────────────────────────────────────────────────
    let existing =
        refs_repo::list_by_entity(pool, app_id, ENTITY_TYPE_AR_INVOICE, invoice_id).await?;
    if existing.iter().any(|r| r.system == SYSTEM_QBO_INVOICE) {
        tracing::info!(
            app_id,
            invoice_id,
            "qbo_outbound: already synced — skipping duplicate"
        );
        return Ok(());
    }

    // ── Resolve AR customer → QBO customer ────────────────────────────────────
    let customer_refs =
        refs_repo::list_by_entity(pool, app_id, ENTITY_TYPE_AR_CUSTOMER, customer_id).await?;
    let qbo_customer_id = match customer_refs
        .iter()
        .find(|r| r.system == SYSTEM_QBO_CUSTOMER)
    {
        Some(r) => r.external_id.clone(),
        None => {
            tracing::error!(
                app_id,
                invoice_id,
                customer_id,
                "qbo_outbound: no QBO customer mapping found — emitting sync_failed event"
            );
            emit_sync_failed(
                pool,
                app_id,
                invoice_id,
                customer_id,
                "no_qbo_customer_mapping",
            )
            .await?;
            return Ok(());
        }
    };

    // ── Resolve QBO connection (realm_id) ─────────────────────────────────────
    let conn = oauth_repo::get_connection(pool, app_id, "quickbooks").await?;
    let realm_id = match conn {
        Some(c) if c.connection_status == "connected" => c.realm_id,
        Some(c) => {
            tracing::error!(
                app_id,
                status = %c.connection_status,
                "qbo_outbound: QBO connection not in 'connected' state — skipping"
            );
            return Ok(());
        }
        None => {
            tracing::error!(
                app_id,
                "qbo_outbound: no QBO OAuth connection found — skipping"
            );
            return Ok(());
        }
    };

    // ── Build and call QBO client ─────────────────────────────────────────────
    let tokens = Arc::new(DbTokenProvider {
        pool: pool.clone(),
        app_id: app_id.to_string(),
    });
    let qbo = QboClient::new(qbo_base_url, &realm_id, tokens);

    let amount = p.amount_cents as f64 / 100.0;
    let item_ref = std::env::var("QBO_DEFAULT_ITEM_REF").unwrap_or_else(|_| "1".to_string());
    let due_date = p.due_at.map(|d| d.format("%Y-%m-%d").to_string());

    let invoice_payload = QboInvoicePayload {
        customer_ref: qbo_customer_id,
        line_items: vec![QboLineItem {
            amount,
            description: Some(format!("AR Invoice {invoice_id}")),
            item_ref: Some(item_ref),
            tax_code_ref: None, // tax_source=external_accounting_software: no override
            department_ref: None,
        }],
        due_date,
        doc_number: Some(invoice_id.to_string()),
        currency_ref: None,
        txn_tax_detail: None, // populated by tax-calc engine when available
        bill_addr: None,
        ship_addr: None,
        department_ref: None,
    };

    let qbo_invoice = qbo
        .create_invoice(&invoice_payload, Uuid::new_v4())
        .await
        .map_err(|e| format!("qbo_outbound: QBO create_invoice failed for {invoice_id}: {e}"))?;

    let qbo_invoice_id = qbo_invoice["Id"]
        .as_str()
        .ok_or_else(|| format!("qbo_outbound: QBO response missing Invoice.Id for {invoice_id}"))?
        .to_string();

    tracing::info!(
        app_id,
        invoice_id,
        qbo_invoice_id = %qbo_invoice_id,
        "qbo_outbound: QBO invoice created"
    );

    // ── Atomically store ref + enqueue outbox event ───────────────────────────
    let mut tx = pool.begin().await?;

    let ref_result = refs_repo::upsert(
        &mut tx,
        app_id,
        ENTITY_TYPE_AR_INVOICE,
        invoice_id,
        SYSTEM_QBO_INVOICE,
        &qbo_invoice_id,
        &None,
        &None,
    )
    .await;

    if let Err(ref db_err) = ref_result {
        // QBO invoice already created — must log for manual reconciliation
        tracing::error!(
            app_id,
            invoice_id,
            qbo_invoice_id = %qbo_invoice_id,
            error = %db_err,
            "ORPHANED QBO INVOICE: created in QBO but external ref insert failed — \
             manual reconciliation required"
        );
        // Propagate: caller can re-queue the message and retry the upsert next time
        return Err(Box::new(sqlx::Error::RowNotFound));
    }

    let event_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4().to_string();
    let created_payload = QboInvoiceCreatedPayload {
        ar_invoice_id: invoice_id.to_string(),
        qbo_invoice_id: qbo_invoice_id.clone(),
        app_id: app_id.to_string(),
        realm_id: realm_id.clone(),
    };

    let envelope_out = create_integrations_envelope(
        event_id,
        app_id.to_string(),
        EVENT_TYPE_QBO_INVOICE_CREATED.to_string(),
        correlation_id,
        None,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        created_payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_QBO_INVOICE_CREATED,
        "qbo_invoice",
        &qbo_invoice_id,
        app_id,
        &envelope_out,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

// ============================================================================
// Order-ingested consumer
// ============================================================================

/// Minimal deserialization wrapper for `EventEnvelope<OrderIngestedPayload>`.
#[derive(Debug, Deserialize)]
struct OrderIngestedEnvelope {
    payload: OrderIngestedPayload,
}

/// Process a single `integrations.order.ingested` NATS message.
///
/// Skips silently if `source == "qbo"` to prevent circular invoice creation.
/// On success, stores a `marketplace_order → qbo_invoice` external ref and
/// enqueues an `integrations.qbo.invoice_created` outbox event.
pub async fn process_order_ingested(
    pool: &PgPool,
    msg: &BusMessage,
    qbo_base_url: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let envelope: OrderIngestedEnvelope = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("qbo_outbound: failed to deserialize order.ingested event: {e}"))?;

    let p = &envelope.payload;
    let app_id = p.tenant_id.as_str();
    let order_id = p.order_id.as_str();
    let source = p.source.as_str();

    // ── Source guard — prevents circular creation when source is QBO ──────────
    if source == "qbo" {
        tracing::debug!(app_id, order_id, "qbo_outbound: skipping QBO-sourced order");
        return Ok(());
    }

    // ── Idempotency guard ─────────────────────────────────────────────────────
    let existing =
        refs_repo::list_by_entity(pool, app_id, ENTITY_TYPE_MARKETPLACE_ORDER, order_id).await?;
    if existing.iter().any(|r| r.system == SYSTEM_QBO_INVOICE) {
        tracing::info!(
            app_id,
            order_id,
            "qbo_outbound: order already synced — skipping duplicate"
        );
        return Ok(());
    }

    // ── Resolve marketplace customer → QBO customer ───────────────────────────
    let customer_ref = match p.customer_ref.as_deref() {
        Some(cr) => cr,
        None => {
            tracing::error!(
                app_id,
                order_id,
                "qbo_outbound: order has no customer_ref — emitting sync_failed"
            );
            emit_order_sync_failed(pool, app_id, order_id, "", "no_customer_ref").await?;
            return Ok(());
        }
    };

    let customer_refs =
        refs_repo::list_by_entity(pool, app_id, ENTITY_TYPE_MARKETPLACE_CUSTOMER, customer_ref)
            .await?;
    let qbo_customer_id = match customer_refs
        .iter()
        .find(|r| r.system == SYSTEM_QBO_CUSTOMER)
    {
        Some(r) => r.external_id.clone(),
        None => {
            tracing::error!(
                app_id,
                order_id,
                customer_ref,
                "qbo_outbound: no QBO customer mapping for marketplace customer — emitting sync_failed"
            );
            emit_order_sync_failed(
                pool,
                app_id,
                order_id,
                customer_ref,
                "no_qbo_customer_mapping",
            )
            .await?;
            return Ok(());
        }
    };

    // ── Resolve QBO connection (realm_id) ─────────────────────────────────────
    let conn = oauth_repo::get_connection(pool, app_id, "quickbooks").await?;
    let realm_id = match conn {
        Some(c) if c.connection_status == "connected" => c.realm_id,
        Some(c) => {
            tracing::error!(
                app_id,
                status = %c.connection_status,
                "qbo_outbound: QBO not in 'connected' state — skipping order"
            );
            return Ok(());
        }
        None => {
            tracing::error!(
                app_id,
                "qbo_outbound: no QBO OAuth connection — skipping order"
            );
            return Ok(());
        }
    };

    // ── Build QBO invoice payload from order line items ───────────────────────
    let tokens = Arc::new(DbTokenProvider {
        pool: pool.clone(),
        app_id: app_id.to_string(),
    });
    let qbo = QboClient::new(qbo_base_url, &realm_id, tokens);

    let item_ref = std::env::var("QBO_DEFAULT_ITEM_REF").unwrap_or_else(|_| "1".to_string());

    let line_items: Vec<QboLineItem> = p
        .line_items
        .iter()
        .map(|li| {
            let unit_price: f64 = li.price.parse().unwrap_or(0.0);
            let amount = unit_price * li.quantity as f64;
            QboLineItem {
                amount,
                description: Some(format!("{} (qty {})", li.title, li.quantity)),
                item_ref: Some(item_ref.clone()),
                tax_code_ref: None, // tax_source=external_accounting_software: no override
                department_ref: None,
            }
        })
        .collect();

    let invoice_payload = QboInvoicePayload {
        customer_ref: qbo_customer_id,
        line_items,
        due_date: None,
        doc_number: Some(format!("{source}-{order_id}")),
        currency_ref: None,
        txn_tax_detail: None, // populated by tax-calc engine when available
        bill_addr: None,
        ship_addr: None,
        department_ref: None,
    };

    let qbo_invoice = qbo
        .create_invoice(&invoice_payload, Uuid::new_v4())
        .await
        .map_err(|e| {
            format!("qbo_outbound: QBO create_invoice failed for order {order_id}: {e}")
        })?;

    let qbo_invoice_id = qbo_invoice["Id"]
        .as_str()
        .ok_or_else(|| {
            format!("qbo_outbound: QBO response missing Invoice.Id for order {order_id}")
        })?
        .to_string();

    tracing::info!(
        app_id,
        order_id,
        qbo_invoice_id = %qbo_invoice_id,
        "qbo_outbound: QBO invoice created for marketplace order"
    );

    // ── Atomically store ref + enqueue outbox event ───────────────────────────
    let mut tx = pool.begin().await?;

    let ref_result = refs_repo::upsert(
        &mut tx,
        app_id,
        ENTITY_TYPE_MARKETPLACE_ORDER,
        order_id,
        SYSTEM_QBO_INVOICE,
        &qbo_invoice_id,
        &None,
        &None,
    )
    .await;

    if let Err(ref db_err) = ref_result {
        tracing::error!(
            app_id,
            order_id,
            qbo_invoice_id = %qbo_invoice_id,
            error = %db_err,
            "ORPHANED QBO INVOICE: created for marketplace order but external ref insert failed \
             — manual reconciliation required"
        );
        return Err(Box::new(sqlx::Error::RowNotFound));
    }

    let event_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4().to_string();
    let created_payload = QboInvoiceCreatedPayload {
        ar_invoice_id: order_id.to_string(),
        qbo_invoice_id: qbo_invoice_id.clone(),
        app_id: app_id.to_string(),
        realm_id: realm_id.clone(),
    };

    let envelope_out = create_integrations_envelope(
        event_id,
        app_id.to_string(),
        EVENT_TYPE_QBO_INVOICE_CREATED.to_string(),
        correlation_id,
        None,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        created_payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_QBO_INVOICE_CREATED,
        "qbo_invoice",
        &qbo_invoice_id,
        app_id,
        &envelope_out,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Emit a `integrations.qbo.invoice_sync_failed` outbox event for marketplace order failures.
async fn emit_order_sync_failed(
    pool: &PgPool,
    app_id: &str,
    order_id: &str,
    customer_ref: &str,
    reason: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut tx = pool.begin().await?;
    let event_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4().to_string();

    let error_payload = QboInvoiceSyncErrorPayload {
        ar_invoice_id: order_id.to_string(),
        ar_customer_id: customer_ref.to_string(),
        app_id: app_id.to_string(),
        reason: reason.to_string(),
    };

    let envelope = create_integrations_envelope(
        event_id,
        app_id.to_string(),
        EVENT_TYPE_QBO_INVOICE_SYNC_FAILED.to_string(),
        correlation_id,
        None,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        error_payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_QBO_INVOICE_SYNC_FAILED,
        "qbo_invoice_sync",
        order_id,
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Spawn the QBO order-ingested consumer as a background tokio task.
///
/// Subscribes to [`NATS_SUBJECT_ORDER_INGESTED`] and calls
/// [`process_order_ingested`] for each message.
pub fn spawn_order_ingested_consumer(
    pool: PgPool,
    bus: Arc<dyn EventBus>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let qbo_url = default_qbo_base_url();

        let mut stream = match bus.subscribe(NATS_SUBJECT_ORDER_INGESTED).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    subject = NATS_SUBJECT_ORDER_INGESTED,
                    "qbo_outbound: failed to subscribe to order.ingested — consumer not started"
                );
                return;
            }
        };

        tracing::info!(
            subject = NATS_SUBJECT_ORDER_INGESTED,
            "QBO order-ingested consumer started"
        );

        loop {
            tokio::select! {
                maybe_msg = stream.next() => {
                    match maybe_msg {
                        Some(msg) => {
                            if let Err(e) = process_order_ingested(&pool, &msg, &qbo_url).await {
                                tracing::error!(
                                    error = %e,
                                    "qbo_outbound: order.ingested processing failed"
                                );
                            }
                        }
                        None => {
                            tracing::warn!("qbo_outbound: order.ingested stream ended");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    tracing::info!("qbo_outbound: order.ingested consumer shutting down");
                    break;
                }
            }
        }
    })
}

/// Emit a `integrations.qbo.invoice_sync_failed` outbox event without aborting processing.
async fn emit_sync_failed(
    pool: &PgPool,
    app_id: &str,
    invoice_id: &str,
    customer_id: &str,
    reason: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut tx = pool.begin().await?;
    let event_id = Uuid::new_v4();
    let correlation_id = Uuid::new_v4().to_string();

    let error_payload = QboInvoiceSyncErrorPayload {
        ar_invoice_id: invoice_id.to_string(),
        ar_customer_id: customer_id.to_string(),
        app_id: app_id.to_string(),
        reason: reason.to_string(),
    };

    let envelope = create_integrations_envelope(
        event_id,
        app_id.to_string(),
        EVENT_TYPE_QBO_INVOICE_SYNC_FAILED.to_string(),
        correlation_id,
        None,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        error_payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_QBO_INVOICE_SYNC_FAILED,
        "qbo_invoice_sync",
        invoice_id,
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

// ============================================================================
// Feature flag
// ============================================================================

/// Returns true only when `QBO_LEGACY_CONSUMERS_ENABLED=1` is explicitly set.
///
/// Default is OFF so legacy outbound consumers never run concurrently with
/// the authority-gated sync path during cutover.  Set the env var to `1`
/// to restore legacy behaviour when the new path is not yet active.
pub fn legacy_consumers_enabled() -> bool {
    std::env::var("QBO_LEGACY_CONSUMERS_ENABLED")
        .map(|v| v == "1")
        .unwrap_or(false)
}

// ============================================================================
// Consumer worker
// ============================================================================

/// Spawn the QBO outbound consumer as a background tokio task.
///
/// Subscribes to [`NATS_SUBJECT_AR_INVOICE_OPENED`] and calls
/// [`process_ar_invoice_opened`] for each message.  Errors are logged but do
/// not stop the worker.
pub fn spawn_outbound_consumer(
    pool: PgPool,
    bus: Arc<dyn EventBus>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let qbo_url = default_qbo_base_url();

        let mut stream = match bus.subscribe(NATS_SUBJECT_AR_INVOICE_OPENED).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    subject = NATS_SUBJECT_AR_INVOICE_OPENED,
                    "qbo_outbound: failed to subscribe — consumer not started"
                );
                return;
            }
        };

        tracing::info!(
            subject = NATS_SUBJECT_AR_INVOICE_OPENED,
            "QBO outbound consumer started"
        );

        loop {
            tokio::select! {
                maybe_msg = stream.next() => {
                    match maybe_msg {
                        Some(msg) => {
                            if let Err(e) = process_ar_invoice_opened(&pool, &msg, &qbo_url).await {
                                tracing::error!(
                                    error = %e,
                                    "qbo_outbound: message processing failed"
                                );
                            }
                        }
                        None => {
                            tracing::warn!("qbo_outbound: AR invoice event stream ended");
                            break;
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    tracing::info!("qbo_outbound: shutting down");
                    break;
                }
            }
        }
    })
}

// ============================================================================
// Tests (unit-level — no DB required)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn nats_subject_constant() {
        assert_eq!(
            NATS_SUBJECT_AR_INVOICE_OPENED,
            "ar.events.ar.invoice_opened"
        );
    }

    #[test]
    #[serial]
    fn legacy_consumers_flag_off_by_default() {
        std::env::remove_var("QBO_LEGACY_CONSUMERS_ENABLED");
        assert!(!super::legacy_consumers_enabled());
    }

    #[test]
    #[serial]
    fn legacy_consumers_flag_on_when_set_to_1() {
        std::env::set_var("QBO_LEGACY_CONSUMERS_ENABLED", "1");
        let result = super::legacy_consumers_enabled();
        std::env::remove_var("QBO_LEGACY_CONSUMERS_ENABLED");
        assert!(result);
    }

    #[test]
    #[serial]
    fn legacy_consumers_flag_off_for_non_1_values() {
        for val in &["0", "true", "yes", "on", ""] {
            std::env::set_var("QBO_LEGACY_CONSUMERS_ENABLED", val);
            let result = super::legacy_consumers_enabled();
            std::env::remove_var("QBO_LEGACY_CONSUMERS_ENABLED");
            assert!(!result, "expected OFF for value {:?}", val);
        }
    }

    #[test]
    fn event_type_constants() {
        assert!(EVENT_TYPE_QBO_INVOICE_CREATED.starts_with("integrations.qbo."));
        assert!(EVENT_TYPE_QBO_INVOICE_SYNC_FAILED.starts_with("integrations.qbo."));
    }

    #[test]
    fn qbo_outbound_deserialize_ar_invoice_envelope() {
        let json = serde_json::json!({
            "event_id": "00000000-0000-0000-0000-000000000001",
            "event_type": "ar.invoice_opened",
            "occurred_at": "2026-04-08T12:00:00Z",
            "tenant_id": "test-app",
            "source_module": "ar",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "payload": {
                "invoice_id": "inv-001",
                "customer_id": "cust-42",
                "app_id": "test-app",
                "amount_cents": 5000_i64,
                "currency": "usd",
                "created_at": "2026-04-08T12:00:00",
                "due_at": null,
                "paid_at": null
            }
        });
        let raw = serde_json::to_vec(&json).expect("serialize test json");
        let msg = BusMessage::new("ar.events.ar.invoice_opened".to_string(), raw);
        let env: ArInvoiceEnvelope =
            serde_json::from_slice(&msg.payload).expect("deserialize envelope");
        assert_eq!(env.payload.invoice_id, "inv-001");
        assert_eq!(env.payload.customer_id, "cust-42");
        assert_eq!(env.payload.app_id, "test-app");
        assert_eq!(env.payload.amount_cents, 5000);
        assert!(env.payload.due_at.is_none());
    }
}
