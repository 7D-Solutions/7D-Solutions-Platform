//! Bulk import handler for Item Master.
//!
//! POST /api/inventory/import/items
//!
//! Accepts CSV (Content-Type: text/csv) or JSON array
//! (Content-Type: application/json).  Validates ALL rows before writing any.
//! Idempotent by (tenant_id, sku): same SKU same data → skip; same SKU changed
//! data → update.  Max 10 000 rows per request.
//!
//! Accepted CSV/JSON fields:
//!   item_code        → sku (required)
//!   name             → name (required)
//!   unit_of_measure  → uom (optional, defaults to "ea")
//!   tracking_mode    → "none"|"lot"|"serial" (optional, defaults to "none")
//!   inventory_account_ref, cogs_account_ref, variance_account_ref (optional)
//!   reorder_point    → accepted, ignored (no DB column)

use axum::{
    body::Bytes,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use platform_sdk::TenantId;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::AppState;

// ============================================================================
// Public types
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct ItemImportRow {
    pub item_code: String,
    pub name: String,
    #[serde(default)]
    pub unit_of_measure: Option<String>,
    #[serde(default)]
    pub tracking_mode: Option<String>,
    #[serde(default)]
    pub inventory_account_ref: Option<String>,
    #[serde(default)]
    pub cogs_account_ref: Option<String>,
    #[serde(default)]
    pub variance_account_ref: Option<String>,
    /// Accepted but ignored — no DB column.
    #[serde(default)]
    pub reorder_point: Option<String>,
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

pub async fn import_items(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<AppState>>,
    TenantId(tenant_uuid): TenantId,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let tenant_id = tenant_uuid.to_string();

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let rows = if content_type.starts_with("application/json") {
        match serde_json::from_slice::<Vec<ItemImportRow>>(&body) {
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
        match parse_items_csv(&body) {
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

    match run_items_import(&state.pool, &tenant_id, &rows).await {
        Ok(summary) if !summary.errors.is_empty() => {
            (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(summary)).into_response()
        }
        Ok(summary) => (StatusCode::OK, axum::Json(summary)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Items import DB error");
            with_request_id(ApiError::internal("Import failed"), &tracing_ctx).into_response()
        }
    }
}

// ============================================================================
// Core import logic (pub for integration tests)
// ============================================================================

pub async fn run_items_import(
    pool: &PgPool,
    tenant_id: &str,
    rows: &[ItemImportRow],
) -> Result<ImportSummary, sqlx::Error> {
    let mut errors: Vec<ImportRowError> = Vec::new();
    let mut valid: Vec<(usize, &ItemImportRow)> = Vec::new();

    for (idx, row) in rows.iter().enumerate() {
        let row_num = idx + 1;
        if row.item_code.trim().is_empty() {
            errors.push(ImportRowError {
                row: row_num,
                reason: "item_code is required".into(),
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
        if let Some(ref tm) = row.tracking_mode {
            let t = tm.trim().to_lowercase();
            if !["none", "lot", "serial"].contains(&t.as_str()) {
                errors.push(ImportRowError {
                    row: row_num,
                    reason: format!("invalid tracking_mode '{}': expected none|lot|serial", tm),
                });
                continue;
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

    let skus: Vec<String> = valid
        .iter()
        .map(|(_, r)| r.item_code.trim().to_string())
        .collect();

    let existing: HashMap<String, (String, String)> =
        fetch_existing_items(pool, tenant_id, &skus).await?;

    let mut tx = pool.begin().await?;
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;

    for (_, row) in &valid {
        let sku = row.item_code.trim();
        let name = row.name.trim();
        let uom = row
            .unit_of_measure
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("ea");
        let tracking_mode = row
            .tracking_mode
            .as_deref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "none".to_string());
        let inv_ref = row
            .inventory_account_ref
            .as_deref()
            .map(str::trim)
            .unwrap_or("");
        let cogs_ref = row.cogs_account_ref.as_deref().map(str::trim).unwrap_or("");
        let var_ref = row
            .variance_account_ref
            .as_deref()
            .map(str::trim)
            .unwrap_or("");

        match existing.get(sku) {
            None => {
                sqlx::query(
                    r#"
                    INSERT INTO items (
                        id, tenant_id, sku, name,
                        inventory_account_ref, cogs_account_ref, variance_account_ref,
                        uom, tracking_mode, active, created_at, updated_at
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, true, NOW(), NOW())
                    "#,
                )
                .bind(Uuid::new_v4())
                .bind(tenant_id)
                .bind(sku)
                .bind(name)
                .bind(inv_ref)
                .bind(cogs_ref)
                .bind(var_ref)
                .bind(uom)
                .bind(&tracking_mode)
                .execute(&mut *tx)
                .await?;
                created += 1;
            }
            Some((existing_name, existing_uom)) => {
                if existing_name.as_str() == name && existing_uom.as_str() == uom {
                    skipped += 1;
                } else {
                    sqlx::query(
                        r#"
                        UPDATE items SET name = $1, uom = $2, updated_at = NOW()
                        WHERE tenant_id = $3 AND sku = $4
                        "#,
                    )
                    .bind(name)
                    .bind(uom)
                    .bind(tenant_id)
                    .bind(sku)
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

async fn fetch_existing_items(
    pool: &PgPool,
    tenant_id: &str,
    skus: &[String],
) -> Result<HashMap<String, (String, String)>, sqlx::Error> {
    if skus.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        r#"SELECT sku, name, uom FROM items WHERE tenant_id = $1 AND sku = ANY($2)"#,
    )
    .bind(tenant_id)
    .bind(skus)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|(s, n, u)| (s, (n, u))).collect())
}

// ============================================================================
// CSV parsing
// ============================================================================

fn parse_items_csv(body: &[u8]) -> Result<Vec<ItemImportRow>, String> {
    let text = std::str::from_utf8(body).map_err(|_| "CSV body is not valid UTF-8".to_string())?;
    let mut lines = text.lines();

    let header_line = lines.next().ok_or_else(|| "CSV is empty".to_string())?;
    let headers: Vec<String> = csv_fields(header_line)
        .into_iter()
        .map(|h| h.to_lowercase())
        .collect();

    let col = |name: &str| -> Option<usize> { headers.iter().position(|h| h == name) };

    let code_col = col("item_code").ok_or("Missing CSV column: item_code")?;
    let name_col = col("name").ok_or("Missing CSV column: name")?;
    let uom_col = col("unit_of_measure");
    let tracking_col = col("tracking_mode");
    let inv_col = col("inventory_account_ref");
    let cogs_col = col("cogs_account_ref");
    let var_col = col("variance_account_ref");

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

        rows.push(ItemImportRow {
            item_code: get(code_col),
            name: get(name_col),
            unit_of_measure: opt(uom_col),
            tracking_mode: opt(tracking_col),
            inventory_account_ref: opt(inv_col),
            cogs_account_ref: opt(cogs_col),
            variance_account_ref: opt(var_col),
            reorder_point: None,
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
