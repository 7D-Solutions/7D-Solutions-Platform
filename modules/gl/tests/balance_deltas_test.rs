use gl_rs::services::balance_deltas::{compute_deltas, DeltaError, JournalLineInput};

#[test]
fn test_single_line_debit_only() {
    let lines = vec![JournalLineInput {
        account_ref: "1100".to_string(),
        debit_minor: 50000,
        credit_minor: 0,
    }];

    let deltas = compute_deltas(&lines, "USD").unwrap();

    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].account_code, "1100");
    assert_eq!(deltas[0].currency, "USD");
    assert_eq!(deltas[0].debit_delta, 50000);
    assert_eq!(deltas[0].credit_delta, 0);
}

#[test]
fn test_single_line_credit_only() {
    let lines = vec![JournalLineInput {
        account_ref: "2100".to_string(),
        debit_minor: 0,
        credit_minor: 30000,
    }];

    let deltas = compute_deltas(&lines, "EUR").unwrap();

    assert_eq!(deltas.len(), 1);
    assert_eq!(deltas[0].account_code, "2100");
    assert_eq!(deltas[0].currency, "EUR");
    assert_eq!(deltas[0].debit_delta, 0);
    assert_eq!(deltas[0].credit_delta, 30000);
}

#[test]
fn test_balanced_entry_two_accounts() {
    // Standard balanced entry: Debit one account, Credit another
    let lines = vec![
        JournalLineInput {
            account_ref: "1000".to_string(), // Asset account (debit)
            debit_minor: 100000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "3000".to_string(), // Equity account (credit)
            debit_minor: 0,
            credit_minor: 100000,
        },
    ];

    let deltas = compute_deltas(&lines, "GBP").unwrap();

    assert_eq!(deltas.len(), 2);

    // Verify sorted order
    assert_eq!(deltas[0].account_code, "1000");
    assert_eq!(deltas[0].currency, "GBP");
    assert_eq!(deltas[0].debit_delta, 100000);
    assert_eq!(deltas[0].credit_delta, 0);

    assert_eq!(deltas[1].account_code, "3000");
    assert_eq!(deltas[1].currency, "GBP");
    assert_eq!(deltas[1].debit_delta, 0);
    assert_eq!(deltas[1].credit_delta, 100000);
}

#[test]
fn test_complex_entry_multiple_lines() {
    // Complex entry with multiple debits and credits
    // Example: Split payment between multiple expense accounts
    let lines = vec![
        JournalLineInput {
            account_ref: "5100".to_string(), // Rent expense
            debit_minor: 200000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "5200".to_string(), // Utilities expense
            debit_minor: 50000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "1000".to_string(), // Cash (credit)
            debit_minor: 0,
            credit_minor: 250000,
        },
    ];

    let deltas = compute_deltas(&lines, "USD").unwrap();

    assert_eq!(deltas.len(), 3);

    let cash_delta = deltas.iter().find(|d| d.account_code == "1000").unwrap();
    assert_eq!(cash_delta.debit_delta, 0);
    assert_eq!(cash_delta.credit_delta, 250000);

    let rent_delta = deltas.iter().find(|d| d.account_code == "5100").unwrap();
    assert_eq!(rent_delta.debit_delta, 200000);
    assert_eq!(rent_delta.credit_delta, 0);

    let utils_delta = deltas.iter().find(|d| d.account_code == "5200").unwrap();
    assert_eq!(utils_delta.debit_delta, 50000);
    assert_eq!(utils_delta.credit_delta, 0);
}

#[test]
fn test_same_account_multiple_lines_aggregation() {
    // Multiple lines affecting the same account should be summed
    let lines = vec![
        JournalLineInput {
            account_ref: "1200".to_string(), // AR account
            debit_minor: 10000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "1200".to_string(), // AR account (another debit)
            debit_minor: 15000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "1200".to_string(), // AR account (credit)
            debit_minor: 0,
            credit_minor: 5000,
        },
        JournalLineInput {
            account_ref: "4000".to_string(), // Revenue account
            debit_minor: 0,
            credit_minor: 20000,
        },
    ];

    let deltas = compute_deltas(&lines, "USD").unwrap();

    assert_eq!(deltas.len(), 2);

    let ar_delta = deltas.iter().find(|d| d.account_code == "1200").unwrap();
    assert_eq!(ar_delta.debit_delta, 25000); // 10000 + 15000
    assert_eq!(ar_delta.credit_delta, 5000);

    let revenue_delta = deltas.iter().find(|d| d.account_code == "4000").unwrap();
    assert_eq!(revenue_delta.debit_delta, 0);
    assert_eq!(revenue_delta.credit_delta, 20000);
}

#[test]
fn test_reversal_entry_produces_inverted_deltas() {
    // Original entry: Sale transaction
    let original_lines = vec![
        JournalLineInput {
            account_ref: "1200".to_string(), // Accounts Receivable (debit)
            debit_minor: 100000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "4000".to_string(), // Sales Revenue (credit)
            debit_minor: 0,
            credit_minor: 100000,
        },
    ];

    // Reversal entry: Undo the sale
    let reversal_lines = vec![
        JournalLineInput {
            account_ref: "4000".to_string(), // Sales Revenue (debit - reversed)
            debit_minor: 100000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "1200".to_string(), // Accounts Receivable (credit - reversed)
            debit_minor: 0,
            credit_minor: 100000,
        },
    ];

    let original_deltas = compute_deltas(&original_lines, "USD").unwrap();
    let reversal_deltas = compute_deltas(&reversal_lines, "USD").unwrap();

    assert_eq!(original_deltas.len(), 2);
    assert_eq!(reversal_deltas.len(), 2);

    // Original: AR debit +100000, Revenue credit +100000
    let original_ar = original_deltas
        .iter()
        .find(|d| d.account_code == "1200")
        .unwrap();
    assert_eq!(original_ar.debit_delta, 100000);
    assert_eq!(original_ar.credit_delta, 0);

    let original_revenue = original_deltas
        .iter()
        .find(|d| d.account_code == "4000")
        .unwrap();
    assert_eq!(original_revenue.debit_delta, 0);
    assert_eq!(original_revenue.credit_delta, 100000);

    // Reversal: AR credit +100000, Revenue debit +100000 (inverted)
    let reversal_ar = reversal_deltas
        .iter()
        .find(|d| d.account_code == "1200")
        .unwrap();
    assert_eq!(reversal_ar.debit_delta, 0);
    assert_eq!(reversal_ar.credit_delta, 100000);

    let reversal_revenue = reversal_deltas
        .iter()
        .find(|d| d.account_code == "4000")
        .unwrap();
    assert_eq!(reversal_revenue.debit_delta, 100000);
    assert_eq!(reversal_revenue.credit_delta, 0);
}

#[test]
fn test_reversal_net_effect_zero() {
    // Applying original + reversal deltas should result in net zero
    let original_lines = vec![
        JournalLineInput {
            account_ref: "1000".to_string(),
            debit_minor: 50000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "4000".to_string(),
            debit_minor: 0,
            credit_minor: 50000,
        },
    ];

    let reversal_lines = vec![
        JournalLineInput {
            account_ref: "4000".to_string(),
            debit_minor: 50000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "1000".to_string(),
            debit_minor: 0,
            credit_minor: 50000,
        },
    ];

    let original_deltas = compute_deltas(&original_lines, "USD").unwrap();
    let reversal_deltas = compute_deltas(&reversal_lines, "USD").unwrap();

    // For account 1000: original debit +50000, reversal credit +50000 → net 0
    let orig_1000 = original_deltas
        .iter()
        .find(|d| d.account_code == "1000")
        .unwrap();
    let rev_1000 = reversal_deltas
        .iter()
        .find(|d| d.account_code == "1000")
        .unwrap();

    let net_debit_1000 = orig_1000.debit_delta + rev_1000.debit_delta;
    let net_credit_1000 = orig_1000.credit_delta + rev_1000.credit_delta;
    assert_eq!(net_debit_1000, 50000);
    assert_eq!(net_credit_1000, 50000);
    assert_eq!(net_debit_1000 - net_credit_1000, 0); // Net effect: zero

    // For account 4000: original credit +50000, reversal debit +50000 → net 0
    let orig_4000 = original_deltas
        .iter()
        .find(|d| d.account_code == "4000")
        .unwrap();
    let rev_4000 = reversal_deltas
        .iter()
        .find(|d| d.account_code == "4000")
        .unwrap();

    let net_debit_4000 = orig_4000.debit_delta + rev_4000.debit_delta;
    let net_credit_4000 = orig_4000.credit_delta + rev_4000.credit_delta;
    assert_eq!(net_debit_4000, 50000);
    assert_eq!(net_credit_4000, 50000);
    assert_eq!(net_debit_4000 - net_credit_4000, 0); // Net effect: zero
}

#[test]
fn test_empty_lines_error() {
    let lines: Vec<JournalLineInput> = vec![];

    let result = compute_deltas(&lines, "USD");

    assert!(result.is_err());
    match result {
        Err(DeltaError::EmptyLines) => {} // Expected
        _ => panic!("Expected EmptyLines error"),
    }
}

#[test]
fn test_deterministic_ordering() {
    // Verify deltas are always returned in same order regardless of input order
    let lines = vec![
        JournalLineInput {
            account_ref: "5000".to_string(),
            debit_minor: 1000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "1000".to_string(),
            debit_minor: 2000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "3000".to_string(),
            debit_minor: 3000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "2000".to_string(),
            debit_minor: 4000,
            credit_minor: 0,
        },
    ];

    let deltas = compute_deltas(&lines, "JPY").unwrap();

    assert_eq!(deltas.len(), 4);

    // Should be sorted by account_code
    assert_eq!(deltas[0].account_code, "1000");
    assert_eq!(deltas[1].account_code, "2000");
    assert_eq!(deltas[2].account_code, "3000");
    assert_eq!(deltas[3].account_code, "5000");

    // All should have same currency
    for delta in &deltas {
        assert_eq!(delta.currency, "JPY");
    }
}

#[test]
fn test_multi_currency_correct_assignment() {
    // Test that currency is correctly assigned to all deltas
    let lines = vec![
        JournalLineInput {
            account_ref: "1000".to_string(),
            debit_minor: 10000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "4000".to_string(),
            debit_minor: 0,
            credit_minor: 10000,
        },
    ];

    let deltas_usd = compute_deltas(&lines, "USD").unwrap();
    let deltas_eur = compute_deltas(&lines, "EUR").unwrap();

    // Verify all deltas get correct currency
    for delta in &deltas_usd {
        assert_eq!(delta.currency, "USD");
    }

    for delta in &deltas_eur {
        assert_eq!(delta.currency, "EUR");
    }
}

#[test]
fn test_large_amounts() {
    // Test with large amounts (e.g., $1 million = 100,000,000 cents)
    let lines = vec![
        JournalLineInput {
            account_ref: "1500".to_string(), // Long-term asset
            debit_minor: 100_000_000,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "1000".to_string(), // Cash
            debit_minor: 0,
            credit_minor: 100_000_000,
        },
    ];

    let deltas = compute_deltas(&lines, "USD").unwrap();

    assert_eq!(deltas.len(), 2);

    let asset_delta = deltas.iter().find(|d| d.account_code == "1500").unwrap();
    assert_eq!(asset_delta.debit_delta, 100_000_000);

    let cash_delta = deltas.iter().find(|d| d.account_code == "1000").unwrap();
    assert_eq!(cash_delta.credit_delta, 100_000_000);
}

#[test]
fn test_zero_amounts() {
    // Lines with zero amounts should still be included
    let lines = vec![
        JournalLineInput {
            account_ref: "1000".to_string(),
            debit_minor: 0,
            credit_minor: 0,
        },
        JournalLineInput {
            account_ref: "4000".to_string(),
            debit_minor: 10000,
            credit_minor: 0,
        },
    ];

    let deltas = compute_deltas(&lines, "USD").unwrap();

    assert_eq!(deltas.len(), 2);

    let zero_delta = deltas.iter().find(|d| d.account_code == "1000").unwrap();
    assert_eq!(zero_delta.debit_delta, 0);
    assert_eq!(zero_delta.credit_delta, 0);
}
