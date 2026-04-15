/// Repository layer for TTP billing runs.
///
/// All SQL queries for billing live here. Domain logic in `billing.rs`
/// delegates to these functions for database access.
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::billing::{BillingError, PartyBillingWork};

// ---------------------------------------------------------------------------
// Run lifecycle
// ---------------------------------------------------------------------------

/// Check if a billing run already exists for (tenant_id, billing_period).
///
/// Returns `(run_id, status)` if found.
pub async fn fetch_existing_run(
    pool: &PgPool,
    tenant_id: Uuid,
    billing_period: &str,
) -> Result<Option<(Uuid, String)>, BillingError> {
    let row = sqlx::query(
        "SELECT run_id, status FROM ttp_billing_runs WHERE tenant_id = $1 AND billing_period = $2",
    )
    .bind(tenant_id)
    .bind(billing_period)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            let run_id: Uuid = r.try_get("run_id")?;
            let status: String = r.try_get("status")?;
            Ok(Some((run_id, status)))
        }
        None => Ok(None),
    }
}

/// Insert a new billing run. ON CONFLICT DO NOTHING for race safety.
pub async fn insert_billing_run(
    pool: &PgPool,
    run_id: Uuid,
    tenant_id: Uuid,
    billing_period: &str,
    idempotency_key: &str,
) -> Result<(), BillingError> {
    sqlx::query(
        r#"
        INSERT INTO ttp_billing_runs (run_id, tenant_id, billing_period, status, idempotency_key)
        VALUES ($1, $2, $3, 'pending', $4)
        ON CONFLICT (tenant_id, billing_period) DO NOTHING
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .bind(billing_period)
    .bind(idempotency_key)
    .execute(pool)
    .await?;
    Ok(())
}

/// Re-fetch the canonical run_id after a potential conflict.
pub async fn fetch_canonical_run_id(
    pool: &PgPool,
    tenant_id: Uuid,
    billing_period: &str,
) -> Result<Uuid, BillingError> {
    let run_id: Uuid = sqlx::query_scalar(
        "SELECT run_id FROM ttp_billing_runs WHERE tenant_id = $1 AND billing_period = $2",
    )
    .bind(tenant_id)
    .bind(billing_period)
    .fetch_one(pool)
    .await?;
    Ok(run_id)
}

/// Transition a run to 'processing' (unless already completed).
pub async fn set_run_processing(pool: &PgPool, run_id: Uuid) -> Result<(), BillingError> {
    sqlx::query(
        "UPDATE ttp_billing_runs SET status = 'processing' WHERE run_id = $1 AND status != 'completed'",
    )
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark a run as completed.
pub async fn set_run_completed(pool: &PgPool, run_id: Uuid) -> Result<(), BillingError> {
    sqlx::query("UPDATE ttp_billing_runs SET status = 'completed' WHERE run_id = $1")
        .bind(run_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Run items
// ---------------------------------------------------------------------------

/// Check if a billing item already exists for (run_id, party_id).
///
/// Returns the item status if found (e.g. "invoiced").
pub async fn fetch_existing_item_status(
    pool: &PgPool,
    run_id: Uuid,
    party_id: Uuid,
) -> Result<Option<String>, BillingError> {
    let row =
        sqlx::query("SELECT status FROM ttp_billing_run_items WHERE run_id = $1 AND party_id = $2")
            .bind(run_id)
            .bind(party_id)
            .fetch_optional(pool)
            .await?;

    match row {
        Some(r) => Ok(Some(r.try_get("status")?)),
        None => Ok(None),
    }
}

/// Upsert a billing run item after invoicing a party.
pub async fn upsert_billing_item(
    pool: &PgPool,
    run_id: Uuid,
    party_id: Uuid,
    ar_invoice_id: Uuid,
    amount_minor: i64,
    currency: &str,
    trace_hash: &Option<String>,
) -> Result<(), BillingError> {
    sqlx::query(
        r#"
        INSERT INTO ttp_billing_run_items (run_id, party_id, ar_invoice_id, amount_minor, currency, status, trace_hash)
        VALUES ($1, $2, $3, $4, $5, 'invoiced', $6)
        ON CONFLICT (run_id, party_id) DO UPDATE
          SET ar_invoice_id = EXCLUDED.ar_invoice_id, status = 'invoiced', trace_hash = EXCLUDED.trace_hash
        "#,
    )
    .bind(run_id)
    .bind(party_id)
    .bind(ar_invoice_id)
    .bind(amount_minor)
    .bind(currency)
    .bind(trace_hash)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark one-time charges as billed after AR invoice finalization.
pub async fn mark_charges_billed(
    pool: &PgPool,
    ar_invoice_id: Uuid,
    charge_ids: &[Uuid],
    tenant_id: Uuid,
) -> Result<(), BillingError> {
    sqlx::query(
        r#"
        UPDATE ttp_one_time_charges
        SET status = 'billed', ar_invoice_id = $1
        WHERE charge_id = ANY($2) AND tenant_id = $3 AND status = 'pending'
        "#,
    )
    .bind(ar_invoice_id)
    .bind(charge_ids)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Party collection
// ---------------------------------------------------------------------------

/// Collect all parties that still need to be billed in this run.
///
/// Merges parties with active service agreements AND parties with pending one-time
/// charges. Excludes parties already marked as invoiced in this run.
pub async fn collect_parties_to_bill(
    pool: &PgPool,
    tenant_id: Uuid,
    run_id: Uuid,
) -> Result<Vec<PartyBillingWork>, BillingError> {
    // Parties with active agreements
    let agreement_rows = sqlx::query(
        r#"
        SELECT a.party_id, SUM(a.amount_minor) AS amount_minor, a.currency
        FROM ttp_service_agreements a
        WHERE a.tenant_id = $1 AND a.status = 'active'
        GROUP BY a.party_id, a.currency
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    // Parties with pending one-time charges
    let charge_rows = sqlx::query(
        r#"
        SELECT c.party_id, SUM(c.amount_minor) AS amount_minor, c.currency,
               ARRAY_AGG(c.charge_id) AS charge_ids
        FROM ttp_one_time_charges c
        WHERE c.tenant_id = $1 AND c.status = 'pending'
        GROUP BY c.party_id, c.currency
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    // Already-invoiced parties in this run — skip them
    let invoiced_parties: Vec<Uuid> = sqlx::query_scalar(
        "SELECT party_id FROM ttp_billing_run_items \
         WHERE run_id = $1 AND status = 'invoiced' \
           AND run_id IN (SELECT run_id FROM ttp_billing_runs WHERE tenant_id = $2)",
    )
    .bind(run_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    // Merge agreement and charge amounts per party
    let mut work_map: HashMap<Uuid, PartyBillingWork> = HashMap::new();

    for row in &agreement_rows {
        let party_id: Uuid = row.try_get("party_id")?;
        if invoiced_parties.contains(&party_id) {
            continue;
        }
        let amount: i64 = row.try_get::<i64, _>("amount_minor")?;
        let currency: String = row.try_get("currency")?;

        work_map
            .entry(party_id)
            .or_insert_with(|| PartyBillingWork {
                party_id,
                total_amount_minor: 0,
                currency: currency.clone(),
                charge_ids: vec![],
                trace_hash: None,
            })
            .total_amount_minor += amount;
    }

    for row in &charge_rows {
        let party_id: Uuid = row.try_get("party_id")?;
        if invoiced_parties.contains(&party_id) {
            continue;
        }
        let amount: i64 = row.try_get::<i64, _>("amount_minor")?;
        let currency: String = row.try_get("currency")?;
        let charge_ids: Vec<Uuid> = row.try_get("charge_ids")?;

        let entry = work_map
            .entry(party_id)
            .or_insert_with(|| PartyBillingWork {
                party_id,
                total_amount_minor: 0,
                currency: currency.clone(),
                charge_ids: vec![],
                trace_hash: None,
            });
        entry.total_amount_minor += amount;
        entry.charge_ids.extend(charge_ids);
    }

    Ok(work_map.into_values().collect())
}

/// Fetch summary (parties_billed, total_amount, currency) for a completed run.
pub async fn fetch_run_summary(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<(u32, i64, String), BillingError> {
    let row = sqlx::query(
        r#"
        SELECT COUNT(*) AS parties_billed,
               COALESCE(SUM(amount_minor), 0) AS total_amount_minor,
               COALESCE(MAX(currency), 'usd') AS currency
        FROM ttp_billing_run_items
        WHERE run_id = $1 AND status = 'invoiced'
        "#,
    )
    .bind(run_id)
    .fetch_one(pool)
    .await?;

    let count: i64 = row.try_get("parties_billed")?;
    let total: i64 = row.try_get("total_amount_minor")?;
    let currency: String = row.try_get("currency")?;

    Ok((count as u32, total, currency))
}
