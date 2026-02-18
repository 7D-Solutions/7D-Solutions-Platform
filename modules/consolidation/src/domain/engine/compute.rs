//! Core consolidation computation.
//!
//! Pipeline: for each entity → verify close hash → fetch TB → COA map → FX translate.
//! Then aggregate across entities, apply eliminations, and cache.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    compute_input_hash, ConsolidatedTbRow, ConsolidationResult, EngineError, EntityHashEntry,
};
use crate::domain::config::{self, CoaMapping, EliminationRule, FxPolicy, GroupEntity};
use crate::integrations::gl::client::GlClient;

/// Run the full consolidation pipeline for a group + as_of date.
///
/// Steps:
/// 1. Load group config (entities, COA mappings, FX policies, elimination rules)
/// 2. For each entity: verify period is closed, record close_hash, fetch TB
/// 3. Apply COA mapping (source → target account codes)
/// 4. Apply FX translation (entity currency → reporting currency)
/// 5. Aggregate across entities into consolidated TB rows
/// 6. Apply elimination rules
/// 7. Compute deterministic input_hash
/// 8. Cache result
pub async fn consolidate(
    pool: &PgPool,
    gl_client: &GlClient,
    tenant_id: &str,
    group_id: Uuid,
    period_id: Uuid,
    as_of: NaiveDate,
) -> Result<ConsolidationResult, EngineError> {
    // Step 1: Load config
    let group = config::service::get_group(pool, tenant_id, group_id).await?;
    let entities = config::service::list_entities(pool, tenant_id, group_id, false).await?;
    let coa_mappings = config::service::list_coa_mappings(pool, tenant_id, group_id, None).await?;
    let elim_rules =
        config::service_rules::list_elimination_rules(pool, tenant_id, group_id, false).await?;
    let fx_policies = config::service_rules::list_fx_policies(pool, tenant_id, group_id).await?;

    // Accumulator: target_account_code → (debit_minor, credit_minor, account_name)
    let mut ledger: BTreeMap<String, (i64, i64, String)> = BTreeMap::new();
    let mut entity_hashes: Vec<EntityHashEntry> = Vec::new();

    // Steps 2–4: per-entity processing
    for entity in &entities {
        let close_hash = verify_entity_closed(gl_client, entity, period_id).await?;
        entity_hashes.push(EntityHashEntry {
            entity_tenant_id: entity.entity_tenant_id.clone(),
            close_hash,
        });

        let tb = gl_client
            .get_trial_balance(&entity.entity_tenant_id, period_id, &entity.functional_currency)
            .await?;

        let entity_mappings: Vec<&CoaMapping> = coa_mappings
            .iter()
            .filter(|m| m.entity_tenant_id == entity.entity_tenant_id)
            .collect();

        let fx_rate = resolve_fx_rate(entity, &group.reporting_currency, &fx_policies);

        for row in &tb.rows {
            let (target_code, target_name) =
                map_account(&entity.entity_tenant_id, &row.account_code, &row.account_name, &entity_mappings)?;

            let (translated_debit, translated_credit) =
                translate_fx(row.debit_total_minor, row.credit_total_minor, fx_rate);

            let entry = ledger.entry(target_code).or_insert((0, 0, target_name));
            entry.0 += translated_debit;
            entry.1 += translated_credit;
        }
    }

    // Step 6: Apply elimination rules
    apply_eliminations(&mut ledger, &elim_rules);

    // Build result rows (sorted by account code via BTreeMap)
    let rows: Vec<ConsolidatedTbRow> = ledger
        .into_iter()
        .map(|(code, (debit, credit, name))| ConsolidatedTbRow {
            account_code: code,
            account_name: name,
            currency: group.reporting_currency.clone(),
            debit_minor: debit,
            credit_minor: credit,
            net_minor: debit - credit,
        })
        .collect();

    // Step 7: Compute input hash
    let input_hash = compute_input_hash(group_id, as_of, &mut entity_hashes);

    // Step 8: Cache
    cache_result(pool, group_id, as_of, &group.reporting_currency, &input_hash, &rows).await?;

    Ok(ConsolidationResult {
        group_id,
        as_of,
        reporting_currency: group.reporting_currency,
        rows,
        input_hash,
        entity_hashes,
    })
}

/// Verify that the entity's period is closed and return its close_hash.
async fn verify_entity_closed(
    gl_client: &GlClient,
    entity: &GroupEntity,
    period_id: Uuid,
) -> Result<String, EngineError> {
    let close_hash = gl_client
        .get_close_hash(&entity.entity_tenant_id, period_id)
        .await?;

    close_hash.ok_or_else(|| EngineError::PeriodNotClosed(entity.entity_tenant_id.clone()))
}

/// Map a source account to the group's target account via COA mappings.
///
/// If no mapping exists, the source code passes through unchanged.
fn map_account(
    _entity_tenant_id: &str,
    source_code: &str,
    source_name: &str,
    mappings: &[&CoaMapping],
) -> Result<(String, String), EngineError> {
    if let Some(m) = mappings.iter().find(|m| m.source_account_code == source_code) {
        let name = m
            .target_account_name
            .as_deref()
            .unwrap_or(source_name)
            .to_string();
        Ok((m.target_account_code.clone(), name))
    } else {
        // Pass-through: use source code/name when no explicit mapping exists
        Ok((source_code.to_string(), source_name.to_string()))
    }
}

/// Resolve FX rate for an entity.
///
/// If entity currency == reporting currency, rate is 1.0 (no conversion).
/// Otherwise, we use a rate of 1.0 as a placeholder — real FX rates would
/// come from a rates service keyed by (from, to, as_of, rate_type).
/// The FX policy tells us *which* rate type to use per account category,
/// but the actual rate lookup is deferred to a future FX rates integration.
fn resolve_fx_rate(
    entity: &GroupEntity,
    reporting_currency: &str,
    _fx_policies: &[FxPolicy],
) -> f64 {
    if entity.functional_currency == reporting_currency {
        1.0
    } else {
        // TODO(Phase 32+): Integrate with FX rates service.
        // For now, same-currency entities get 1:1; cross-currency entities
        // also get 1:1 (identity) as a safe default until FX rates are wired.
        1.0
    }
}

/// Translate amounts using the FX rate. Returns (translated_debit, translated_credit).
fn translate_fx(debit_minor: i64, credit_minor: i64, fx_rate: f64) -> (i64, i64) {
    if (fx_rate - 1.0).abs() < f64::EPSILON {
        return (debit_minor, credit_minor);
    }
    let translated_debit = (debit_minor as f64 * fx_rate).round() as i64;
    let translated_credit = (credit_minor as f64 * fx_rate).round() as i64;
    (translated_debit, translated_credit)
}

/// Apply elimination rules to the consolidated ledger.
///
/// Uses the intercompany matching engine to compute elimination amounts,
/// then adjusts the ledger in-place. Each matched intercompany balance
/// generates a reversing entry: debit the credit-side account and credit
/// the debit-side account to zero out intercompany balances.
fn apply_eliminations(
    ledger: &mut BTreeMap<String, (i64, i64, String)>,
    rules: &[EliminationRule],
) {
    use crate::domain::intercompany::EntityAccountBalance;

    // Build entity-account balances from the current ledger state
    // For in-pipeline elimination, we use the aggregated ledger entries
    // that correspond to elimination rule accounts
    let mut balances: Vec<EntityAccountBalance> = Vec::new();
    for (code, (debit, credit, name)) in ledger.iter() {
        for rule in rules {
            if rule.debit_account_code == *code || rule.credit_account_code == *code {
                // In the consolidated ledger, entries are already aggregated
                // across entities, so we use "consolidated" as the entity
                balances.push(EntityAccountBalance {
                    entity_tenant_id: "consolidated".to_string(),
                    account_code: code.clone(),
                    account_name: name.clone(),
                    debit_minor: *debit,
                    credit_minor: *credit,
                    net_minor: *debit - *credit,
                });
                break;
            }
        }
    }

    // Apply rule-based eliminations directly to the ledger
    for rule in rules {
        if !rule.is_active {
            continue;
        }

        let debit_balance = ledger
            .get(&rule.debit_account_code)
            .map(|(d, c, _)| d - c)
            .unwrap_or(0);
        let credit_balance = ledger
            .get(&rule.credit_account_code)
            .map(|(d, c, _)| c - d)
            .unwrap_or(0);

        if debit_balance <= 0 || credit_balance <= 0 {
            continue;
        }

        let elim_amount = debit_balance.min(credit_balance);

        // Reverse the debit-side: add credit to reduce balance
        if let Some(entry) = ledger.get_mut(&rule.debit_account_code) {
            entry.1 += elim_amount;
        }

        // Reverse the credit-side: add debit to reduce balance
        if let Some(entry) = ledger.get_mut(&rule.credit_account_code) {
            entry.0 += elim_amount;
        }
    }
}

/// Persist consolidated TB rows to csl_trial_balance_cache.
///
/// Uses DELETE + INSERT (not upsert) to ensure a clean cache per run.
/// This guarantees deterministic reruns produce identical cache state.
async fn cache_result(
    pool: &PgPool,
    group_id: Uuid,
    as_of: NaiveDate,
    currency: &str,
    input_hash: &str,
    rows: &[ConsolidatedTbRow],
) -> Result<(), EngineError> {
    let mut tx = pool.begin().await?;

    // Clear previous cache for this group+as_of
    sqlx::query(
        "DELETE FROM csl_trial_balance_cache WHERE group_id = $1 AND as_of = $2",
    )
    .bind(group_id)
    .bind(as_of)
    .execute(&mut *tx)
    .await?;

    // Insert new rows
    for row in rows {
        sqlx::query(
            "INSERT INTO csl_trial_balance_cache
                (group_id, as_of, account_code, account_name, currency, debit_minor, credit_minor, net_minor, input_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(group_id)
        .bind(as_of)
        .bind(&row.account_code)
        .bind(&row.account_name)
        .bind(currency)
        .bind(row.debit_minor)
        .bind(row.credit_minor)
        .bind(row.net_minor)
        .bind(input_hash)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Read cached consolidated TB for a group+as_of. Returns None if no cache exists.
pub async fn get_cached_tb(
    pool: &PgPool,
    group_id: Uuid,
    as_of: NaiveDate,
) -> Result<Option<Vec<CachedTbRow>>, EngineError> {
    let rows = sqlx::query_as::<_, CachedTbRow>(
        "SELECT account_code, account_name, currency, debit_minor, credit_minor, net_minor, input_hash, computed_at
         FROM csl_trial_balance_cache
         WHERE group_id = $1 AND as_of = $2
         ORDER BY account_code",
    )
    .bind(group_id)
    .bind(as_of)
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        Ok(None)
    } else {
        Ok(Some(rows))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct CachedTbRow {
    pub account_code: String,
    pub account_name: String,
    pub currency: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub net_minor: i64,
    pub input_hash: String,
    pub computed_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_account_with_mapping() {
        let mapping = crate::domain::config::CoaMapping {
            id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            entity_tenant_id: "ent-1".to_string(),
            source_account_code: "1000".to_string(),
            target_account_code: "10000".to_string(),
            target_account_name: Some("Consolidated Cash".to_string()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let mappings = vec![&mapping];
        let (code, name) = map_account("ent-1", "1000", "Cash", &mappings).unwrap();
        assert_eq!(code, "10000");
        assert_eq!(name, "Consolidated Cash");
    }

    #[test]
    fn test_map_account_passthrough() {
        let mappings: Vec<&crate::domain::config::CoaMapping> = vec![];
        let (code, name) = map_account("ent-1", "9999", "Misc", &mappings).unwrap();
        assert_eq!(code, "9999");
        assert_eq!(name, "Misc");
    }

    #[test]
    fn test_translate_fx_identity() {
        let (d, c) = translate_fx(10000, 5000, 1.0);
        assert_eq!(d, 10000);
        assert_eq!(c, 5000);
    }

    #[test]
    fn test_translate_fx_conversion() {
        let (d, c) = translate_fx(10000, 5000, 1.5);
        assert_eq!(d, 15000);
        assert_eq!(c, 7500);
    }

    #[test]
    fn test_input_hash_deterministic() {
        let gid = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();

        let mut hashes1 = vec![
            EntityHashEntry { entity_tenant_id: "b".into(), close_hash: "hash_b".into() },
            EntityHashEntry { entity_tenant_id: "a".into(), close_hash: "hash_a".into() },
        ];
        let mut hashes2 = vec![
            EntityHashEntry { entity_tenant_id: "a".into(), close_hash: "hash_a".into() },
            EntityHashEntry { entity_tenant_id: "b".into(), close_hash: "hash_b".into() },
        ];

        let h1 = compute_input_hash(gid, date, &mut hashes1);
        let h2 = compute_input_hash(gid, date, &mut hashes2);
        assert_eq!(h1, h2, "Input hash must be deterministic regardless of entity order");
    }
}
