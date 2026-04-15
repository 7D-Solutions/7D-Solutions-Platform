//! CSV parser for statement transaction lines.
//!
//! The generic parser expects a header row with at least: date, description,
//! amount. Optional column: reference. Column matching is case-insensitive.
//!
//! For CC issuer-specific formats (Chase, Amex), use
//! [`parse_csv_with_format`] which dispatches to the appropriate adapter.
//! If no format is specified, the function auto-detects from CSV headers.

use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use super::LineError;

// ============================================================================
// Parsed line
// ============================================================================

#[derive(Debug, Clone)]
pub struct ParsedLine {
    pub date: NaiveDate,
    pub description: String,
    pub amount_minor: i64,
    pub reference: Option<String>,
}

// ============================================================================
// Parse result
// ============================================================================

pub struct ParseOutput {
    pub lines: Vec<ParsedLine>,
    pub errors: Vec<LineError>,
}

// ============================================================================
// Column index mapping
// ============================================================================

struct ColumnMap {
    date: usize,
    description: usize,
    amount: usize,
    reference: Option<usize>,
}

fn map_columns(headers: &csv::StringRecord) -> Result<ColumnMap, String> {
    let lower: Vec<String> = headers.iter().map(|h| h.trim().to_lowercase()).collect();

    let date = lower
        .iter()
        .position(|h| h == "date" || h == "transaction_date")
        .ok_or("Missing required column: date")?;
    let description = lower
        .iter()
        .position(|h| h == "description" || h == "memo" || h == "payee")
        .ok_or("Missing required column: description")?;
    let amount = lower
        .iter()
        .position(|h| h == "amount")
        .ok_or("Missing required column: amount")?;
    let reference = lower
        .iter()
        .position(|h| h == "reference" || h == "ref" || h == "check_number");

    Ok(ColumnMap {
        date,
        description,
        amount,
        reference,
    })
}

// ============================================================================
// Amount parsing
// ============================================================================

/// Parse a decimal amount string into minor units (cents).
///
/// Handles: "$1,234.56", "-99.90", "1234", "1234.5" (→ 123450).
fn parse_amount(raw: &str) -> Result<i64, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("amount is empty".to_string());
    }

    // Strip currency symbol and commas
    let cleaned: String = s
        .chars()
        .filter(|c| *c != '$' && *c != ',' && *c != ' ')
        .collect();

    if cleaned.is_empty() {
        return Err("amount is empty after cleanup".to_string());
    }

    let value: Decimal = cleaned
        .parse()
        .map_err(|_| format!("cannot parse amount: '{}'", raw.trim()))?;

    Ok((value * Decimal::from(100))
        .to_i64()
        .ok_or("amount out of range")?)
}

// ============================================================================
// Date parsing
// ============================================================================

/// Parse a date string, supporting YYYY-MM-DD and MM/DD/YYYY formats.
fn parse_date(raw: &str) -> Result<NaiveDate, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("date is empty".to_string());
    }

    // Try ISO format first (YYYY-MM-DD)
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d);
    }
    // US format (MM/DD/YYYY)
    if let Ok(d) = NaiveDate::parse_from_str(s, "%m/%d/%Y") {
        return Ok(d);
    }
    // US short year (MM/DD/YY)
    if let Ok(d) = NaiveDate::parse_from_str(s, "%m/%d/%y") {
        return Ok(d);
    }

    Err(format!("cannot parse date: '{}'", s))
}

// ============================================================================
// CSV parser entry point
// ============================================================================

/// Parse bank statement CSV bytes into transaction lines.
///
/// The CSV must have a header row. Lines that fail validation are collected
/// into `errors` rather than aborting the entire import.
pub fn parse_csv(data: &[u8]) -> ParseOutput {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(data);

    let headers = match reader.headers() {
        Ok(h) => h.clone(),
        Err(e) => {
            return ParseOutput {
                lines: vec![],
                errors: vec![LineError {
                    line: 1,
                    reason: format!("Cannot read CSV headers: {}", e),
                }],
            };
        }
    };

    let col_map = match map_columns(&headers) {
        Ok(m) => m,
        Err(msg) => {
            return ParseOutput {
                lines: vec![],
                errors: vec![LineError {
                    line: 1,
                    reason: msg,
                }],
            };
        }
    };

    let mut lines = Vec::new();
    let mut errors = Vec::new();

    for (idx, result) in reader.records().enumerate() {
        let line_num = idx + 2; // 1-indexed, +1 for header row
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                errors.push(LineError {
                    line: line_num,
                    reason: format!("CSV read error: {}", e),
                });
                continue;
            }
        };

        // Extract fields by column index
        let date_raw = record.get(col_map.date).unwrap_or("").trim();
        let desc_raw = record.get(col_map.description).unwrap_or("").trim();
        let amount_raw = record.get(col_map.amount).unwrap_or("").trim();
        let ref_raw = col_map.reference.and_then(|i| record.get(i)).map(str::trim);

        // Skip fully blank rows
        if date_raw.is_empty() && desc_raw.is_empty() && amount_raw.is_empty() {
            continue;
        }

        // Parse date
        let date = match parse_date(date_raw) {
            Ok(d) => d,
            Err(reason) => {
                errors.push(LineError {
                    line: line_num,
                    reason,
                });
                continue;
            }
        };

        // Parse amount
        let amount_minor = match parse_amount(amount_raw) {
            Ok(a) => a,
            Err(reason) => {
                errors.push(LineError {
                    line: line_num,
                    reason,
                });
                continue;
            }
        };

        // Validate description
        if desc_raw.is_empty() {
            errors.push(LineError {
                line: line_num,
                reason: "description is empty".to_string(),
            });
            continue;
        }

        lines.push(ParsedLine {
            date,
            description: desc_raw.to_string(),
            amount_minor,
            reference: ref_raw.filter(|s| !s.is_empty()).map(String::from),
        });
    }

    ParseOutput { lines, errors }
}

// ============================================================================
// Format-aware entry point
// ============================================================================

/// Parse statement CSV bytes with an optional format hint.
///
/// When `format` is `None`, the function auto-detects from CSV headers
/// and falls back to the generic parser if no issuer pattern matches.
pub fn parse_csv_with_format(
    data: &[u8],
    format: Option<super::adapters::CsvFormat>,
) -> ParseOutput {
    use super::adapters::{self, CsvFormat};

    let resolved =
        format.unwrap_or_else(|| adapters::detect_format(data).unwrap_or(CsvFormat::Generic));

    adapters::parse_with_format(data, resolved)
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_amount_basic() {
        assert_eq!(parse_amount("1234.56").unwrap(), 123456);
        assert_eq!(parse_amount("-99.90").unwrap(), -9990);
        assert_eq!(parse_amount("$1,234.56").unwrap(), 123456);
        assert_eq!(parse_amount("0").unwrap(), 0);
        assert_eq!(parse_amount("100").unwrap(), 10000);
    }

    #[test]
    fn parse_amount_edge_cases() {
        assert!(parse_amount("").is_err());
        assert!(parse_amount("abc").is_err());
        assert_eq!(parse_amount("  42.5  ").unwrap(), 4250);
    }

    #[test]
    fn parse_date_formats() {
        assert_eq!(
            parse_date("2024-01-15").unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
        assert_eq!(
            parse_date("01/15/2024").unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
        assert!(parse_date("").is_err());
        assert!(parse_date("not-a-date").is_err());
    }

    #[test]
    fn parse_csv_basic() {
        let csv = b"date,description,amount,reference\n\
                     2024-01-15,Coffee Shop,-4.50,TXN001\n\
                     2024-01-16,Salary,5000.00,SAL001\n";
        let result = parse_csv(csv);
        assert_eq!(result.lines.len(), 2);
        assert!(result.errors.is_empty());
        assert_eq!(result.lines[0].amount_minor, -450);
        assert_eq!(result.lines[1].amount_minor, 500000);
        assert_eq!(result.lines[0].reference.as_deref(), Some("TXN001"));
    }

    #[test]
    fn parse_csv_with_errors() {
        let csv = b"date,description,amount\n\
                     2024-01-15,Valid,-10.00\n\
                     bad-date,Also Valid,20.00\n\
                     2024-01-17,,30.00\n";
        let result = parse_csv(csv);
        assert_eq!(result.lines.len(), 1);
        assert_eq!(result.errors.len(), 2);
        assert_eq!(result.errors[0].line, 3); // bad date
        assert_eq!(result.errors[1].line, 4); // empty description
    }

    #[test]
    fn parse_csv_missing_headers() {
        let csv = b"foo,bar,baz\n1,2,3\n";
        let result = parse_csv(csv);
        assert!(result.lines.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].reason.contains("Missing required column"));
    }

    #[test]
    fn parse_csv_skips_blank_rows() {
        let csv = b"date,description,amount\n\
                     2024-01-15,Item,-5.00\n\
                     ,,\n\
                     2024-01-16,Item2,-10.00\n";
        let result = parse_csv(csv);
        assert_eq!(result.lines.len(), 2);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn parse_csv_us_date_format() {
        let csv = b"date,description,amount\n01/15/2024,Purchase,-25.00\n";
        let result = parse_csv(csv);
        assert_eq!(result.lines.len(), 1);
        assert_eq!(
            result.lines[0].date,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }
}
