/// TTP billing run domain logic.
///
/// # Idempotency model
///
/// - One billing run per (tenant_id, billing_period) — enforced by UNIQUE constraint.
/// - One billing run item per (run_id, party_id) — enforced by UNIQUE constraint.
/// - If a run already exists (status = completed) for the period, the call is a no-op.
/// - One-time charges are marked billed only AFTER the AR invoice is finalized,
///   preventing partial double-billing if the process crashes mid-run.
///
/// # Revenue path
///
/// For each party with an active service agreement or pending charges:
///   1. Look up (or create) AR customer record by party_id.
///   2. Compute total amount (agreement amount + sum of pending charges).
///   3. Create a draft AR invoice with idempotency key = sha256(run_id || party_id).
///   4. Finalize the AR invoice (draft → open).
///   5. Mark one-time charges as billed with ar_invoice_id.
///   6. Upsert a billing run item with status = invoiced.
use platform_sdk::{PlatformClient, VerifiedClaims};
use sqlx::PgPool;
use uuid::Uuid;

use super::billing_repo;
use super::metering;
use crate::clients::ar::{ArClient, ArClientError};
use crate::clients::tenant_registry::{TenantRegistryClient, TenantRegistryError};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("tenant-registry error: {0}")]
    Registry(#[from] TenantRegistryError),

    #[error("AR client error: {0}")]
    Ar(#[from] ArClientError),

    #[error("metering error: {0}")]
    Metering(#[from] metering::MeteringError),
}

// ---------------------------------------------------------------------------
// Value objects
// ---------------------------------------------------------------------------

/// A party that needs to be billed in this run.
pub struct PartyBillingWork {
    pub party_id: Uuid,
    /// Total amount to invoice (agreement + pending charges + metering), minor units.
    pub total_amount_minor: i64,
    /// Currency code (e.g. "usd").
    pub currency: String,
    /// Pending one-time charge IDs to mark billed.
    pub charge_ids: Vec<Uuid>,
    /// SHA-256 hash of the metering PriceTrace, if metered usage is included.
    pub trace_hash: Option<String>,
}

/// Summary returned after a run completes.
#[derive(Debug)]
pub struct BillingRunSummary {
    pub run_id: Uuid,
    pub parties_billed: u32,
    pub total_amount_minor: i64,
    pub currency: String,
    /// `true` if the run was already completed (idempotent no-op).
    pub was_noop: bool,
}

// ---------------------------------------------------------------------------
// Core function
// ---------------------------------------------------------------------------

/// Execute a billing run for a tenant + period.
///
/// Idempotent: calling with the same (tenant_id, billing_period) again returns
/// the existing summary without creating new invoices.
pub async fn run_billing(
    pool: &PgPool,
    registry_client: &TenantRegistryClient,
    ar_client: &ArClient,
    claims: &VerifiedClaims,
    tenant_id: Uuid,
    billing_period: &str,
    idempotency_key: &str,
) -> Result<BillingRunSummary, BillingError> {
    // 1. Idempotency check: has this period already been billed?
    if let Some((run_id, status)) =
        billing_repo::fetch_existing_run(pool, tenant_id, billing_period).await?
    {
        if status == "completed" {
            let (parties_billed, total_amount_minor, currency) =
                billing_repo::fetch_run_summary(pool, run_id).await?;
            return Ok(BillingRunSummary {
                run_id,
                parties_billed,
                total_amount_minor,
                currency,
                was_noop: true,
            });
        }

        if status == "processing" || status == "failed" {
            return execute_run(pool, ar_client, tenant_id, run_id, billing_period).await;
        }
    }

    // 2. Resolve tenant_id → app_id (fail-closed: abort if registry is unreachable)
    let _app_id = registry_client.get_app_id(claims, tenant_id).await?;

    // 3. Create the billing run record (UNIQUE on tenant+period prevents races)
    let run_id = Uuid::new_v4();
    billing_repo::insert_billing_run(pool, run_id, tenant_id, billing_period, idempotency_key)
        .await?;

    // Re-fetch canonical run_id (another writer may have won the conflict)
    let canonical =
        billing_repo::fetch_canonical_run_id(pool, tenant_id, billing_period).await?;

    execute_run(pool, ar_client, tenant_id, canonical, billing_period).await
}

// ---------------------------------------------------------------------------
// Inner execution
// ---------------------------------------------------------------------------

/// Drive billing once we have a confirmed run_id.
async fn execute_run(
    pool: &PgPool,
    ar_client: &ArClient,
    tenant_id: Uuid,
    run_id: Uuid,
    billing_period: &str,
) -> Result<BillingRunSummary, BillingError> {
    billing_repo::set_run_processing(pool, run_id).await?;

    let mut parties = billing_repo::collect_parties_to_bill(pool, tenant_id, run_id).await?;

    // Compute metering trace and add metered usage as a billing item
    let trace = metering::compute_price_trace(pool, tenant_id, billing_period).await?;
    if trace.total_minor > 0 {
        let hash = compute_trace_hash(&trace);
        parties.push(PartyBillingWork {
            party_id: tenant_id,
            total_amount_minor: trace.total_minor,
            currency: trace.currency.clone(),
            charge_ids: vec![],
            trace_hash: Some(hash),
        });
    }

    let mut parties_billed: u32 = 0;
    let mut total_amount_minor: i64 = 0;
    let mut run_currency = "usd".to_string();

    for party in &parties {
        if party.total_amount_minor == 0 {
            continue;
        }

        let item_key = derive_item_key(run_id, party.party_id);

        // Skip already-invoiced items (re-entry after crash)
        if let Some(item_status) =
            billing_repo::fetch_existing_item_status(pool, run_id, party.party_id).await?
        {
            if item_status == "invoiced" {
                parties_billed += 1;
                total_amount_minor += party.total_amount_minor;
                run_currency = party.currency.clone();
                continue;
            }
        }

        bill_party(
            pool,
            ar_client,
            tenant_id,
            run_id,
            party,
            &item_key,
            &mut parties_billed,
            &mut total_amount_minor,
            &mut run_currency,
        )
        .await?;
    }

    billing_repo::set_run_completed(pool, run_id).await?;

    Ok(BillingRunSummary {
        run_id,
        parties_billed,
        total_amount_minor,
        currency: run_currency,
        was_noop: false,
    })
}

/// Invoice a single party: create + finalize AR invoice, mark charges billed.
#[allow(clippy::too_many_arguments)]
async fn bill_party(
    pool: &PgPool,
    ar_client: &ArClient,
    tenant_id: Uuid,
    run_id: Uuid,
    party: &PartyBillingWork,
    item_key: &str,
    parties_billed: &mut u32,
    total_amount_minor: &mut i64,
    run_currency: &mut String,
) -> Result<(), BillingError> {
    let claims = PlatformClient::service_claims(tenant_id);
    let email = format!("party-{}@tenant.internal", party.party_id);
    let ar_customer_id = ar_client
        .find_or_create_customer(&claims, party.party_id, &email)
        .await?;

    let invoice = ar_client
        .create_invoice(
            &claims,
            ar_customer_id,
            party.total_amount_minor,
            &party.currency,
            item_key,
            party.party_id,
        )
        .await?;

    let finalized = ar_client.finalize_invoice(&claims, invoice.id).await?;

    let ar_invoice_uuid = Uuid::new_v5(&Uuid::NAMESPACE_OID, finalized.id.to_string().as_bytes());

    billing_repo::upsert_billing_item(
        pool,
        run_id,
        party.party_id,
        ar_invoice_uuid,
        party.total_amount_minor,
        &party.currency,
        &party.trace_hash,
    )
    .await?;

    if !party.charge_ids.is_empty() {
        billing_repo::mark_charges_billed(pool, ar_invoice_uuid, &party.charge_ids, tenant_id)
            .await?;
    }

    *parties_billed += 1;
    *total_amount_minor += party.total_amount_minor;
    *run_currency = party.currency.clone();
    Ok(())
}

// ---------------------------------------------------------------------------
// Key derivation
// ---------------------------------------------------------------------------

/// Derive a deterministic idempotency key for a (run, party) pair.
///
/// Uses SHA-256 of "{run_id}/{party_id}" as a hex string.
pub fn derive_item_key(run_id: Uuid, party_id: Uuid) -> String {
    use sha2::{Digest, Sha256};
    let input = format!("{}/{}", run_id, party_id);
    let digest = Sha256::digest(input.as_bytes());
    format!("{:x}", digest)
}

/// Compute a deterministic SHA-256 hash of a PriceTrace.
///
/// The trace's line_items are already sorted by dimension, so JSON serialization
/// produces a stable string. Same trace inputs → same hash, always.
pub fn compute_trace_hash(trace: &metering::PriceTrace) -> String {
    use sha2::{Digest, Sha256};
    let json = serde_json::to_string(trace).expect("PriceTrace serializes to JSON");
    let digest = Sha256::digest(json.as_bytes());
    format!("{:x}", digest)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_item_key_is_deterministic() {
        let run_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("valid uuid");
        let party_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").expect("valid uuid");
        let key1 = derive_item_key(run_id, party_id);
        let key2 = derive_item_key(run_id, party_id);
        assert_eq!(key1, key2, "key must be deterministic");
        assert_eq!(key1.len(), 64, "sha256 hex is 64 chars");
    }

    #[test]
    fn derive_item_key_differs_by_party() {
        let run_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("valid uuid");
        let party_a = Uuid::parse_str("00000000-0000-0000-0000-000000000002").expect("valid uuid");
        let party_b = Uuid::parse_str("00000000-0000-0000-0000-000000000003").expect("valid uuid");
        assert_ne!(
            derive_item_key(run_id, party_a),
            derive_item_key(run_id, party_b)
        );
    }

    #[test]
    fn derive_item_key_unit_test_passes_without_db() {
        let run_id = Uuid::new_v4();
        let party_id = Uuid::new_v4();
        let key = derive_item_key(run_id, party_id);
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn compute_trace_hash_is_deterministic() {
        use chrono::NaiveDate;
        use metering::{PriceTrace, TraceLineItem};

        let tenant_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").expect("valid uuid");
        let trace = PriceTrace {
            tenant_id,
            period: "2026-02".to_string(),
            period_start: NaiveDate::from_ymd_opt(2026, 2, 1).expect("valid date"),
            period_end: NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date"),
            line_items: vec![
                TraceLineItem {
                    dimension: "api_calls".to_string(),
                    total_quantity: 175,
                    event_count: 3,
                    unit_price_minor: 10,
                    currency: "usd".to_string(),
                    line_total_minor: 1750,
                },
                TraceLineItem {
                    dimension: "storage_gb".to_string(),
                    total_quantity: 5,
                    event_count: 1,
                    unit_price_minor: 500,
                    currency: "usd".to_string(),
                    line_total_minor: 2500,
                },
            ],
            total_minor: 4250,
            currency: "usd".to_string(),
        };

        let hash1 = compute_trace_hash(&trace);
        let hash2 = compute_trace_hash(&trace);
        assert_eq!(hash1, hash2, "trace hash must be deterministic");
        assert_eq!(hash1.len(), 64, "sha256 hex is 64 chars");
    }
}
