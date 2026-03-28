//! American Express credit card CSV adapter.
//!
//! Expected columns (case-insensitive):
//!   Date, Description, Amount
//!   — or —
//!   Date, Reference, Description, Card Member, Amount
//!
//! Amex convention: **charges are positive**, credits/payments are negative.
//! We normalise to the same convention as bank imports (charges negative,
//! credits positive) by flipping the sign so all statement lines follow a
//! consistent debit/credit direction.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::super::parser::{ParseOutput, ParsedLine};
use super::super::LineError;

// ============================================================================
// Column mapping
// ============================================================================

struct AmexColumns {
    date: usize,
    description: usize,
    amount: usize,
    reference: Option<usize>,
}

fn map_columns(headers: &csv::StringRecord) -> Result<AmexColumns, String> {
    let lower: Vec<String> = headers.iter().map(|h| h.trim().to_lowercase()).collect();

    let date = lower
        .iter()
        .position(|h| h == "date")
        .ok_or("Missing Amex column: Date")?;
    let description = lower
        .iter()
        .position(|h| h == "description")
        .ok_or("Missing Amex column: Description")?;
    let amount = lower
        .iter()
        .position(|h| h == "amount")
        .ok_or("Missing Amex column: Amount")?;
    let reference = lower.iter().position(|h| h == "reference");

    Ok(AmexColumns {
        date,
        description,
        amount,
        reference,
    })
}

// ============================================================================
// Amount / date parsing
// ============================================================================

fn parse_amount(raw: &str) -> Result<i64, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("amount is empty".to_string());
    }
    let cleaned: String = s
        .chars()
        .filter(|c| *c != '$' && *c != ',' && *c != ' ')
        .collect();
    if cleaned.is_empty() {
        return Err("amount is empty after cleanup".to_string());
    }
    let value: Decimal = cleaned
        .parse()
        .map_err(|_| format!("cannot parse amount: '{}'", s))?;
    // Amex: positive = charge, negative = credit. Flip sign to match bank
    // convention (charges negative, credits positive).
    Ok((-value * Decimal::from(100))
        .to_i64()
        .ok_or("amount out of range")?)
}

fn parse_date(raw: &str) -> Result<NaiveDate, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("date is empty".to_string());
    }
    // Amex uses MM/DD/YYYY
    if let Ok(d) = NaiveDate::parse_from_str(s, "%m/%d/%Y") {
        return Ok(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%m/%d/%y") {
        return Ok(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d);
    }
    Err(format!("cannot parse Amex date: '{}'", s))
}

// ============================================================================
// Entry point
// ============================================================================

pub fn parse_amex_csv(data: &[u8]) -> ParseOutput {
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
                    reason: format!("Cannot read Amex CSV headers: {}", e),
                }],
            };
        }
    };

    let cols = match map_columns(&headers) {
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
        let line_num = idx + 2;
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

        let date_raw = record.get(cols.date).unwrap_or("").trim();
        let desc_raw = record.get(cols.description).unwrap_or("").trim();
        let amount_raw = record.get(cols.amount).unwrap_or("").trim();
        let ref_raw = cols.reference.and_then(|i| record.get(i)).map(str::trim);

        // Skip blank rows
        if date_raw.is_empty() && desc_raw.is_empty() && amount_raw.is_empty() {
            continue;
        }

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
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_amex_basic_3col() {
        let csv = b"Date,Description,Amount\n\
                     01/15/2024,STARBUCKS STORE,4.50\n\
                     01/18/2024,AMAZON.COM,89.99\n\
                     01/20/2024,PAYMENT RECEIVED,-250.00\n";
        let result = parse_amex_csv(csv);
        assert_eq!(result.lines.len(), 3);
        assert!(result.errors.is_empty());
        // Amex positive charge → negative (normalised)
        assert_eq!(result.lines[0].amount_minor, -450);
        assert_eq!(result.lines[1].amount_minor, -8999);
        // Amex negative (credit/payment) → positive
        assert_eq!(result.lines[2].amount_minor, 25000);
    }

    #[test]
    fn parse_amex_with_reference() {
        let csv = b"Date,Reference,Description,Amount\n\
                     01/15/2024,REF001,GROCERY STORE,52.30\n";
        let result = parse_amex_csv(csv);
        assert_eq!(result.lines.len(), 1);
        assert_eq!(result.lines[0].reference.as_deref(), Some("REF001"));
        assert_eq!(result.lines[0].amount_minor, -5230);
    }

    #[test]
    fn parse_amex_bad_date() {
        let csv = b"Date,Description,Amount\n\
                     bad-date,Item,10.00\n";
        let result = parse_amex_csv(csv);
        assert!(result.lines.is_empty());
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn parse_amex_missing_columns() {
        let csv = b"Datum,Beschreibung,Betrag\n1,2,3\n";
        let result = parse_amex_csv(csv);
        assert!(result.lines.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].reason.contains("Missing Amex column"));
    }

    #[test]
    fn parse_amex_skips_blank_rows() {
        let csv = b"Date,Description,Amount\n\
                     01/15/2024,Item,5.00\n\
                     ,,\n\
                     01/17/2024,Item2,10.00\n";
        let result = parse_amex_csv(csv);
        assert_eq!(result.lines.len(), 2);
        assert!(result.errors.is_empty());
    }
}
