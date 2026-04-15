//! Chase credit card CSV adapter.
//!
//! Expected columns (case-insensitive):
//!   Transaction Date, Post Date, Description, Category, Type, Amount, Memo
//!
//! Chase exports charges as **negative** amounts and credits/payments as
//! positive. We preserve this convention in `amount_minor`.
//! `Category` is stored in the `reference` field of [`ParsedLine`] to
//! preserve raw audit data without adding adapter-specific columns.

use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use super::super::parser::{ParseOutput, ParsedLine};
use super::super::LineError;

// ============================================================================
// Column mapping
// ============================================================================

struct ChaseColumns {
    txn_date: usize,
    description: usize,
    amount: usize,
    category: Option<usize>,
}

fn map_columns(headers: &csv::StringRecord) -> Result<ChaseColumns, String> {
    let lower: Vec<String> = headers.iter().map(|h| h.trim().to_lowercase()).collect();

    let txn_date = lower
        .iter()
        .position(|h| h == "transaction date" || h == "trans date")
        .ok_or("Missing Chase column: Transaction Date")?;
    let description = lower
        .iter()
        .position(|h| h == "description")
        .ok_or("Missing Chase column: Description")?;
    let amount = lower
        .iter()
        .position(|h| h == "amount")
        .ok_or("Missing Chase column: Amount")?;
    let category = lower.iter().position(|h| h == "category");

    Ok(ChaseColumns {
        txn_date,
        description,
        amount,
        category,
    })
}

// ============================================================================
// Amount / date parsing (reuses conventions from generic parser)
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
    Ok((value * Decimal::from(100))
        .to_i64()
        .ok_or("amount out of range")?)
}

fn parse_date(raw: &str) -> Result<NaiveDate, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("date is empty".to_string());
    }
    // Chase uses MM/DD/YYYY
    if let Ok(d) = NaiveDate::parse_from_str(s, "%m/%d/%Y") {
        return Ok(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%m/%d/%y") {
        return Ok(d);
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d);
    }
    Err(format!("cannot parse Chase date: '{}'", s))
}

// ============================================================================
// Entry point
// ============================================================================

pub fn parse_chase_csv(data: &[u8]) -> ParseOutput {
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
                    reason: format!("Cannot read Chase CSV headers: {}", e),
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

        let date_raw = record.get(cols.txn_date).unwrap_or("").trim();
        let desc_raw = record.get(cols.description).unwrap_or("").trim();
        let amount_raw = record.get(cols.amount).unwrap_or("").trim();
        let category = cols.category.and_then(|i| record.get(i)).map(str::trim);

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
            reference: category.filter(|s| !s.is_empty()).map(String::from),
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
    fn parse_chase_basic() {
        let csv = b"Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n\
                     01/15/2024,01/16/2024,STARBUCKS STORE,Food & Drink,Sale,-4.50,\n\
                     01/18/2024,01/19/2024,AMAZON.COM,Shopping,Sale,-89.99,\n\
                     01/20/2024,01/20/2024,PAYMENT RECEIVED,,Payment,250.00,\n";
        let result = parse_chase_csv(csv);
        assert_eq!(result.lines.len(), 3);
        assert!(result.errors.is_empty());
        assert_eq!(result.lines[0].amount_minor, -450);
        assert_eq!(result.lines[0].description, "STARBUCKS STORE");
        assert_eq!(result.lines[0].reference.as_deref(), Some("Food & Drink"));
        assert_eq!(result.lines[1].amount_minor, -8999);
        assert_eq!(result.lines[2].amount_minor, 25000);
        assert_eq!(
            result.lines[0].date,
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }

    #[test]
    fn parse_chase_bad_date() {
        let csv = b"Transaction Date,Post Date,Description,Category,Type,Amount\n\
                     bad-date,01/16/2024,Item,Cat,Sale,-10.00\n";
        let result = parse_chase_csv(csv);
        assert!(result.lines.is_empty());
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn parse_chase_missing_headers() {
        let csv = b"Date,Desc,Amt\n1,2,3\n";
        let result = parse_chase_csv(csv);
        assert!(result.lines.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].reason.contains("Missing Chase column"));
    }

    #[test]
    fn parse_chase_skips_blank_rows() {
        let csv = b"Transaction Date,Post Date,Description,Category,Type,Amount\n\
                     01/15/2024,01/16/2024,Item,Cat,Sale,-5.00\n\
                     ,,,,,,\n\
                     01/17/2024,01/18/2024,Item2,Cat,Sale,-10.00\n";
        let result = parse_chase_csv(csv);
        assert_eq!(result.lines.len(), 2);
        assert!(result.errors.is_empty());
    }
}
