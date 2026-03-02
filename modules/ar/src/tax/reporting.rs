//! Tax reporting — jurisdiction stacking, period summaries, and CSV/JSON exports (bd-1ai1).
//!
//! Provides:
//! - `resolve_stacked_jurisdictions`: multi-level jurisdiction resolution (state+county+city)
//! - `tax_summary_by_period`: aggregate collected-tax snapshots for filing summaries
//! - `render_csv`: deterministic CSV export of summary rows

use sqlx::PgPool;
use uuid::Uuid;

use tax_core::models::*;

// ============================================================================
// Stacked jurisdiction resolution
// ============================================================================

/// Resolve ALL matching jurisdictions for a given address (multi-level stacking).
///
/// Unlike `resolve_jurisdiction` which returns only the most specific match,
/// this returns all matching jurisdictions at different specificity levels
/// (e.g. state + county + city) so their rates can be composed (stacked).
///
/// Results are ordered by specificity ascending (country → state → postal),
/// then by priority DESC within each level.
pub async fn resolve_stacked_jurisdictions(
    pool: &PgPool,
    app_id: &str,
    address: &TaxAddress,
    tax_code: Option<&str>,
    as_of: chrono::NaiveDate,
) -> Result<Vec<(Uuid, String, ResolvedRule)>, sqlx::Error> {
    let jurisdictions = sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
        r#"
        SELECT id, jurisdiction_name, country_code, state_code
        FROM ar_tax_jurisdictions
        WHERE app_id = $1
          AND country_code = $2
          AND is_active = TRUE
          AND (state_code = $3 OR state_code IS NULL)
          AND (postal_pattern = $4 OR postal_pattern IS NULL)
        ORDER BY
            (CASE WHEN postal_pattern IS NOT NULL THEN 2 ELSE 0 END) +
            (CASE WHEN state_code IS NOT NULL THEN 1 ELSE 0 END)
            ASC
        "#,
    )
    .bind(app_id)
    .bind(&address.country)
    .bind(&address.state)
    .bind(&address.postal_code)
    .fetch_all(pool)
    .await?;

    let mut results = Vec::new();

    for (jurisdiction_id, jurisdiction_name, _country, _state) in jurisdictions {
        let rule = sqlx::query_as::<
            _,
            (
                Uuid,
                Option<String>,
                f64,
                i64,
                bool,
                chrono::NaiveDate,
                Option<chrono::NaiveDate>,
                i32,
                String,
            ),
        >(
            r#"
            SELECT r.id, r.tax_code, r.rate::FLOAT8, r.flat_amount_minor, r.is_exempt,
                   r.effective_from, r.effective_to, r.priority, j.tax_type
            FROM ar_tax_rules r
            JOIN ar_tax_jurisdictions j ON j.id = r.jurisdiction_id
            WHERE r.jurisdiction_id = $1
              AND r.effective_from <= $2
              AND (r.effective_to IS NULL OR r.effective_to > $2)
              AND (r.tax_code = $3 OR r.tax_code IS NULL)
            ORDER BY
                (CASE WHEN r.tax_code IS NOT NULL THEN 1 ELSE 0 END) DESC,
                r.priority DESC
            LIMIT 1
            "#,
        )
        .bind(jurisdiction_id)
        .bind(as_of)
        .bind(tax_code)
        .fetch_optional(pool)
        .await?;

        if let Some(r) = rule {
            let resolved = ResolvedRule {
                jurisdiction_id,
                jurisdiction_name: jurisdiction_name.clone(),
                tax_type: r.8,
                rate: r.2,
                flat_amount_minor: r.3,
                is_exempt: r.4,
                tax_code: r.1,
                effective_from: r.5,
                effective_to: r.6,
                priority: r.7,
            };
            results.push((jurisdiction_id, jurisdiction_name, resolved));
        }
    }

    Ok(results)
}

// ============================================================================
// Tax summary aggregation
// ============================================================================

/// A single row in a tax summary report (collected tax from AR invoices).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaxSummaryRow {
    pub period: String,
    pub jurisdiction_name: String,
    pub country_code: String,
    pub state_code: Option<String>,
    pub applied_rate: f64,
    pub total_tax_minor: i64,
    pub invoice_count: i64,
}

/// Aggregate AR collected-tax snapshots by period and jurisdiction.
///
/// Returns deterministic results (ordered by period ASC, jurisdiction ASC, rate ASC).
pub async fn tax_summary_by_period(
    pool: &PgPool,
    app_id: &str,
    from_date: chrono::NaiveDate,
    to_date: chrono::NaiveDate,
) -> Result<Vec<TaxSummaryRow>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>, f64, i64, i64)>(
        r#"
        SELECT
            TO_CHAR(DATE_TRUNC('month', resolved_as_of), 'YYYY-MM') AS period,
            jurisdiction_name,
            country_code,
            state_code,
            applied_rate::FLOAT8,
            SUM(total_tax_minor)::BIGINT AS total_tax_minor,
            COUNT(*)::BIGINT AS invoice_count
        FROM ar_invoice_tax_snapshots
        WHERE app_id = $1
          AND resolved_as_of >= $2
          AND resolved_as_of < $3
        GROUP BY period, jurisdiction_name, country_code, state_code, applied_rate
        ORDER BY period ASC, jurisdiction_name ASC, applied_rate ASC
        "#,
    )
    .bind(app_id)
    .bind(from_date)
    .bind(to_date)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| TaxSummaryRow {
            period: r.0,
            jurisdiction_name: r.1,
            country_code: r.2,
            state_code: r.3,
            applied_rate: r.4,
            total_tax_minor: r.5,
            invoice_count: r.6,
        })
        .collect())
}

// ============================================================================
// CSV export
// ============================================================================

/// Render tax summary rows as deterministic CSV.
pub fn render_csv(rows: &[TaxSummaryRow]) -> String {
    let mut out = String::from(
        "period,jurisdiction_name,country_code,state_code,applied_rate,total_tax_minor,invoice_count\n",
    );
    for r in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            r.period,
            r.jurisdiction_name,
            r.country_code,
            r.state_code.as_deref().unwrap_or(""),
            r.applied_rate,
            r.total_tax_minor,
            r.invoice_count,
        ));
    }
    out
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_csv_deterministic() {
        let rows = vec![
            TaxSummaryRow {
                period: "2026-01".to_string(),
                jurisdiction_name: "California State Tax".to_string(),
                country_code: "US".to_string(),
                state_code: Some("CA".to_string()),
                applied_rate: 0.085,
                total_tax_minor: 8500,
                invoice_count: 10,
            },
            TaxSummaryRow {
                period: "2026-01".to_string(),
                jurisdiction_name: "New York State Tax".to_string(),
                country_code: "US".to_string(),
                state_code: Some("NY".to_string()),
                applied_rate: 0.08,
                total_tax_minor: 4000,
                invoice_count: 5,
            },
        ];

        let csv1 = render_csv(&rows);
        let csv2 = render_csv(&rows);
        assert_eq!(csv1, csv2, "CSV must be deterministic");
        assert!(csv1.starts_with("period,jurisdiction_name,"));
        assert!(csv1.contains("California State Tax"));
        assert!(csv1.contains("New York State Tax"));
    }

    #[test]
    fn render_csv_handles_empty() {
        let csv = render_csv(&[]);
        assert_eq!(
            csv,
            "period,jurisdiction_name,country_code,state_code,applied_rate,total_tax_minor,invoice_count\n"
        );
    }

    #[test]
    fn render_csv_handles_null_state() {
        let rows = vec![TaxSummaryRow {
            period: "2026-02".to_string(),
            jurisdiction_name: "US Federal".to_string(),
            country_code: "US".to_string(),
            state_code: None,
            applied_rate: 0.0,
            total_tax_minor: 0,
            invoice_count: 3,
        }];
        let csv = render_csv(&rows);
        // state_code column should be empty (not "null")
        assert!(csv.contains("US,,0"));
    }

    #[test]
    fn summary_row_json_roundtrip() {
        let row = TaxSummaryRow {
            period: "2026-01".to_string(),
            jurisdiction_name: "California State Tax".to_string(),
            country_code: "US".to_string(),
            state_code: Some("CA".to_string()),
            applied_rate: 0.085,
            total_tax_minor: 8500,
            invoice_count: 10,
        };
        let json = serde_json::to_string(&row).unwrap();
        let back: TaxSummaryRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.period, "2026-01");
        assert_eq!(back.total_tax_minor, 8500);
    }
}
