/// Database query helpers for TTP billing runs.
///
/// Extracted from billing.rs to stay under the 500 LOC file size limit.
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::billing::{BillingError, PartyBillingWork};

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
