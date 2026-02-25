//! Integration tests — jurisdiction resolution from address.
//!
//! Exercises `resolve_jurisdiction` through the public API with realistic
//! multi-jurisdiction configs, nexus rules, and date-sensitivity.

use chrono::NaiveDate;
use tax_core::jurisdiction::{
    resolve_jurisdiction, JurisdictionConfig, JurisdictionEntry, JurisdictionResult, TaxRuleConfig,
};
use tax_core::TaxAddress;
use uuid::Uuid;

// ── Helpers ────────────────────────────────────────────────────────────

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn addr(country: &str, state: &str) -> TaxAddress {
    TaxAddress {
        line1: "1 Test St".into(),
        line2: None,
        city: "Testville".into(),
        state: state.into(),
        postal_code: "00000".into(),
        country: country.into(),
    }
}

fn rule(tax_type: &str, rate: f64, from: NaiveDate, to: Option<NaiveDate>) -> TaxRuleConfig {
    TaxRuleConfig {
        tax_type: tax_type.into(),
        rate,
        flat_amount_minor: 0,
        tax_codes: None,
        effective_from: from,
        effective_to: to,
        is_exempt: false,
        priority: 10,
    }
}

fn multi_jurisdiction_config() -> JurisdictionConfig {
    JurisdictionConfig {
        version: "test-2.0".into(),
        jurisdictions: vec![
            JurisdictionEntry {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                name: "California Sales Tax".into(),
                country: "US".into(),
                state: Some("CA".into()),
                rules: vec![
                    rule("sales_tax", 0.085, date(2025, 1, 1), None),
                    TaxRuleConfig {
                        tax_type: "sales_tax".into(),
                        rate: 0.0,
                        flat_amount_minor: 0,
                        tax_codes: Some(vec!["EXEMPT_FOOD".into()]),
                        effective_from: date(2025, 1, 1),
                        effective_to: None,
                        is_exempt: true,
                        priority: 20,
                    },
                ],
            },
            JurisdictionEntry {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                name: "New York Sales Tax".into(),
                country: "US".into(),
                state: Some("NY".into()),
                rules: vec![rule("sales_tax", 0.08, date(2025, 1, 1), None)],
            },
            JurisdictionEntry {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
                name: "Germany VAT".into(),
                country: "DE".into(),
                state: None, // country-level
                rules: vec![
                    rule("vat", 0.19, date(2025, 1, 1), None),
                    TaxRuleConfig {
                        tax_type: "vat_reduced".into(),
                        rate: 0.07,
                        flat_amount_minor: 0,
                        tax_codes: Some(vec!["FOOD".into()]),
                        effective_from: date(2025, 1, 1),
                        effective_to: None,
                        is_exempt: false,
                        priority: 20,
                    },
                ],
            },
            // Jurisdiction with only an expired rule
            JurisdictionEntry {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap(),
                name: "Expired-only Jurisdiction".into(),
                country: "US".into(),
                state: Some("WA".into()),
                rules: vec![rule(
                    "sales_tax",
                    0.065,
                    date(2020, 1, 1),
                    Some(date(2024, 12, 31)),
                )],
            },
        ],
    }
}

// ── Basic resolution ───────────────────────────────────────────────────

#[test]
fn resolves_california_by_ship_to_address() {
    let cfg = multi_jurisdiction_config();
    let ship_to = addr("US", "CA");
    let result = resolve_jurisdiction(&ship_to, &ship_to, &[], None, date(2026, 2, 1), &cfg);

    match result {
        JurisdictionResult::Resolved {
            jurisdiction_id,
            jurisdiction_name,
            rules,
        } => {
            assert_eq!(
                jurisdiction_id,
                Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
            );
            assert_eq!(jurisdiction_name, "California Sales Tax");
            assert_eq!(rules.len(), 1);
            assert_eq!(rules[0].rate, 0.085);
            assert_eq!(rules[0].tax_type, "sales_tax");
        }
        JurisdictionResult::Unknown => panic!("expected Resolved for CA"),
    }
}

#[test]
fn resolves_new_york_by_ship_to_address() {
    let cfg = multi_jurisdiction_config();
    let ship_to = addr("US", "NY");
    let result = resolve_jurisdiction(&ship_to, &ship_to, &[], None, date(2026, 2, 1), &cfg);

    match result {
        JurisdictionResult::Resolved {
            jurisdiction_name,
            rules,
            ..
        } => {
            assert_eq!(jurisdiction_name, "New York Sales Tax");
            assert_eq!(rules.len(), 1);
            assert_eq!(rules[0].rate, 0.08);
        }
        JurisdictionResult::Unknown => panic!("expected Resolved for NY"),
    }
}

#[test]
fn country_level_jurisdiction_matches_any_state() {
    let cfg = multi_jurisdiction_config();
    // DE is configured as country-level (state=None), should match any German state
    let berlin = addr("DE", "BE");
    let result = resolve_jurisdiction(&berlin, &berlin, &[], None, date(2026, 2, 1), &cfg);

    match result {
        JurisdictionResult::Resolved {
            jurisdiction_name,
            rules,
            ..
        } => {
            assert_eq!(jurisdiction_name, "Germany VAT");
            // Generic VAT rule matches (no tax_code filter)
            assert!(rules.iter().any(|r| r.tax_type == "vat" && r.rate == 0.19));
        }
        JurisdictionResult::Unknown => panic!("expected Resolved for DE"),
    }
}

#[test]
fn country_level_matches_different_german_state() {
    let cfg = multi_jurisdiction_config();
    let munich = addr("DE", "BY"); // Bavaria
    let result = resolve_jurisdiction(&munich, &munich, &[], None, date(2026, 2, 1), &cfg);

    assert!(
        matches!(result, JurisdictionResult::Resolved { .. }),
        "country-level DE should match any German state"
    );
}

// ── Unknown jurisdictions ──────────────────────────────────────────────

#[test]
fn unconfigured_country_returns_unknown() {
    let cfg = multi_jurisdiction_config();
    let ship_to = addr("JP", "TK");
    let result = resolve_jurisdiction(&ship_to, &ship_to, &[], None, date(2026, 2, 1), &cfg);
    assert!(matches!(result, JurisdictionResult::Unknown));
}

#[test]
fn unconfigured_state_returns_unknown() {
    let cfg = multi_jurisdiction_config();
    let ship_to = addr("US", "TX"); // TX not configured
    let result = resolve_jurisdiction(&ship_to, &ship_to, &[], None, date(2026, 2, 1), &cfg);
    assert!(matches!(result, JurisdictionResult::Unknown));
}

#[test]
fn jurisdiction_with_only_expired_rules_returns_unknown() {
    let cfg = multi_jurisdiction_config();
    let ship_to = addr("US", "WA");
    // Query in 2026 — WA's only rule expired end of 2024
    let result = resolve_jurisdiction(&ship_to, &ship_to, &[], None, date(2026, 2, 1), &cfg);
    assert!(
        matches!(result, JurisdictionResult::Unknown),
        "WA has no active rules in 2026, should return Unknown"
    );
}

#[test]
fn empty_config_returns_unknown() {
    let cfg = JurisdictionConfig {
        version: "empty".into(),
        jurisdictions: vec![],
    };
    let ship_to = addr("US", "CA");
    let result = resolve_jurisdiction(&ship_to, &ship_to, &[], None, date(2026, 2, 1), &cfg);
    assert!(matches!(result, JurisdictionResult::Unknown));
}

// ── Nexus verification ─────────────────────────────────────────────────

#[test]
fn nexus_in_same_state_allows_resolution() {
    let cfg = multi_jurisdiction_config();
    let ca = addr("US", "CA");
    let nexus = addr("US", "CA");
    let result = resolve_jurisdiction(&ca, &ca, &[nexus], None, date(2026, 2, 1), &cfg);
    assert!(matches!(result, JurisdictionResult::Resolved { .. }));
}

#[test]
fn no_nexus_in_destination_state_returns_unknown() {
    let cfg = multi_jurisdiction_config();
    let ca = addr("US", "CA");
    let nexus_ny = addr("US", "NY");
    // Seller has nexus only in NY, buyer is in CA
    let result = resolve_jurisdiction(&ca, &ca, &[nexus_ny], None, date(2026, 2, 1), &cfg);
    assert!(
        matches!(result, JurisdictionResult::Unknown),
        "no nexus in CA should return Unknown"
    );
}

#[test]
fn multiple_nexus_addresses_one_matching() {
    let cfg = multi_jurisdiction_config();
    let ca = addr("US", "CA");
    let nexus_ny = addr("US", "NY");
    let nexus_ca = addr("US", "CA");
    // Seller has nexus in both NY and CA
    let result = resolve_jurisdiction(
        &ca,
        &ca,
        &[nexus_ny, nexus_ca],
        None,
        date(2026, 2, 1),
        &cfg,
    );
    assert!(
        matches!(result, JurisdictionResult::Resolved { .. }),
        "at least one nexus in CA should resolve"
    );
}

#[test]
fn empty_nexus_list_skips_nexus_check() {
    let cfg = multi_jurisdiction_config();
    let ca = addr("US", "CA");
    // Empty nexus = no nexus enforcement
    let result = resolve_jurisdiction(&ca, &ca, &[], None, date(2026, 2, 1), &cfg);
    assert!(matches!(result, JurisdictionResult::Resolved { .. }));
}

// ── Date sensitivity ───────────────────────────────────────────────────

#[test]
fn expired_rule_active_during_valid_period() {
    let cfg = multi_jurisdiction_config();
    let wa = addr("US", "WA");
    // WA rule valid 2020-01-01 to 2024-12-31
    let result = resolve_jurisdiction(&wa, &wa, &[], None, date(2023, 6, 15), &cfg);
    match result {
        JurisdictionResult::Resolved { rules, .. } => {
            assert_eq!(rules.len(), 1);
            assert_eq!(rules[0].rate, 0.065);
        }
        JurisdictionResult::Unknown => panic!("WA rule should be active in 2023"),
    }
}

#[test]
fn rule_on_exact_effective_from_date() {
    let cfg = multi_jurisdiction_config();
    let ca = addr("US", "CA");
    let result = resolve_jurisdiction(&ca, &ca, &[], None, date(2025, 1, 1), &cfg);
    assert!(
        matches!(result, JurisdictionResult::Resolved { .. }),
        "rule should be active on its effective_from date"
    );
}

#[test]
fn rule_on_exact_effective_to_date() {
    let cfg = multi_jurisdiction_config();
    let wa = addr("US", "WA");
    // WA rule expires 2024-12-31 — should still be active on that date
    let result = resolve_jurisdiction(&wa, &wa, &[], None, date(2024, 12, 31), &cfg);
    assert!(
        matches!(result, JurisdictionResult::Resolved { .. }),
        "rule should be active on its effective_to date"
    );
}

#[test]
fn rule_day_after_expiry_returns_unknown() {
    let cfg = multi_jurisdiction_config();
    let wa = addr("US", "WA");
    let result = resolve_jurisdiction(&wa, &wa, &[], None, date(2025, 1, 1), &cfg);
    assert!(
        matches!(result, JurisdictionResult::Unknown),
        "rule expired 2024-12-31, should not match on 2025-01-01"
    );
}

#[test]
fn before_effective_from_returns_unknown() {
    let cfg = multi_jurisdiction_config();
    let ca = addr("US", "CA");
    // CA rules effective from 2025-01-01
    let result = resolve_jurisdiction(&ca, &ca, &[], None, date(2024, 12, 31), &cfg);
    assert!(
        matches!(result, JurisdictionResult::Unknown),
        "rules not yet effective should not match"
    );
}

// ── Tax code specificity ───────────────────────────────────────────────

#[test]
fn specific_tax_code_overrides_generic_rule() {
    let cfg = multi_jurisdiction_config();
    let ca = addr("US", "CA");
    // EXEMPT_FOOD has a specific exempt rule; the generic sales_tax should be dropped
    let result = resolve_jurisdiction(
        &ca,
        &ca,
        &[],
        Some("EXEMPT_FOOD"),
        date(2026, 2, 1),
        &cfg,
    );
    match result {
        JurisdictionResult::Resolved { rules, .. } => {
            assert_eq!(rules.len(), 1, "specific rule should override generic");
            assert!(rules[0].is_exempt);
            assert_eq!(rules[0].rate, 0.0);
        }
        JurisdictionResult::Unknown => panic!("expected Resolved with exempt rule"),
    }
}

#[test]
fn unmatched_tax_code_falls_back_to_generic() {
    let cfg = multi_jurisdiction_config();
    let ca = addr("US", "CA");
    // "ELECTRONICS" has no specific rule — should get the generic sales_tax
    let result = resolve_jurisdiction(
        &ca,
        &ca,
        &[],
        Some("ELECTRONICS"),
        date(2026, 2, 1),
        &cfg,
    );
    match result {
        JurisdictionResult::Resolved { rules, .. } => {
            assert_eq!(rules.len(), 1);
            assert_eq!(rules[0].rate, 0.085);
            assert!(!rules[0].is_exempt);
        }
        JurisdictionResult::Unknown => panic!("generic rule should apply"),
    }
}

#[test]
fn germany_food_tax_code_gets_reduced_rate() {
    let cfg = multi_jurisdiction_config();
    let de = addr("DE", "BE");
    let result = resolve_jurisdiction(&de, &de, &[], Some("FOOD"), date(2026, 2, 1), &cfg);
    match result {
        JurisdictionResult::Resolved { rules, .. } => {
            // Specific FOOD rule should override generic vat rule for vat_reduced type
            assert!(rules.iter().any(|r| r.tax_type == "vat_reduced" && r.rate == 0.07));
            // vat generic has tax_codes=None, vat_reduced has tax_codes=Some(["FOOD"])
            // They are different tax_types so specificity override only applies per type.
            // Generic vat still applies since its tax_type differs from vat_reduced.
            assert!(rules.iter().any(|r| r.tax_type == "vat"));
        }
        JurisdictionResult::Unknown => panic!("expected Resolved for DE/FOOD"),
    }
}

// ── Case-insensitive matching ──────────────────────────────────────────

#[test]
fn country_code_case_insensitive() {
    let cfg = multi_jurisdiction_config();
    let ship_to = addr("us", "CA"); // lowercase country
    let result = resolve_jurisdiction(&ship_to, &ship_to, &[], None, date(2026, 2, 1), &cfg);
    assert!(matches!(result, JurisdictionResult::Resolved { .. }));
}

#[test]
fn state_code_case_insensitive() {
    let cfg = multi_jurisdiction_config();
    let ship_to = addr("US", "ca"); // lowercase state
    let result = resolve_jurisdiction(&ship_to, &ship_to, &[], None, date(2026, 2, 1), &cfg);
    assert!(matches!(result, JurisdictionResult::Resolved { .. }));
}
