//! Attachment handlers — entity-agnostic file attach/download via presigned URLs.
//!
//! No file bytes flow through this service. Each handler issues a presigned S3
//! URL for the client to PUT or GET directly against the object store.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use blob_storage::{validate_mime_type, validate_size, BlobKeyBuilder};
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

use crate::handlers::AppState;
use crate::models::{Attachment, AttachmentListQuery, CreateAttachmentRequest};

// ── POST /api/attachments ─────────────────────────────────────────────
//
// Guard:    validate MIME type + declared size against ADR-018 allowlist/limit
// Mutation: insert attachment record with status='pending'
// Response: presigned PUT URL + attachment_id

pub async fn create_attachment(
    State(state): State<Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Json(req): Json<CreateAttachmentRequest>,
) -> impl IntoResponse {
    let claims = match claims {
        Some(axum::Extension(c)) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
        }
    };

    let tenant_id = claims.tenant_id;
    let actor_id = claims.user_id;

    if req.entity_type.trim().is_empty()
        || req.entity_id.trim().is_empty()
        || req.filename.trim().is_empty()
        || req.mime_type.trim().is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "entity_type, entity_id, filename, and mime_type are required"}),
            ),
        );
    }

    // MIME allowlist check
    if let Err(e) = validate_mime_type(&req.mime_type) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": e.to_string()})),
        );
    }

    // Declared-size check
    let max_bytes = state.blob.config.max_upload_bytes;
    if let Some(size) = req.size_bytes {
        if let Err(e) = validate_size(size, max_bytes) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": e.to_string()})),
            );
        }
    }

    let attachment_id = Uuid::new_v4();
    let now = Utc::now();

    let s3_key = BlobKeyBuilder {
        tenant_id: &tenant_id.to_string(),
        service: "doc-mgmt",
        artifact_type: "attachment",
        entity_id: &req.entity_id,
        object_id: &attachment_id.to_string(),
        filename: &req.filename,
    }
    .build_today();

    let mut tx = match state.db.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "begin transaction failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
    };

    if let Err(e) = sqlx::query(
        "INSERT INTO attachments (id, tenant_id, entity_type, entity_id, filename, mime_type, size_bytes, s3_key, status, created_by, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9, $10)",
    )
    .bind(attachment_id)
    .bind(tenant_id)
    .bind(&req.entity_type)
    .bind(&req.entity_id)
    .bind(&req.filename)
    .bind(&req.mime_type)
    .bind(req.size_bytes.unwrap_or(0) as i64)
    .bind(&s3_key)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    {
        tracing::error!(error = %e, "insert attachment failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    let outbox_payload = serde_json::json!({
        "tenant_id": tenant_id,
        "attachment_id": attachment_id,
        "entity_type": req.entity_type,
        "entity_id": req.entity_id,
        "filename": req.filename,
        "mime_type": req.mime_type,
        "size_bytes": req.size_bytes.unwrap_or(0),
        "uploaded_by": actor_id,
    });

    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("docmgmt.attachment.created")
            .bind("docmgmt.attachment.created")
            .bind(outbox_payload)
            .execute(&mut *tx)
            .await
    {
        tracing::error!(error = %e, "insert doc_outbox event failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "transaction commit failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    let upload_url = match state.blob.presign_put(&s3_key, &req.mime_type, None).await {
        Ok(url) => url,
        Err(e) => {
            tracing::error!(error = %e, "presign PUT failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "failed to generate upload URL"})),
            );
        }
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "attachment_id": attachment_id,
            "upload_url": upload_url,
            "s3_key": s3_key,
            "expires_in_seconds": state.blob.config.presign_ttl_seconds,
        })),
    )
}

// ── GET /api/attachments/{id} ─────────────────────────────────────────
//
// Returns a short-lived presigned GET URL for the requested attachment.

pub async fn get_attachment(
    State(state): State<Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Path(attachment_id): Path<Uuid>,
) -> impl IntoResponse {
    let claims = match claims {
        Some(axum::Extension(c)) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
        }
    };

    let tenant_id = claims.tenant_id;

    let attachment: Option<Attachment> = sqlx::query_as::<_, Attachment>(
        "SELECT id, tenant_id, entity_type, entity_id, filename, mime_type, size_bytes, s3_key,
                status, uploaded_at, deleted_at, created_by, created_at
         FROM attachments
         WHERE id = $1 AND tenant_id = $2 AND status != 'deleted'",
    )
    .bind(attachment_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let attachment = match attachment {
        Some(a) => a,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "attachment not found"})),
            )
        }
    };

    let download_url = match state.blob.presign_get(&attachment.s3_key, None).await {
        Ok(url) => url,
        Err(e) => {
            tracing::error!(error = %e, "presign GET failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "failed to generate download URL"})),
            );
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "attachment": attachment,
            "download_url": download_url,
            "expires_in_seconds": state.blob.config.presign_ttl_seconds,
        })),
    )
}

// ── GET /api/attachments?entity_type=X&entity_id=Y ───────────────────
//
// Lists all non-deleted attachments for the given entity, newest first.

pub async fn list_attachments(
    State(state): State<Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Query(params): Query<AttachmentListQuery>,
) -> impl IntoResponse {
    let claims = match claims {
        Some(axum::Extension(c)) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
        }
    };

    let tenant_id = claims.tenant_id;

    let attachments: Vec<Attachment> = sqlx::query_as::<_, Attachment>(
        "SELECT id, tenant_id, entity_type, entity_id, filename, mime_type, size_bytes, s3_key,
                status, uploaded_at, deleted_at, created_by, created_at
         FROM attachments
         WHERE tenant_id = $1 AND entity_type = $2 AND entity_id = $3 AND status != 'deleted'
         ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .bind(&params.entity_type)
    .bind(&params.entity_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::json!({"attachments": attachments})),
    )
}

// ── DELETE /api/attachments/{id} ──────────────────────────────────────
//
// Soft-deletes the attachment (marks deleted, preserves S3 key for deferred cleanup).

pub async fn delete_attachment(
    State(state): State<Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Path(attachment_id): Path<Uuid>,
) -> impl IntoResponse {
    let claims = match claims {
        Some(axum::Extension(c)) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
        }
    };

    let tenant_id = claims.tenant_id;
    let now = Utc::now();

    let result = sqlx::query(
        "UPDATE attachments SET status = 'deleted', deleted_at = $1
         WHERE id = $2 AND tenant_id = $3 AND status != 'deleted'",
    )
    .bind(now)
    .bind(attachment_id)
    .bind(tenant_id)
    .execute(&state.db)
    .await;

    match result {
        Err(e) => {
            tracing::error!(error = %e, "delete attachment failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
        Ok(r) if r.rows_affected() == 0 => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "attachment not found"})),
        ),
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"attachment_id": attachment_id, "status": "deleted"})),
        ),
    }
}
