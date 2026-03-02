//! Pure formatting engine for numbering patterns.
//!
//! Formatting is a deterministic, side-effect-free function that transforms
//! a raw sequence number into a human-readable document number according to
//! a policy's pattern string.
//!
//! Supported tokens:
//! - `{prefix}`  — literal prefix value
//! - `{YYYY}`    — 4-digit year
//! - `{YY}`      — 2-digit year
//! - `{MM}`      — 2-digit month (01–12)
//! - `{DD}`      — 2-digit day (01–31)
//! - `{number}`  — raw number, zero-padded to `padding` digits

use chrono::NaiveDate;

/// Formatting policy — all fields needed by the pure formatter.
#[derive(Debug, Clone)]
pub struct FormatPolicy {
    pub pattern: String,
    pub prefix: String,
    pub padding: u32,
}

/// Format a raw number according to a policy and reference date.
///
/// This function is pure: same inputs always produce the same output.
/// It never touches the database or any external state.
pub fn format_number(policy: &FormatPolicy, number: i64, reference_date: NaiveDate) -> String {
    let padded = if policy.padding > 0 {
        format!("{:0>width$}", number, width = policy.padding as usize)
    } else {
        number.to_string()
    };

    policy
        .pattern
        .replace("{prefix}", &policy.prefix)
        .replace("{YYYY}", &format!("{:04}", reference_date.year()))
        .replace("{YY}", &format!("{:02}", reference_date.year() % 100))
        .replace("{MM}", &format!("{:02}", reference_date.month()))
        .replace("{DD}", &format!("{:02}", reference_date.day()))
        .replace("{number}", &padded)
}

// ── Trait import for .year() / .month() / .day() ──────────────────────
use chrono::Datelike;

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("invalid test date")
    }

    // ── Golden tests: deterministic output for fixed inputs ────────────

    #[test]
    fn golden_simple_number_only() {
        let policy = FormatPolicy {
            pattern: "{number}".to_string(),
            prefix: String::new(),
            padding: 0,
        };
        assert_eq!(format_number(&policy, 42, date(2026, 3, 2)), "42");
    }

    #[test]
    fn golden_padded_number() {
        let policy = FormatPolicy {
            pattern: "{number}".to_string(),
            prefix: String::new(),
            padding: 5,
        };
        assert_eq!(format_number(&policy, 7, date(2026, 3, 2)), "00007");
    }

    #[test]
    fn golden_prefix_dash_number() {
        let policy = FormatPolicy {
            pattern: "{prefix}-{number}".to_string(),
            prefix: "INV".to_string(),
            padding: 5,
        };
        assert_eq!(
            format_number(&policy, 1, date(2026, 3, 2)),
            "INV-00001"
        );
    }

    #[test]
    fn golden_prefix_year_number() {
        let policy = FormatPolicy {
            pattern: "{prefix}-{YYYY}-{number}".to_string(),
            prefix: "QUO".to_string(),
            padding: 5,
        };
        assert_eq!(
            format_number(&policy, 42, date(2026, 6, 15)),
            "QUO-2026-00042"
        );
    }

    #[test]
    fn golden_prefix_yearmonth_number() {
        let policy = FormatPolicy {
            pattern: "{prefix}-{YYYY}{MM}-{number}".to_string(),
            prefix: "WO".to_string(),
            padding: 4,
        };
        assert_eq!(
            format_number(&policy, 3, date(2026, 12, 1)),
            "WO-202612-0003"
        );
    }

    #[test]
    fn golden_full_date() {
        let policy = FormatPolicy {
            pattern: "{prefix}/{YY}{MM}{DD}-{number}".to_string(),
            prefix: "REC".to_string(),
            padding: 6,
        };
        assert_eq!(
            format_number(&policy, 999, date(2026, 1, 5)),
            "REC/260105-000999"
        );
    }

    #[test]
    fn golden_no_prefix_with_date() {
        let policy = FormatPolicy {
            pattern: "{YYYY}-{number}".to_string(),
            prefix: String::new(),
            padding: 3,
        };
        assert_eq!(format_number(&policy, 1, date(2026, 3, 2)), "2026-001");
    }

    #[test]
    fn golden_no_padding() {
        let policy = FormatPolicy {
            pattern: "{prefix}-{number}".to_string(),
            prefix: "PO".to_string(),
            padding: 0,
        };
        assert_eq!(format_number(&policy, 12345, date(2026, 3, 2)), "PO-12345");
    }

    #[test]
    fn golden_literal_text_passthrough() {
        let policy = FormatPolicy {
            pattern: "DOC#{number}".to_string(),
            prefix: String::new(),
            padding: 4,
        };
        assert_eq!(format_number(&policy, 5, date(2026, 3, 2)), "DOC#0005");
    }

    #[test]
    fn golden_large_number_exceeds_padding() {
        let policy = FormatPolicy {
            pattern: "{prefix}-{number}".to_string(),
            prefix: "INV".to_string(),
            padding: 3,
        };
        // Number wider than padding — no truncation, just prints the number
        assert_eq!(
            format_number(&policy, 99999, date(2026, 3, 2)),
            "INV-99999"
        );
    }

    #[test]
    fn golden_year_boundary() {
        let policy = FormatPolicy {
            pattern: "{prefix}-{YY}-{number}".to_string(),
            prefix: "Q".to_string(),
            padding: 4,
        };
        assert_eq!(format_number(&policy, 1, date(2000, 1, 1)), "Q-00-0001");
        assert_eq!(format_number(&policy, 1, date(2099, 12, 31)), "Q-99-0001");
    }
}
