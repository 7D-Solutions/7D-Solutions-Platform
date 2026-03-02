//! Jurisdiction resolution and tax rule management (bd-360).
//!
//! Resolves the applicable tax jurisdiction and rules for a given address,
//! persists snapshots for deterministic invoice tax, and provides seeding helpers.

use sqlx::PgPool;
use uuid::Uuid;

use tax_core::models::*;
use tax_core::TaxProviderError;

// ============================================================================
// Resolution hash
// ============================================================================

/// Compute a deterministic resolution hash from (address, tax_code, as_of_date).
///
/// Used to validate that the same inputs produce the same jurisdiction resolution.
pub fn compute_resolution_hash(
    address: &TaxAddress,
    tax_code: Option<&str>,
    as_of: chrono::NaiveDate,
) -> String {
    use sha2::{Digest, Sha256};

    let canonical = serde_json::json!({
        "country": address.country,
        "state": address.state,
        "postal_code": address.postal_code,
        "tax_code": tax_code,
        "as_of": as_of.to_string(),
    });

    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let hash = Sha256::digest(&bytes);
    hex::encode(hash)
}

// ============================================================================
// Jurisdiction resolution
// ============================================================================

/// Resolve jurisdiction and applicable rules for a given address and tax code.
///
/// Resolution algorithm (most-specific-first):
/// 1. Match by (app_id, country_code, state_code, postal_pattern) with is_active=true
/// 2. Fall back to (app_id, country_code, state_code, NULL postal_pattern)
/// 3. Fall back to (app_id, country_code, NULL state, NULL postal)
/// 4. Within matched jurisdiction, find rules where:
///    a. tax_code matches exactly (highest priority)
///    b. tax_code IS NULL (default rule for jurisdiction)
///    c. effective_from <= as_of_date AND (effective_to IS NULL OR effective_to > as_of_date)
/// 5. Order by priority DESC, pick the first matching rule
///
/// Returns None if no jurisdiction is configured for the given region.
pub async fn resolve_jurisdiction(
    pool: &PgPool,
    app_id: &str,
    address: &TaxAddress,
    tax_code: Option<&str>,
    as_of: chrono::NaiveDate,
) -> Result<Option<(Uuid, String, ResolvedRule)>, sqlx::Error> {
    // Step 1: Find the most specific jurisdiction
    let jurisdiction = sqlx::query_as::<_, (Uuid, String, String, Option<String>)>(
        r#"
        SELECT id, jurisdiction_name, country_code, state_code
        FROM ar_tax_jurisdictions
        WHERE app_id = $1
          AND country_code = $2
          AND is_active = TRUE
          AND (state_code = $3 OR state_code IS NULL)
          AND (postal_pattern = $4 OR postal_pattern IS NULL)
        ORDER BY
            -- Most specific first: postal > state > country
            (CASE WHEN postal_pattern IS NOT NULL THEN 2 ELSE 0 END) +
            (CASE WHEN state_code IS NOT NULL THEN 1 ELSE 0 END)
            DESC
        LIMIT 1
        "#,
    )
    .bind(app_id)
    .bind(&address.country)
    .bind(&address.state)
    .bind(&address.postal_code)
    .fetch_optional(pool)
    .await?;

    let (jurisdiction_id, jurisdiction_name, _country, _state) = match jurisdiction {
        Some(j) => j,
        None => return Ok(None),
    };

    // Step 2: Find the best matching rule within this jurisdiction
    let rule = sqlx::query_as::<
        _,
        (
            Uuid,                      // id
            Option<String>,            // tax_code
            f64,                       // rate (as NUMERIC → f64)
            i64,                       // flat_amount_minor
            bool,                      // is_exempt
            chrono::NaiveDate,         // effective_from
            Option<chrono::NaiveDate>, // effective_to
            i32,                       // priority
            String,                    // tax_type (from jurisdiction)
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
            -- Prefer specific tax_code match over default (NULL)
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

    match rule {
        Some(r) => {
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
            Ok(Some((jurisdiction_id, jurisdiction_name, resolved)))
        }
        None => Ok(None),
    }
}

// ============================================================================
// Snapshot persistence
// ============================================================================

/// Persist a jurisdiction resolution snapshot for an invoice.
///
/// Uses ON CONFLICT to handle recalculation — replaces the existing snapshot.
pub async fn persist_jurisdiction_snapshot(
    pool: &PgPool,
    app_id: &str,
    invoice_id: &str,
    snapshot: &JurisdictionSnapshot,
) -> Result<Uuid, sqlx::Error> {
    let resolved_rules_json =
        serde_json::to_value(&snapshot.resolved_rules).unwrap_or_else(|_| serde_json::json!([]));
    let ship_to_json =
        serde_json::to_value(&snapshot.ship_to_address).unwrap_or_else(|_| serde_json::json!({}));

    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoice_tax_snapshots (
            app_id, invoice_id, jurisdiction_id, jurisdiction_name,
            country_code, state_code, ship_to_address, resolved_rules,
            total_tax_minor, tax_code, applied_rate, resolution_hash,
            resolved_as_of
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT (app_id, invoice_id) DO UPDATE SET
            jurisdiction_id = EXCLUDED.jurisdiction_id,
            jurisdiction_name = EXCLUDED.jurisdiction_name,
            country_code = EXCLUDED.country_code,
            state_code = EXCLUDED.state_code,
            ship_to_address = EXCLUDED.ship_to_address,
            resolved_rules = EXCLUDED.resolved_rules,
            total_tax_minor = EXCLUDED.total_tax_minor,
            tax_code = EXCLUDED.tax_code,
            applied_rate = EXCLUDED.applied_rate,
            resolution_hash = EXCLUDED.resolution_hash,
            resolved_as_of = EXCLUDED.resolved_as_of
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .bind(snapshot.jurisdiction_id)
    .bind(&snapshot.jurisdiction_name)
    .bind(&snapshot.country_code)
    .bind(&snapshot.state_code)
    .bind(&ship_to_json)
    .bind(&resolved_rules_json)
    .bind(snapshot.total_tax_minor)
    .bind(&snapshot.tax_code)
    .bind(snapshot.applied_rate)
    .bind(&snapshot.resolution_hash)
    .bind(snapshot.resolved_as_of)
    .fetch_one(pool)
    .await?;

    Ok(id)
}

/// Retrieve the persisted jurisdiction snapshot for an invoice.
pub async fn get_jurisdiction_snapshot(
    pool: &PgPool,
    app_id: &str,
    invoice_id: &str,
) -> Result<Option<JurisdictionSnapshot>, sqlx::Error> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,              // jurisdiction_id
            String,            // jurisdiction_name
            String,            // country_code
            Option<String>,    // state_code
            serde_json::Value, // ship_to_address
            serde_json::Value, // resolved_rules
            i64,               // total_tax_minor
            Option<String>,    // tax_code
            f64,               // applied_rate (NUMERIC → f64)
            String,            // resolution_hash
            chrono::NaiveDate, // resolved_as_of
        ),
    >(
        r#"
        SELECT jurisdiction_id, jurisdiction_name, country_code, state_code,
               ship_to_address, resolved_rules, total_tax_minor, tax_code,
               applied_rate::FLOAT8, resolution_hash, resolved_as_of
        FROM ar_invoice_tax_snapshots
        WHERE app_id = $1 AND invoice_id = $2
        "#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            let ship_to: TaxAddress = serde_json::from_value(r.4).unwrap_or(TaxAddress {
                line1: String::new(),
                line2: None,
                city: String::new(),
                state: String::new(),
                postal_code: String::new(),
                country: String::new(),
            });
            let resolved_rules: Vec<ResolvedRule> = serde_json::from_value(r.5).unwrap_or_default();

            Ok(Some(JurisdictionSnapshot {
                jurisdiction_id: r.0,
                jurisdiction_name: r.1,
                country_code: r.2,
                state_code: r.3,
                ship_to_address: ship_to,
                resolved_rules,
                total_tax_minor: r.6,
                tax_code: r.7,
                applied_rate: r.8,
                resolution_hash: r.9,
                resolved_as_of: r.10,
            }))
        }
        None => Ok(None),
    }
}

// ============================================================================
// Combined resolution + persistence
// ============================================================================

/// Resolve jurisdiction, compute tax for line items, and persist the snapshot.
///
/// This is the main entry point for jurisdiction-based tax calculation on invoices.
/// It combines jurisdiction resolution with tax computation and snapshot persistence
/// in a single deterministic operation.
///
/// Returns the snapshot (for the caller to use) or None if no jurisdiction is configured.
pub async fn resolve_and_persist_tax(
    pool: &PgPool,
    app_id: &str,
    invoice_id: &str,
    address: &TaxAddress,
    tax_code: Option<&str>,
    line_items: &[TaxLineItem],
    as_of: chrono::NaiveDate,
) -> Result<Option<JurisdictionSnapshot>, TaxProviderError> {
    let resolution = resolve_jurisdiction(pool, app_id, address, tax_code, as_of)
        .await
        .map_err(|e| {
            TaxProviderError::Provider(format!("jurisdiction resolution failed: {}", e))
        })?;

    let (jurisdiction_id, jurisdiction_name, rule) = match resolution {
        Some(r) => r,
        None => return Ok(None),
    };

    // Compute tax for each line item using the resolved rule
    let mut total_tax: i64 = 0;
    let applied_rate = if rule.is_exempt { 0.0 } else { rule.rate };

    for item in line_items {
        if rule.is_exempt {
            continue;
        }
        let tax =
            ((item.amount_minor as f64) * applied_rate).round() as i64 + rule.flat_amount_minor;
        total_tax += tax;
    }

    let resolution_hash = compute_resolution_hash(address, tax_code, as_of);

    let snapshot = JurisdictionSnapshot {
        jurisdiction_id,
        jurisdiction_name,
        country_code: address.country.clone(),
        state_code: Some(address.state.clone()),
        ship_to_address: address.clone(),
        resolved_rules: vec![rule],
        total_tax_minor: total_tax,
        tax_code: tax_code.map(String::from),
        applied_rate,
        resolution_hash,
        resolved_as_of: as_of,
    };

    persist_jurisdiction_snapshot(pool, app_id, invoice_id, &snapshot)
        .await
        .map_err(|e| TaxProviderError::Provider(format!("snapshot persist failed: {}", e)))?;

    Ok(Some(snapshot))
}

// ============================================================================
// Jurisdiction seeding helpers (bd-360)
// ============================================================================

/// Insert a jurisdiction record. Returns the jurisdiction UUID.
pub async fn insert_jurisdiction(
    pool: &PgPool,
    app_id: &str,
    country_code: &str,
    state_code: Option<&str>,
    postal_pattern: Option<&str>,
    jurisdiction_name: &str,
    tax_type: &str,
) -> Result<Uuid, sqlx::Error> {
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO ar_tax_jurisdictions (
            app_id, country_code, state_code, postal_pattern,
            jurisdiction_name, tax_type, is_active
        )
        VALUES ($1, $2, $3, $4, $5, $6, TRUE)
        ON CONFLICT (app_id, country_code, state_code, postal_pattern, tax_type)
        DO UPDATE SET jurisdiction_name = EXCLUDED.jurisdiction_name,
                      is_active = TRUE,
                      updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(country_code)
    .bind(state_code)
    .bind(postal_pattern)
    .bind(jurisdiction_name)
    .bind(tax_type)
    .fetch_one(pool)
    .await?;

    Ok(id)
}

/// Insert a tax rule for a jurisdiction. Returns the rule UUID.
pub async fn insert_tax_rule(
    pool: &PgPool,
    jurisdiction_id: Uuid,
    app_id: &str,
    tax_code: Option<&str>,
    rate: f64,
    flat_amount_minor: i64,
    is_exempt: bool,
    effective_from: chrono::NaiveDate,
    effective_to: Option<chrono::NaiveDate>,
    priority: i32,
) -> Result<Uuid, sqlx::Error> {
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO ar_tax_rules (
            jurisdiction_id, app_id, tax_code, rate, flat_amount_minor,
            is_exempt, effective_from, effective_to, priority
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (jurisdiction_id, tax_code, effective_from)
        DO UPDATE SET rate = EXCLUDED.rate,
                      flat_amount_minor = EXCLUDED.flat_amount_minor,
                      is_exempt = EXCLUDED.is_exempt,
                      effective_to = EXCLUDED.effective_to,
                      priority = EXCLUDED.priority,
                      updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(jurisdiction_id)
    .bind(app_id)
    .bind(tax_code)
    .bind(rate)
    .bind(flat_amount_minor)
    .bind(is_exempt)
    .bind(effective_from)
    .bind(effective_to)
    .bind(priority)
    .fetch_one(pool)
    .await?;

    Ok(id)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_address() -> TaxAddress {
        TaxAddress {
            line1: "123 Main St".to_string(),
            line2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94102".to_string(),
            country: "US".to_string(),
        }
    }

    #[test]
    fn resolution_hash_is_deterministic() {
        let addr = sample_address();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 2, 17).unwrap();
        let h1 = compute_resolution_hash(&addr, Some("SW050000"), date);
        let h2 = compute_resolution_hash(&addr, Some("SW050000"), date);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn resolution_hash_changes_with_state() {
        let mut addr1 = sample_address();
        addr1.state = "CA".to_string();
        let mut addr2 = sample_address();
        addr2.state = "NY".to_string();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 2, 17).unwrap();
        let h1 = compute_resolution_hash(&addr1, Some("SW050000"), date);
        let h2 = compute_resolution_hash(&addr2, Some("SW050000"), date);
        assert_ne!(h1, h2);
    }

    #[test]
    fn resolution_hash_changes_with_tax_code() {
        let addr = sample_address();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 2, 17).unwrap();
        let h1 = compute_resolution_hash(&addr, Some("SW050000"), date);
        let h2 = compute_resolution_hash(&addr, None, date);
        assert_ne!(h1, h2);
    }
}
