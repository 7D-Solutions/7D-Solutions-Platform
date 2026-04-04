//! Atomic stock receipt service.
//!
//! Invariants:
//! - Ledger row + FIFO layer + outbox event created in a single transaction
//! - Idempotency key prevents double-processing on retry
//! - Guards run before any mutation
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)

use chrono::{Duration, NaiveDate, Utc};
use event_bus::TracingContext;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    domain::{
        expiry::compute_expiry_from_policy,
        guards::{
            guard_convert_to_base, guard_cost_present, guard_item_active, guard_quantity_positive,
            GuardError,
        },
        items::TrackingMode,
        lots_serials::receipt::{insert_serial_instances, upsert_lot},
        projections::on_hand,
        receipt_repo,
        reorder::evaluator,
    },
    events::{
        build_expiry_set_envelope,
        contracts::{build_item_received_envelope, ItemReceivedPayload},
        ExpirySetPayload, EVENT_TYPE_EXPIRY_SET, EVENT_TYPE_ITEM_RECEIVED,
    },
};

// ============================================================================
// Types
// ============================================================================

/// Allowed values for receipt source_type.
pub const SOURCE_TYPE_PURCHASE: &str = "purchase";
pub const SOURCE_TYPE_PRODUCTION: &str = "production";
pub const SOURCE_TYPE_RETURN: &str = "return";

/// Input for POST /api/inventory/receipts
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReceiptRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    /// Optional storage location within the warehouse (bin, shelf, zone).
    /// When absent, the receipt is location-agnostic — existing behavior.
    #[serde(default)]
    pub location_id: Option<Uuid>,
    /// Quantity received (must be > 0)
    pub quantity: i64,
    /// Unit cost in minor currency units, e.g. cents (must be > 0)
    pub unit_cost_minor: i64,
    pub currency: String,
    /// Origin of receipt: "purchase" (default) | "production" | "return".
    /// Production receipts require caller-provided unit_cost.
    #[serde(default = "default_source_type")]
    pub source_type: String,
    pub purchase_order_id: Option<Uuid>,
    /// Caller-supplied idempotency key (required; scoped per tenant)
    pub idempotency_key: String,
    /// Distributed trace correlation ID (optional; generated if absent)
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    /// Required when item.tracking_mode == Lot.
    /// Identifies the lot; creates the lot row if it does not exist yet.
    pub lot_code: Option<String>,
    /// Required when item.tracking_mode == Serial.
    /// Length MUST equal `quantity`; each code must be unique per tenant+item.
    pub serial_codes: Option<Vec<String>>,
    /// UoM id for the input `quantity`. When present, `quantity` is in this unit
    /// and will be converted to the item's base_uom before writing to the ledger.
    /// When absent, `quantity` is assumed to already be in base_uom units.
    #[serde(default)]
    pub uom_id: Option<Uuid>,
}

fn default_source_type() -> String {
    SOURCE_TYPE_PURCHASE.to_string()
}

/// Result returned on successful or replayed receipt
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReceiptResult {
    /// Stable business key for this receipt (from ledger.entry_id)
    pub receipt_line_id: Uuid,
    /// BIGSERIAL ledger row id (used for FIFO ordering)
    pub ledger_entry_id: i64,
    /// FIFO layer id
    pub layer_id: Uuid,
    /// Event id used in outbox (also = ledger.source_event_id)
    pub event_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    #[serde(default)]
    pub location_id: Option<Uuid>,
    pub quantity: i64,
    pub unit_cost_minor: i64,
    pub currency: String,
    /// Origin of receipt: "purchase" | "production" | "return"
    pub source_type: String,
    pub received_at: chrono::DateTime<Utc>,
    /// Lot id, present when item.tracking_mode == Lot.
    #[serde(default)]
    pub lot_id: Option<Uuid>,
    /// Serial instance ids created, in the same order as request.serial_codes.
    /// Empty for non-serial items.
    #[serde(default)]
    pub serial_instance_ids: Vec<Uuid>,
}

#[derive(Debug, Error)]
pub enum ReceiptError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("lot_code is required for lot-tracked items")]
    LotCodeRequired,

    #[error("serial_codes is required for serial-tracked items")]
    SerialCodesRequired,

    #[error("serial_codes length {got} must equal quantity {expected}")]
    SerialCountMismatch { expected: i64, got: usize },

    #[error("duplicate serial code: a serial code already exists for this tenant/item")]
    DuplicateSerialCode,

    #[error("Idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("Expiry policy error: {0}")]
    ExpiryPolicy(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Service
// ============================================================================

/// Process a stock receipt atomically.
///
/// Returns `(ReceiptResult, is_replay)`.
/// - `is_replay = false`: new receipt created; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200 with stored result.
pub async fn process_receipt(
    pool: &PgPool,
    req: &ReceiptRequest,
    tracing_ctx: Option<&TracingContext>,
) -> Result<(ReceiptResult, bool), ReceiptError> {
    // --- Stateless input validation ---
    validate_request(req)?;

    // --- Compute request hash for idempotency conflict detection ---
    let request_hash = serde_json::to_string(req)?;

    // --- Idempotency check (read outside tx; fast path for replays) ---
    if let Some(record) =
        receipt_repo::find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await?
    {
        if record.request_hash != request_hash {
            return Err(ReceiptError::ConflictingIdempotencyKey);
        }
        let result: ReceiptResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- DB guard: item must exist and be active ---
    let item = guard_item_active(pool, req.item_id, &req.tenant_id).await?;

    // --- UoM conversion: canonicalize quantity to base_uom units ---
    let quantity = guard_convert_to_base(
        pool,
        req.item_id,
        &req.tenant_id,
        req.quantity,
        req.uom_id,
        item.base_uom_id,
    )
    .await?;

    // --- Tracking requirements guard (requires item.tracking_mode from DB) ---
    validate_tracking_requirements(item.tracking_mode, req)?;

    // --- Atomic transaction: ledger + lot/serial + FIFO layer + outbox + idempotency key ---
    let event_id = Uuid::new_v4();
    let received_at = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let lot_expiry_for_upsert: Option<NaiveDate> = if item.tracking_mode == TrackingMode::Lot {
        compute_expiry_from_policy(pool, &req.tenant_id, req.item_id, received_at)
            .await
            .map_err(|e| ReceiptError::ExpiryPolicy(e.to_string()))?
    } else {
        None
    };
    let lot_expiry_source = if lot_expiry_for_upsert.is_some() {
        Some("policy".to_string())
    } else {
        None
    };

    let mut tx = pool.begin().await?;

    // Step 1: Insert ledger row
    let ledger_row = receipt_repo::insert_ledger_row(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        req.location_id,
        quantity,
        req.unit_cost_minor,
        &req.currency,
        event_id,
        EVENT_TYPE_ITEM_RECEIVED,
        req.purchase_order_id,
        &req.source_type,
        received_at,
    )
    .await?;

    let ledger_entry_id = ledger_row.id;
    let receipt_line_id = ledger_row.entry_id;

    // Step 2: Upsert lot if lot-tracked (must precede layer insert to get lot_id)
    let lot_id: Option<Uuid> = if item.tracking_mode == TrackingMode::Lot {
        // validated above: lot_code is Some and non-empty
        let code = req.lot_code.as_deref().ok_or_else(|| ReceiptError::Guard(GuardError::Validation("lot_code required for lot-tracked item".into())))?;
        let id = upsert_lot(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            code,
            lot_expiry_for_upsert,
            None,
        )
        .await?;
        Some(id)
    } else {
        None
    };

    // Step 3: Insert FIFO layer (with lot_id association when lot-tracked)
    let layer_id = receipt_repo::insert_fifo_layer(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        ledger_entry_id,
        received_at,
        quantity,
        req.unit_cost_minor,
        &req.currency,
        lot_id,
    )
    .await?;

    // Step 3b: Insert serial instances if serial-tracked
    let serial_instance_ids: Vec<Uuid> = if item.tracking_mode == TrackingMode::Serial {
        // validated above: serial_codes is Some and len == quantity
        let codes = req.serial_codes.as_deref().ok_or_else(|| ReceiptError::Guard(GuardError::Validation("serial_codes required for serial-tracked item".into())))?;
        insert_serial_instances(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            codes,
            ledger_entry_id,
            layer_id,
        )
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return ReceiptError::DuplicateSerialCode;
                }
            }
            ReceiptError::Database(e)
        })?
    } else {
        vec![]
    };

    // Step 4: Upsert on-hand projection (quantity_on_hand + available_status_on_hand)
    on_hand::upsert_after_receipt(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        req.location_id,
        quantity,
        req.unit_cost_minor,
        &req.currency,
        ledger_entry_id,
    )
    .await?;

    // Step 4b: Increment 'available' status bucket
    on_hand::add_to_available_bucket(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        quantity,
    )
    .await?;

    // Step 5: Build event envelope and enqueue in outbox
    let payload = ItemReceivedPayload {
        receipt_line_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        sku: item.sku,
        warehouse_id: req.warehouse_id,
        quantity,
        unit_cost_minor: req.unit_cost_minor,
        currency: req.currency.clone(),
        source_type: req.source_type.clone(),
        purchase_order_id: req.purchase_order_id,
        received_at,
    };

    let default_ctx = TracingContext::default();
    let ctx = tracing_ctx.unwrap_or(&default_ctx);
    let envelope = build_item_received_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    )
    .with_tracing_context(ctx);
    let envelope_json = serde_json::to_string(&envelope)?;

    receipt_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_ITEM_RECEIVED,
        "inventory_item",
        &req.item_id.to_string(),
        &req.tenant_id,
        &envelope_json,
        &correlation_id,
        req.causation_id.as_deref(),
    )
    .await?;

    if let (Some(lot_id), Some(expires_on), Some(expiry_source), Some(lot_code)) = (
        lot_id,
        lot_expiry_for_upsert,
        lot_expiry_source.as_deref(),
        req.lot_code.as_deref(),
    ) {
        let expiry_event_id = Uuid::new_v4();
        let expiry_payload = ExpirySetPayload {
            lot_id,
            tenant_id: req.tenant_id.clone(),
            item_id: req.item_id,
            lot_code: lot_code.to_string(),
            expiry_date: expires_on,
            source: expiry_source.to_string(),
            set_at: received_at,
        };
        let expiry_envelope = build_expiry_set_envelope(
            expiry_event_id,
            req.tenant_id.clone(),
            correlation_id.clone(),
            req.causation_id.clone(),
            expiry_payload,
        );
        let expiry_envelope_json = serde_json::to_string(&expiry_envelope)?;

        receipt_repo::insert_outbox_event(
            &mut tx,
            expiry_event_id,
            EVENT_TYPE_EXPIRY_SET,
            "inventory_lot",
            &lot_id.to_string(),
            &req.tenant_id,
            &expiry_envelope_json,
            &correlation_id,
            req.causation_id.as_deref(),
        )
        .await?;
    }

    // Step 6: Build result
    let result = ReceiptResult {
        receipt_line_id,
        ledger_entry_id,
        layer_id,
        event_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        warehouse_id: req.warehouse_id,
        location_id: req.location_id,
        quantity,
        unit_cost_minor: req.unit_cost_minor,
        currency: req.currency.clone(),
        source_type: req.source_type.clone(),
        received_at,
        lot_id,
        serial_instance_ids,
    };

    // Step 7: Store idempotency key with response (expires in 7 days)
    let response_json = serde_json::to_string(&result)?;
    let expires_at = received_at + Duration::days(7);

    receipt_repo::store_idempotency_key(
        &mut tx,
        &req.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &response_json,
        expires_at,
    )
    .await?;

    tx.commit().await?;

    // Best-effort low-stock state evaluation — a receipt may push stock back above the
    // reorder_point, re-arming the dedup state for future crossings.
    let _ = evaluator::evaluate_low_stock(
        pool,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        req.location_id,
        &correlation_id,
        req.causation_id.clone(),
    )
    .await;

    Ok((result, false))
}

// ============================================================================
// Helpers
// ============================================================================

fn validate_request(req: &ReceiptRequest) -> Result<(), ReceiptError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(ReceiptError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    if req.tenant_id.trim().is_empty() {
        return Err(ReceiptError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
    }
    if req.currency.trim().is_empty() {
        return Err(ReceiptError::Guard(GuardError::Validation(
            "currency is required".to_string(),
        )));
    }
    // Validate source_type enum
    match req.source_type.as_str() {
        SOURCE_TYPE_PURCHASE | SOURCE_TYPE_PRODUCTION | SOURCE_TYPE_RETURN => {}
        _ => {
            return Err(ReceiptError::Guard(GuardError::Validation(format!(
                "source_type must be one of: purchase, production, return (got '{}')",
                req.source_type
            ))));
        }
    }
    guard_quantity_positive(req.quantity)?;
    guard_cost_present(req.unit_cost_minor)?;
    Ok(())
}

/// Validate lot/serial tracking requirements against the item's tracking_mode.
///
/// Called after `guard_item_active` since tracking_mode comes from the item row.
fn validate_tracking_requirements(
    tracking_mode: TrackingMode,
    req: &ReceiptRequest,
) -> Result<(), ReceiptError> {
    match tracking_mode {
        TrackingMode::Lot => {
            if req.lot_code.as_deref().unwrap_or("").trim().is_empty() {
                return Err(ReceiptError::LotCodeRequired);
            }
        }
        TrackingMode::Serial => {
            let codes = req
                .serial_codes
                .as_ref()
                .ok_or(ReceiptError::SerialCodesRequired)?;
            if codes.len() as i64 != req.quantity {
                return Err(ReceiptError::SerialCountMismatch {
                    expected: req.quantity,
                    got: codes.len(),
                });
            }
        }
        TrackingMode::None => {}
    }
    Ok(())
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_req() -> ReceiptRequest {
        ReceiptRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            quantity: 10,
            unit_cost_minor: 5000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "idem-001".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        }
    }

    #[test]
    fn validate_rejects_invalid_source_type() {
        let mut r = valid_req();
        r.source_type = "invalid".to_string();
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_accepts_production_source_type() {
        let mut r = valid_req();
        r.source_type = "production".to_string();
        assert!(validate_request(&r).is_ok());
    }

    #[test]
    fn validate_accepts_return_source_type() {
        let mut r = valid_req();
        r.source_type = "return".to_string();
        assert!(validate_request(&r).is_ok());
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let mut r = valid_req();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = valid_req();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_rejects_zero_quantity() {
        let mut r = valid_req();
        r.quantity = 0;
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_rejects_zero_cost() {
        let mut r = valid_req();
        r.unit_cost_minor = 0;
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_currency() {
        let mut r = valid_req();
        r.currency = "".to_string();
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_request(&valid_req()).is_ok());
    }

    #[test]
    fn tracking_lot_requires_lot_code() {
        let req = valid_req();
        assert!(matches!(
            validate_tracking_requirements(TrackingMode::Lot, &req),
            Err(ReceiptError::LotCodeRequired)
        ));
    }

    #[test]
    fn tracking_lot_accepts_lot_code() {
        let mut req = valid_req();
        req.lot_code = Some("LOT-001".to_string());
        assert!(validate_tracking_requirements(TrackingMode::Lot, &req).is_ok());
    }

    #[test]
    fn tracking_serial_requires_serial_codes() {
        let req = valid_req();
        assert!(matches!(
            validate_tracking_requirements(TrackingMode::Serial, &req),
            Err(ReceiptError::SerialCodesRequired)
        ));
    }

    #[test]
    fn tracking_serial_rejects_count_mismatch() {
        let mut req = valid_req(); // quantity = 10
        req.serial_codes = Some(vec!["SN-001".to_string(), "SN-002".to_string()]); // len=2 != 10
        assert!(matches!(
            validate_tracking_requirements(TrackingMode::Serial, &req),
            Err(ReceiptError::SerialCountMismatch {
                expected: 10,
                got: 2
            })
        ));
    }

    #[test]
    fn tracking_serial_accepts_matching_count() {
        let mut req = valid_req(); // quantity = 10
        req.serial_codes = Some((0..10).map(|i| format!("SN-{:03}", i)).collect());
        assert!(validate_tracking_requirements(TrackingMode::Serial, &req).is_ok());
    }

    #[test]
    fn tracking_none_is_a_noop() {
        let req = valid_req();
        assert!(validate_tracking_requirements(TrackingMode::None, &req).is_ok());
    }
}
