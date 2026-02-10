use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{Duration, Utc};
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::models::{ErrorResponse, IdempotencyKey};

/// Extract app_id from request (placeholder - should come from auth middleware)
fn extract_app_id(_headers: &HeaderMap) -> Option<String> {
    // TODO: Extract from auth middleware
    // For now, use "test-app" to match the routes
    Some("test-app".to_string())
}

/// Check if request has already been processed via idempotency key
pub async fn check_idempotency(
    State(db): State<PgPool>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    // Extract headers from request
    let headers = request.headers();

    // Extract idempotency key from headers
    let idempotency_key = match headers.get("Idempotency-Key") {
        Some(key) => key.to_str().unwrap_or("").to_string(),
        None => {
            // No idempotency key - proceed with request normally
            return Ok(next.run(request).await);
        }
    };

    // Get app_id from auth context
    let app_id = match extract_app_id(headers) {
        Some(id) => id,
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse::new("auth_error", "Missing app_id")),
            ))
        }
    };

    // Only apply idempotency to write operations (POST, PUT, DELETE)
    let method = request.method().clone();
    if !matches!(
        method.as_str(),
        "POST" | "PUT" | "DELETE" | "PATCH"
    ) {
        // Read operations don't need idempotency
        return Ok(next.run(request).await);
    }

    // Check if this idempotency key has been processed before
    let existing = sqlx::query_as::<_, IdempotencyKey>(
        r#"
        SELECT id, app_id, idempotency_key, request_hash, response_body, status_code, created_at, expires_at
        FROM ar_idempotency_keys
        WHERE app_id = $1 AND idempotency_key = $2 AND expires_at > NOW()
        "#,
    )
    .bind(&app_id)
    .bind(&idempotency_key)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("database_error", e.to_string())),
        )
    })?;

    if let Some(cached) = existing {
        // Return cached response
        let status = StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK);
        let json_response = Json(cached.response_body);
        return Ok((status, json_response).into_response());
    }

    // Process the request
    let response = next.run(request).await;

    // Store the response for future idempotent requests
    // Note: We only cache successful responses (2xx status codes)
    let status_code = response.status().as_u16() as i32;

    if (200..300).contains(&(status_code as u16)) {
        // Buffer the response body so we can cache it
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new("internal_error", "Failed to read response")),
            )
        })?;

        // Parse as JSON to store in database
        let response_body: JsonValue = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            serde_json::json!({"raw": String::from_utf8_lossy(&bytes)})
        });

        // Hash the request
        let request_hash = format!("{:x}", Sha256::digest(idempotency_key.as_bytes()));

        // Store idempotency record (synchronously to ensure it's saved before next request)
        let expires_at = Utc::now() + Duration::hours(24);

        let _ = sqlx::query(
            r#"
            INSERT INTO ar_idempotency_keys
                (app_id, idempotency_key, request_hash, response_body, status_code, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (app_id, idempotency_key) DO NOTHING
            "#,
        )
        .bind(&app_id)
        .bind(&idempotency_key)
        .bind(&request_hash)
        .bind(&response_body)
        .bind(status_code)
        .bind(expires_at.naive_utc())
        .execute(&db)
        .await;

        // Reconstruct response with the buffered body
        let new_body = Body::from(bytes);
        let response = axum::http::Response::from_parts(parts, new_body);
        return Ok(response);
    }

    Ok(response)
}

/// Log an event to the ar_events table
pub async fn log_event(
    db: &PgPool,
    app_id: &str,
    event_type: &str,
    source: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    payload: Option<JsonValue>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO ar_events
            (app_id, event_type, source, entity_type, entity_id, payload)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(app_id)
    .bind(event_type)
    .bind(source)
    .bind(entity_type)
    .bind(entity_id)
    .bind(payload)
    .execute(db)
    .await?;

    Ok(())
}

/// Async event logging (fire-and-forget)
pub fn log_event_async(
    db: PgPool,
    app_id: String,
    event_type: String,
    source: String,
    entity_type: Option<String>,
    entity_id: Option<String>,
    payload: Option<JsonValue>,
) {
    tokio::spawn(async move {
        let _ = log_event(
            &db,
            &app_id,
            &event_type,
            &source,
            entity_type.as_deref(),
            entity_id.as_deref(),
            payload,
        )
        .await;
        // We ignore errors in async logging to not disrupt the main flow
    });
}
