//! Bulk import handler for Customer Master.
//!
//! POST /api/ar/import/customers
//!
//! Accepts CSV (Content-Type: text/csv) or JSON array
//! (Content-Type: application/json).  Validates ALL rows before writing any.
//! Idempotent by (app_id, external_customer_id): same customer_code same data
//! → skip; same code changed data → update.  Max 10 000 rows per request.
//!
//! Accepted fields:
//!   customer_code  → external_customer_id (required)
//!   name           → name (optional)
//!   payment_terms  → stored in metadata (no dedicated DB column)
//!   currency       → stored in metadata (no dedicated DB column)
//!   credit_limit   → stored in metadata (no dedicated DB column)
//!
//! Note: `email` is NOT NULL in ar_customers.  If not supplied the import
//! synthesises `{customer_code}@import.placeholder` so rows are always valid.

use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashMap;
use utoipa::ToSchema;

use super::tenant::extract_tenant;
use crate::models::ApiError;

// ============================================================================
// Public types
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct CustomerImportRow {
    pub customer_code: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub payment_terms: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub credit_limit: Option<String>,
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

pub async fn import_customers(
    axum::extract::State(db): axum::extract::State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let rows = if content_type.starts_with("application/json") {
        match serde_json::from_slice::<Vec<CustomerImportRow>>(&body) {
            Ok(r) => r,
            Err(e) => return ApiError::bad_request(format!("Invalid JSON: {}", e)).into_response(),
        }
    } else {
        match parse_customers_csv(&body) {
            Ok(r) => r,
            Err(e) => return ApiError::bad_request(e).into_response(),
        }
    };

    if rows.len() > 10_000 {
        return ApiError::new(
            413,
            "payload_too_large",
            format!("Import exceeds 10 000 row limit (got {})", rows.len()),
        )
        .into_response();
    }

    match run_customers_import(&db, &app_id, &rows).await {
        Ok(summary) if !summary.errors.is_empty() => {
            (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(summary)).into_response()
        }
        Ok(summary) => (StatusCode::OK, axum::Json(summary)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Customers import DB error");
            ApiError::internal("Import failed").into_response()
        }
    }
}

// ============================================================================
// Core import logic (pub for integration tests)
// ============================================================================

pub async fn run_customers_import(
    pool: &PgPool,
    app_id: &str,
    rows: &[CustomerImportRow],
) -> Result<ImportSummary, sqlx::Error> {
    let mut errors: Vec<ImportRowError> = Vec::new();
    let mut valid: Vec<(usize, &CustomerImportRow)> = Vec::new();

    for (idx, row) in rows.iter().enumerate() {
        let row_num = idx + 1;
        if row.customer_code.trim().is_empty() {
            errors.push(ImportRowError {
                row: row_num,
                reason: "customer_code is required".into(),
            });
            continue;
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

    let ext_ids: Vec<String> = valid
        .iter()
        .map(|(_, r)| r.customer_code.trim().to_string())
        .collect();

    let existing: HashMap<String, (Option<String>, Option<serde_json::Value>)> =
        fetch_existing_customers(pool, app_id, &ext_ids).await?;

    let mut tx = pool.begin().await?;
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;

    for (_, row) in &valid {
        let ext_id = row.customer_code.trim();
        let name = row.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let synthesised_email;
        let email: &str = match row
            .email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(e) => e,
            None => {
                synthesised_email = format!("{}@import.placeholder", ext_id);
                &synthesised_email
            }
        };

        // Build metadata from optional fields
        let mut meta = serde_json::Map::new();
        if let Some(pt) = row
            .payment_terms
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            meta.insert("payment_terms".into(), json!(pt.trim()));
        }
        if let Some(cur) = row.currency.as_deref().filter(|s| !s.trim().is_empty()) {
            meta.insert("currency".into(), json!(cur.trim()));
        }
        if let Some(cl) = row.credit_limit.as_deref().filter(|s| !s.trim().is_empty()) {
            meta.insert("credit_limit".into(), json!(cl.trim()));
        }
        let metadata = if meta.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(meta))
        };

        match existing.get(ext_id) {
            None => {
                sqlx::query(
                    r#"
                    INSERT INTO ar_customers (app_id, external_customer_id, email, name, metadata, status)
                    VALUES ($1, $2, $3, $4, $5, 'active')
                    "#,
                )
                .bind(app_id)
                .bind(ext_id)
                .bind(email)
                .bind(name)
                .bind(metadata.as_ref())
                .execute(&mut *tx)
                .await?;
                created += 1;
            }
            Some((existing_name, existing_meta)) => {
                let name_same = existing_name.as_deref().map(str::trim) == name;
                let meta_same = existing_meta.as_ref() == metadata.as_ref();
                if name_same && meta_same {
                    skipped += 1;
                } else {
                    sqlx::query(
                        r#"
                        UPDATE ar_customers
                        SET name = $1, metadata = $2, updated_at = NOW()
                        WHERE app_id = $3 AND external_customer_id = $4
                        "#,
                    )
                    .bind(name)
                    .bind(metadata.as_ref())
                    .bind(app_id)
                    .bind(ext_id)
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

async fn fetch_existing_customers(
    pool: &PgPool,
    app_id: &str,
    ext_ids: &[String],
) -> Result<HashMap<String, (Option<String>, Option<serde_json::Value>)>, sqlx::Error> {
    if ext_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, Option<String>, Option<serde_json::Value>)> = sqlx::query_as(
        r#"
        SELECT external_customer_id, name, metadata
        FROM ar_customers
        WHERE app_id = $1 AND external_customer_id = ANY($2)
        "#,
    )
    .bind(app_id)
    .bind(ext_ids)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|(id, n, m)| (id, (n, m))).collect())
}

// ============================================================================
// CSV parsing
// ============================================================================

fn parse_customers_csv(body: &[u8]) -> Result<Vec<CustomerImportRow>, String> {
    let text = std::str::from_utf8(body).map_err(|_| "CSV body is not valid UTF-8".to_string())?;
    let mut lines = text.lines();

    let header_line = lines.next().ok_or_else(|| "CSV is empty".to_string())?;
    let headers: Vec<String> = csv_fields(header_line)
        .into_iter()
        .map(|h| h.to_lowercase())
        .collect();

    let col = |name: &str| -> Option<usize> { headers.iter().position(|h| h == name) };

    let code_col = col("customer_code").ok_or("Missing CSV column: customer_code")?;
    let name_col = col("name");
    let email_col = col("email");
    let terms_col = col("payment_terms");
    let currency_col = col("currency");
    let limit_col = col("credit_limit");

    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = csv_fields(line);
        let get = |i: usize| {
            fields
                .get(i)
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        let opt = |i: Option<usize>| -> Option<String> { i.map(get) };

        rows.push(CustomerImportRow {
            customer_code: get(code_col),
            name: opt(name_col),
            email: opt(email_col),
            payment_terms: opt(terms_col),
            currency: opt(currency_col),
            credit_limit: opt(limit_col),
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
