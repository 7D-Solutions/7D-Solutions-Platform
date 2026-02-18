//! Intercompany matching orchestration.
//!
//! Fetches per-entity trial balances and applies elimination rules
//! to produce stable match suggestions. No DB writes.

use std::collections::HashSet;

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::{matching, EntityAccountBalance, IntercompanyMatchResult};
use crate::domain::config::{self, EliminationRule};
use crate::domain::engine::EngineError;
use crate::integrations::gl::client::GlClient;

/// Run intercompany matching for a consolidation group.
///
/// Fetches per-entity trial balances and applies elimination rules
/// to produce stable match suggestions. No DB writes.
pub async fn match_intercompany_for_group(
    pool: &PgPool,
    gl_client: &GlClient,
    tenant_id: &str,
    group_id: Uuid,
    period_id: Uuid,
    as_of: NaiveDate,
) -> Result<IntercompanyMatchResult, EngineError> {
    let entities = config::service::list_entities(pool, tenant_id, group_id, false).await?;
    let elim_rules =
        config::service_rules::list_elimination_rules(pool, tenant_id, group_id, false).await?;
    let coa_mappings =
        config::service::list_coa_mappings(pool, tenant_id, group_id, None).await?;

    let target_accounts = collect_target_accounts(&elim_rules);
    let mut all_balances: Vec<EntityAccountBalance> = Vec::new();

    for entity in &entities {
        let tb = gl_client
            .get_trial_balance(
                &entity.entity_tenant_id,
                period_id,
                &entity.functional_currency,
            )
            .await?;

        let entity_mappings: Vec<_> = coa_mappings
            .iter()
            .filter(|m| m.entity_tenant_id == entity.entity_tenant_id)
            .collect();

        for row in &tb.rows {
            let target_code = entity_mappings
                .iter()
                .find(|m| m.source_account_code == row.account_code)
                .map(|m| m.target_account_code.clone())
                .unwrap_or_else(|| row.account_code.clone());

            if target_accounts.contains(&target_code) {
                all_balances.push(EntityAccountBalance {
                    entity_tenant_id: entity.entity_tenant_id.clone(),
                    account_code: target_code,
                    account_name: row.account_name.clone(),
                    debit_minor: row.debit_total_minor,
                    credit_minor: row.credit_total_minor,
                    net_minor: row.net_balance_minor,
                });
            }
        }
    }

    let matches = matching::match_intercompany(&elim_rules, &all_balances);
    let total_matched: i64 = matches.iter().map(|m| m.match_amount_minor).sum();
    let unmatched = matches
        .iter()
        .filter(|m| m.debit_unmatched_minor > 0 || m.credit_unmatched_minor > 0)
        .count();

    Ok(IntercompanyMatchResult {
        group_id,
        as_of,
        matches,
        unmatched_count: unmatched,
        total_matched_minor: total_matched,
    })
}

/// Collect all account codes referenced by elimination rules.
fn collect_target_accounts(rules: &[EliminationRule]) -> HashSet<String> {
    let mut accounts = HashSet::new();
    for rule in rules {
        accounts.insert(rule.debit_account_code.clone());
        accounts.insert(rule.credit_account_code.clone());
    }
    accounts
}
