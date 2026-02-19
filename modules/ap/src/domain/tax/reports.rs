//! AP tax reporting — paid-tax summaries by period/jurisdiction (bd-1ai1).
//!
//! Aggregates committed tax snapshots from `ap_tax_snapshots` using the
//! jurisdiction data embedded in the `tax_by_line` JSONB column.
//! Produces deterministic CSV/JSON exports for filing summaries.

use sqlx::PgPool;

// ============================================================================
// Summary types
// ============================================================================

/// A single row in an AP paid-tax summary report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApTaxSummaryRow {
    pub period: String,
    pub jurisdiction: String,
    pub tax_type: String,
    pub applied_rate: f64,
    pub total_tax_minor: i64,
    pub bill_count: i64,
}

// ============================================================================
// Aggregation
// ============================================================================

/// Aggregate AP paid-tax snapshots by period, jurisdiction, and rate.
///
/// Only includes committed snapshots (tax actually paid). Voided snapshots
/// are excluded. Results are deterministically ordered.
pub async fn ap_tax_summary_by_period(
    pool: &PgPool,
    tenant_id: &str,
    from_date: chrono::NaiveDate,
    to_date: chrono::NaiveDate,
) -> Result<Vec<ApTaxSummaryRow>, sqlx::Error> {
    // Unnest tax_by_line JSONB array to extract per-line jurisdiction data,
    // then aggregate by period + jurisdiction + tax_type + rate.
    let rows = sqlx::query_as::<_, (String, String, String, f64, i64, i64)>(
        r#"
        SELECT
            TO_CHAR(DATE_TRUNC('month', s.committed_at), 'YYYY-MM') AS period,
            line_data->>'jurisdiction' AS jurisdiction,
            line_data->>'tax_type' AS tax_type,
            (line_data->>'rate')::FLOAT8 AS applied_rate,
            SUM((line_data->>'tax_minor')::BIGINT) AS total_tax_minor,
            COUNT(DISTINCT s.bill_id)::BIGINT AS bill_count
        FROM ap_tax_snapshots s,
             jsonb_array_elements(s.tax_by_line) AS line_data
        WHERE s.tenant_id = $1
          AND s.status = 'committed'
          AND s.committed_at >= $2
          AND s.committed_at < $3
        GROUP BY period, jurisdiction, tax_type, applied_rate
        ORDER BY period ASC, jurisdiction ASC, applied_rate ASC
        "#,
    )
    .bind(tenant_id)
    .bind(from_date)
    .bind(to_date)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ApTaxSummaryRow {
            period: r.0,
            jurisdiction: r.1,
            tax_type: r.2,
            applied_rate: r.3,
            total_tax_minor: r.4,
            bill_count: r.5,
        })
        .collect())
}

// ============================================================================
// CSV export
// ============================================================================

/// Render AP tax summary rows as deterministic CSV.
pub fn render_csv(rows: &[ApTaxSummaryRow]) -> String {
    let mut out =
        String::from("period,jurisdiction,tax_type,applied_rate,total_tax_minor,bill_count\n");
    for r in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{}\n",
            r.period, r.jurisdiction, r.tax_type, r.applied_rate, r.total_tax_minor, r.bill_count,
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
            ApTaxSummaryRow {
                period: "2026-01".to_string(),
                jurisdiction: "California State Tax".to_string(),
                tax_type: "sales_tax".to_string(),
                applied_rate: 0.085,
                total_tax_minor: 4250,
                bill_count: 5,
            },
            ApTaxSummaryRow {
                period: "2026-01".to_string(),
                jurisdiction: "zero-tax".to_string(),
                tax_type: "none".to_string(),
                applied_rate: 0.0,
                total_tax_minor: 0,
                bill_count: 3,
            },
        ];

        let csv1 = render_csv(&rows);
        let csv2 = render_csv(&rows);
        assert_eq!(csv1, csv2, "CSV must be deterministic");
        assert!(csv1.starts_with("period,jurisdiction,"));
        assert!(csv1.contains("California State Tax"));
    }

    #[test]
    fn render_csv_empty() {
        let csv = render_csv(&[]);
        assert_eq!(
            csv,
            "period,jurisdiction,tax_type,applied_rate,total_tax_minor,bill_count\n"
        );
    }

    #[test]
    fn summary_row_json_roundtrip() {
        let row = ApTaxSummaryRow {
            period: "2026-02".to_string(),
            jurisdiction: "California State Tax".to_string(),
            tax_type: "sales_tax".to_string(),
            applied_rate: 0.085,
            total_tax_minor: 8500,
            bill_count: 10,
        };
        let json = serde_json::to_string(&row).unwrap();
        let back: ApTaxSummaryRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.period, "2026-02");
        assert_eq!(back.total_tax_minor, 8500);
    }
}
