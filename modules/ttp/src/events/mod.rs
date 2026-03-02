/// TTP event definitions and envelope helpers.
///
/// All TTP events are tenant-scoped: `merchant_context = TENANT(tenant_id)`.
/// This enforces money-mixing prevention — TTP processes revenue on behalf
/// of tenants, never on behalf of the platform itself.
use event_bus::{EventEnvelope, MerchantContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Event payload types
// ---------------------------------------------------------------------------

/// Emitted when a billing run is created (status = pending).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingRunCreated {
    pub run_id: Uuid,
    pub tenant_id: Uuid,
    pub billing_period: String,
    pub idempotency_key: String,
}

/// Emitted when a billing run completes successfully.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingRunCompleted {
    pub run_id: Uuid,
    pub tenant_id: Uuid,
    pub billing_period: String,
    pub parties_billed: u32,
    pub total_amount_minor: i64,
    pub currency: String,
}

/// Emitted when a single party has been invoiced within a billing run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyInvoiced {
    pub run_id: Uuid,
    pub tenant_id: Uuid,
    pub party_id: Uuid,
    pub ar_invoice_id: i32,
    pub amount_minor: i64,
    pub currency: String,
}

/// Emitted when a billing run fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingRunFailed {
    pub run_id: Uuid,
    pub tenant_id: Uuid,
    pub billing_period: String,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Envelope helpers — all carry merchant_context = TENANT(tenant_id)
// ---------------------------------------------------------------------------

/// Wrap a TTP event payload in a platform EventEnvelope with TENANT context.
///
/// All TTP events MUST use this helper (or `create_ttp_envelope_with_actor`)
/// to ensure `merchant_context = TENANT(tenant_id)` is always set.
pub fn create_ttp_envelope<T: Serialize>(
    tenant_id: Uuid,
    event_type: impl Into<String>,
    correlation_id: impl Into<String>,
    mutation_class: impl Into<String>,
    payload: T,
) -> EventEnvelope<T> {
    let tenant_str = tenant_id.to_string();
    let corr = correlation_id.into();
    let merchant_ctx = MerchantContext::Tenant(tenant_str.clone());

    EventEnvelope::new(
        tenant_str.clone(),
        "ttp".to_string(),
        event_type.into(),
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_trace_id(Some(corr.clone()))
    .with_correlation_id(Some(corr))
    .with_mutation_class(Some(mutation_class.into()))
    .with_merchant_context(Some(merchant_ctx))
}

// ---------------------------------------------------------------------------
// Subject helpers
// ---------------------------------------------------------------------------

pub const BILLING_RUN_CREATED: &str = "ttp.billing_run.created";
pub const BILLING_RUN_COMPLETED: &str = "ttp.billing_run.completed";
pub const BILLING_RUN_FAILED: &str = "ttp.billing_run.failed";
pub const PARTY_INVOICED: &str = "ttp.party.invoiced";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_carries_tenant_merchant_context() {
        let tenant_id = Uuid::new_v4();
        let payload = BillingRunCreated {
            run_id: Uuid::new_v4(),
            tenant_id,
            billing_period: "2026-02".to_string(),
            idempotency_key: "key-abc".to_string(),
        };

        let env = create_ttp_envelope(
            tenant_id,
            BILLING_RUN_CREATED,
            "corr-123",
            "billing",
            payload,
        );

        assert_eq!(
            env.merchant_context,
            Some(MerchantContext::Tenant(tenant_id.to_string())),
            "merchant_context must be TENANT(tenant_id)"
        );
        assert_eq!(env.tenant_id, tenant_id.to_string());
        assert_eq!(env.source_module, "ttp");
    }

    #[test]
    fn all_event_variants_serialize_without_error() {
        let tid = Uuid::new_v4();
        let rid = Uuid::new_v4();
        let pid = Uuid::new_v4();

        let _ = serde_json::to_string(&BillingRunCreated {
            run_id: rid,
            tenant_id: tid,
            billing_period: "2026-02".to_string(),
            idempotency_key: "k".to_string(),
        })
        .unwrap();

        let _ = serde_json::to_string(&BillingRunCompleted {
            run_id: rid,
            tenant_id: tid,
            billing_period: "2026-02".to_string(),
            parties_billed: 3,
            total_amount_minor: 30000,
            currency: "usd".to_string(),
        })
        .unwrap();

        let _ = serde_json::to_string(&PartyInvoiced {
            run_id: rid,
            tenant_id: tid,
            party_id: pid,
            ar_invoice_id: 42,
            amount_minor: 10000,
            currency: "usd".to_string(),
        })
        .unwrap();

        let _ = serde_json::to_string(&BillingRunFailed {
            run_id: rid,
            tenant_id: tid,
            billing_period: "2026-02".to_string(),
            reason: "AR unavailable".to_string(),
        })
        .unwrap();
    }
}
