//! CC statement CSV adapters — issuer-specific format normalisation.
//!
//! Each adapter converts a proprietary CSV layout into the shared
//! [`ParseOutput`](super::parser::ParseOutput) used by the import pipeline.
//! Format can be specified explicitly or auto-detected from CSV headers.

pub mod amex;
pub mod chase;

use serde::{Deserialize, Serialize};

use super::parser::ParseOutput;

// ============================================================================
// Format enum
// ============================================================================

/// Supported CSV formats for statement import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvFormat {
    /// Generic bank CSV (date, description, amount, optional reference).
    Generic,
    /// Chase credit card export (Transaction Date, Post Date, Description,
    /// Category, Type, Amount, Memo).
    ChaseCredit,
    /// American Express credit card export (Date, Description, Amount —
    /// charges are positive, credits negative).
    AmexCredit,
}

// ============================================================================
// Auto-detection
// ============================================================================

/// Try to detect the CSV format from the first (header) line.
///
/// Returns `None` if no issuer-specific pattern matches — caller should
/// fall back to `CsvFormat::Generic`.
pub fn detect_format(data: &[u8]) -> Option<CsvFormat> {
    // Read just the first line (header row)
    let header_end = data.iter().position(|&b| b == b'\n').unwrap_or(data.len());
    let header = String::from_utf8_lossy(&data[..header_end]).to_lowercase();
    let header = header.trim();

    if header.contains("post date") && header.contains("category") && header.contains("type") {
        return Some(CsvFormat::ChaseCredit);
    }

    // Amex: exactly Date, Description, Amount (no "post date", no "category")
    // Some Amex exports also include "Reference" or "Card Member"
    if !header.contains("post date")
        && !header.contains("category")
        && (header.contains("card member")
            || header.contains("account #")
            || is_amex_three_column(header))
    {
        return Some(CsvFormat::AmexCredit);
    }

    None
}

/// Matches the minimal Amex 3-column pattern: date + description + amount,
/// with no extra bank-style columns like "reference", "ref", "check_number",
/// "memo", or "payee" which indicate a generic bank CSV.
fn is_amex_three_column(header: &str) -> bool {
    let cols: Vec<&str> = header.split(',').map(str::trim).collect();
    if cols.len() < 3 || cols.len() > 5 {
        return false;
    }
    // Must have the core Amex trio
    let has_core = cols[0] == "date" && cols.contains(&"amount") && cols.contains(&"description");
    if !has_core {
        return false;
    }
    // Reject if it looks like a generic bank CSV (has reference/ref/check columns)
    let has_bank_cols = cols.iter().any(|c| {
        *c == "reference" || *c == "ref" || *c == "check_number" || *c == "memo" || *c == "payee"
    });
    !has_bank_cols
}

// ============================================================================
// Dispatch
// ============================================================================

/// Parse CSV bytes using the specified format.
pub fn parse_with_format(data: &[u8], format: CsvFormat) -> ParseOutput {
    match format {
        CsvFormat::Generic => super::parser::parse_csv(data),
        CsvFormat::ChaseCredit => chase::parse_chase_csv(data),
        CsvFormat::AmexCredit => amex::parse_amex_csv(data),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_chase_format() {
        let h = b"Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n";
        assert_eq!(detect_format(h), Some(CsvFormat::ChaseCredit));
    }

    #[test]
    fn detect_amex_format() {
        let h = b"Date,Description,Amount\n";
        assert_eq!(detect_format(h), Some(CsvFormat::AmexCredit));
    }

    #[test]
    fn detect_amex_with_reference() {
        let h = b"Date,Reference,Description,Card Member,Amount\n";
        assert_eq!(detect_format(h), Some(CsvFormat::AmexCredit));
    }

    #[test]
    fn detect_generic_fallback() {
        let h = b"date,description,amount,reference\n";
        assert_eq!(detect_format(h), None);
    }
}
