//! Financial Statement Contract Snapshot Tests (Phase 14.9 - bd-2zu)
//!
//! These tests serve as contract guards to prevent accidental breaking changes to
//! financial statement API response structures. They validate:
//! 1. Complete field presence (no accidental removals)
//! 2. Field ordering determinism
//! 3. JSON serialization stability
//! 4. Type safety (no silent type changes)
//!
//! **CI Guardrails**: These tests MUST pass. Field removal or reordering is a breaking change.
//! **SemVer Policy**: See CONTRACT-VERSIONING-POLICY.md for version bump requirements.

use serde_json::{json, Value};
use uuid::Uuid;

use gl_rs::domain::statements::{
    BalanceSheetRow, CurrencyTotals, IncomeStatementRow, StatementTotals, TrialBalanceRow,
};
use gl_rs::services::balance_sheet_service::{BalanceSheetResponse, BalanceSheetTotals};
use gl_rs::services::income_statement_service::{IncomeStatementResponse, IncomeStatementTotals};
use gl_rs::services::trial_balance_service::TrialBalanceResponse;

/// Test: Trial Balance Response Contract Snapshot
///
/// **Purpose**: Prevent accidental field removal or reordering in TrialBalanceResponse
/// **Breaking Changes**: Adding/removing fields, changing field types, reordering fields
#[test]
fn test_trial_balance_response_contract_snapshot() {
    let period_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

    let response = TrialBalanceResponse {
        tenant_id: "tenant_123".to_string(),
        period_id,
        currency: "USD".to_string(),
        rows: vec![
            TrialBalanceRow {
                account_code: "1100".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                normal_balance: "debit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 100000,
                credit_total_minor: 0,
                net_balance_minor: 100000,
            },
            TrialBalanceRow {
                account_code: "4000".to_string(),
                account_name: "Revenue".to_string(),
                account_type: "revenue".to_string(),
                normal_balance: "credit".to_string(),
                currency: "USD".to_string(),
                debit_total_minor: 0,
                credit_total_minor: 100000,
                net_balance_minor: -100000,
            },
        ],
        totals: StatementTotals {
            total_debits: 100000,
            total_credits: 100000,
            is_balanced: true,
        },
    };

    // Serialize to JSON Value for field inspection
    let json_value =
        serde_json::to_value(&response).expect("Failed to serialize TrialBalanceResponse");

    // ===== CONTRACT ASSERTION: Top-Level Fields =====
    assert!(
        json_value.get("tenant_id").is_some(),
        "Missing field: tenant_id"
    );
    assert!(
        json_value.get("period_id").is_some(),
        "Missing field: period_id"
    );
    assert!(
        json_value.get("currency").is_some(),
        "Missing field: currency"
    );
    assert!(json_value.get("rows").is_some(), "Missing field: rows");
    assert!(json_value.get("totals").is_some(), "Missing field: totals");

    // ===== CONTRACT ASSERTION: Field Count =====
    // Prevents accidental addition of fields without explicit version bump
    let top_level_fields = json_value.as_object().expect("Should be an object");
    assert_eq!(
        top_level_fields.len(),
        5,
        "TrialBalanceResponse should have exactly 5 fields (tenant_id, period_id, currency, rows, totals). Found: {:?}",
        top_level_fields.keys()
    );

    // ===== CONTRACT ASSERTION: Field Types =====
    assert!(
        json_value["tenant_id"].is_string(),
        "tenant_id should be string"
    );
    assert!(
        json_value["period_id"].is_string(),
        "period_id should be string (UUID)"
    );
    assert!(
        json_value["currency"].is_string(),
        "currency should be string"
    );
    assert!(json_value["rows"].is_array(), "rows should be array");
    assert!(json_value["totals"].is_object(), "totals should be object");

    // ===== CONTRACT ASSERTION: Row Structure =====
    let row = &json_value["rows"][0];
    let row_obj = row.as_object().expect("Row should be an object");

    // Check all required row fields exist
    assert!(
        row.get("account_code").is_some(),
        "Missing field: account_code"
    );
    assert!(
        row.get("account_name").is_some(),
        "Missing field: account_name"
    );
    assert!(
        row.get("account_type").is_some(),
        "Missing field: account_type"
    );
    assert!(
        row.get("normal_balance").is_some(),
        "Missing field: normal_balance"
    );
    assert!(row.get("currency").is_some(), "Missing field: currency");
    assert!(
        row.get("debit_total_minor").is_some(),
        "Missing field: debit_total_minor"
    );
    assert!(
        row.get("credit_total_minor").is_some(),
        "Missing field: credit_total_minor"
    );
    assert!(
        row.get("net_balance_minor").is_some(),
        "Missing field: net_balance_minor"
    );

    // Verify exact field count (8 fields)
    assert_eq!(
        row_obj.len(),
        8,
        "TrialBalanceRow should have exactly 8 fields. Found: {:?}",
        row_obj.keys()
    );

    // ===== CONTRACT ASSERTION: Totals Structure =====
    let totals = &json_value["totals"];
    let totals_obj = totals.as_object().expect("Totals should be an object");

    assert!(
        totals.get("total_debits").is_some(),
        "Missing field: total_debits"
    );
    assert!(
        totals.get("total_credits").is_some(),
        "Missing field: total_credits"
    );
    assert!(
        totals.get("is_balanced").is_some(),
        "Missing field: is_balanced"
    );

    assert_eq!(
        totals_obj.len(),
        3,
        "StatementTotals should have exactly 3 fields. Found: {:?}",
        totals_obj.keys()
    );

    // ===== CONTRACT ASSERTION: Deterministic Ordering =====
    // Serialize to JSON string to verify deterministic serialization
    // Note: Field order matches struct definition order (serde default)
    let json_str = serde_json::to_string_pretty(&response).expect("Failed to serialize");

    // Verify tenant_id appears in JSON
    let tenant_idx = json_str
        .find("\"tenant_id\"")
        .expect("tenant_id should be in JSON");
    let period_idx = json_str
        .find("\"period_id\"")
        .expect("period_id should be in JSON");
    let currency_idx = json_str
        .find("\"currency\"")
        .expect("currency should be in JSON");
    let rows_idx = json_str.find("\"rows\"").expect("rows should be in JSON");
    let totals_idx = json_str
        .find("\"totals\"")
        .expect("totals should be in JSON");

    // Field order matches struct definition: tenant_id, period_id, currency, rows, totals
    assert!(
        tenant_idx < period_idx,
        "Field ordering changed: tenant_id should appear before period_id (struct field order)"
    );
    assert!(
        period_idx < currency_idx,
        "Field ordering changed: period_id should appear before currency (struct field order)"
    );
    assert!(
        currency_idx < rows_idx,
        "Field ordering changed: currency should appear before rows (struct field order)"
    );
    assert!(
        rows_idx < totals_idx,
        "Field ordering changed: rows should appear before totals (struct field order)"
    );

    // ===== CONTRACT ASSERTION: Round-Trip Stability =====
    let roundtrip: TrialBalanceResponse = serde_json::from_value(json_value.clone())
        .expect("Failed to deserialize TrialBalanceResponse");
    assert_eq!(roundtrip.tenant_id, response.tenant_id);
    assert_eq!(roundtrip.period_id, response.period_id);
    assert_eq!(roundtrip.currency, response.currency);
    assert_eq!(roundtrip.rows.len(), response.rows.len());
    assert_eq!(roundtrip.totals.total_debits, response.totals.total_debits);

    println!("✅ TrialBalanceResponse contract snapshot validated");
}

/// Test: Income Statement Response Contract Snapshot
///
/// **Purpose**: Prevent accidental field removal or reordering in IncomeStatementResponse
/// **Breaking Changes**: Adding/removing fields, changing field types, reordering fields
#[test]
fn test_income_statement_response_contract_snapshot() {
    let period_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap();

    let response = IncomeStatementResponse {
        tenant_id: "tenant_456".to_string(),
        period_id,
        currency: "EUR".to_string(),
        rows: vec![
            IncomeStatementRow {
                account_code: "4000".to_string(),
                account_name: "Sales Revenue".to_string(),
                account_type: "revenue".to_string(),
                currency: "EUR".to_string(),
                amount_minor: 500000,
            },
            IncomeStatementRow {
                account_code: "5000".to_string(),
                account_name: "Salaries Expense".to_string(),
                account_type: "expense".to_string(),
                currency: "EUR".to_string(),
                amount_minor: -300000,
            },
        ],
        totals: IncomeStatementTotals {
            total_revenue: 500000,
            total_expenses: -300000,
            net_income: 200000,
        },
    };

    let json_value =
        serde_json::to_value(&response).expect("Failed to serialize IncomeStatementResponse");

    // ===== CONTRACT ASSERTION: Top-Level Fields =====
    assert!(
        json_value.get("tenant_id").is_some(),
        "Missing field: tenant_id"
    );
    assert!(
        json_value.get("period_id").is_some(),
        "Missing field: period_id"
    );
    assert!(
        json_value.get("currency").is_some(),
        "Missing field: currency"
    );
    assert!(json_value.get("rows").is_some(), "Missing field: rows");
    assert!(json_value.get("totals").is_some(), "Missing field: totals");

    let top_level_fields = json_value.as_object().expect("Should be an object");
    assert_eq!(
        top_level_fields.len(),
        5,
        "IncomeStatementResponse should have exactly 5 fields. Found: {:?}",
        top_level_fields.keys()
    );

    // ===== CONTRACT ASSERTION: Row Structure =====
    let row = &json_value["rows"][0];
    let row_obj = row.as_object().expect("Row should be an object");

    assert!(
        row.get("account_code").is_some(),
        "Missing field: account_code"
    );
    assert!(
        row.get("account_name").is_some(),
        "Missing field: account_name"
    );
    assert!(
        row.get("account_type").is_some(),
        "Missing field: account_type"
    );
    assert!(row.get("currency").is_some(), "Missing field: currency");
    assert!(
        row.get("amount_minor").is_some(),
        "Missing field: amount_minor"
    );

    assert_eq!(
        row_obj.len(),
        5,
        "IncomeStatementRow should have exactly 5 fields. Found: {:?}",
        row_obj.keys()
    );

    // ===== CONTRACT ASSERTION: Totals Structure =====
    let totals = &json_value["totals"];
    let totals_obj = totals.as_object().expect("Totals should be an object");

    assert!(
        totals.get("total_revenue").is_some(),
        "Missing field: total_revenue"
    );
    assert!(
        totals.get("total_expenses").is_some(),
        "Missing field: total_expenses"
    );
    assert!(
        totals.get("net_income").is_some(),
        "Missing field: net_income"
    );

    assert_eq!(
        totals_obj.len(),
        3,
        "IncomeStatementTotals should have exactly 3 fields. Found: {:?}",
        totals_obj.keys()
    );

    // ===== CONTRACT ASSERTION: Field Types =====
    assert!(row["amount_minor"].is_i64(), "amount_minor should be i64");
    assert!(
        totals["total_revenue"].is_i64(),
        "total_revenue should be i64"
    );
    assert!(
        totals["total_expenses"].is_i64(),
        "total_expenses should be i64"
    );
    assert!(totals["net_income"].is_i64(), "net_income should be i64");

    // ===== CONTRACT ASSERTION: Round-Trip Stability =====
    let roundtrip: IncomeStatementResponse = serde_json::from_value(json_value.clone())
        .expect("Failed to deserialize IncomeStatementResponse");
    assert_eq!(roundtrip.tenant_id, response.tenant_id);
    assert_eq!(roundtrip.period_id, response.period_id);
    assert_eq!(roundtrip.currency, response.currency);
    assert_eq!(roundtrip.rows.len(), response.rows.len());
    assert_eq!(roundtrip.totals.net_income, response.totals.net_income);

    println!("✅ IncomeStatementResponse contract snapshot validated");
}

/// Test: Balance Sheet Response Contract Snapshot
///
/// **Purpose**: Prevent accidental field removal or reordering in BalanceSheetResponse
/// **Breaking Changes**: Adding/removing fields, changing field types, reordering fields
#[test]
fn test_balance_sheet_response_contract_snapshot() {
    let period_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440002").unwrap();

    let response = BalanceSheetResponse {
        tenant_id: "tenant_789".to_string(),
        period_id,
        currency: "GBP".to_string(),
        rows: vec![
            BalanceSheetRow {
                account_code: "1000".to_string(),
                account_name: "Cash".to_string(),
                account_type: "asset".to_string(),
                currency: "GBP".to_string(),
                amount_minor: 100000,
            },
            BalanceSheetRow {
                account_code: "2000".to_string(),
                account_name: "Accounts Payable".to_string(),
                account_type: "liability".to_string(),
                currency: "GBP".to_string(),
                amount_minor: 60000,
            },
            BalanceSheetRow {
                account_code: "3000".to_string(),
                account_name: "Retained Earnings".to_string(),
                account_type: "equity".to_string(),
                currency: "GBP".to_string(),
                amount_minor: 40000,
            },
        ],
        totals: BalanceSheetTotals {
            total_assets: 100000,
            total_liabilities: 60000,
            total_equity: 40000,
            is_balanced: true,
        },
    };

    let json_value =
        serde_json::to_value(&response).expect("Failed to serialize BalanceSheetResponse");

    // ===== CONTRACT ASSERTION: Top-Level Fields =====
    assert!(
        json_value.get("tenant_id").is_some(),
        "Missing field: tenant_id"
    );
    assert!(
        json_value.get("period_id").is_some(),
        "Missing field: period_id"
    );
    assert!(
        json_value.get("currency").is_some(),
        "Missing field: currency"
    );
    assert!(json_value.get("rows").is_some(), "Missing field: rows");
    assert!(json_value.get("totals").is_some(), "Missing field: totals");

    let top_level_fields = json_value.as_object().expect("Should be an object");
    assert_eq!(
        top_level_fields.len(),
        5,
        "BalanceSheetResponse should have exactly 5 fields. Found: {:?}",
        top_level_fields.keys()
    );

    // ===== CONTRACT ASSERTION: Row Structure =====
    let row = &json_value["rows"][0];
    let row_obj = row.as_object().expect("Row should be an object");

    assert!(
        row.get("account_code").is_some(),
        "Missing field: account_code"
    );
    assert!(
        row.get("account_name").is_some(),
        "Missing field: account_name"
    );
    assert!(
        row.get("account_type").is_some(),
        "Missing field: account_type"
    );
    assert!(row.get("currency").is_some(), "Missing field: currency");
    assert!(
        row.get("amount_minor").is_some(),
        "Missing field: amount_minor"
    );

    assert_eq!(
        row_obj.len(),
        5,
        "BalanceSheetRow should have exactly 5 fields. Found: {:?}",
        row_obj.keys()
    );

    // ===== CONTRACT ASSERTION: Totals Structure =====
    let totals = &json_value["totals"];
    let totals_obj = totals.as_object().expect("Totals should be an object");

    assert!(
        totals.get("total_assets").is_some(),
        "Missing field: total_assets"
    );
    assert!(
        totals.get("total_liabilities").is_some(),
        "Missing field: total_liabilities"
    );
    assert!(
        totals.get("total_equity").is_some(),
        "Missing field: total_equity"
    );
    assert!(
        totals.get("is_balanced").is_some(),
        "Missing field: is_balanced"
    );

    assert_eq!(
        totals_obj.len(),
        4,
        "BalanceSheetTotals should have exactly 4 fields. Found: {:?}",
        totals_obj.keys()
    );

    // ===== CONTRACT ASSERTION: Field Types =====
    assert!(
        totals["total_assets"].is_i64(),
        "total_assets should be i64"
    );
    assert!(
        totals["total_liabilities"].is_i64(),
        "total_liabilities should be i64"
    );
    assert!(
        totals["total_equity"].is_i64(),
        "total_equity should be i64"
    );
    assert!(
        totals["is_balanced"].is_boolean(),
        "is_balanced should be boolean"
    );

    // ===== CONTRACT ASSERTION: Accounting Equation in Contract =====
    // This validates that the accounting equation fields are present and correct types
    let assets = totals["total_assets"].as_i64().unwrap();
    let liabilities = totals["total_liabilities"].as_i64().unwrap();
    let equity = totals["total_equity"].as_i64().unwrap();
    assert_eq!(
        assets,
        liabilities + equity,
        "Contract should preserve accounting equation: Assets = Liabilities + Equity"
    );

    // ===== CONTRACT ASSERTION: Round-Trip Stability =====
    let roundtrip: BalanceSheetResponse = serde_json::from_value(json_value.clone())
        .expect("Failed to deserialize BalanceSheetResponse");
    assert_eq!(roundtrip.tenant_id, response.tenant_id);
    assert_eq!(roundtrip.period_id, response.period_id);
    assert_eq!(roundtrip.currency, response.currency);
    assert_eq!(roundtrip.rows.len(), response.rows.len());
    assert_eq!(roundtrip.totals.total_assets, response.totals.total_assets);
    assert_eq!(roundtrip.totals.is_balanced, response.totals.is_balanced);

    println!("✅ BalanceSheetResponse contract snapshot validated");
}

/// Test: Currency Totals Contract Snapshot
///
/// **Purpose**: Prevent accidental field removal in CurrencyTotals struct
/// **Breaking Changes**: Adding/removing fields, changing field types
#[test]
fn test_currency_totals_contract_snapshot() {
    let totals = CurrencyTotals {
        currency: "JPY".to_string(),
        total_debits: 150000,
        total_credits: 150000,
        is_balanced: true,
    };

    let json_value = serde_json::to_value(&totals).expect("Failed to serialize CurrencyTotals");
    let json_obj = json_value.as_object().expect("Should be an object");

    // ===== CONTRACT ASSERTION: Field Presence =====
    assert!(
        json_value.get("currency").is_some(),
        "Missing field: currency"
    );
    assert!(
        json_value.get("total_debits").is_some(),
        "Missing field: total_debits"
    );
    assert!(
        json_value.get("total_credits").is_some(),
        "Missing field: total_credits"
    );
    assert!(
        json_value.get("is_balanced").is_some(),
        "Missing field: is_balanced"
    );

    // ===== CONTRACT ASSERTION: Field Count =====
    assert_eq!(
        json_obj.len(),
        4,
        "CurrencyTotals should have exactly 4 fields. Found: {:?}",
        json_obj.keys()
    );

    // ===== CONTRACT ASSERTION: Round-Trip Stability =====
    let roundtrip: CurrencyTotals =
        serde_json::from_value(json_value).expect("Failed to deserialize CurrencyTotals");
    assert_eq!(roundtrip.currency, totals.currency);
    assert_eq!(roundtrip.total_debits, totals.total_debits);
    assert_eq!(roundtrip.total_credits, totals.total_credits);
    assert_eq!(roundtrip.is_balanced, totals.is_balanced);

    println!("✅ CurrencyTotals contract snapshot validated");
}

/// Test: Comprehensive Cross-Statement Field Consistency
///
/// **Purpose**: Ensure common fields across statements maintain consistent naming and types
/// **Breaking Changes**: Renaming common fields, changing types of shared fields
#[test]
fn test_cross_statement_field_consistency() {
    let period_id = Uuid::new_v4();

    // Create all three statement types
    let trial_balance = TrialBalanceResponse {
        tenant_id: "tenant_consistency".to_string(),
        period_id,
        currency: "USD".to_string(),
        rows: vec![],
        totals: StatementTotals {
            total_debits: 0,
            total_credits: 0,
            is_balanced: true,
        },
    };

    let income_statement = IncomeStatementResponse {
        tenant_id: "tenant_consistency".to_string(),
        period_id,
        currency: "USD".to_string(),
        rows: vec![],
        totals: IncomeStatementTotals {
            total_revenue: 0,
            total_expenses: 0,
            net_income: 0,
        },
    };

    let balance_sheet = BalanceSheetResponse {
        tenant_id: "tenant_consistency".to_string(),
        period_id,
        currency: "USD".to_string(),
        rows: vec![],
        totals: BalanceSheetTotals {
            total_assets: 0,
            total_liabilities: 0,
            total_equity: 0,
            is_balanced: true,
        },
    };

    // Serialize all three
    let tb_json = serde_json::to_value(&trial_balance).unwrap();
    let is_json = serde_json::to_value(&income_statement).unwrap();
    let bs_json = serde_json::to_value(&balance_sheet).unwrap();

    // ===== CONTRACT ASSERTION: Common Field Names =====
    // All statements MUST have these fields with identical names
    for (name, json) in [
        ("TrialBalance", &tb_json),
        ("IncomeStatement", &is_json),
        ("BalanceSheet", &bs_json),
    ] {
        assert!(
            json.get("tenant_id").is_some(),
            "{} missing common field: tenant_id",
            name
        );
        assert!(
            json.get("period_id").is_some(),
            "{} missing common field: period_id",
            name
        );
        assert!(
            json.get("currency").is_some(),
            "{} missing common field: currency",
            name
        );
        assert!(
            json.get("rows").is_some(),
            "{} missing common field: rows",
            name
        );
        assert!(
            json.get("totals").is_some(),
            "{} missing common field: totals",
            name
        );
    }

    // ===== CONTRACT ASSERTION: Common Field Types =====
    for (name, json) in [
        ("TrialBalance", &tb_json),
        ("IncomeStatement", &is_json),
        ("BalanceSheet", &bs_json),
    ] {
        assert!(
            json["tenant_id"].is_string(),
            "{} tenant_id should be string",
            name
        );
        assert!(
            json["period_id"].is_string(),
            "{} period_id should be string (UUID)",
            name
        );
        assert!(
            json["currency"].is_string(),
            "{} currency should be string",
            name
        );
        assert!(json["rows"].is_array(), "{} rows should be array", name);
        assert!(
            json["totals"].is_object(),
            "{} totals should be object",
            name
        );
    }

    println!("✅ Cross-statement field consistency validated");
}

/// Test: JSON Schema Stability (Prevents Silent Breaking Changes)
///
/// **Purpose**: Golden snapshot of full JSON structure to catch ANY structural change
/// **Breaking Changes**: ANY modification to JSON structure (field additions, removals, reordering)
#[test]
fn test_json_schema_stability_golden_snapshot() {
    let period_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

    // Create deterministic test data (sorted by account_code)
    let trial_balance = TrialBalanceResponse {
        tenant_id: "tenant_golden".to_string(),
        period_id,
        currency: "USD".to_string(),
        rows: vec![TrialBalanceRow {
            account_code: "1000".to_string(),
            account_name: "Cash".to_string(),
            account_type: "asset".to_string(),
            normal_balance: "debit".to_string(),
            currency: "USD".to_string(),
            debit_total_minor: 50000,
            credit_total_minor: 0,
            net_balance_minor: 50000,
        }],
        totals: StatementTotals {
            total_debits: 50000,
            total_credits: 50000,
            is_balanced: true,
        },
    };

    // Generate pretty JSON for readability in diffs
    let json_str = serde_json::to_string_pretty(&trial_balance).expect("Failed to serialize");

    // Golden snapshot - this exact structure MUST NOT change without version bump
    let expected_structure = json!({
        "tenant_id": "tenant_golden",
        "period_id": "00000000-0000-0000-0000-000000000001",
        "currency": "USD",
        "rows": [{
            "account_code": "1000",
            "account_name": "Cash",
            "account_type": "asset",
            "normal_balance": "debit",
            "currency": "USD",
            "debit_total_minor": 50000,
            "credit_total_minor": 0,
            "net_balance_minor": 50000
        }],
        "totals": {
            "total_debits": 50000,
            "total_credits": 50000,
            "is_balanced": true
        }
    });

    let actual_value: Value = serde_json::from_str(&json_str).expect("Failed to parse JSON");

    // Deep equality check - ANY difference is a breaking change
    assert_eq!(
        actual_value, expected_structure,
        "JSON schema has changed! This is a BREAKING CHANGE.\n\
         If intentional, bump version in modules/gl/VERSION and update this test.\n\
         Actual JSON:\n{}\n",
        json_str
    );

    println!("✅ JSON schema stability validated (golden snapshot matched)");
}
