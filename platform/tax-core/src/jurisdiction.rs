//! Jurisdiction resolution — deterministic pure function for tax jurisdiction lookup.

use crate::models::{ResolvedRule, TaxAddress};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Static tax configuration loaded from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JurisdictionConfig {
    pub version: String,
    pub jurisdictions: Vec<JurisdictionEntry>,
}

/// A configured tax jurisdiction with its rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JurisdictionEntry {
    pub id: Uuid,
    pub name: String,
    /// ISO 3166-1 alpha-2 country code
    pub country: String,
    /// ISO 3166-2 state/province code (None = country-level jurisdiction)
    pub state: Option<String>,
    pub rules: Vec<TaxRuleConfig>,
}

/// A single tax rule within a jurisdiction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxRuleConfig {
    pub tax_type: String,
    pub rate: f64,
    #[serde(default)]
    pub flat_amount_minor: i64,
    /// If set, rule only applies to these product tax codes
    pub tax_codes: Option<Vec<String>>,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    #[serde(default)]
    pub is_exempt: bool,
    #[serde(default)]
    pub priority: i32,
}

/// Result of jurisdiction resolution.
#[derive(Debug, Clone)]
pub enum JurisdictionResult {
    /// Jurisdiction found with applicable rules.
    Resolved {
        jurisdiction_id: Uuid,
        jurisdiction_name: String,
        rules: Vec<ResolvedRule>,
    },
    /// No matching jurisdiction or no nexus — zero-tax applies.
    Unknown,
}

/// Resolve applicable tax jurisdiction and rules from addresses and config.
///
/// This is a pure, deterministic function: same inputs always produce the same output.
///
/// Resolution logic:
/// 1. Match `ship_to` country + state against configured jurisdictions
/// 2. If `nexus_addresses` is non-empty, verify seller has nexus in that jurisdiction
/// 3. Filter rules by effective date and `tax_code`
/// 4. Return matched rules or `Unknown` if nothing applies
pub fn resolve_jurisdiction(
    ship_to: &TaxAddress,
    _bill_to: &TaxAddress,
    nexus_addresses: &[TaxAddress],
    tax_code: Option<&str>,
    as_of: NaiveDate,
    config: &JurisdictionConfig,
) -> JurisdictionResult {
    // Step 1: Find jurisdiction matching ship_to country + state
    let entry = match config.jurisdictions.iter().find(|j| {
        j.country.eq_ignore_ascii_case(&ship_to.country)
            && match &j.state {
                Some(s) => s.eq_ignore_ascii_case(&ship_to.state),
                None => true, // country-level jurisdiction matches any state
            }
    }) {
        Some(e) => e,
        None => return JurisdictionResult::Unknown,
    };

    // Step 2: Verify seller has nexus (if nexus addresses provided)
    if !nexus_addresses.is_empty() {
        let has_nexus = nexus_addresses.iter().any(|n| {
            n.country.eq_ignore_ascii_case(&entry.country)
                && entry
                    .state
                    .as_ref()
                    .map_or(true, |s| s.eq_ignore_ascii_case(&n.state))
        });
        if !has_nexus {
            return JurisdictionResult::Unknown;
        }
    }

    // Step 3+4: Filter rules by effective date and tax_code, tracking specificity
    struct Candidate {
        is_specific: bool,
        rule: ResolvedRule,
    }
    let candidates: Vec<Candidate> = entry
        .rules
        .iter()
        .filter(|r| {
            r.effective_from <= as_of && r.effective_to.map_or(true, |end| as_of <= end)
        })
        .filter_map(|r| {
            let (matches, is_specific) = match (&r.tax_codes, tax_code) {
                (Some(codes), Some(tc)) => (codes.iter().any(|c| c == tc), true),
                (Some(_), None) => (false, true),
                (None, _) => (true, false),
            };
            if !matches {
                return None;
            }
            Some(Candidate {
                is_specific,
                rule: ResolvedRule {
                    jurisdiction_id: entry.id,
                    jurisdiction_name: entry.name.clone(),
                    tax_type: r.tax_type.clone(),
                    rate: r.rate,
                    flat_amount_minor: r.flat_amount_minor,
                    is_exempt: r.is_exempt,
                    tax_code: tax_code.map(|s| s.to_string()),
                    effective_from: r.effective_from,
                    effective_to: r.effective_to,
                    priority: r.priority,
                },
            })
        })
        .collect();

    // Specificity override: for each tax_type, if a specific-code rule matched,
    // drop generic (tax_codes=None) rules of that same type.
    let specific_types: std::collections::HashSet<String> = candidates
        .iter()
        .filter(|c| c.is_specific)
        .map(|c| c.rule.tax_type.clone())
        .collect();

    let rules: Vec<ResolvedRule> = candidates
        .into_iter()
        .filter(|c| c.is_specific || !specific_types.contains(&c.rule.tax_type))
        .map(|c| c.rule)
        .collect();

    if rules.is_empty() {
        return JurisdictionResult::Unknown;
    }

    JurisdictionResult::Resolved {
        jurisdiction_id: entry.id,
        jurisdiction_name: entry.name.clone(),
        rules,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us_ca_addr() -> TaxAddress {
        TaxAddress {
            line1: "123 Main St".into(),
            line2: None,
            city: "San Francisco".into(),
            state: "CA".into(),
            postal_code: "94102".into(),
            country: "US".into(),
        }
    }

    fn us_tx_addr() -> TaxAddress {
        TaxAddress {
            line1: "456 Elm St".into(),
            line2: None,
            city: "Austin".into(),
            state: "TX".into(),
            postal_code: "73301".into(),
            country: "US".into(),
        }
    }

    fn de_addr() -> TaxAddress {
        TaxAddress {
            line1: "1 Berliner Str".into(),
            line2: None,
            city: "Berlin".into(),
            state: "BE".into(),
            postal_code: "10115".into(),
            country: "DE".into(),
        }
    }

    fn test_config() -> JurisdictionConfig {
        let ca_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        JurisdictionConfig {
            version: "1.0.0".into(),
            jurisdictions: vec![JurisdictionEntry {
                id: ca_id,
                name: "California State Tax".into(),
                country: "US".into(),
                state: Some("CA".into()),
                rules: vec![
                    TaxRuleConfig {
                        tax_type: "sales_tax".into(),
                        rate: 0.085,
                        flat_amount_minor: 0,
                        tax_codes: None,
                        effective_from: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                        effective_to: None,
                        is_exempt: false,
                        priority: 10,
                    },
                    TaxRuleConfig {
                        tax_type: "sales_tax".into(),
                        rate: 0.0,
                        flat_amount_minor: 0,
                        tax_codes: Some(vec!["EXEMPT_FOOD".into()]),
                        effective_from: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                        effective_to: None,
                        is_exempt: true,
                        priority: 20,
                    },
                    // Expired rule
                    TaxRuleConfig {
                        tax_type: "old_tax".into(),
                        rate: 0.10,
                        flat_amount_minor: 0,
                        tax_codes: None,
                        effective_from: NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
                        effective_to: Some(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()),
                        is_exempt: false,
                        priority: 5,
                    },
                ],
            }],
        }
    }

    #[test]
    fn configured_jurisdiction_resolves() {
        let cfg = test_config();
        let addr = us_ca_addr();
        let result = resolve_jurisdiction(&addr, &addr, &[], None, date(2026, 2, 1), &cfg);
        match result {
            JurisdictionResult::Resolved { rules, .. } => {
                assert_eq!(rules.len(), 1); // only the current sales_tax rule (no tax_code)
                assert_eq!(rules[0].rate, 0.085);
                assert_eq!(rules[0].tax_type, "sales_tax");
            }
            JurisdictionResult::Unknown => panic!("expected Resolved"),
        }
    }

    #[test]
    fn unconfigured_country_returns_unknown() {
        let cfg = test_config();
        let addr = de_addr();
        let result = resolve_jurisdiction(&addr, &addr, &[], None, date(2026, 2, 1), &cfg);
        assert!(matches!(result, JurisdictionResult::Unknown));
    }

    #[test]
    fn unconfigured_state_returns_unknown() {
        let cfg = test_config();
        let addr = us_tx_addr();
        let result = resolve_jurisdiction(&addr, &addr, &[], None, date(2026, 2, 1), &cfg);
        assert!(matches!(result, JurisdictionResult::Unknown));
    }

    #[test]
    fn expired_rule_excluded() {
        let cfg = test_config();
        let addr = us_ca_addr();
        // Use a date in the expired rule's range — but "old_tax" has no tax_codes filter
        // so it would match. Let's query at a date where old_tax is valid and current is not.
        // Actually, current rule has no end date, so it's always valid from 2025.
        // old_tax is only valid 2020-2024. Query at 2023:
        let result = resolve_jurisdiction(&addr, &addr, &[], None, date(2023, 6, 1), &cfg);
        match result {
            JurisdictionResult::Resolved { rules, .. } => {
                assert_eq!(rules.len(), 1);
                assert_eq!(rules[0].tax_type, "old_tax");
                assert_eq!(rules[0].rate, 0.10);
            }
            JurisdictionResult::Unknown => panic!("expected Resolved"),
        }
    }

    #[test]
    fn tax_code_filters_to_matching_rules() {
        let cfg = test_config();
        let addr = us_ca_addr();
        let result = resolve_jurisdiction(
            &addr,
            &addr,
            &[],
            Some("EXEMPT_FOOD"),
            date(2026, 2, 1),
            &cfg,
        );
        match result {
            JurisdictionResult::Resolved { rules, .. } => {
                assert_eq!(rules.len(), 1);
                assert!(rules[0].is_exempt);
            }
            JurisdictionResult::Unknown => panic!("expected Resolved with exempt rule"),
        }
    }

    #[test]
    fn no_nexus_returns_unknown() {
        let cfg = test_config();
        let ca = us_ca_addr();
        let tx = us_tx_addr();
        // Seller only has nexus in TX, buyer is in CA
        let result = resolve_jurisdiction(&ca, &ca, &[tx], None, date(2026, 2, 1), &cfg);
        assert!(matches!(result, JurisdictionResult::Unknown));
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }
}
