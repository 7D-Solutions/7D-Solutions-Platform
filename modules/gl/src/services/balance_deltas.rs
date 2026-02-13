//! Balance delta computation from journal entries
//!
//! This module provides deterministic computation of balance deltas from journal lines.
//! Deltas are grouped by (account_code, currency) to support multi-currency accounting.

use std::collections::HashMap;
use thiserror::Error;

/// Balance delta for a single account/currency combination
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceDelta {
    pub account_code: String,
    pub currency: String,
    pub debit_delta: i64,
    pub credit_delta: i64,
}

/// Key for grouping deltas by account and currency
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DeltaKey {
    account_code: String,
    currency: String,
}

/// Errors that can occur during delta computation
#[derive(Debug, Error)]
pub enum DeltaError {
    #[error("Currency mismatch: journal entry must have single currency, found multiple")]
    CurrencyMismatch,

    #[error("Empty journal lines: cannot compute deltas from empty line set")]
    EmptyLines,
}

/// Input journal line for delta computation
///
/// This is a simplified view of journal lines that only includes
/// the fields needed for balance delta calculation.
#[derive(Debug, Clone)]
pub struct JournalLineInput {
    pub account_ref: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
}

/// Compute balance deltas from journal lines
///
/// Groups debits and credits by (account_code, currency) to determine
/// which balances need to be updated.
///
/// # Arguments
/// * `lines` - Journal lines to process
/// * `currency` - Journal entry currency (all lines must have same currency)
///
/// # Returns
/// Vector of balance deltas, one per unique (account_code, currency) combination
///
/// # Errors
/// * `DeltaError::EmptyLines` - If lines vector is empty
///
/// # Example
/// ```
/// use gl_rs::services::balance_deltas::{compute_deltas, JournalLineInput};
///
/// let lines = vec![
///     JournalLineInput {
///         account_ref: "1000".to_string(),
///         debit_minor: 10000,
///         credit_minor: 0,
///     },
///     JournalLineInput {
///         account_ref: "4000".to_string(),
///         debit_minor: 0,
///         credit_minor: 10000,
///     },
/// ];
///
/// let deltas = compute_deltas(&lines, "USD").unwrap();
/// assert_eq!(deltas.len(), 2);
/// ```
pub fn compute_deltas(
    lines: &[JournalLineInput],
    currency: &str,
) -> Result<Vec<BalanceDelta>, DeltaError> {
    if lines.is_empty() {
        return Err(DeltaError::EmptyLines);
    }

    // Accumulate deltas by (account_code, currency)
    let mut delta_map: HashMap<DeltaKey, (i64, i64)> = HashMap::new();

    for line in lines {
        let key = DeltaKey {
            account_code: line.account_ref.clone(),
            currency: currency.to_string(),
        };

        let (debit_sum, credit_sum) = delta_map.entry(key).or_insert((0, 0));
        *debit_sum += line.debit_minor;
        *credit_sum += line.credit_minor;
    }

    // Convert map to vector of deltas
    let mut deltas: Vec<BalanceDelta> = delta_map
        .into_iter()
        .map(|(key, (debit_delta, credit_delta))| BalanceDelta {
            account_code: key.account_code,
            currency: key.currency,
            debit_delta,
            credit_delta,
        })
        .collect();

    // Sort for deterministic ordering (important for testing and audit)
    deltas.sort_by(|a, b| {
        a.account_code
            .cmp(&b.account_code)
            .then_with(|| a.currency.cmp(&b.currency))
    });

    Ok(deltas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_deltas_single_account() {
        let lines = vec![JournalLineInput {
            account_ref: "1000".to_string(),
            debit_minor: 10000,
            credit_minor: 0,
        }];

        let deltas = compute_deltas(&lines, "USD").unwrap();

        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].account_code, "1000");
        assert_eq!(deltas[0].currency, "USD");
        assert_eq!(deltas[0].debit_delta, 10000);
        assert_eq!(deltas[0].credit_delta, 0);
    }

    #[test]
    fn test_compute_deltas_multiple_accounts() {
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

        let deltas = compute_deltas(&lines, "USD").unwrap();

        assert_eq!(deltas.len(), 2);

        // Verify ordering (should be sorted by account_code)
        assert_eq!(deltas[0].account_code, "1000");
        assert_eq!(deltas[0].debit_delta, 10000);
        assert_eq!(deltas[0].credit_delta, 0);

        assert_eq!(deltas[1].account_code, "4000");
        assert_eq!(deltas[1].debit_delta, 0);
        assert_eq!(deltas[1].credit_delta, 10000);
    }

    #[test]
    fn test_compute_deltas_mixed_debits_credits() {
        let lines = vec![
            JournalLineInput {
                account_ref: "1000".to_string(),
                debit_minor: 10000,
                credit_minor: 0,
            },
            JournalLineInput {
                account_ref: "2000".to_string(),
                debit_minor: 5000,
                credit_minor: 0,
            },
            JournalLineInput {
                account_ref: "4000".to_string(),
                debit_minor: 0,
                credit_minor: 15000,
            },
        ];

        let deltas = compute_deltas(&lines, "EUR").unwrap();

        assert_eq!(deltas.len(), 3);
        assert_eq!(deltas[0].account_code, "1000");
        assert_eq!(deltas[0].currency, "EUR");
        assert_eq!(deltas[0].debit_delta, 10000);

        assert_eq!(deltas[1].account_code, "2000");
        assert_eq!(deltas[1].debit_delta, 5000);

        assert_eq!(deltas[2].account_code, "4000");
        assert_eq!(deltas[2].credit_delta, 15000);
    }

    #[test]
    fn test_compute_deltas_same_account_multiple_lines() {
        // Multiple lines affecting the same account should be summed
        let lines = vec![
            JournalLineInput {
                account_ref: "1000".to_string(),
                debit_minor: 10000,
                credit_minor: 0,
            },
            JournalLineInput {
                account_ref: "1000".to_string(),
                debit_minor: 5000,
                credit_minor: 0,
            },
            JournalLineInput {
                account_ref: "1000".to_string(),
                debit_minor: 0,
                credit_minor: 3000,
            },
        ];

        let deltas = compute_deltas(&lines, "USD").unwrap();

        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].account_code, "1000");
        assert_eq!(deltas[0].debit_delta, 15000); // 10000 + 5000
        assert_eq!(deltas[0].credit_delta, 3000);
    }

    #[test]
    fn test_compute_deltas_reversal_entry() {
        // Reversal entry: inverted debits/credits from original
        // Original: Debit Cash 100, Credit Revenue 100
        // Reversal: Debit Revenue 100, Credit Cash 100
        let original_lines = vec![
            JournalLineInput {
                account_ref: "1000".to_string(), // Cash
                debit_minor: 10000,
                credit_minor: 0,
            },
            JournalLineInput {
                account_ref: "4000".to_string(), // Revenue
                debit_minor: 0,
                credit_minor: 10000,
            },
        ];

        let reversal_lines = vec![
            JournalLineInput {
                account_ref: "4000".to_string(), // Revenue (inverted)
                debit_minor: 10000,
                credit_minor: 0,
            },
            JournalLineInput {
                account_ref: "1000".to_string(), // Cash (inverted)
                debit_minor: 0,
                credit_minor: 10000,
            },
        ];

        let original_deltas = compute_deltas(&original_lines, "USD").unwrap();
        let reversal_deltas = compute_deltas(&reversal_lines, "USD").unwrap();

        assert_eq!(original_deltas.len(), 2);
        assert_eq!(reversal_deltas.len(), 2);

        // Verify reversal inverts the deltas
        // Original: Cash debit +10000, Revenue credit +10000
        // Reversal: Cash credit +10000, Revenue debit +10000

        // Cash (1000)
        let original_cash = original_deltas
            .iter()
            .find(|d| d.account_code == "1000")
            .unwrap();
        let reversal_cash = reversal_deltas
            .iter()
            .find(|d| d.account_code == "1000")
            .unwrap();

        assert_eq!(original_cash.debit_delta, 10000);
        assert_eq!(original_cash.credit_delta, 0);
        assert_eq!(reversal_cash.debit_delta, 0);
        assert_eq!(reversal_cash.credit_delta, 10000);

        // Revenue (4000)
        let original_revenue = original_deltas
            .iter()
            .find(|d| d.account_code == "4000")
            .unwrap();
        let reversal_revenue = reversal_deltas
            .iter()
            .find(|d| d.account_code == "4000")
            .unwrap();

        assert_eq!(original_revenue.debit_delta, 0);
        assert_eq!(original_revenue.credit_delta, 10000);
        assert_eq!(reversal_revenue.debit_delta, 10000);
        assert_eq!(reversal_revenue.credit_delta, 0);
    }

    #[test]
    fn test_compute_deltas_empty_lines() {
        let lines: Vec<JournalLineInput> = vec![];

        let result = compute_deltas(&lines, "USD");

        assert!(result.is_err());
        match result {
            Err(DeltaError::EmptyLines) => {} // Expected
            _ => panic!("Expected EmptyLines error"),
        }
    }

    #[test]
    fn test_delta_deterministic_ordering() {
        // Verify deltas are sorted deterministically (by account_code, then currency)
        let lines = vec![
            JournalLineInput {
                account_ref: "3000".to_string(),
                debit_minor: 1000,
                credit_minor: 0,
            },
            JournalLineInput {
                account_ref: "1000".to_string(),
                debit_minor: 2000,
                credit_minor: 0,
            },
            JournalLineInput {
                account_ref: "2000".to_string(),
                debit_minor: 3000,
                credit_minor: 0,
            },
        ];

        let deltas = compute_deltas(&lines, "USD").unwrap();

        assert_eq!(deltas.len(), 3);
        assert_eq!(deltas[0].account_code, "1000"); // Sorted order
        assert_eq!(deltas[1].account_code, "2000");
        assert_eq!(deltas[2].account_code, "3000");
    }
}
