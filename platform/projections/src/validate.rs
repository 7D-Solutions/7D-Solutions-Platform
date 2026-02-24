//! Projection table name validation
//!
//! Provides a two-layer defense against SQL injection in dynamic table queries:
//! 1. Regex: only lowercase snake_case identifiers allowed
//! 2. Allowlist: only known projection table names are accepted
//!
//! Both layers must pass. The allowlist is the primary defense; regex is defense-in-depth.

use std::sync::LazyLock;

/// Known projection table names across all modules.
///
/// Add new tables here when creating new projections.
pub const ALLOWED_PROJECTION_TABLES: &[&str] = &[
    // Platform
    "projection_cursors",
    // GL
    "account_balances",
    "period_summary_snapshots",
    // AR
    "ar_tax_quote_cache",
    // Reporting
    "rpt_trial_balance_cache",
    "rpt_statement_cache",
    "rpt_ar_aging_cache",
    "rpt_ap_aging_cache",
    "rpt_cashflow_cache",
    "rpt_kpi_cache",
    "rpt_open_invoices_cache",
    // Consolidation
    "csl_trial_balance_cache",
    "csl_statement_cache",
];

/// Known safe column names for ORDER BY clauses.
pub const ALLOWED_ORDER_COLUMNS: &[&str] = &[
    "tenant_id",
    "id",
    "created_at",
    "updated_at",
    "period",
    "account_id",
];

/// Regex: lowercase snake_case identifier (letters, digits, underscores).
static IDENTIFIER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[a-z_][a-z0-9_]*$").unwrap());

/// Error returned when validation fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ValidationError {}

/// Validate a projection table name against regex AND allowlist.
///
/// Returns the validated name on success, or a `ValidationError` on failure.
pub fn validate_projection_name(name: &str) -> Result<&str, ValidationError> {
    if !IDENTIFIER_RE.is_match(name) {
        return Err(ValidationError {
            message: format!("Invalid identifier format: {name:?}"),
        });
    }

    if !ALLOWED_PROJECTION_TABLES.contains(&name) {
        return Err(ValidationError {
            message: format!("Unknown projection table: {name:?}"),
        });
    }

    Ok(name)
}

/// Validate an ORDER BY column name against regex AND allowlist.
pub fn validate_order_column(name: &str) -> Result<&str, ValidationError> {
    if !IDENTIFIER_RE.is_match(name) {
        return Err(ValidationError {
            message: format!("Invalid identifier format: {name:?}"),
        });
    }

    if !ALLOWED_ORDER_COLUMNS.contains(&name) {
        return Err(ValidationError {
            message: format!("Unknown order column: {name:?}"),
        });
    }

    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Table name validation ───────────────────────────────────────────

    #[test]
    fn valid_allowlisted_table() {
        assert_eq!(
            validate_projection_name("account_balances").unwrap(),
            "account_balances"
        );
    }

    #[test]
    fn rejects_sql_injection_semicolon() {
        let result = validate_projection_name("users; DROP TABLE credentials; --");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Invalid identifier"));
    }

    #[test]
    fn rejects_schema_qualified_name() {
        let result = validate_projection_name("public.projection_cursors");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Invalid identifier"));
    }

    #[test]
    fn rejects_quoted_identifier() {
        let result = validate_projection_name("\"projection_cursors\"");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_unknown_but_valid_format() {
        let result = validate_projection_name("not_a_real_table");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Unknown projection"));
    }

    #[test]
    fn rejects_empty_string() {
        let result = validate_projection_name("");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_uppercase() {
        let result = validate_projection_name("Account_Balances");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_spaces() {
        let result = validate_projection_name("account balances");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_leading_digit() {
        let result = validate_projection_name("1_table");
        assert!(result.is_err());
    }

    // ── Order column validation ─────────────────────────────────────────

    #[test]
    fn valid_order_column() {
        assert_eq!(validate_order_column("tenant_id").unwrap(), "tenant_id");
    }

    #[test]
    fn rejects_sql_in_order_by() {
        let result = validate_order_column("id; DROP TABLE users");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_unknown_column() {
        let result = validate_order_column("secret_column");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Unknown order column"));
    }
}
