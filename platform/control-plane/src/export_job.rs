//! Tenant offboarding export job
//!
//! Generates a deterministic ZIP bundle containing the control-plane-owned
//! tenant records needed for GDPR portability/offboarding. The same database
//! state always produces the same ZIP bytes because:
//! - rows are queried in a stable order,
//! - file timestamps are fixed,
//! - files are written in a fixed order,
//! - the archive uses stored entries instead of non-deterministic compression.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
    Json,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use std::io::{Cursor, Write};
use std::sync::Arc;
use uuid::Uuid;
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime as ZipDateTime, ZipWriter};

use crate::models::ErrorBody;
use crate::state::AppState;

#[derive(Debug, FromRow)]
struct TenantRow {
    tenant_id: Uuid,
    status: String,
    environment: String,
    module_schema_versions: serde_json::Value,
    product_code: Option<String>,
    plan_code: Option<String>,
    app_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow)]
struct RetentionRow {
    data_retention_days: i32,
    export_format: String,
    auto_tombstone_days: i32,
    export_ready_at: Option<DateTime<Utc>>,
    data_tombstoned_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct EntitlementRow {
    plan_code: String,
    concurrent_user_limit: i32,
    effective_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct ProvisioningRow {
    environment: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct BundleManifest<'a> {
    tenant_id: Uuid,
    exported_at: DateTime<Utc>,
    bundle_format: &'a str,
    files: Vec<BundleManifestFile<'a>>,
}

#[derive(Debug, Serialize)]
struct BundleManifestFile<'a> {
    name: &'a str,
    record_count: usize,
}

#[derive(Debug)]
struct ExportBundle {
    bytes: Vec<u8>,
    digest: String,
}

#[derive(Debug, thiserror::Error)]
enum ExportError {
    #[error("tenant not found")]
    TenantNotFound,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("archive error: {0}")]
    Archive(#[from] zip::result::ZipError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// POST /api/control/tenants/:tenant_id/export
///
/// Returns a deterministic ZIP bundle with tenant registry, retention, and
/// provisioning metadata. The route also advances `export_ready_at` so the
/// tombstone grace window can begin.
pub async fn export_tenant(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Response, (StatusCode, Json<ErrorBody>)> {
    let bundle = build_bundle(&state.pool, tenant_id)
        .await
        .map_err(map_error)?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"tenant-{tenant_id}-export.zip\""),
        )
        .header("x-export-sha256", bundle.digest.clone())
        .body(Body::from(bundle.bytes))
        .expect("valid export response"))
}

async fn build_bundle(pool: &sqlx::PgPool, tenant_id: Uuid) -> Result<ExportBundle, ExportError> {
    let tenant: Option<TenantRow> = sqlx::query_as(
        r#"SELECT tenant_id,
                  status,
                  environment,
                  module_schema_versions,
                  product_code,
                  plan_code,
                  app_id,
                  created_at,
                  updated_at,
                  deleted_at
           FROM tenants
           WHERE tenant_id = $1"#,
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let tenant = tenant.ok_or(ExportError::TenantNotFound)?;

    let retention: Option<RetentionRow> = sqlx::query_as(
        r#"SELECT data_retention_days,
                  export_format,
                  auto_tombstone_days,
                  export_ready_at,
                  data_tombstoned_at,
                  created_at,
                  updated_at
           FROM cp_retention_policies
           WHERE tenant_id = $1"#,
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let entitlement: Option<EntitlementRow> = sqlx::query_as(
        r#"SELECT plan_code,
                  concurrent_user_limit,
                  effective_at,
                  updated_at
           FROM cp_entitlements
           WHERE tenant_id = $1"#,
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let provisioning_requests: Vec<ProvisioningRow> = sqlx::query_as(
        r#"SELECT environment, created_at
           FROM provisioning_requests
           WHERE tenant_id = $1
           ORDER BY created_at ASC"#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    let tenant_lines = vec![serde_json::to_string(&serde_json::json!({
        "record_type": "tenant",
        "tenant_id": tenant.tenant_id,
        "status": tenant.status,
        "environment": tenant.environment,
        "module_schema_versions": tenant.module_schema_versions,
        "product_code": tenant.product_code,
        "plan_code": tenant.plan_code,
        "app_id": tenant.app_id,
        "created_at": tenant.created_at,
        "updated_at": tenant.updated_at,
        "deleted_at": tenant.deleted_at,
    }))?];

    let retention_lines = if let Some(row) = retention.as_ref() {
        vec![serde_json::to_string(&serde_json::json!({
            "record_type": "retention_policy",
            "tenant_id": tenant_id,
            "data_retention_days": row.data_retention_days,
            "export_format": row.export_format,
            "auto_tombstone_days": row.auto_tombstone_days,
            "export_ready_at": row.export_ready_at,
            "data_tombstoned_at": row.data_tombstoned_at,
            "created_at": row.created_at,
            "updated_at": row.updated_at,
        }))?]
    } else {
        Vec::new()
    };

    let entitlement_lines = if let Some(row) = entitlement.as_ref() {
        vec![serde_json::to_string(&serde_json::json!({
            "record_type": "entitlement",
            "tenant_id": tenant_id,
            "plan_code": row.plan_code,
            "concurrent_user_limit": row.concurrent_user_limit,
            "effective_at": row.effective_at,
            "updated_at": row.updated_at,
        }))?]
    } else {
        Vec::new()
    };

    let provisioning_lines: Vec<String> = provisioning_requests
        .iter()
        .enumerate()
        .map(|(seq, row)| {
            serde_json::to_string(&serde_json::json!({
                "record_type": "provisioning_request",
                "tenant_id": tenant_id,
                "seq": seq,
                "environment": row.environment,
                "created_at": row.created_at,
            }))
        })
        .collect::<Result<_, _>>()?;

    let manifest = BundleManifest {
        tenant_id,
        exported_at: Utc::now(),
        bundle_format: "zip",
        files: vec![
            BundleManifestFile {
                name: "tenant.jsonl",
                record_count: tenant_lines.len(),
            },
            BundleManifestFile {
                name: "retention_policy.jsonl",
                record_count: retention_lines.len(),
            },
            BundleManifestFile {
                name: "entitlements.jsonl",
                record_count: entitlement_lines.len(),
            },
            BundleManifestFile {
                name: "provisioning_requests.jsonl",
                record_count: provisioning_lines.len(),
            },
        ],
    };

    let zip_bytes = build_zip(
        &manifest,
        &tenant_lines,
        &retention_lines,
        &entitlement_lines,
        &provisioning_lines,
    )?;

    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(&zip_bytes);
        hex::encode(hasher.finalize())
    };

    let now = Utc::now();
    sqlx::query(
        r#"INSERT INTO cp_retention_policies (tenant_id, export_ready_at, created_at, updated_at)
           VALUES ($1, $2, $2, $2)
           ON CONFLICT (tenant_id) DO UPDATE
               SET export_ready_at = EXCLUDED.export_ready_at,
                   updated_at = EXCLUDED.updated_at"#,
    )
    .bind(tenant_id)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(ExportBundle {
        bytes: zip_bytes,
        digest,
    })
}

fn build_zip(
    manifest: &BundleManifest<'_>,
    tenant_lines: &[String],
    retention_lines: &[String],
    entitlement_lines: &[String],
    provisioning_lines: &[String],
) -> Result<Vec<u8>, ExportError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(ZipDateTime::default());

    write_jsonl_file(&mut writer, "tenant.jsonl", tenant_lines, options)?;
    write_jsonl_file(
        &mut writer,
        "retention_policy.jsonl",
        retention_lines,
        options,
    )?;
    write_jsonl_file(
        &mut writer,
        "entitlements.jsonl",
        entitlement_lines,
        options,
    )?;
    write_jsonl_file(
        &mut writer,
        "provisioning_requests.jsonl",
        provisioning_lines,
        options,
    )?;
    writer.start_file("manifest.json", options)?;
    writer.write_all(serde_json::to_string(manifest)?.as_bytes())?;

    let cursor = writer.finish()?;
    Ok(cursor.into_inner())
}

fn write_jsonl_file(
    writer: &mut ZipWriter<Cursor<Vec<u8>>>,
    path: &str,
    lines: &[String],
    options: SimpleFileOptions,
) -> Result<(), ExportError> {
    writer.start_file(path, options)?;
    for line in lines {
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

fn map_error(err: ExportError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        ExportError::TenantNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "Tenant not found".to_string(),
            }),
        ),
        ExportError::Database(e) => {
            tracing::error!("Database error while exporting tenant bundle: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Internal database error".to_string(),
                }),
            )
        }
        ExportError::Archive(e) => {
            tracing::error!("Archive error while exporting tenant bundle: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Failed to build export bundle".to_string(),
                }),
            )
        }
        ExportError::Io(e) => {
            tracing::error!("IO error while exporting tenant bundle: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Failed to build export bundle".to_string(),
                }),
            )
        }
        ExportError::Serialization(e) => {
            tracing::error!("Serialization error while exporting tenant bundle: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Failed to build export bundle".to_string(),
                }),
            )
        }
    }
}
