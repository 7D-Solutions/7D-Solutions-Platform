//! Pure matching logic — no I/O, no DB.
//!
//! Deterministic intercompany balance matching across entities.

use std::collections::HashMap;

use crate::domain::config::EliminationRule;

use super::{EntityAccountBalance, IntercompanyMatch};

/// Run intercompany matching across entities for a set of elimination rules.
///
/// For each active elimination rule:
///   - Find entities that have a balance on the rule's debit_account_code
///   - Find entities that have a balance on the rule's credit_account_code
///   - Match by entity pairs: entity A's debit account vs entity B's credit account
///
/// Produces stable, deterministic output sorted by (rule_name, entity_a, entity_b).
pub fn match_intercompany(
    rules: &[EliminationRule],
    entity_balances: &[EntityAccountBalance],
) -> Vec<IntercompanyMatch> {
    // Index: account_code → Vec<(entity_tenant_id, net_balance)>
    let mut account_index: HashMap<&str, Vec<(&str, i64)>> = HashMap::new();
    for bal in entity_balances {
        account_index
            .entry(&bal.account_code)
            .or_default()
            .push((&bal.entity_tenant_id, bal.net_minor));
    }

    let mut matches = Vec::new();

    for rule in rules {
        if !rule.is_active {
            continue;
        }

        let debit_entities = account_index
            .get(rule.debit_account_code.as_str())
            .cloned()
            .unwrap_or_default();
        let credit_entities = account_index
            .get(rule.credit_account_code.as_str())
            .cloned()
            .unwrap_or_default();

        for (entity_a, debit_net) in &debit_entities {
            // Only match entities with positive net (debit normal balance)
            if *debit_net <= 0 {
                continue;
            }

            for (entity_b, credit_net) in &credit_entities {
                if entity_a == entity_b {
                    continue;
                }

                // Credit-side has negative net (credit normal balance)
                let credit_abs = credit_net.abs();
                if credit_abs <= 0 {
                    continue;
                }

                let match_amount = (*debit_net).min(credit_abs);
                let debit_unmatched = *debit_net - match_amount;
                let credit_unmatched = credit_abs - match_amount;

                matches.push(IntercompanyMatch {
                    rule_id: rule.id,
                    rule_name: rule.rule_name.clone(),
                    rule_type: rule.rule_type.clone(),
                    entity_a_tenant_id: entity_a.to_string(),
                    entity_b_tenant_id: entity_b.to_string(),
                    debit_account_code: rule.debit_account_code.clone(),
                    credit_account_code: rule.credit_account_code.clone(),
                    match_amount_minor: match_amount,
                    debit_unmatched_minor: debit_unmatched,
                    credit_unmatched_minor: credit_unmatched,
                });
            }
        }
    }

    // Deterministic ordering
    matches.sort_by(|a, b| {
        a.rule_name
            .cmp(&b.rule_name)
            .then(a.entity_a_tenant_id.cmp(&b.entity_a_tenant_id))
            .then(a.entity_b_tenant_id.cmp(&b.entity_b_tenant_id))
    });

    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_rule(name: &str, debit: &str, credit: &str) -> EliminationRule {
        EliminationRule {
            id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            rule_name: name.to_string(),
            rule_type: "intercompany_receivable_payable".to_string(),
            debit_account_code: debit.to_string(),
            credit_account_code: credit.to_string(),
            description: None,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_balance(entity: &str, account: &str, net: i64) -> EntityAccountBalance {
        EntityAccountBalance {
            entity_tenant_id: entity.to_string(),
            account_code: account.to_string(),
            account_name: format!("Account {}", account),
            debit_minor: if net > 0 { net } else { 0 },
            credit_minor: if net < 0 { net.abs() } else { 0 },
            net_minor: net,
        }
    }

    #[test]
    fn test_exact_match() {
        let rule = make_rule("IC Recv/Pay", "1200", "2100");
        let balances = vec![
            make_balance("ent-a", "1200", 50000),
            make_balance("ent-b", "2100", -50000),
        ];

        let matches = match_intercompany(&[rule], &balances);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].match_amount_minor, 50000);
        assert_eq!(matches[0].debit_unmatched_minor, 0);
        assert_eq!(matches[0].credit_unmatched_minor, 0);
    }

    #[test]
    fn test_partial_match() {
        let rule = make_rule("IC Recv/Pay", "1200", "2100");
        let balances = vec![
            make_balance("ent-a", "1200", 50000),
            make_balance("ent-b", "2100", -30000),
        ];

        let matches = match_intercompany(&[rule], &balances);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].match_amount_minor, 30000);
        assert_eq!(matches[0].debit_unmatched_minor, 20000);
        assert_eq!(matches[0].credit_unmatched_minor, 0);
    }

    #[test]
    fn test_no_match_same_entity() {
        let rule = make_rule("IC Recv/Pay", "1200", "2100");
        let balances = vec![
            make_balance("ent-a", "1200", 50000),
            make_balance("ent-a", "2100", -50000),
        ];

        let matches = match_intercompany(&[rule], &balances);
        assert_eq!(matches.len(), 0);
    }

    #[test]
    fn test_inactive_rule_skipped() {
        let mut rule = make_rule("IC Recv/Pay", "1200", "2100");
        rule.is_active = false;
        let balances = vec![
            make_balance("ent-a", "1200", 50000),
            make_balance("ent-b", "2100", -50000),
        ];

        let matches = match_intercompany(&[rule], &balances);
        assert_eq!(matches.len(), 0);
    }

    #[test]
    fn test_deterministic_ordering() {
        let rule = make_rule("IC Recv/Pay", "1200", "2100");
        let balances = vec![
            make_balance("ent-c", "1200", 10000),
            make_balance("ent-a", "1200", 20000),
            make_balance("ent-b", "2100", -15000),
        ];

        let matches = match_intercompany(&[rule], &balances);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].entity_a_tenant_id, "ent-a");
        assert_eq!(matches[1].entity_a_tenant_id, "ent-c");
    }

    #[test]
    fn test_multiple_rules() {
        let rule1 = make_rule("IC Recv/Pay", "1200", "2100");
        let rule2 = make_rule("IC Rev/Cost", "4000", "5000");
        let balances = vec![
            make_balance("ent-a", "1200", 50000),
            make_balance("ent-b", "2100", -50000),
            make_balance("ent-a", "4000", 30000),
            make_balance("ent-b", "5000", -30000),
        ];

        let matches = match_intercompany(&[rule1, rule2], &balances);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].rule_name, "IC Recv/Pay");
        assert_eq!(matches[1].rule_name, "IC Rev/Cost");
    }

    #[test]
    fn test_no_balances() {
        let rule = make_rule("IC Recv/Pay", "1200", "2100");
        let matches = match_intercompany(&[rule], &[]);
        assert_eq!(matches.len(), 0);
    }
}
