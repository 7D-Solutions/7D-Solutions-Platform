//! Elimination suggestion generation and GL posting.
//!
//! Produces elimination journal suggestions from intercompany matches
//! and posts them to GL with exactly-once idempotency per group+period.

use chrono::{NaiveDate, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::{EliminationPostResult, EliminationSuggestion};
use crate::domain::engine::EngineError;
use crate::domain::intercompany::IntercompanyMatchResult;
use crate::integrations::gl::client::GlClient;

/// Generate elimination suggestions from intercompany matches.
///
/// Each suggestion reverses an intercompany balance: debits the
/// credit-side account and credits the debit-side account to
/// eliminate the intercompany balances from the consolidated view.
pub fn suggest_eliminations(match_result: &IntercompanyMatchResult) -> Vec<EliminationSuggestion> {
    let mut suggestions = Vec::new();

    for m in &match_result.matches {
        if m.match_amount_minor <= 0 {
            continue;
        }

        suggestions.push(EliminationSuggestion {
            rule_id: m.rule_id,
            rule_name: m.rule_name.clone(),
            rule_type: m.rule_type.clone(),
            entity_a_tenant_id: m.entity_a_tenant_id.clone(),
            entity_b_tenant_id: m.entity_b_tenant_id.clone(),
            // Elimination reverses: debit the credit account, credit the debit account
            debit_account_code: m.credit_account_code.clone(),
            credit_account_code: m.debit_account_code.clone(),
            amount_minor: m.match_amount_minor,
            description: format!(
                "Elimination [{}]: {} <-> {}",
                m.rule_name, m.entity_a_tenant_id, m.entity_b_tenant_id
            ),
        });
    }

    suggestions
}

/// Compute a deterministic idempotency key for elimination posting.
///
/// SHA-256(group_id | period_id | as_of | sorted suggestion fingerprints)
pub fn compute_idempotency_key(
    group_id: Uuid,
    period_id: Uuid,
    as_of: NaiveDate,
    suggestions: &[EliminationSuggestion],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(group_id.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(period_id.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(as_of.to_string().as_bytes());

    for s in suggestions {
        hasher.update(b"|");
        hasher.update(s.rule_id.to_string().as_bytes());
        hasher.update(b":");
        hasher.update(s.entity_a_tenant_id.as_bytes());
        hasher.update(b":");
        hasher.update(s.entity_b_tenant_id.as_bytes());
        hasher.update(b":");
        hasher.update(s.amount_minor.to_string().as_bytes());
    }

    format!("{:x}", hasher.finalize())
}

/// Post elimination journals to GL. Exactly-once per group+period+key.
///
/// If journals were already posted with the same idempotency key, returns
/// the existing result without re-posting.
pub async fn post_eliminations(
    pool: &PgPool,
    gl_client: &GlClient,
    tenant_id: &str,
    group_id: Uuid,
    period_id: Uuid,
    as_of: NaiveDate,
    suggestions: &[EliminationSuggestion],
    reporting_currency: &str,
) -> Result<EliminationPostResult, EngineError> {
    if suggestions.is_empty() {
        return Ok(EliminationPostResult {
            group_id,
            period_id,
            as_of,
            posted_count: 0,
            idempotency_key: String::new(),
            journal_entry_ids: Vec::new(),
            posted_at: Utc::now(),
            already_posted: false,
        });
    }

    let idempotency_key = compute_idempotency_key(group_id, period_id, as_of, suggestions);

    // Check existing posting (exactly-once guard)
    if let Some(result) =
        check_existing_posting(pool, group_id, period_id, &idempotency_key).await?
    {
        return Ok(result);
    }

    // Post each suggestion as a journal entry via GL
    let mut journal_entry_ids = Vec::new();
    let total_amount: i64 = suggestions.iter().map(|s| s.amount_minor).sum();

    for (idx, suggestion) in suggestions.iter().enumerate() {
        let source_doc_id = format!("elim-{}-{}-{}", group_id, period_id, idx);

        let entry_id = gl_client
            .post_elimination_journal(
                tenant_id,
                &as_of.to_string(),
                reporting_currency,
                &suggestion.debit_account_code,
                &suggestion.credit_account_code,
                suggestion.amount_minor,
                &suggestion.description,
                &source_doc_id,
            )
            .await?;

        journal_entry_ids.push(entry_id);
    }

    let now = Utc::now();

    // Record the posting for idempotency
    record_posting(
        pool,
        group_id,
        period_id,
        &idempotency_key,
        &journal_entry_ids,
        suggestions.len(),
        total_amount,
        now,
    )
    .await?;

    Ok(EliminationPostResult {
        group_id,
        period_id,
        as_of,
        posted_count: journal_entry_ids.len(),
        idempotency_key,
        journal_entry_ids,
        posted_at: now,
        already_posted: false,
    })
}

/// Check if eliminations have already been posted for this key.
async fn check_existing_posting(
    pool: &PgPool,
    group_id: Uuid,
    period_id: Uuid,
    idempotency_key: &str,
) -> Result<Option<EliminationPostResult>, EngineError> {
    let row = sqlx::query_as::<_, (chrono::DateTime<Utc>, serde_json::Value, chrono::NaiveDate)>(
        "SELECT posted_at, journal_entry_ids, (posted_at::date) as as_of_date
         FROM csl_elimination_postings
         WHERE group_id = $1 AND period_id = $2 AND idempotency_key = $3",
    )
    .bind(group_id)
    .bind(period_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((posted_at, ids_json, as_of_date)) => {
            let journal_entry_ids: Vec<Uuid> = serde_json::from_value(ids_json).unwrap_or_default();
            Ok(Some(EliminationPostResult {
                group_id,
                period_id,
                as_of: as_of_date,
                posted_count: journal_entry_ids.len(),
                idempotency_key: idempotency_key.to_string(),
                journal_entry_ids,
                posted_at,
                already_posted: true,
            }))
        }
        None => Ok(None),
    }
}

/// Record a successful elimination posting for idempotency.
async fn record_posting(
    pool: &PgPool,
    group_id: Uuid,
    period_id: Uuid,
    idempotency_key: &str,
    journal_entry_ids: &[Uuid],
    suggestion_count: usize,
    total_amount_minor: i64,
    posted_at: chrono::DateTime<Utc>,
) -> Result<(), EngineError> {
    let ids_json = serde_json::to_value(journal_entry_ids)
        .map_err(|e| EngineError::Database(sqlx::Error::Protocol(e.to_string())))?;

    sqlx::query(
        "INSERT INTO csl_elimination_postings
            (group_id, period_id, idempotency_key, journal_entry_ids,
             suggestion_count, total_amount_minor, posted_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (group_id, period_id, idempotency_key) DO NOTHING",
    )
    .bind(group_id)
    .bind(period_id)
    .bind(idempotency_key)
    .bind(&ids_json)
    .bind(suggestion_count as i32)
    .bind(total_amount_minor)
    .bind(posted_at)
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::intercompany::{IntercompanyMatch, IntercompanyMatchResult};

    #[test]
    fn test_suggest_eliminations_basic() {
        let match_result = IntercompanyMatchResult {
            group_id: Uuid::new_v4(),
            as_of: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            matches: vec![IntercompanyMatch {
                rule_id: Uuid::new_v4(),
                rule_name: "IC Recv/Pay".into(),
                rule_type: "intercompany_receivable_payable".into(),
                entity_a_tenant_id: "ent-a".into(),
                entity_b_tenant_id: "ent-b".into(),
                debit_account_code: "1200".into(),
                credit_account_code: "2100".into(),
                match_amount_minor: 50000,
                debit_unmatched_minor: 0,
                credit_unmatched_minor: 0,
            }],
            unmatched_count: 0,
            total_matched_minor: 50000,
        };

        let suggestions = suggest_eliminations(&match_result);
        assert_eq!(suggestions.len(), 1);
        // Elimination reverses: debit the credit account, credit the debit
        assert_eq!(suggestions[0].debit_account_code, "2100");
        assert_eq!(suggestions[0].credit_account_code, "1200");
        assert_eq!(suggestions[0].amount_minor, 50000);
    }

    #[test]
    fn test_suggest_eliminations_skips_zero() {
        let match_result = IntercompanyMatchResult {
            group_id: Uuid::new_v4(),
            as_of: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            matches: vec![IntercompanyMatch {
                rule_id: Uuid::new_v4(),
                rule_name: "Zero".into(),
                rule_type: "custom".into(),
                entity_a_tenant_id: "ent-a".into(),
                entity_b_tenant_id: "ent-b".into(),
                debit_account_code: "1200".into(),
                credit_account_code: "2100".into(),
                match_amount_minor: 0,
                debit_unmatched_minor: 100,
                credit_unmatched_minor: 0,
            }],
            unmatched_count: 1,
            total_matched_minor: 0,
        };

        let suggestions = suggest_eliminations(&match_result);
        assert_eq!(suggestions.len(), 0);
    }

    #[test]
    fn test_idempotency_key_deterministic() {
        let gid = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let pid = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let suggestions = vec![EliminationSuggestion {
            rule_id: Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
            rule_name: "IC".into(),
            rule_type: "custom".into(),
            entity_a_tenant_id: "a".into(),
            entity_b_tenant_id: "b".into(),
            debit_account_code: "2100".into(),
            credit_account_code: "1200".into(),
            amount_minor: 50000,
            description: "Test".into(),
        }];

        let k1 = compute_idempotency_key(gid, pid, date, &suggestions);
        let k2 = compute_idempotency_key(gid, pid, date, &suggestions);
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_idempotency_key_changes_with_amount() {
        let gid = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let pid = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let rid = Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap();

        let s1 = vec![EliminationSuggestion {
            rule_id: rid,
            rule_name: "IC".into(),
            rule_type: "custom".into(),
            entity_a_tenant_id: "a".into(),
            entity_b_tenant_id: "b".into(),
            debit_account_code: "2100".into(),
            credit_account_code: "1200".into(),
            amount_minor: 50000,
            description: "Test".into(),
        }];
        let s2 = vec![EliminationSuggestion {
            rule_id: rid,
            rule_name: "IC".into(),
            rule_type: "custom".into(),
            entity_a_tenant_id: "a".into(),
            entity_b_tenant_id: "b".into(),
            debit_account_code: "2100".into(),
            credit_account_code: "1200".into(),
            amount_minor: 60000,
            description: "Test".into(),
        }];

        let k1 = compute_idempotency_key(gid, pid, date, &s1);
        let k2 = compute_idempotency_key(gid, pid, date, &s2);
        assert_ne!(k1, k2);
    }
}
