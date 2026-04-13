//! Bulk import handler for Vendor Master.
//!
//! POST /api/ap/import/vendors
//!
//! Accepts CSV (Content-Type: text/csv) or JSON array
//! (Content-Type: application/json).  Validates ALL rows before writing any.
//! Idempotent by (tenant_id, name): same vendor name same data → skip; same
//! name changed data → update.  Max 10 000 rows per request.
//!
//! Accepted fields:
//!   vendor_code  → accepted for caller convenience; the DB key is `name`
//!   name         → required; unique key per tenant (active vendors)
//!   payment_terms → payment terms in days (integer, optional — defaults to 30)
//!   currency      → ISO-4217, 3 chars (optional — defaults to "USD")

use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use platform_sdk::extract_tenant;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::http::tenant::with_request_id;
use crate::AppState;

// ============================================================================
// Public types
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct VendorImportRow {
    /// Accepted for caller convenience; not stored — DB key is `name`.
    #[serde(default)]
    pub vendor_code: Option<String>,
    pub name: String,
    /// Payment terms in days (defaults to 30).
    #[serde(default)]
    pub payment_terms: Option<String>,
    /// ISO-4217 currency code (defaults to "USD").
    #[serde(default)]
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImportRowError {
    pub row: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImportSummary {
    pub created: usize,
    pub updated: usize,
    pub skipped: usize,
    pub errors: Vec<ImportRowError>,
}

// ============================================================================
// HTTP handler
// ============================================================================

pub async fn import_vendors(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let rows = if content_type.starts_with("application/json") {
        match serde_json::from_slice::<Vec<VendorImportRow>>(&body) {
            Ok(r) => r,
            Err(e) => {
                return with_request_id(
                    ApiError::bad_request(format!("Invalid JSON: {}", e)),
                    &tracing_ctx,
                )
                .into_response()
            }
        }
    } else {
        match parse_vendors_csv(&body) {
            Ok(r) => r,
            Err(e) => {
                return with_request_id(ApiError::bad_request(e), &tracing_ctx).into_response()
            }
        }
    };

    if rows.len() > 10_000 {
        return with_request_id(
            ApiError::new(
                413,
                "payload_too_large",
                format!("Import exceeds 10 000 row limit (got {})", rows.len()),
            ),
            &tracing_ctx,
        )
        .into_response();
    }

    match run_vendors_import(&state.pool, &tenant_id, &rows).await {
        Ok(summary) if !summary.errors.is_empty() => {
            (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(summary)).into_response()
        }
        Ok(summary) => (StatusCode::OK, axum::Json(summary)).into_response(),
        Err(e) => {
            tracing::error!("Vendors import DB error: {}", e);
            with_request_id(ApiError::internal("Import failed"), &tracing_ctx).into_response()
        }
    }
}

// ============================================================================
// Core import logic (pub for integration tests)
// ============================================================================

pub async fn run_vendors_import(
    pool: &PgPool,
    tenant_id: &str,
    rows: &[VendorImportRow],
) -> Result<ImportSummary, sqlx::Error> {
    let mut errors: Vec<ImportRowError> = Vec::new();
    let mut valid: Vec<(usize, &VendorImportRow)> = Vec::new();

    for (idx, row) in rows.iter().enumerate() {
        let row_num = idx + 1;
        if row.name.trim().is_empty() {
            errors.push(ImportRowError {
                row: row_num,
                reason: "name is required".into(),
            });
            continue;
        }
        if let Some(ref c) = row.currency {
            let c = c.trim();
            if !c.is_empty() && c.len() != 3 {
                errors.push(ImportRowError {
                    row: row_num,
                    reason: format!("currency '{}' must be a 3-character ISO-4217 code", c),
                });
                continue;
            }
        }
        if let Some(ref pt) = row.payment_terms {
            if !pt.trim().is_empty() {
                if pt.trim().parse::<i32>().is_err() {
                    errors.push(ImportRowError {
                        row: row_num,
                        reason: format!(
                            "payment_terms '{}' must be an integer (days)",
                            pt
                        ),
                    });
                    continue;
                }
            }
        }
        valid.push((row_num, row));
    }

    if !errors.is_empty() {
        return Ok(ImportSummary {
            created: 0,
            updated: 0,
            skipped: 0,
            errors,
        });
    }

    if valid.is_empty() {
        return Ok(ImportSummary {
            created: 0,
            updated: 0,
            skipped: 0,
            errors: vec![],
        });
    }

    let names: Vec<String> = valid
        .iter()
        .map(|(_, r)| r.name.trim().to_string())
        .collect();

    let existing: HashMap<String, (i32, String)> =
        fetch_existing_vendors(pool, tenant_id, &names).await?;

    let mut tx = pool.begin().await?;
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;

    for (_, row) in &valid {
        let name = row.name.trim();
        let currency = row
            .currency
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("USD");
        let payment_terms_days: i32 = row
            .payment_terms
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);

        match existing.get(name) {
            None => {
                sqlx::query(
                    r#"
                    INSERT INTO vendors (
                        vendor_id, tenant_id, name, currency, payment_terms_days, is_active,
                        created_at, updated_at
                    )
                    VALUES ($1, $2, $3, $4, $5, true, NOW(), NOW())
                    "#,
                )
                .bind(Uuid::new_v4())
                .bind(tenant_id)
                .bind(name)
                .bind(currency)
                .bind(payment_terms_days)
                .execute(&mut *tx)
                .await?;
                created += 1;
            }
            Some((existing_terms, existing_currency)) => {
                if *existing_terms == payment_terms_days
                    && existing_currency.as_str() == currency
                {
                    skipped += 1;
                } else {
                    sqlx::query(
                        r#"
                        UPDATE vendors
                        SET currency = $1, payment_terms_days = $2, updated_at = NOW()
                        WHERE tenant_id = $3 AND name = $4 AND is_active = true
                        "#,
                    )
                    .bind(currency)
                    .bind(payment_terms_days)
                    .bind(tenant_id)
                    .bind(name)
                    .execute(&mut *tx)
                    .await?;
                    updated += 1;
                }
            }
        }
    }

    tx.commit().await?;

    Ok(ImportSummary {
        created,
        updated,
        skipped,
        errors: vec![],
    })
}

// ============================================================================
// Private helpers
// ============================================================================

async fn fetch_existing_vendors(
    pool: &PgPool,
    tenant_id: &str,
    names: &[String],
) -> Result<HashMap<String, (i32, String)>, sqlx::Error> {
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, i32, String)> = sqlx::query_as(
        r#"
        SELECT name, payment_terms_days, currency
        FROM vendors
        WHERE tenant_id = $1 AND name = ANY($2) AND is_active = true
        "#,
    )
    .bind(tenant_id)
    .bind(names)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(n, t, c)| (n, (t, c)))
        .collect())
}

// ============================================================================
// CSV parsing
// ============================================================================

fn parse_vendors_csv(body: &[u8]) -> Result<Vec<VendorImportRow>, String> {
    let text =
        std::str::from_utf8(body).map_err(|_| "CSV body is not valid UTF-8".to_string())?;
    let mut lines = text.lines();

    let header_line = lines.next().ok_or_else(|| "CSV is empty".to_string())?;
    let headers: Vec<String> = csv_fields(header_line)
        .into_iter()
        .map(|h| h.to_lowercase())
        .collect();

    let col = |name: &str| -> Option<usize> { headers.iter().position(|h| h == name) };

    let name_col = col("name").ok_or("Missing CSV column: name")?;
    let code_col = col("vendor_code");
    let terms_col = col("payment_terms");
    let currency_col = col("currency");

    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = csv_fields(line);
        let get = |i: usize| fields.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
        let opt = |i: Option<usize>| -> Option<String> { i.map(get) };

        rows.push(VendorImportRow {
            vendor_code: opt(code_col),
            name: get(name_col),
            payment_terms: opt(terms_col),
            currency: opt(currency_col),
        });
    }
    Ok(rows)
}

fn csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                fields.push(field.trim().to_string());
                field = String::new();
            }
            _ => field.push(ch),
        }
    }
    fields.push(field.trim().to_string());
    fields
}
