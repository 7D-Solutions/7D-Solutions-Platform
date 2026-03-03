//! Format adapters for QuickBooks (IIF) and Xero (CSV) exports.

use crate::repos::account_repo::AccountType;

/// Account row used by format adapters
pub struct AccountRow {
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
}

/// Journal entry row with lines, used by format adapters
pub struct JournalEntryRow {
    pub posted_at: String,
    pub description: String,
    pub reference_id: String,
    pub lines: Vec<JournalLineRow>,
}

pub struct JournalLineRow {
    pub account_code: String,
    pub account_name: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub memo: Option<String>,
}

// ---------------------------------------------------------------------------
// QuickBooks IIF format
// ---------------------------------------------------------------------------

fn qb_account_type(at: &AccountType) -> &'static str {
    match at {
        AccountType::Asset => "OA",
        AccountType::Liability => "OLIAB",
        AccountType::Equity => "EQUITY",
        AccountType::Revenue => "INC",
        AccountType::Expense => "EXP",
    }
}

pub fn quickbooks_chart_of_accounts(accounts: &[AccountRow]) -> String {
    let mut out = String::from("!ACCNT\tNAME\tACCNTTYPE\tACCNUM\n");
    for a in accounts {
        out.push_str(&format!(
            "ACCNT\t{}\t{}\t{}\n",
            a.name,
            qb_account_type(&a.account_type),
            a.code,
        ));
    }
    out
}

pub fn quickbooks_journal_entries(entries: &[JournalEntryRow]) -> String {
    let mut out = String::from("!TRNS\tTRNSID\tTRNSTYPE\tDATE\tACCNT\tAMOUNT\tMEMO\n");
    out.push_str("!SPL\tSPLID\tTRNSTYPE\tDATE\tACCNT\tAMOUNT\tMEMO\n");
    out.push_str("!ENDTRNS\n");

    for entry in entries {
        let date = &entry.posted_at;
        for (i, line) in entry.lines.iter().enumerate() {
            let amount = minor_to_signed(line.debit_minor, line.credit_minor);
            let memo = line.memo.as_deref().unwrap_or(&entry.description);
            let tag = if i == 0 { "TRNS" } else { "SPL" };
            out.push_str(&format!(
                "{}\t{}\tGENERAL JOURNAL\t{}\t{}\t{}\t{}\n",
                tag, entry.reference_id, date, line.account_name, amount, memo,
            ));
        }
        out.push_str("ENDTRNS\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Xero CSV format
// ---------------------------------------------------------------------------

fn xero_account_type(at: &AccountType) -> &'static str {
    match at {
        AccountType::Asset => "CURRENT",
        AccountType::Liability => "CURRLIAB",
        AccountType::Equity => "EQUITY",
        AccountType::Revenue => "REVENUE",
        AccountType::Expense => "OVERHEADS",
    }
}

pub fn xero_chart_of_accounts(accounts: &[AccountRow]) -> String {
    let mut out = String::from("*Code,*Name,*Type\n");
    for a in accounts {
        out.push_str(&format!(
            "{},{},{}\n",
            csv_escape(&a.code),
            csv_escape(&a.name),
            xero_account_type(&a.account_type),
        ));
    }
    out
}

pub fn xero_journal_entries(entries: &[JournalEntryRow]) -> String {
    let mut out = String::from("*Date,*Description,*AccountCode,*Debit,*Credit,Reference\n");
    for entry in entries {
        let date = &entry.posted_at;
        for line in &entry.lines {
            let debit = if line.debit_minor > 0 {
                format_minor(line.debit_minor)
            } else {
                String::new()
            };
            let credit = if line.credit_minor > 0 {
                format_minor(line.credit_minor)
            } else {
                String::new()
            };
            let memo = line.memo.as_deref().unwrap_or(&entry.description);
            out.push_str(&format!(
                "{},{},{},{},{},{}\n",
                date,
                csv_escape(memo),
                csv_escape(&line.account_code),
                debit,
                credit,
                csv_escape(&entry.reference_id),
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert debit/credit minor units to a signed decimal string.
/// Debits are positive, credits are negative (QuickBooks convention).
fn minor_to_signed(debit_minor: i64, credit_minor: i64) -> String {
    let cents = debit_minor - credit_minor;
    format!("{:.2}", cents as f64 / 100.0)
}

fn format_minor(minor: i64) -> String {
    format!("{:.2}", minor as f64 / 100.0)
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_minor_to_signed() {
        assert_eq!(minor_to_signed(150000, 0), "1500.00");
        assert_eq!(minor_to_signed(0, 150000), "-1500.00");
    }

    #[test]
    fn test_csv_escape() {
        assert_eq!(csv_escape("hello"), "hello");
        assert_eq!(csv_escape("hello,world"), "\"hello,world\"");
    }

    #[test]
    fn test_qb_coa_format() {
        let accounts = vec![AccountRow {
            code: "1100".to_string(),
            name: "AR".to_string(),
            account_type: AccountType::Asset,
        }];
        let output = quickbooks_chart_of_accounts(&accounts);
        assert!(output.contains("!ACCNT\tNAME\tACCNTTYPE\tACCNUM"));
        assert!(output.contains("ACCNT\tAR\tOA\t1100"));
    }

    #[test]
    fn test_xero_coa_format() {
        let accounts = vec![AccountRow {
            code: "4000".to_string(),
            name: "Revenue".to_string(),
            account_type: AccountType::Revenue,
        }];
        let output = xero_chart_of_accounts(&accounts);
        assert!(output.contains("*Code,*Name,*Type"));
        assert!(output.contains("4000,Revenue,REVENUE"));
    }
}
