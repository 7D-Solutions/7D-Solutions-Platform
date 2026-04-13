//! Bulk import handler for Chart of Accounts.
//!
//! POST /api/gl/import/chart-of-accounts
//!
//! Accepts CSV (Content-Type: text/csv) or JSON array
//! (Content-Type: application/json).  Validates ALL rows before writing any —
//! a single bad row causes a 422 with the full error list.  Idempotent by
//! (tenant_id, account_code): same code same data → skip; same code changed
//! data → update.  Max 10 000 rows per request.

use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use crate::AppState;
use super::auth::with_request_id;

// ============================================================================
// Public types (used by integration tests)
// ============================================================================

/// A single row from the import payload.
#[derive(Debug, Clone, Deserialize)]
pub struct CoaRow {
    pub account_code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub account_type: String,
    /// Accepted in input but ignored — schema has no parent column.
    #[serde(default)]
    pub parent_code: Option<String>,
}

/// Per-row validation error returned in a 422 response.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImportRowError {
    /// 1-indexed row number in the original payload.
    pub row: usize,
    pub reason: String,
}

/// Response body for a successful import.
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

pub async fn import_chart_of_accounts(
    axum::extract::State(app_state): axum::extract::State<std::sync::Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let rows = if content_type.starts_with("application/json") {
        match serde_json::from_slice::<Vec<CoaRow>>(&body) {
            Ok(r) => r,
            Err(e) => {
                return with_request_id(
                    ApiError::bad_request(format!("Invalid JSON: {}", e)),
                    &ctx,
                )
                .into_response()
            }
        }
    } else {
        // Default to CSV parsing
        match parse_coa_csv(&body) {
            Ok(r) => r,
            Err(e) => {
                return with_request_id(ApiError::bad_request(e), &ctx).into_response()
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
            &ctx,
        )
        .into_response();
    }

    match run_coa_import(&app_state.pool, &tenant_id, &rows).await {
        Ok(summary) if !summary.errors.is_empty() => {
            (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(summary)).into_response()
        }
        Ok(summary) => (StatusCode::OK, axum::Json(summary)).into_response(),
        Err(e) => {
            tracing::error!("COA import DB error: {}", e);
            with_request_id(ApiError::internal("Import failed"), &ctx).into_response()
        }
    }
}

// ============================================================================
// Core import logic (pub for integration tests)
// ============================================================================

/// Validate and upsert all COA rows for the given tenant.
///
/// Validates ALL rows before writing any.  Returns the summary with populated
/// `errors` if any rows fail validation — no DB writes happen in that case.
pub async fn run_coa_import(
    pool: &PgPool,
    tenant_id: &str,
    rows: &[CoaRow],
) -> Result<ImportSummary, sqlx::Error> {
    // Phase 1 — validate all rows in memory.
    let mut errors: Vec<ImportRowError> = Vec::new();
    let mut valid_rows: Vec<(usize, &CoaRow)> = Vec::new();

    for (idx, row) in rows.iter().enumerate() {
        let row_num = idx + 1;
        if row.account_code.trim().is_empty() {
            errors.push(ImportRowError {
                row: row_num,
                reason: "account_code is required".into(),
            });
            continue;
        }
        if row.name.trim().is_empty() {
            errors.push(ImportRowError {
                row: row_num,
                reason: "name is required".into(),
            });
            continue;
        }
        let at = row.account_type.trim().to_lowercase();
        if !["asset", "liability", "equity", "revenue", "expense"].contains(&at.as_str()) {
            errors.push(ImportRowError {
                row: row_num,
                reason: format!(
                    "invalid type '{}': expected asset|liability|equity|revenue|expense",
                    row.account_type
                ),
            });
            continue;
        }
        valid_rows.push((row_num, row));
    }

    if !errors.is_empty() {
        return Ok(ImportSummary {
            created: 0,
            updated: 0,
            skipped: 0,
            errors,
        });
    }

    if valid_rows.is_empty() {
        return Ok(ImportSummary {
            created: 0,
            updated: 0,
            skipped: 0,
            errors: vec![],
        });
    }

    // Phase 2 — fetch existing accounts for this tenant matching any input code.
    let codes: Vec<String> = valid_rows
        .iter()
        .map(|(_, r)| r.account_code.trim().to_string())
        .collect();

    let existing: HashMap<String, (String, String)> =
        fetch_existing_accounts(pool, tenant_id, &codes).await?;

    // Phase 3 — categorise and write inside a single transaction.
    let mut tx = pool.begin().await?;
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;

    for (_, row) in &valid_rows {
        let code = row.account_code.trim();
        let name = row.name.trim();
        let account_type = row.account_type.trim().to_lowercase();
        let normal_balance = infer_normal_balance(&account_type);

        match existing.get(code) {
            None => {
                // Insert
                sqlx::query(
                    r#"
                    INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
                    VALUES ($1, $2, $3, $4, $5::account_type, $6::normal_balance, true, NOW())
                    "#,
                )
                .bind(Uuid::new_v4())
                .bind(tenant_id)
                .bind(code)
                .bind(name)
                .bind(&account_type)
                .bind(normal_balance)
                .execute(&mut *tx)
                .await?;
                created += 1;
            }
            Some((existing_name, existing_type)) => {
                if existing_name.as_str() == name && existing_type.as_str() == account_type {
                    skipped += 1;
                } else {
                    // Update name and/or type
                    sqlx::query(
                        r#"
                        UPDATE accounts
                        SET name = $1, type = $2::account_type, normal_balance = $3::normal_balance
                        WHERE tenant_id = $4 AND code = $5
                        "#,
                    )
                    .bind(name)
                    .bind(&account_type)
                    .bind(normal_balance)
                    .bind(tenant_id)
                    .bind(code)
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

fn infer_normal_balance(account_type: &str) -> &'static str {
    match account_type {
        "asset" | "expense" => "debit",
        _ => "credit",
    }
}

/// Fetch existing (code, name, type) for the given tenant+codes combination.
async fn fetch_existing_accounts(
    pool: &PgPool,
    tenant_id: &str,
    codes: &[String],
) -> Result<HashMap<String, (String, String)>, sqlx::Error> {
    if codes.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        r#"SELECT code, name, type::text FROM accounts WHERE tenant_id = $1 AND code = ANY($2)"#,
    )
    .bind(tenant_id)
    .bind(codes)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|(c, n, t)| (c, (n, t))).collect())
}

// ============================================================================
// CSV parsing
// ============================================================================

fn parse_coa_csv(body: &[u8]) -> Result<Vec<CoaRow>, String> {
    let text = std::str::from_utf8(body).map_err(|_| "CSV body is not valid UTF-8".to_string())?;
    let mut lines = text.lines();

    // Parse header row
    let header_line = lines
        .next()
        .ok_or_else(|| "CSV is empty".to_string())?;
    let headers: Vec<String> = parse_csv_fields(header_line)
        .into_iter()
        .map(|h| h.to_lowercase())
        .collect();

    let col = |name: &str| -> Option<usize> { headers.iter().position(|h| h == name) };

    let code_col = col("account_code").ok_or("Missing CSV column: account_code")?;
    let name_col = col("name").ok_or("Missing CSV column: name")?;
    let type_col = col("type").ok_or("Missing CSV column: type")?;
    let parent_col = col("parent_code");

    let mut rows = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_csv_fields(line);
        let get = |i: usize| fields.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
        rows.push(CoaRow {
            account_code: get(code_col),
            name: get(name_col),
            account_type: get(type_col),
            parent_code: parent_col.map(get),
        });
    }
    Ok(rows)
}

/// Minimal RFC-4180 CSV field parser.
/// Handles quoted fields (with embedded commas and escaped double-quotes).
pub fn parse_csv_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    // Escaped double-quote inside quoted field
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
